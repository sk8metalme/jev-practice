use std::path::PathBuf;

use async_trait::async_trait;
use jevx::hooks::{append_shadow_record, compact_evaluation, run_shadow};
use jevx::{Config, JevxError, Judge, JudgeRequest, JudgeResponse, SkillRecord};
use tempfile::tempdir;

struct StubJudge;

#[async_trait]
impl Judge for StubJudge {
    async fn evaluate(&self, _request: JudgeRequest) -> Result<JudgeResponse, JevxError> {
        Ok(JudgeResponse::selected("pdf", 0.95, 12, Some((31, 7))))
    }
}

struct ErrorJudge;

#[async_trait]
impl Judge for ErrorJudge {
    async fn evaluate(&self, _request: JudgeRequest) -> Result<JudgeResponse, JevxError> {
        Err(JevxError::Provider("fixture provider error".to_owned()))
    }
}

fn skill(root: &std::path::Path, id: &str, description: &str) -> SkillRecord {
    SkillRecord::new(
        id.to_owned(),
        description.to_owned(),
        root.join(id).join("SKILL.md"),
        "eval".to_owned(),
    )
}

#[tokio::test]
async fn hook_shadow_preserves_codex_flow_and_redacts_prompt() {
    let root = tempdir().expect("tempdir");
    let skills = vec![skill(root.path(), "pdf", "PDF pdf 結合")];
    let config = Config::for_test(root.path().join("data"));
    let result = run_shadow(
        r#"{"hook_event_name":"UserPromptSubmit","prompt":"PDFを結合したい secret=fixture-only","cwd":"/tmp/project"}"#,
        Some("UserPromptSubmit"),
        &skills,
        &config,
        Some(&StubJudge),
    )
    .await
    .expect("shadow hook");

    assert!(result.response.continue_running);
    assert!(result.response.suppress_output);
    assert_eq!(result.record.hook_event_name, "UserPromptSubmit");
    assert_eq!(result.record.selected_skill.as_deref(), Some("pdf"));
    assert!(result.record.prompt_sha256.is_some());
    let json = serde_json::to_string(&result).expect("shadow json");
    assert!(!json.contains("PDFを結合したい"));
    assert!(!json.contains("secret=fixture-only"));
}

#[tokio::test]
async fn hook_shadow_observes_compaction_events_without_judging() {
    let config = Config::for_test(PathBuf::from("/tmp/jevx-hook-test"));
    let result = run_shadow(
        r#"{"hook_event_name":"PreCompact","trigger":"auto","turn_id":"turn-fixture"}"#,
        Some("PreCompact"),
        &[],
        &config,
        None,
    )
    .await
    .expect("pre compact hook");

    assert_eq!(result.record.hook_event_name, "PreCompact");
    assert_eq!(result.record.trigger.as_deref(), Some("auto"));
    assert_eq!(result.record.selected_skill, None);
    assert!(result.response.continue_running);
}

#[tokio::test]
async fn hook_shadow_records_safe_errors_and_session_start_metadata() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().join("data"));
    let missing_prompt = run_shadow(
        r#"{"hook_event_name":"UserPromptSubmit"}"#,
        None,
        &[],
        &config,
        None,
    )
    .await
    .expect("missing prompt hook");
    assert_eq!(
        missing_prompt.record.error_code.as_deref(),
        Some("invalid_input")
    );

    let missing_judge = run_shadow(
        r#"{"hook_event_name":"UserPromptSubmit","prompt":"PDF"}"#,
        None,
        &[],
        &config,
        None,
    )
    .await
    .expect("missing judge hook");
    assert_eq!(
        missing_judge.record.error_code.as_deref(),
        Some("missing_api_key")
    );

    let provider_error = run_shadow(
        r#"{"hook_event_name":"UserPromptSubmit","prompt":"PDF"}"#,
        None,
        &[skill(root.path(), "pdf", "PDF pdf")],
        &config,
        Some(&ErrorJudge),
    )
    .await
    .expect("provider error hook");
    assert_eq!(
        provider_error.record.error_code.as_deref(),
        Some("provider_error")
    );

    let session_start = run_shadow(
        r#"{"hook_event_name":"SessionStart","source":"compact"}"#,
        None,
        &[],
        &config,
        None,
    )
    .await
    .expect("session start hook");
    assert_eq!(session_start.record.source.as_deref(), Some("compact"));
}

#[test]
fn compaction_evaluation_keeps_required_facts_and_redacts_fixture_markers() {
    let report = compact_evaluation(3).expect("compaction evaluation");
    assert_eq!(report.run_count, 3);
    assert_eq!(report.summary.passed, 3);
    assert_eq!(report.summary.secret_leaks, 0);
    assert_eq!(report.summary.error_rate, Some(0.0));
    assert!(report.summary.duration_ms_p95.is_some());
    let json = serde_json::to_string(&report).expect("compaction json");
    assert!(!json.contains("api_key="));
    assert!(!json.contains("fixture-only"));
}

#[tokio::test]
async fn hook_outputs_can_be_appended_to_nested_safe_path_and_zero_runs_are_rejected() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().join("data"));
    let result = run_shadow(
        r#"{"hook_event_name":"PostCompact","trigger":"manual"}"#,
        None,
        &[],
        &config,
        None,
    )
    .await
    .expect("post compact hook");
    let output = root.path().join("nested/events/shadow.jsonl");
    append_shadow_record(&output, &result.record).expect("append record");
    assert!(output.exists());
    assert!(
        matches!(compact_evaluation(0), Err(JevxError::InvalidInput(message)) if message.contains("runs"))
    );
}

#[tokio::test]
async fn hook_shadow_rejects_mismatched_event() {
    let config = Config::for_test(PathBuf::from("/tmp/jevx-hook-test"));
    let error = run_shadow(
        r#"{"hook_event_name":"PostCompact","trigger":"manual"}"#,
        Some("PreCompact"),
        &[],
        &config,
        None,
    )
    .await;
    assert!(matches!(error, Err(JevxError::InvalidInput(message)) if message.contains("event")));
}

#[tokio::test]
async fn hook_shadow_rejects_unknown_event() {
    let config = Config::for_test(PathBuf::from("/tmp/jevx-hook-test"));
    let error = run_shadow(r#"{"hook_event_name":"Unknown"}"#, None, &[], &config, None).await;
    assert!(
        matches!(error, Err(JevxError::InvalidInput(message)) if message.contains("unsupported"))
    );
}
