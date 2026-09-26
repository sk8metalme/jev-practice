use std::fs;
use std::io::Write;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use jevx::compact_assist::run_compact_assist;
use jevx::hooks::{TokenUsageSnapshot, analyze_hook_stats, run_shadow};
use jevx::{Config, Judge, JudgeRequest, JudgeResponse, SkillRecord};
use tempfile::tempdir;
use tokio::sync::Notify;

struct CountingJudge {
    calls: Arc<AtomicUsize>,
}

struct FailingJudge {
    calls: Arc<AtomicUsize>,
}

struct BlockingJudge {
    calls: Arc<AtomicUsize>,
    started: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl Judge for FailingJudge {
    async fn evaluate(&self, _request: JudgeRequest) -> Result<JudgeResponse, jevx::JevxError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(jevx::JevxError::Provider("fixture failure".to_owned()))
    }
}

#[async_trait]
impl Judge for CountingJudge {
    async fn evaluate(&self, _request: JudgeRequest) -> Result<JudgeResponse, jevx::JevxError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(JudgeResponse::selected("pdf", 0.95, 12, Some((31, 7))))
    }
}

#[async_trait]
impl Judge for BlockingJudge {
    async fn evaluate(&self, _request: JudgeRequest) -> Result<JudgeResponse, jevx::JevxError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        self.release.notified().await;
        Ok(JudgeResponse::selected("pdf", 0.95, 12, Some((31, 7))))
    }
}

fn skill(root: &std::path::Path) -> SkillRecord {
    SkillRecord::new(
        "pdf".to_owned(),
        "PDFを処理する".to_owned(),
        root.join("pdf/SKILL.md"),
        "fixture".to_owned(),
    )
}

#[tokio::test]
async fn identical_user_prompt_hooks_reuse_a_successful_decision() {
    let root = tempdir().expect("tempdir");
    let mut config = Config::for_test(root.path().join("data"));
    config.timeout = Duration::from_millis(100);
    let calls = Arc::new(AtomicUsize::new(0));
    let judge = CountingJudge {
        calls: Arc::clone(&calls),
    };
    let input = r#"{"hook_event_name":"UserPromptSubmit","prompt":"PDFを処理したい","cwd":"/tmp/project","session_id":"session-1","turn_id":"turn-1","model":"model-1"}"#;

    let first = run_shadow(input, None, &[skill(root.path())], &config, Some(&judge))
        .await
        .expect("first hook");
    let second = run_shadow(input, None, &[skill(root.path())], &config, Some(&judge))
        .await
        .expect("second hook");

    assert!(!first.record.dedupe_hit);
    assert!(second.record.dedupe_hit);
    assert_eq!(second.record.selected_skill.as_deref(), Some("pdf"));
    assert!(second.record.discovery_ms.is_none());
    assert!(second.record.jev_response_ms.is_none());
    assert!(second.record.total_ms.is_none());
    assert!(second.record.input_tokens.is_none());
    assert!(second.record.output_tokens.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn zero_timeout_owner_releases_without_calling_jev() {
    let root = tempdir().expect("tempdir");
    let mut config = Config::for_test(root.path().join("data"));
    config.timeout = Duration::ZERO;
    let calls = Arc::new(AtomicUsize::new(0));
    let judge = CountingJudge {
        calls: Arc::clone(&calls),
    };
    let result = run_shadow(
        r#"{"hook_event_name":"UserPromptSubmit","prompt":"PDFを処理したい","cwd":"/tmp/project","session_id":"session-zero","turn_id":"turn-zero"}"#,
        None,
        &[skill(root.path())],
        &config,
        Some(&judge),
    )
    .await
    .expect("zero timeout hook");

    assert_eq!(result.record.dedupe_error.as_deref(), Some("wait_timeout"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn changed_skill_set_or_decision_config_does_not_reuse_a_cached_decision() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().join("data"));
    let calls = Arc::new(AtomicUsize::new(0));
    let judge = CountingJudge {
        calls: Arc::clone(&calls),
    };
    let input = r#"{"hook_event_name":"UserPromptSubmit","prompt":"PDFを処理したい","cwd":"/tmp/project","session_id":"session-state","turn_id":"turn-state","model":"model-1"}"#;
    let skills = [skill(root.path())];

    run_shadow(input, None, &skills, &config, Some(&judge))
        .await
        .expect("initial hook");
    let changed_skill = SkillRecord::new(
        "pdf".to_owned(),
        "説明が変わった".to_owned(),
        root.path().join("pdf/SKILL.md"),
        "fixture".to_owned(),
    );
    let changed_skill_result = run_shadow(input, None, &[changed_skill], &config, Some(&judge))
        .await
        .expect("changed skill hook");
    assert!(!changed_skill_result.record.dedupe_hit);

    let mut changed_config = config.clone();
    changed_config.min_margin = 0.2;
    let changed_config_result = run_shadow(input, None, &skills, &changed_config, Some(&judge))
        .await
        .expect("changed config hook");
    assert!(!changed_config_result.record.dedupe_hit);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn dedupe_storage_errors_are_recorded_without_blocking_the_hook() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().join("data"));
    fs::create_dir_all(config.telemetry_path.parent().expect("data parent"))
        .expect("data directory");
    fs::write(
        config
            .telemetry_path
            .parent()
            .expect("data parent")
            .join("hook-dedupe.json"),
        "not-json\n",
    )
    .expect("corrupt dedupe state");
    let calls = Arc::new(AtomicUsize::new(0));
    let judge = CountingJudge {
        calls: Arc::clone(&calls),
    };
    let result = run_shadow(
        r#"{"hook_event_name":"UserPromptSubmit","prompt":"PDFを処理したい","cwd":"/tmp/project","session_id":"session-dedupe-error","turn_id":"turn-dedupe-error"}"#,
        None,
        &[skill(root.path())],
        &config,
        Some(&judge),
    )
    .await
    .expect("hook continues after dedupe error");

    assert_eq!(result.record.error_code, None);
    assert_eq!(result.record.dedupe_error.as_deref(), Some("claim_error"));
    assert!(!result.record.dedupe_hit);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let failing_calls = Arc::new(AtomicUsize::new(0));
    let failing = FailingJudge {
        calls: Arc::clone(&failing_calls),
    };
    let failed = run_shadow(
        r#"{"hook_event_name":"UserPromptSubmit","prompt":"失敗させる","cwd":"/tmp/project","session_id":"session-dedupe-provider-error","turn_id":"turn-dedupe-provider-error"}"#,
        None,
        &[skill(root.path())],
        &config,
        Some(&failing),
    )
    .await
    .expect("provider error remains visible");
    assert_eq!(failed.record.error_code.as_deref(), Some("provider_error"));
    assert_eq!(failed.record.dedupe_error.as_deref(), Some("claim_error"));
    assert_eq!(failing_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn concurrent_identical_user_prompt_hooks_call_jev_once() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().join("data"));
    let calls = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let judge = Arc::new(BlockingJudge {
        calls: Arc::clone(&calls),
        started: Arc::clone(&started),
        release: Arc::clone(&release),
    });
    let input = r#"{"hook_event_name":"UserPromptSubmit","prompt":"PDFを処理したい","cwd":"/tmp/project","session_id":"session-concurrent","turn_id":"turn-concurrent","model":"model-1"}"#.to_owned();

    let first_config = config.clone();
    let first_input = input.clone();
    let first_skills = vec![skill(root.path())];
    let first_judge = Arc::clone(&judge);
    let first = tokio::spawn(async move {
        run_shadow(
            &first_input,
            None,
            &first_skills,
            &first_config,
            Some(first_judge.as_ref()),
        )
        .await
    });
    started.notified().await;

    let second_config = config.clone();
    let second_input = input;
    let second_skills = vec![skill(root.path())];
    let second_judge = Arc::clone(&judge);
    let second = tokio::spawn(async move {
        run_shadow(
            &second_input,
            None,
            &second_skills,
            &second_config,
            Some(second_judge.as_ref()),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    release.notify_waiters();

    let first = first.await.expect("first task").expect("first hook");
    let second = second.await.expect("second task").expect("second hook");
    assert!(!first.record.dedupe_hit);
    assert!(second.record.dedupe_hit);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn concurrent_dedupe_wait_uses_the_existing_timeout_budget() {
    let root = tempdir().expect("tempdir");
    let mut config = Config::for_test(root.path().join("data"));
    config.timeout = Duration::ZERO;
    let calls = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let judge = Arc::new(BlockingJudge {
        calls: Arc::clone(&calls),
        started: Arc::clone(&started),
        release: Arc::clone(&release),
    });
    let input = r#"{"hook_event_name":"UserPromptSubmit","prompt":"PDFを処理したい","cwd":"/tmp/project","session_id":"session-timeout","turn_id":"turn-timeout","codexUsage":{"inputTokens":10,"outputTokens":2,"cost":{"amount":0.25,"currency":"USD","priceVersion":"codex-fixture","status":"available","basis":"actual"}}}"#;
    let first_judge = Arc::clone(&judge);
    let mut first_config = config.clone();
    first_config.timeout = Duration::from_secs(1);
    let first_skills = vec![skill(root.path())];
    let first = tokio::spawn(async move {
        run_shadow(
            input,
            None,
            &first_skills,
            &first_config,
            Some(first_judge.as_ref()),
        )
        .await
    });
    started.notified().await;

    let second = run_shadow(
        input,
        None,
        &[skill(root.path())],
        &config,
        Some(judge.as_ref()),
    )
    .await
    .expect("timeout hook");
    assert_eq!(second.record.dedupe_error.as_deref(), Some("wait_timeout"));
    assert_eq!(second.record.cost.codex.amount, Some(0.25));
    assert_eq!(second.record.cost.total.amount, Some(0.25));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    release.notify_one();
    tokio::task::yield_now().await;
    release.notify_waiters();
    tokio::time::timeout(Duration::from_secs(2), first)
        .await
        .expect("first hook must release within timeout")
        .expect("first task")
        .expect("first hook");
}

#[tokio::test]
async fn whitespace_prompt_releases_dedupe_claim() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().join("data"));
    let calls = Arc::new(AtomicUsize::new(0));
    let judge = CountingJudge {
        calls: Arc::clone(&calls),
    };
    let empty = r#"{"hook_event_name":"UserPromptSubmit","prompt":"   ","cwd":"/tmp/project","session_id":"session-empty","turn_id":"turn-empty"}"#;
    let first = run_shadow(empty, None, &[skill(root.path())], &config, Some(&judge))
        .await
        .expect("empty prompt hook");
    assert_eq!(first.record.error_code.as_deref(), Some("invalid_input"));

    let second = run_shadow(empty, None, &[skill(root.path())], &config, Some(&judge))
        .await
        .expect("retry after empty prompt");
    assert_eq!(second.record.error_code.as_deref(), Some("invalid_input"));
    assert!(!second.record.dedupe_hit);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn early_user_prompt_returns_preserve_provider_cost() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().join("data"));
    let empty = r#"{"hook_event_name":"UserPromptSubmit","prompt":"   ","cwd":"/tmp/project","session_id":"session-empty-cost","turn_id":"turn-empty-cost","codexUsage":{"inputTokens":10,"outputTokens":2,"cost":{"amount":0.25,"currency":"USD","priceVersion":"codex-fixture","status":"available","basis":"actual"}}}"#;
    let empty_result = run_shadow(empty, None, &[], &config, None)
        .await
        .expect("empty prompt hook");
    assert_eq!(
        empty_result.record.error_code.as_deref(),
        Some("invalid_input")
    );
    assert_eq!(empty_result.record.cost.codex.amount, Some(0.25));
    assert_eq!(empty_result.record.cost.total.amount, Some(0.25));

    let missing_judge = r#"{"hook_event_name":"UserPromptSubmit","prompt":"valid prompt","cwd":"/tmp/project","session_id":"session-missing-cost","turn_id":"turn-missing-cost","codexUsage":{"inputTokens":10,"outputTokens":2,"cost":{"amount":0.5,"currency":"USD","priceVersion":"codex-fixture","status":"available","basis":"actual"}}}"#;
    let missing_result = run_shadow(missing_judge, None, &[], &config, None)
        .await
        .expect("missing judge hook");
    assert_eq!(
        missing_result.record.error_code.as_deref(),
        Some("missing_api_key")
    );
    assert_eq!(missing_result.record.cost.codex.amount, Some(0.5));
    assert_eq!(missing_result.record.cost.total.amount, Some(0.5));
}

#[tokio::test]
async fn missing_identity_and_failed_decisions_are_not_deduplicated() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().join("data"));
    let calls = Arc::new(AtomicUsize::new(0));
    let judge = CountingJudge {
        calls: Arc::clone(&calls),
    };
    let no_identity = r#"{"hook_event_name":"UserPromptSubmit","prompt":"PDFを処理したい"}"#;
    let first = run_shadow(
        no_identity,
        None,
        &[skill(root.path())],
        &config,
        Some(&judge),
    )
    .await
    .expect("first hook");
    let second = run_shadow(
        no_identity,
        None,
        &[skill(root.path())],
        &config,
        Some(&judge),
    )
    .await
    .expect("second hook");
    assert!(!first.record.dedupe_hit);
    assert!(!second.record.dedupe_hit);
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let failure_calls = Arc::new(AtomicUsize::new(0));
    let failing = FailingJudge {
        calls: Arc::clone(&failure_calls),
    };
    let input = r#"{"hook_event_name":"UserPromptSubmit","prompt":"失敗させる","cwd":"/tmp/project","session_id":"session-2","turn_id":"turn-2"}"#;
    let first = run_shadow(input, None, &[skill(root.path())], &config, Some(&failing))
        .await
        .expect("failed hook");
    let second = run_shadow(input, None, &[skill(root.path())], &config, Some(&failing))
        .await
        .expect("retried failed hook");
    assert_eq!(first.record.error_code.as_deref(), Some("provider_error"));
    assert_eq!(second.record.error_code.as_deref(), Some("provider_error"));
    assert!(!second.record.dedupe_hit);
    assert_eq!(failure_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn token_savings_are_derived_only_when_before_and_after_totals_exist() {
    let before = TokenUsageSnapshot {
        total_tokens: 1_000,
        input_tokens: 900,
        ..TokenUsageSnapshot::default()
    };
    let after = TokenUsageSnapshot {
        total_tokens: 600,
        input_tokens: 500,
        ..TokenUsageSnapshot::default()
    };
    let savings =
        jevx::hooks::TokenSavings::from_snapshots(Some(&before), Some(&after), "hook_payload");
    assert_eq!(savings.before_tokens, Some(1_000));
    assert_eq!(savings.after_tokens, Some(600));
    assert_eq!(savings.saved_tokens, Some(400));
    assert_eq!(savings.reduction_rate, Some(0.4));
    assert_eq!(savings.status, jevx::hooks::MeasurementStatus::Measured);

    let unavailable =
        jevx::hooks::TokenSavings::from_snapshots(Some(&before), None, "hook_payload");
    assert_eq!(unavailable.saved_tokens, None);
    assert_eq!(
        unavailable.status,
        jevx::hooks::MeasurementStatus::Unavailable
    );
}

#[tokio::test]
async fn hook_stats_reports_latency_dedupe_and_compaction_savings() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().join("data"));
    let pre = run_shadow(
        r#"{"hook_event_name":"PreCompact","session_id":"session-1","turn_id":"turn-1","preCompactionUsage":{"inputTokens":900,"totalTokens":1000}}"#,
        None,
        &[],
        &config,
        None,
    )
    .await
    .expect("pre compact");
    let post = run_shadow(
        r#"{"hook_event_name":"PostCompact","session_id":"session-1","turn_id":"turn-1","preCompactionUsage":{"inputTokens":900,"totalTokens":1000},"postCompactionUsage":{"inputTokens":500,"totalTokens":600},"postCompactionCost":{"amount":0.012,"currency":"USD","priceVersion":"provider-usage","status":"available","basis":"actual"}}"#,
        None,
        &[],
        &config,
        None,
    )
    .await
    .expect("post compact");

    let report = analyze_hook_stats(&[pre.record, post.record.clone()]).expect("hook stats");
    assert_eq!(report.record_count, 2);
    assert_eq!(report.event_counts["PreCompact"], 1);
    assert_eq!(report.event_counts["PostCompact"], 1);
    assert_eq!(report.token_savings.saved_tokens, None);
    assert_eq!(report.token_savings.measured_records, 0);
    assert_eq!(report.dedupe_rate, None);
    assert_eq!(report.jev_cost, Some(0.0));
    assert_eq!(report.codex_cost, None);
    assert_eq!(report.total_cost, None);

    let priced_report = analyze_hook_stats(&[post.record]).expect("priced hook stats");
    assert_eq!(priced_report.jev_cost, Some(0.0));
    assert_eq!(priced_report.codex_cost, Some(0.012));
    assert_eq!(priced_report.total_cost, Some(0.012));
}

#[tokio::test]
async fn compact_assist_carries_pre_usage_into_post_checkpoint() {
    let root = tempdir().expect("tempdir");
    let state_dir = root.path().join("compaction");
    let config = Config::for_test(root.path().join("data"));
    run_compact_assist(
        r#"{"hook_event_name":"PreCompact","session_id":"session-1","turn_id":"turn-1","preCompactionUsage":{"inputTokens":900,"totalTokens":1000}}"#,
        &config,
        &state_dir,
    )
    .await
    .expect("pre compact assist");
    let post = run_compact_assist(
        r#"{"event":"PostCompact","session_id":"session-1","turn_id":"turn-1","postCompactionUsage":{"inputTokens":500,"totalTokens":600}}"#,
        &config,
        &state_dir,
    )
    .await
    .expect("post compact assist");

    assert_eq!(post.checkpoint.schema_version, 5);
    assert_eq!(
        post.checkpoint
            .token_savings
            .as_ref()
            .and_then(|savings| savings.saved_tokens),
        Some(400)
    );
    let records = fs::read_to_string(state_dir.join("hook-records.jsonl")).expect("records");
    assert!(records.contains("\"savedTokens\":400"));

    let alias_resume = run_compact_assist(
        r#"{"event":"SessionStart","source":"compact","session_id":"session-1"}"#,
        &config,
        &state_dir,
    )
    .await
    .expect("event alias compact resume");
    assert!(
        alias_resume.response["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .is_some()
    );

    let second_post = run_compact_assist(
        r#"{"hook_event_name":"PostCompact","session_id":"session-1","turn_id":"turn-1","postCompactionUsage":{"inputTokens":400,"totalTokens":500}}"#,
        &config,
        &state_dir,
    )
    .await
    .expect("second post compact assist");
    assert!(second_post.checkpoint.token_savings.is_none());

    let mismatch_state_dir = root.path().join("compaction-mismatch");
    run_compact_assist(
        r#"{"hook_event_name":"PreCompact","session_id":"session-mismatch","turn_id":"turn-before","preCompactionUsage":{"inputTokens":900,"totalTokens":1000}}"#,
        &config,
        &mismatch_state_dir,
    )
    .await
    .expect("mismatch pre compact assist");
    let mismatch_post = run_compact_assist(
        r#"{"hook_event_name":"PostCompact","session_id":"session-mismatch","turn_id":"turn-after","postCompactionUsage":{"inputTokens":500,"totalTokens":600}}"#,
        &config,
        &mismatch_state_dir,
    )
    .await
    .expect("mismatch post compact assist");
    assert!(mismatch_post.checkpoint.token_savings.is_none());

    let concurrent_state_dir = root.path().join("compaction-concurrent");
    run_compact_assist(
        r#"{"hook_event_name":"PreCompact","session_id":"session-concurrent-compact","turn_id":"turn-concurrent-compact","preCompactionUsage":{"inputTokens":900,"totalTokens":1000}}"#,
        &config,
        &concurrent_state_dir,
    )
    .await
    .expect("concurrent pre compact assist");
    let (left, right) = tokio::join!(
        run_compact_assist(
            r#"{"hook_event_name":"PostCompact","session_id":"session-concurrent-compact","turn_id":"turn-concurrent-compact","postCompactionUsage":{"inputTokens":500,"totalTokens":600}}"#,
            &config,
            &concurrent_state_dir,
        ),
        run_compact_assist(
            r#"{"hook_event_name":"PostCompact","session_id":"session-concurrent-compact","turn_id":"turn-concurrent-compact","postCompactionUsage":{"inputTokens":500,"totalTokens":600}}"#,
            &config,
            &concurrent_state_dir,
        )
    );
    let left = left.expect("left concurrent post");
    let right = right.expect("right concurrent post");
    let measured = [left, right]
        .into_iter()
        .filter(|result| {
            result
                .checkpoint
                .token_savings
                .as_ref()
                .is_some_and(|savings| savings.status == jevx::hooks::MeasurementStatus::Measured)
        })
        .count();
    assert_eq!(measured, 1);

    let interleaved_state_dir = root.path().join("compaction-interleaved");
    for (turn, total) in [("turn-a", 1_000), ("turn-b", 2_000)] {
        run_compact_assist(
            &format!(
                "{{\"hook_event_name\":\"PreCompact\",\"session_id\":\"session-interleaved\",\"turn_id\":\"{turn}\",\"preCompactionUsage\":{{\"inputTokens\":900,\"totalTokens\":{total}}}}}"
            ),
            &config,
            &interleaved_state_dir,
        )
        .await
        .expect("interleaved pre compact");
    }
    let post_a = run_compact_assist(
        r#"{"hook_event_name":"PostCompact","session_id":"session-interleaved","turn_id":"turn-a","postCompactionUsage":{"inputTokens":800,"totalTokens":900}}"#,
        &config,
        &interleaved_state_dir,
    )
    .await
    .expect("interleaved post a");
    let post_b = run_compact_assist(
        r#"{"hook_event_name":"PostCompact","session_id":"session-interleaved","turn_id":"turn-b","postCompactionUsage":{"inputTokens":1400,"totalTokens":1500}}"#,
        &config,
        &interleaved_state_dir,
    )
    .await
    .expect("interleaved post b");
    assert_eq!(
        post_a
            .checkpoint
            .token_savings
            .as_ref()
            .and_then(|savings| savings.saved_tokens),
        Some(100)
    );
    assert_eq!(
        post_b
            .checkpoint
            .token_savings
            .as_ref()
            .and_then(|savings| savings.saved_tokens),
        Some(500)
    );

    let broken_state_dir = root.path().join("compaction-broken");
    run_compact_assist(
        r#"{"hook_event_name":"PreCompact","session_id":"session-broken","turn_id":"turn-broken","preCompactionUsage":{"inputTokens":900,"totalTokens":1000}}"#,
        &config,
        &broken_state_dir,
    )
    .await
    .expect("broken pre compact");
    fs::OpenOptions::new()
        .append(true)
        .open(broken_state_dir.join("checkpoints.jsonl"))
        .expect("open broken checkpoint")
        .write_all(b"{broken-checkpoint}\n")
        .expect("write broken checkpoint");
    let error = run_compact_assist(
        r#"{"hook_event_name":"PostCompact","session_id":"session-broken","turn_id":"turn-broken","postCompactionUsage":{"inputTokens":500,"totalTokens":600}}"#,
        &config,
        &broken_state_dir,
    )
    .await
    .expect_err("broken checkpoint must fail explicitly");
    assert!(error.to_string().contains("invalid compaction checkpoint"));
}

#[tokio::test]
async fn provider_style_snake_case_usage_is_normalized_without_raw_payload() {
    let config = Config::for_test(tempdir().expect("tempdir").path().join("data"));
    let result = run_shadow(
        r#"{"hook_event_name":"PostCompact","preCompactionUsage":{"input_tokens":900,"output_tokens":100,"total_tokens":1000,"input_tokens_details":{"cached_tokens":700,"cache_write_tokens":10},"output_tokens_details":{"reasoning_tokens":40}},"postCompactionUsage":{"input_tokens":500,"output_tokens":100,"total_tokens":600}}"#,
        None,
        &[],
        &config,
        None,
    )
    .await
    .expect("provider usage");

    let usage = result
        .record
        .pre_compaction_usage
        .as_ref()
        .expect("pre usage");
    assert_eq!(usage.cached_input_tokens, 700);
    assert_eq!(usage.cache_write_input_tokens, 10);
    assert_eq!(usage.reasoning_output_tokens, 40);
    assert_eq!(
        result
            .record
            .token_savings
            .as_ref()
            .map(|savings| savings.status),
        Some(jevx::hooks::MeasurementStatus::Unavailable)
    );
    assert_eq!(
        result
            .record
            .token_savings
            .as_ref()
            .and_then(|savings| savings.saved_tokens),
        None
    );
    let serialized = serde_json::to_string(&result.record).expect("record json");
    assert!(!serialized.contains("input_tokens_details"));
}
