//! `jevx hooks ...`: Codex Hookの導入・shadow・Compaction補助・評価。

use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use jevx::compact_assist::run_compact_assist;
use jevx::hook_config::{
    HookInstallOptions, HookScope, HookUninstallOptions, install_hooks, uninstall_hooks,
};
use jevx::hooks::{
    analyze_hook_correlations, append_shadow_record, compact_evaluation,
    evaluate_conversation_compaction, load_conversation_cases, load_hook_records, run_shadow,
    write_compaction_report,
};
use jevx::{Config, GatewayJudge, JevxError, discover_skill_roots};

use super::*;

pub(super) async fn run_hooks_with_config(
    command: HooksCommand,
    config: Config,
) -> Result<i32, JevxError> {
    match command {
        HooksCommand::Shadow(args) => run_hook_shadow_with_config(args, config).await,
        HooksCommand::Install(args) => run_hook_install(args, &config),
        HooksCommand::Uninstall(args) => run_hook_uninstall(args),
        HooksCommand::CompactAssist(args) => run_compact_assist_with_config(args, &config).await,
        HooksCommand::CompactEval(args) => run_compact_eval(args),
        HooksCommand::ConversationEval(args) => run_conversation_eval(args),
        HooksCommand::Correlate(args) => run_hook_correlation(args),
    }
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
        append_shadow_record(&output, &result.record)?;
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
    if let Some(backup) = report.backup_path {
        println!("Backup: {}", backup.display());
    }
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
    let result = run_compact_assist(&input, config, &state_dir).await?;
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
        if let Some(cache_rate) = report.summary.post_compaction_cache_hit_rate_p95 {
            println!("Post-compaction cache hit p95: {:.1}%", cache_rate * 100.0);
        }
        if let Some(tokens) = report.summary.post_compaction_estimated_billable_tokens_p95 {
            println!("Post-compaction estimated billable tokens p95: {tokens}");
        }
        println!("Error rate: {}", format_ratio(report.summary.error_rate));
        if let Some(p95) = report.summary.duration_ms_p95 {
            println!("Compaction p95: {p95} ms");
        }
    }
    Ok(0)
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
