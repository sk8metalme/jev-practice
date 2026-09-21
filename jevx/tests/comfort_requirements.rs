use std::fs;
use std::path::Path;

use jevx::compact_assist::run_compact_assist;
use jevx::hook_config::{HookInstallOptions, HookScope, install_hooks};
use jevx::{CandidateDecision, Config, Metrics, TelemetryEvent, append_telemetry, read_stats};
use serde_json::json;
use tempfile::tempdir;

fn install_options(root: &Path, dry_run: bool) -> HookInstallOptions {
    HookInstallOptions {
        scope: HookScope::Project,
        repo: root.to_path_buf(),
        home: root.join("home"),
        codex_home: None,
        executable: root.join("bin/jevx with space"),
        records_path: root.join("state/hook-records.jsonl"),
        state_dir: root.join("state/compaction"),
        dry_run,
    }
}

#[test]
fn hook_install_preserves_existing_handlers_and_is_idempotent() {
    let root = tempdir().expect("tempdir");
    let config_path = root.path().join(".codex/hooks.json");
    fs::create_dir_all(config_path.parent().expect("parent")).expect("mkdir");
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&json!({
            "description": "keep me",
            "hooks": {
                "SessionStart": [{
                    "matcher": "startup",
                    "hooks": [{
                        "type": "command",
                        "command": "custom-session-start"
                    }]
                }],
                "UserPromptSubmit": [{
                    "hooks": [{
                        "type": "command",
                        "command": "/old/jevx hooks shadow --output /old/events.jsonl"
                    }]
                }]
            }
        }))
        .expect("json"),
    )
    .expect("write config");

    let first = install_hooks(&install_options(root.path(), false)).expect("install");
    assert!(first.changed);
    assert!(first.backup_path.is_some());
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("config")).expect("json");
    assert_eq!(written["description"], "keep me");
    assert_eq!(
        written["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "custom-session-start"
    );
    let user_prompt = written["hooks"]["UserPromptSubmit"]
        .as_array()
        .expect("user prompt groups");
    assert_eq!(user_prompt.len(), 1);
    let command = user_prompt[0]["hooks"][0]["command"]
        .as_str()
        .expect("command");
    assert!(command.contains("hooks shadow"));
    assert!(command.contains("jevx with space"));

    let second = install_hooks(&install_options(root.path(), false)).expect("reinstall");
    assert!(!second.changed);
    assert!(second.backup_path.is_none());
}

#[test]
fn hook_install_dry_run_does_not_write_and_rejects_invalid_shapes() {
    let root = tempdir().expect("tempdir");
    let options = install_options(root.path(), true);
    let report = install_hooks(&options).expect("dry run");
    assert!(report.changed);
    assert!(!report.path.exists());

    let config_path = root.path().join(".codex/hooks.json");
    fs::create_dir_all(config_path.parent().expect("parent")).expect("mkdir");
    fs::write(&config_path, "[]").expect("invalid root");
    let error = install_hooks(&install_options(root.path(), false)).expect_err("shape error");
    assert!(error.to_string().contains("object"));
}

#[tokio::test]
async fn compact_assist_records_safe_state_and_restores_redacted_manifest() {
    let root = tempdir().expect("tempdir");
    let repo = root.path().join("repo");
    fs::create_dir_all(repo.join(".jevx")).expect("mkdir");
    fs::write(
        repo.join(".jevx/compact-context.md"),
        "goal: preserve the release checklist\nsecret=fixture-only\nnext: run tests",
    )
    .expect("context");
    let state_dir = root.path().join("state");
    let config = Config::for_test(root.path().join("data"));
    let pre = run_compact_assist(
        &json!({
            "hook_event_name": "PreCompact",
            "trigger": "manual",
            "session_id": "session-fixture",
            "turn_id": "turn-before",
            "cwd": repo
        })
        .to_string(),
        &config,
        &state_dir,
    )
    .await
    .expect("pre compact");
    assert_eq!(pre.response["continue"], true);
    assert!(state_dir.join("checkpoints.jsonl").exists());
    let checkpoint = fs::read_to_string(state_dir.join("checkpoints.jsonl")).expect("checkpoint");
    assert!(!checkpoint.contains("release checklist"));
    assert!(!checkpoint.contains("fixture-only"));

    let post = run_compact_assist(
        &json!({
            "hook_event_name": "PostCompact",
            "trigger": "manual",
            "session_id": "session-fixture",
            "turn_id": "turn-after",
            "cwd": repo
        })
        .to_string(),
        &config,
        &state_dir,
    )
    .await
    .expect("post compact");
    assert_eq!(post.response["continue"], true);

    let resume = run_compact_assist(
        &json!({
            "hook_event_name": "SessionStart",
            "source": "compact",
            "session_id": "session-fixture",
            "cwd": repo
        })
        .to_string(),
        &config,
        &state_dir,
    )
    .await
    .expect("compact resume");
    let context = resume.response["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additional context");
    assert!(context.contains("release checklist"));
    assert!(context.contains("<redacted>"));
    assert!(!context.contains("fixture-only"));
}

#[test]
fn operational_stats_report_rates_percentiles_and_usage() {
    let root = tempdir().expect("tempdir");
    let path = root.path().join("events.jsonl");
    for (prompt, decision, response, total, input, output) in [
        (
            "one",
            CandidateDecision::Selected,
            10,
            14,
            Some(100),
            Some(20),
        ),
        ("two", CandidateDecision::None, 20, 24, Some(120), Some(25)),
        ("three", CandidateDecision::Error, 0, 2, None, None),
    ] {
        append_telemetry(
            &path,
            &TelemetryEvent::from_result(
                prompt,
                &decision,
                None,
                &Metrics {
                    jev_response_ms: response,
                    total_ms: total,
                    input_tokens: input,
                    output_tokens: output,
                    ..Metrics::default()
                },
            ),
        )
        .expect("append");
    }

    let stats = read_stats(&path).expect("stats");
    assert_eq!(stats.events, 3);
    assert_eq!(stats.selected_rate, Some(1.0 / 3.0));
    assert_eq!(stats.none_rate, Some(1.0 / 3.0));
    assert_eq!(stats.error_rate, Some(1.0 / 3.0));
    assert_eq!(stats.jev_response_ms_p50, Some(10));
    assert_eq!(stats.jev_response_ms_p95, Some(20));
    assert_eq!(stats.total_ms_p50, Some(14));
    assert_eq!(stats.average_input_tokens, Some(110.0));
    assert_eq!(stats.average_output_tokens, Some(22.5));
    assert_eq!(stats.usage_events, 2);
}
