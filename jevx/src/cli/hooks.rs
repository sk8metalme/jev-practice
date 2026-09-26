//! `jevx hooks ...`: Codex Hookの導入・shadow・Compaction補助・評価。

use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use jevx::compact_assist::run_compact_assist_with_opt_in;
use jevx::hook_config::{
    HookInstallOptions, HookScope, HookUninstallOptions, install_hooks, uninstall_hooks,
};
use jevx::hooks::{
    analyze_hook_correlations, analyze_hook_stats, append_shadow_record, compact_evaluation,
    evaluate_conversation_compaction, load_conversation_cases, load_hook_records, run_shadow,
    write_compaction_report,
};
use jevx::review::{
    FixPlan, ReviewRequest, ReviewTarget, append_review_receipt, apply_fix_plan,
    fix_operations_from_json, review_with_optional_judge,
};
use jevx::route::route_evidence_from_payload;
use jevx::{CodexUsage, Config, GatewayJudge, JevxError, discover_skill_roots};

use super::*;

pub(super) async fn run_hooks_with_config(
    command: HooksCommand,
    config: Config,
) -> Result<i32, JevxError> {
    match command {
        HooksCommand::Shadow(args) => run_hook_shadow_with_config(args, config).await,
        HooksCommand::Review(args) => run_hook_review_with_config(args, &config).await,
        HooksCommand::ReviewStats(args) => run_review_stats(args),
        HooksCommand::Install(args) => run_hook_install(args, &config),
        HooksCommand::Uninstall(args) => run_hook_uninstall(args),
        HooksCommand::CompactAssist(args) => run_compact_assist_with_config(args, &config).await,
        HooksCommand::CompactEval(args) => run_compact_eval(args),
        HooksCommand::ConversationEval(args) => run_conversation_eval(args),
        HooksCommand::Correlate(args) => run_hook_correlation(args),
        HooksCommand::Stats(args) => run_hook_stats(args),
    }
}

pub(super) fn run_review_stats(args: ReviewStatsArgs) -> Result<i32, JevxError> {
    let stats = jevx::read_review_stats(&args.input)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("review events: {}", stats.events);
        println!(
            "status: completed={} none={} degraded={} failed={} unknown={}",
            stats.completed, stats.none, stats.degraded, stats.failed, stats.unknown
        );
        println!("findings: {}", stats.findings);
        println!(
            "route: applied={} degraded={} failed={}",
            stats.route_applied, stats.route_degraded, stats.route_failed
        );
        println!(
            "latency p50/p95: {}/{} ms",
            format_optional_u64(stats.latency_ms_p50),
            format_optional_u64(stats.latency_ms_p95)
        );
        println!(
            "cost: Jev={} Codex={} total={} fallbackExtra={} averageTotal={}",
            display_cost_amount(stats.jev_cost),
            display_cost_amount(stats.codex_cost),
            display_cost_amount(stats.total_cost),
            display_cost_amount(stats.fallback_extra_cost),
            display_cost_amount(stats.average_total_cost)
        );
        println!(
            "cost status counts: {}",
            serde_json::to_string(&stats.cost_status_counts).unwrap_or_else(|_| "{}".to_owned())
        );
    }
    Ok(0)
}

pub(super) async fn run_hook_review_with_config(
    args: ReviewArgs,
    config: &Config,
) -> Result<i32, JevxError> {
    run_hook_review_from_reader(args, config, &mut io::stdin()).await
}

pub(super) async fn run_hook_review_from_reader<R: Read>(
    args: ReviewArgs,
    config: &Config,
    reader: &mut R,
) -> Result<i32, JevxError> {
    let mut input = String::new();
    reader.read_to_string(&mut input)?;
    let payload: serde_json::Value = serde_json::from_str(&input)?;
    let target = review_target(args.target);
    if args.auto_fix && !args.yes {
        return Err(JevxError::InvalidInput(
            "--auto-fix requires explicit --yes because fixes have no backup or rollback"
                .to_owned(),
        ));
    }
    let content = review_content(&payload, target, args.allow_content);
    let cwd = args
        .cwd
        .clone()
        .or_else(|| {
            payload
                .get("cwd")
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from)
        })
        .or_else(|| env::current_dir().ok());
    let file_count = payload
        .get("files")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let request = ReviewRequest::from_content(
        target,
        content.as_deref(),
        cwd.as_deref(),
        args.allow_content,
    )
    .with_identifiers(
        payload.get("taskId").and_then(serde_json::Value::as_str),
        payload
            .get("sessionId")
            .or_else(|| payload.get("session_id"))
            .and_then(serde_json::Value::as_str),
        payload
            .get("turnId")
            .or_else(|| payload.get("turn_id"))
            .and_then(serde_json::Value::as_str),
    )
    // Skill本文・設定はopt-inでも外部送信しない。flagsも「送信した内容」を表すためfalseに固定する。
    .with_metadata(file_count, false, false);
    let codex_usage = payload
        .get("codexUsage")
        .or_else(|| payload.get("codex"))
        .map(|value| serde_json::from_value::<CodexUsage>(value.clone()))
        .transpose()
        .map_err(|_| JevxError::InvalidInput("codexUsage must be a valid object".to_owned()))?;
    let route_evidence = payload
        .get("routeApplied")
        .and_then(serde_json::Value::as_object)
        .and_then(|route| {
            route_evidence_from_payload(
                route.get("model").and_then(serde_json::Value::as_str),
                route.get("reasoning").and_then(serde_json::Value::as_str),
            )
        });
    let operations = if args.auto_fix {
        fix_operations_from_json(payload.get("fixes"))?
    } else {
        Vec::new()
    };
    let plan = if args.auto_fix {
        FixPlan::requested(operations)
    } else {
        FixPlan::not_requested()
    };
    let judge = GatewayJudge::from_config(config).ok();
    let mut response = review_with_optional_judge(
        &request,
        config,
        judge
            .as_ref()
            .map(|judge| judge as &dyn jevx::DecisionJudge),
        codex_usage.as_ref(),
        route_evidence.as_ref(),
        plan.clone(),
    )
    .await?;
    if args.auto_fix {
        let workspace = cwd.unwrap_or_else(|| PathBuf::from("."));
        let application = apply_fix_plan(
            &plan,
            &workspace,
            args.yes,
            response.status,
            &response.findings,
        );
        response.fix_plan.status = application.status;
        response.fix_plan.reason = application.reason.clone();
        response.receipt.fix_status = application.status;
    }
    let output_path = args
        .output
        .unwrap_or_else(|| data_home_of(config).join("reviews.jsonl"));
    append_review_receipt(&output_path, &response.receipt)?;
    let output = serde_json::json!({
        "continue": true,
        "suppressOutput": true,
        "review": response,
    });
    println!("{}", serde_json::to_string(&output)?);
    Ok(0)
}

fn review_target(target: ReviewTargetArg) -> ReviewTarget {
    match target {
        ReviewTargetArg::Prompt => ReviewTarget::Prompt,
        ReviewTargetArg::Plan => ReviewTarget::Plan,
        ReviewTargetArg::Diff => ReviewTarget::Diff,
        ReviewTargetArg::FinalAnswer => ReviewTarget::FinalAnswer,
        ReviewTargetArg::Turn => ReviewTarget::Turn,
    }
}

pub(super) fn review_content(
    payload: &serde_json::Value,
    target: ReviewTarget,
    _allow_content: bool,
) -> Option<String> {
    let keys: &[&str] = match target {
        ReviewTarget::Prompt => &["prompt", "content"],
        ReviewTarget::Plan => &["plan", "content"],
        ReviewTarget::Diff => &["diff", "content"],
        ReviewTarget::FinalAnswer => &["finalAnswer", "content"],
        ReviewTarget::Turn => &["content", "prompt", "plan", "diff", "finalAnswer"],
    };
    let chunks = keys
        .iter()
        .filter_map(|key| payload.get(*key).and_then(review_value_text))
        .collect::<Vec<_>>();
    (!chunks.is_empty()).then(|| chunks.join("\n\n"))
}

fn review_value_text(value: &serde_json::Value) -> Option<String> {
    value.as_str().map(str::to_owned).or_else(|| {
        value
            .is_object()
            .then(|| serde_json::to_string(value).ok())
            .flatten()
    })
}

pub(super) async fn run_hook_shadow_with_config(
    args: HookShadowArgs,
    config: Config,
) -> Result<i32, JevxError> {
    run_hook_shadow_from_reader(args, config, &mut io::stdin()).await
}

pub(super) async fn run_hook_shadow_from_reader<R: Read>(
    args: HookShadowArgs,
    config: Config,
    reader: &mut R,
) -> Result<i32, JevxError> {
    let mut input = String::new();
    reader.read_to_string(&mut input)?;
    let cwd = args
        .cwd
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let skills = discover_skill_roots(&skill_roots(&cwd, &args.skill_dirs))?;
    let judge = GatewayJudge::from_config(&config).ok();
    let result = run_shadow(
        &input,
        args.event.as_deref(),
        &skills,
        &config,
        judge.as_ref().map(|judge| judge as &dyn jevx::Judge),
    )
    .await?;
    if let Some(output) = args.output {
        match append_shadow_record(&output, &result.record) {
            Ok(()) => {}
            Err(JevxError::Io(_)) => eprintln!("jevx warning: hook_record_write_failed"),
            Err(error) => return Err(error),
        }
    }
    println!("{}", serde_json::to_string(&result.response)?);
    Ok(0)
}

pub(super) fn run_compact_eval(args: CompactEvalArgs) -> Result<i32, JevxError> {
    let report = compact_evaluation(args.runs)?;
    if let Some(output) = args.output {
        write_compaction_report(&output, &report)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("jevx compaction evaluation");
        println!("Runs: {}", report.run_count);
        println!("Passed: {}", report.summary.passed);
        println!("Retention: {}", format_ratio(report.summary.retention_rate));
        println!("Secret leaks: {}", report.summary.secret_leaks);
        println!("Error rate: {}", format_ratio(report.summary.error_rate));
        if let Some(p95) = report.summary.duration_ms_p95 {
            println!("Duration p95: {p95} ms");
        }
    }
    Ok(0)
}

pub(super) fn hook_scope(scope: HookScopeArg) -> HookScope {
    match scope {
        HookScopeArg::User => HookScope::User,
        HookScopeArg::Project => HookScope::Project,
    }
}

pub(super) fn resolve_hook_paths(
    scope: HookScope,
    home: Option<PathBuf>,
    codex_home: Option<PathBuf>,
) -> Result<(PathBuf, Option<PathBuf>), JevxError> {
    if scope == HookScope::User && home.is_none() && codex_home.is_none() {
        return Err(JevxError::InvalidInput(
            "HOME or CODEX_HOME is required for user hook installation".to_owned(),
        ));
    }
    Ok((home.unwrap_or_else(|| PathBuf::from(".")), codex_home))
}

fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(super) fn run_hook_uninstall(args: HookUninstallArgs) -> Result<i32, JevxError> {
    let scope = hook_scope(args.scope);
    let (home, codex_home) = resolve_hook_paths(scope, env_path("HOME"), env_path("CODEX_HOME"))?;
    let report = uninstall_hooks(&HookUninstallOptions {
        scope,
        repo: args.repo,
        home,
        codex_home,
        dry_run: args.dry_run,
    })?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(0);
    }
    if !report.exists {
        println!("No hook config at {}", report.path.display());
        return Ok(0);
    }
    let verb = match (report.changed, report.dry_run) {
        (false, _) => "No jevx hooks in",
        (true, true) => "Would remove jevx hooks from",
        (true, false) => "Removed jevx hooks from",
    };
    println!("{verb} {}", report.path.display());
    println!("Removed handlers: {}", report.removed_handlers);
    Ok(0)
}

pub(super) fn run_hook_install(args: HookInstallArgs, config: &Config) -> Result<i32, JevxError> {
    let data_home = data_home_of(config);
    let scope = hook_scope(args.scope);
    let (home, codex_home) = resolve_hook_paths(scope, env_path("HOME"), env_path("CODEX_HOME"))?;
    let report = install_hooks(&HookInstallOptions {
        scope,
        repo: args.repo,
        home,
        codex_home,
        executable: env::current_exe()?,
        records_path: data_home.join("hooks.jsonl"),
        state_dir: data_home.join("compaction"),
        allow_review_content: args.allow_content,
        allow_compact_context: args.allow_compact_context,
        dry_run: args.dry_run,
    })?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "{} {}",
            if report.dry_run {
                "Would update"
            } else {
                "Updated"
            },
            report.path.display()
        );
        println!("Changed: {}", report.changed);
        println!("Hook records: {}", report.records_path.display());
        println!("Compaction state: {}", report.state_dir.display());
        if let Some(backup) = report.backup_path {
            println!("Backup: {}", backup.display());
        }
        println!("Review and trust the hooks with /hooks in Codex before use.");
    }
    Ok(0)
}

pub(super) async fn run_compact_assist_with_config(
    args: CompactAssistArgs,
    config: &Config,
) -> Result<i32, JevxError> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let mut reader = input.as_bytes();
    run_compact_assist_from_reader(args, config, &mut reader).await
}

pub(super) async fn run_compact_assist_from_reader<R: Read>(
    args: CompactAssistArgs,
    config: &Config,
    reader: &mut R,
) -> Result<i32, JevxError> {
    let mut input = String::new();
    reader.read_to_string(&mut input)?;
    let data_home = config
        .telemetry_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let state_dir = args
        .state_dir
        .unwrap_or_else(|| data_home.join("compaction"));
    let result =
        run_compact_assist_with_opt_in(&input, config, &state_dir, args.allow_compact_context)
            .await?;
    if let Some(warning_code) = result.warning_code.as_deref() {
        eprintln!("jevx: compact-assist continued with warning: {warning_code}");
    }
    println!("{}", serde_json::to_string(&result.response)?);
    Ok(0)
}

pub(super) fn run_conversation_eval(args: ConversationEvalArgs) -> Result<i32, JevxError> {
    let cases = load_conversation_cases(&args.input)?;
    let report = evaluate_conversation_compaction(&cases)?;
    if let Some(output) = args.output {
        write_compaction_report(&output, &report)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("jevx conversation compaction evaluation");
        println!("Cases: {}", report.run_count);
        println!("Passed: {}", report.summary.passed);
        println!("Retention: {}", format_ratio(report.summary.retention_rate));
        println!("Secret leaks: {}", report.summary.secret_leaks);
        println!(
            "Compaction completion: {}",
            format_ratio(report.summary.compaction_completion_rate)
        );
        println!(
            "Recovery completion: {}",
            format_ratio(report.summary.recovery_completion_rate)
        );
        println!(
            "Tool history: {} items ({} failures), interrupted turns: {}, recovery turns: {}",
            report.summary.tool_history_items,
            report.summary.tool_failure_count,
            report.summary.interrupted_turns,
            report.summary.recovery_turns
        );
        println!(
            "Usage: {} measured runs, {} total tokens, {} cached input tokens",
            report.summary.usage_measured_runs,
            report.summary.total_tokens,
            report.summary.total_cached_input_tokens
        );
        println!(
            "Token savings: {} measured runs, {} saved tokens, reduction {}",
            report.summary.token_savings_measured_runs,
            format_optional_u64(report.summary.total_saved_tokens),
            format_ratio(report.summary.token_reduction_rate)
        );
        if let Some(cache_rate) = report.summary.post_compaction_cache_hit_rate_p95 {
            println!("Post-compaction cache hit p95: {:.1}%", cache_rate * 100.0);
        }
        if let Some(tokens) = report.summary.post_compaction_estimated_billable_tokens_p95 {
            println!("Post-compaction estimated billable tokens p95: {tokens}");
        }
        println!(
            "Cost: Jev {}, Codex {}, total {}, fallback extra {}",
            display_cost_amount(report.summary.jev_cost),
            display_cost_amount(report.summary.codex_cost),
            display_cost_amount(report.summary.total_cost),
            display_cost_amount(report.summary.fallback_extra_cost)
        );
        println!("Error rate: {}", format_ratio(report.summary.error_rate));
        if let Some(p95) = report.summary.duration_ms_p95 {
            println!("Compaction p95: {p95} ms");
        }
    }
    Ok(0)
}

fn display_cost_amount(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.6}"))
        .unwrap_or_else(|| "unknown/unavailable".to_owned())
}

pub(super) fn run_hook_correlation(args: CorrelationArgs) -> Result<i32, JevxError> {
    let records = load_hook_records(&args.input)?;
    let report = analyze_hook_correlations(&records)?;
    if let Some(output) = args.output {
        fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("jevx hook correlation analysis");
        println!("Records: {}", report.record_count);
        println!("Duplicate groups: {}", report.duplicate_group_count);
        println!("Duplicate records: {}", report.duplicate_record_count);
    }
    Ok(0)
}

pub(super) fn run_hook_stats(args: HookStatsArgs) -> Result<i32, JevxError> {
    let records = load_hook_records(&args.input)?;
    let report = analyze_hook_stats(&records)?;
    if let Some(output) = args.output {
        fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("jevx hook statistics");
        println!("Records: {}", report.record_count);
        println!("Events: {}", serde_json::to_string(&report.event_counts)?);
        println!(
            "Latency p50/p95: {}/{} ms",
            format_optional_u64(report.latency_ms_p50),
            format_optional_u64(report.latency_ms_p95)
        );
        println!(
            "Dedupe: {} hits ({})",
            report.dedupe_hit_count,
            format_ratio(report.dedupe_rate)
        );
        println!(
            "Dedupe latency p50/p95: {}/{} ms",
            format_optional_u64(report.dedupe_latency_ms_p50),
            format_optional_u64(report.dedupe_latency_ms_p95)
        );
        println!(
            "Token savings: {} measured, before={} after={} saved={} reduction={}",
            report.token_savings.measured_records,
            format_optional_u64(report.token_savings.before_tokens),
            format_optional_u64(report.token_savings.after_tokens),
            format_optional_u64(report.token_savings.saved_tokens),
            format_ratio(report.token_savings.reduction_rate)
        );
        println!(
            "Cost: Jev {} Codex {} total {}",
            display_cost_amount(report.jev_cost),
            display_cost_amount(report.codex_cost),
            display_cost_amount(report.total_cost)
        );
        println!(
            "Cost status counts: {}",
            serde_json::to_string(&report.cost_status_counts)?
        );
    }
    Ok(0)
}
