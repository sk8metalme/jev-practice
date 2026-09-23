//! `jevx eval` / `jevx eval-repeat`: Skill選択の評価Runner。

use std::env;
use std::path::PathBuf;

use jevx::evaluation::{
    EvaluationReport, evaluate, evaluate_repeated, load_fixtures, write_case_results,
    write_repeat_report,
};
use jevx::{Config, GatewayJudge, JevxError, discover_skill_roots};

use super::*;

pub(super) async fn run_eval_with_config(
    mut args: EvalArgs,
    mut config: Config,
) -> Result<i32, JevxError> {
    let cwd = args
        .cwd
        .take()
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let fixtures = load_fixtures(&args.fixtures)?;
    let skills = discover_skill_roots(&skill_roots(&cwd, &args.skill_dirs))?;
    config.telemetry_enabled = false;

    let report = if args.dry_run {
        evaluate(&fixtures, &skills, &config, None).await
    } else {
        let judge = GatewayJudge::from_config(&config)?;
        evaluate(&fixtures, &skills, &config, Some(&judge)).await
    };
    if let Some(output) = args.output {
        write_case_results(&output, &report.cases)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_evaluation_human(&report);
    }
    Ok(0)
}

pub(super) async fn run_eval_repeat_with_config(
    mut args: EvalRepeatArgs,
    mut config: Config,
) -> Result<i32, JevxError> {
    let cwd = args
        .cwd
        .take()
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let fixtures = load_fixtures(&args.fixtures)?;
    let skills = discover_skill_roots(&skill_roots(&cwd, &args.skill_dirs))?;
    config.telemetry_enabled = false;
    let report = if args.dry_run {
        evaluate_repeated(&fixtures, &skills, &config, None, args.runs).await?
    } else {
        let judge = GatewayJudge::from_config(&config)?;
        evaluate_repeated(&fixtures, &skills, &config, Some(&judge), args.runs).await?
    };
    if let Some(output) = args.output {
        write_repeat_report(&output, &report)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_repeat_human(&report);
    }
    Ok(0)
}

pub(super) fn print_evaluation_human(report: &EvaluationReport) {
    println!("jevx evaluation");
    println!("Cases: {}", report.case_count);
    for (mode, summary) in &report.modes {
        println!(
            "{mode}: status={} cases={} accuracy={} nonePrecision={} errorRate={} fallbackRate={} cacheHitRate={} retryRate={}",
            summary.status,
            summary.cases,
            format_ratio(summary.accuracy),
            format_ratio(summary.none_precision),
            format_ratio(summary.error_rate),
            format_ratio(summary.fallback_rate),
            format_ratio(summary.cache_hit_rate),
            format_ratio(summary.retry_rate),
        );
        if let Some(cost) = summary.average_relative_cost {
            println!("  Relative cost average: {cost:.2}");
        }
        if let Some(response_ms) = summary.jev_response_ms_p50 {
            println!(
                "  Jev response: p50={response_ms} ms p95={} ms",
                summary.jev_response_ms_p95.unwrap_or(response_ms)
            );
        }
        if let Some(total_ms) = summary.total_ms_p50 {
            println!(
                "  Total: p50={total_ms} ms p95={} ms",
                summary.total_ms_p95.unwrap_or(total_ms)
            );
        }
        if let Some(discovery_ms) = summary.discovery_ms_p50 {
            println!(
                "  Local discovery: p50={discovery_ms} ms p95={} ms",
                summary.discovery_ms_p95.unwrap_or(discovery_ms)
            );
        }
    }
}

pub(super) fn print_repeat_human(report: &jevx::evaluation::RepeatEvaluationReport) {
    println!("jevx repeated evaluation");
    println!("Runs: {}", report.run_count);
    println!("Cases per run: {}", report.case_count);
    for (mode, summary) in &report.modes {
        println!(
            "{mode}: runs={} status={} accuracyMean={} errorRateMean={} fallbackRateMean={} retryRateMean={}",
            summary.runs,
            summary.status,
            format_ratio(summary.accuracy.mean),
            format_ratio(summary.error_rate.mean),
            format_ratio(summary.fallback_rate.mean),
            format_ratio(summary.retry_rate.mean),
        );
        if let Some(cost) = summary.relative_cost.mean {
            println!("  Relative cost mean: {cost:.2}");
        }
        if let Some(p95) = summary.discovery_ms.p95 {
            println!("  Local discovery p95: {p95:.0} ms");
        }
        if let Some(p95) = summary.jev_response_ms.p95 {
            println!("  Jev response p95: {p95:.0} ms");
        }
        if let Some(p95) = summary.total_ms.p95 {
            println!("  Total p95: {p95:.0} ms");
        }
    }
}
