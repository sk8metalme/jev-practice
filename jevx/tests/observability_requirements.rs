use std::fs;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use jevx::compact_assist::run_compact_assist;
use jevx::hooks::{TokenUsageSnapshot, analyze_hook_stats, run_shadow};
use jevx::{Config, Judge, JudgeRequest, JudgeResponse, SkillRecord};
use tempfile::tempdir;

struct CountingJudge {
    calls: Arc<AtomicUsize>,
}

struct FailingJudge {
    calls: Arc<AtomicUsize>,
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
    let config = Config::for_test(root.path().join("data"));
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
    assert_eq!(calls.load(Ordering::SeqCst), 1);
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
        r#"{"hook_event_name":"PostCompact","session_id":"session-1","turn_id":"turn-1","preCompactionUsage":{"inputTokens":900,"totalTokens":1000},"postCompactionUsage":{"inputTokens":500,"totalTokens":600}}"#,
        None,
        &[],
        &config,
        None,
    )
    .await
    .expect("post compact");

    let report = analyze_hook_stats(&[pre.record, post.record]).expect("hook stats");
    assert_eq!(report.record_count, 2);
    assert_eq!(report.event_counts["PreCompact"], 1);
    assert_eq!(report.event_counts["PostCompact"], 1);
    assert_eq!(report.token_savings.saved_tokens, Some(400));
    assert_eq!(report.token_savings.measured_records, 1);
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
        r#"{"hook_event_name":"PostCompact","session_id":"session-1","turn_id":"turn-1","postCompactionUsage":{"inputTokens":500,"totalTokens":600}}"#,
        &config,
        &state_dir,
    )
    .await
    .expect("post compact assist");

    assert_eq!(post.checkpoint.schema_version, 2);
    assert_eq!(
        post.checkpoint
            .token_savings
            .as_ref()
            .and_then(|savings| savings.saved_tokens),
        Some(400)
    );
    let records = fs::read_to_string(state_dir.join("hook-records.jsonl")).expect("records");
    assert!(records.contains("\"savedTokens\":400"));
}

#[tokio::test]
async fn provider_style_snake_case_usage_is_normalized_without_raw_payload() {
    let config = Config::for_test(tempdir().expect("tempdir").path().join("data"));
    let result = run_shadow(
        r#"{"hook_event_name":"PostCompact","preCompactionUsage":{"input_tokens":900,"total_tokens":1000,"input_tokens_details":{"cached_tokens":700,"cache_write_tokens":10},"output_tokens_details":{"reasoning_tokens":40}},"postCompactionUsage":{"input_tokens":500,"total_tokens":600}}"#,
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
            .and_then(|savings| savings.saved_tokens),
        Some(400)
    );
    let serialized = serde_json::to_string(&result.record).expect("record json");
    assert!(!serialized.contains("input_tokens_details"));
}
