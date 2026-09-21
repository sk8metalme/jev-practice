use std::fs;
use std::path::PathBuf;

use async_trait::async_trait;
use jevx::hooks::{
    ConversationCompactionCase, append_shadow_record, compact_evaluation,
    evaluate_conversation_compaction, load_conversation_cases, run_shadow,
};
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

fn conversation_case(id: &str, response: &str) -> ConversationCompactionCase {
    ConversationCompactionCase {
        case_id: id.to_owned(),
        required_facts: vec!["goal=keep-context".to_owned(), "next=verify".to_owned()],
        follow_up_text: response.to_owned(),
        secret_markers: vec!["DECOY_DO_NOT_OUTPUT".to_owned()],
        compaction_completed: true,
        compaction_duration_ms: 120,
        input_chars: 240,
        observed_events: vec!["contextCompaction".to_owned(), "turn/completed".to_owned()],
    }
}

#[test]
fn conversation_compaction_evaluation_aggregates_without_storing_raw_text() {
    let cases = vec![
        conversation_case(
            "case-a",
            "goal=keep-context\nnext=verify\ndecoy_marker=redacted",
        ),
        conversation_case(
            "case-b",
            "goal: keep-context\nnext: verify\ndecoy_marker=redacted",
        ),
        conversation_case(
            "case-c",
            "goal=keep-context\nnext=verify\ndecoy_marker=redacted",
        ),
    ];
    let report = evaluate_conversation_compaction(&cases).expect("conversation evaluation");

    assert_eq!(report.mode, "live");
    assert_eq!(report.scenario, "real-conversation-codex-v1");
    assert_eq!(report.run_count, 3);
    assert_eq!(report.summary.passed, 3);
    assert_eq!(report.summary.retention_rate, Some(1.0));
    assert_eq!(report.summary.compaction_completion_rate, Some(1.0));
    assert_eq!(report.runs[0].case_id.as_deref(), Some("case-a"));
    assert_eq!(report.runs[0].event_count, 2);
    let json = serde_json::to_string(&report).expect("report json");
    assert!(!json.contains("DECOY_DO_NOT_OUTPUT"));
    assert!(!json.contains("goal=keep-context"));
}

#[test]
fn conversation_compaction_evaluation_reports_incomplete_and_lost_facts() {
    let mut incomplete = conversation_case("incomplete", "goal=keep-context");
    incomplete.compaction_completed = false;
    let incomplete_report = evaluate_conversation_compaction(&[incomplete])
        .expect("incomplete conversation evaluation");
    assert_eq!(incomplete_report.summary.passed, 0);
    assert_eq!(
        incomplete_report.runs[0].error_code.as_deref(),
        Some("compaction_incomplete")
    );

    let lost = conversation_case("lost-fact", "goal=keep-context");
    let lost_report = evaluate_conversation_compaction(&[lost]).expect("lost fact evaluation");
    assert_eq!(lost_report.summary.passed, 0);
    assert_eq!(
        lost_report.runs[0].error_code.as_deref(),
        Some("required_fact_lost")
    );
}

#[test]
fn conversation_fixture_loader_rejects_invalid_input_without_echoing_secrets() {
    let root = tempdir().expect("tempdir");
    let valid = root.path().join("conversation.jsonl");
    fs::write(
        &valid,
        serde_json::to_string(&conversation_case(
            "loader-case",
            "goal=keep-context\nnext=verify\ndecoy_marker=redacted",
        ))
        .expect("fixture json"),
    )
    .expect("write fixture");
    let cases = load_conversation_cases(&valid).expect("load conversation fixture");
    assert_eq!(cases.len(), 1);

    let invalid = root.path().join("invalid.jsonl");
    fs::write(
        &invalid,
        r#"{"caseId":"bad case","requiredFacts":[],"followUpText":"TOP_SECRET_INPUT","secretMarkers":["TOP_SECRET_INPUT"],"compactionCompleted":true,"compactionDurationMs":1,"inputChars":1,"observedEvents":["contextCompaction"]}"#,
    )
    .expect("write invalid fixture");
    let error = load_conversation_cases(&invalid).expect_err("invalid fixture must fail");
    let message = error.to_string();
    assert!(message.contains("line 1"));
    assert!(!message.contains("TOP_SECRET_INPUT"));
}

#[test]
fn conversation_fixture_validation_covers_empty_and_boundaries() {
    assert!(evaluate_conversation_compaction(&[]).is_err());

    let root = tempdir().expect("tempdir");
    let empty = root.path().join("empty.jsonl");
    fs::write(&empty, "\n  \n").expect("empty fixture");
    let empty_error = load_conversation_cases(&empty).expect_err("empty fixture must fail");
    assert!(empty_error.to_string().contains("at least one"));

    let malformed = root.path().join("malformed.jsonl");
    fs::write(&malformed, r#"{"caseId":"unterminated""#).expect("malformed fixture");
    let malformed_error =
        load_conversation_cases(&malformed).expect_err("malformed fixture must fail");
    assert!(malformed_error.to_string().contains("line 1"));

    let invalid_fact = |mut case: ConversationCompactionCase| {
        case.required_facts = vec![String::new()];
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    invalid_fact(conversation_case("invalid-fact", "goal=keep-context"));

    let too_many_facts = |mut case: ConversationCompactionCase| {
        case.required_facts = (0..33).map(|index| format!("fact={index}")).collect();
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    too_many_facts(conversation_case("too-many-facts", "goal=keep-context"));

    let empty_markers = |mut case: ConversationCompactionCase| {
        case.secret_markers = Vec::new();
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    empty_markers(conversation_case("empty-markers", "goal=keep-context"));

    let invalid_marker = |mut case: ConversationCompactionCase| {
        case.secret_markers = vec![String::new()];
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    invalid_marker(conversation_case("invalid-marker", "goal=keep-context"));

    let too_many_markers = |mut case: ConversationCompactionCase| {
        case.secret_markers = (0..33).map(|index| format!("marker-{index}")).collect();
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    too_many_markers(conversation_case("too-many-markers", "goal=keep-context"));

    let huge_response = |mut case: ConversationCompactionCase| {
        case.follow_up_text = "x".repeat(1_000_001);
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    huge_response(conversation_case("huge-response", "goal=keep-context"));

    let huge_input = |mut case: ConversationCompactionCase| {
        case.input_chars = 1_000_001;
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    huge_input(conversation_case("huge-input", "goal=keep-context"));

    let empty_events = |mut case: ConversationCompactionCase| {
        case.observed_events = Vec::new();
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    empty_events(conversation_case("empty-events", "goal=keep-context"));

    let too_many_events = |mut case: ConversationCompactionCase| {
        case.observed_events = (0..33).map(|index| format!("event-{index}")).collect();
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    too_many_events(conversation_case("too-many-events", "goal=keep-context"));

    let invalid_event = |mut case: ConversationCompactionCase| {
        case.observed_events = vec!["event with spaces".to_owned()];
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    invalid_event(conversation_case("invalid-event", "goal=keep-context"));

    let no_equals_fact = ConversationCompactionCase {
        required_facts: vec!["plain fact".to_owned()],
        ..conversation_case("plain-fact", "unrelated response")
    };
    let report =
        evaluate_conversation_compaction(&[no_equals_fact]).expect("plain fact evaluation");
    assert_eq!(
        report.runs[0].error_code.as_deref(),
        Some("required_fact_lost")
    );
}
