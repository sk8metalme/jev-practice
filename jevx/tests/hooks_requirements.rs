use std::fs;
use std::path::PathBuf;

use async_trait::async_trait;
use jevx::hooks::{
    ConversationCompactionCase, HookShadowRecord, analyze_hook_correlations, append_shadow_record,
    compact_evaluation, evaluate_conversation_compaction, load_conversation_cases,
    load_hook_records, run_shadow,
};
use jevx::{
    Config, CostEstimate, CostSummary, JevxError, Judge, JudgeRequest, JudgeResponse, SkillRecord,
};
use serde_json::json;
use tempfile::tempdir;

struct StubJudge;

#[async_trait]
impl Judge for StubJudge {
    async fn evaluate(&self, _request: JudgeRequest) -> Result<JudgeResponse, JevxError> {
        Ok(JudgeResponse::selected("pdf", 0.95, 12, Some((31, 7))))
    }
}

struct ChoiceJudge {
    choice: String,
}

#[async_trait]
impl Judge for ChoiceJudge {
    async fn evaluate(&self, _request: JudgeRequest) -> Result<JudgeResponse, JevxError> {
        Ok(JudgeResponse::selected(
            &self.choice,
            0.95,
            12,
            Some((31, 7)),
        ))
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
        r#"{"hook_event_name":"UserPromptSubmit","prompt":"PDFを結合したい secret=fixture-only","cwd":"/tmp/project","session_id":"session-fixture","turn_id":"turn-fixture","model":"gpt-5.6-sol"}"#,
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
    assert!(result.record.session_id_sha256.is_some());
    assert!(result.record.turn_id_sha256.is_some());
    assert!(result.record.model_sha256.is_some());
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
async fn hook_shadow_records_codex_usage_without_treating_missing_total_as_zero() {
    let input = serde_json::json!({
        "hook_event_name": "PostCompact",
        "codexUsage": {
            "model": "gpt-6-luna",
            "reasoningEffort": "max",
            "mainTurns": 1,
            "subagentCount": 2,
            "inputTokens": 100,
            "outputTokens": 20,
            "reasoningTokens": 40,
            "additionalInputTokens": 10,
            "additionalOutputTokens": 3,
            "additionalReasoningTokens": 5,
            "elapsedMs": 321,
            "fallbackStage": "luna",
            "cost": {
                "amount": 0.42,
                "currency": "USD",
                "priceVersion": "codex-fixture-1",
                "status": "available"
            },
            "additionalCost": {
                "amount": 0.07,
                "currency": "USD",
                "priceVersion": "codex-fixture-1",
                "status": "available",
                "basis": "actual"
            }
        }
    });
    let result = run_shadow(
        &serde_json::to_string(&input).expect("input json"),
        None,
        &[],
        &Config::for_test(PathBuf::from("/tmp/jevx-hook-test")),
        None,
    )
    .await
    .expect("codex usage is optional hook metadata");
    let codex = result.record.codex.as_ref().expect("codex usage");
    assert_eq!(codex.model.as_deref(), Some("gpt-6-luna"));
    assert_eq!(codex.reasoning_effort.as_deref(), Some("max"));
    assert_eq!(codex.subagent_count, Some(2));
    assert_eq!(codex.additional_input_tokens, Some(10));
    assert_eq!(codex.additional_output_tokens, Some(3));
    assert_eq!(codex.additional_reasoning_tokens, Some(5));
    assert_eq!(codex.cost.amount, Some(0.42));
    assert_eq!(
        codex.additional_cost.as_ref().and_then(|cost| cost.amount),
        Some(0.07)
    );
    assert_eq!(result.record.cost.codex.amount, Some(0.42));
    assert_eq!(result.record.cost.jev.amount, Some(0.0));
    assert_eq!(result.record.cost.total.amount, Some(0.42));
    let serialized = serde_json::to_string(&result.record).expect("record json");
    assert!(!serialized.contains("PRIVATE"));
}

#[tokio::test]
async fn hook_shadow_sanitizes_lifecycle_and_selected_skill_metadata() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().join("data"));
    let lifecycle = run_shadow(
        r#"{"hook_event_name":"PreCompact","trigger":" manual ","source":"source with space"}"#,
        Some("PreCompact"),
        &[],
        &config,
        None,
    )
    .await
    .expect("lifecycle hook");
    assert_eq!(lifecycle.record.trigger.as_deref(), Some("manual"));
    assert_eq!(lifecycle.record.source, None);
    let lifecycle_json = serde_json::to_string(&lifecycle.record).expect("lifecycle json");
    assert!(!lifecycle_json.contains("source with space"));

    let oversized = format!("source-{}", "a".repeat(128));
    let boundary_input = serde_json::to_string(&json!({
        "hook_event_name": "PreCompact",
        "trigger": "\u{1}",
        "source": oversized,
    }))
    .expect("boundary hook json");
    let boundary = run_shadow(&boundary_input, Some("PreCompact"), &[], &config, None)
        .await
        .expect("boundary hook");
    assert_eq!(boundary.record.trigger, None);
    assert_eq!(boundary.record.source, None);

    let unsafe_skill = run_shadow(
        r#"{"hook_event_name":"UserPromptSubmit","prompt":"unsafe skill"}"#,
        Some("UserPromptSubmit"),
        &[skill(root.path(), "unsafe skill", "unsafe")],
        &config,
        Some(&ChoiceJudge {
            choice: "unsafe skill".to_owned(),
        }),
    )
    .await
    .expect("unsafe selected skill hook");
    assert_eq!(unsafe_skill.record.selected_skill, None);

    let overlong_skill = format!("skill-{}", "a".repeat(128));
    let overlong_selected = run_shadow(
        r#"{"hook_event_name":"UserPromptSubmit","prompt":"overlong skill"}"#,
        Some("UserPromptSubmit"),
        &[skill(root.path(), &overlong_skill, "overlong")],
        &config,
        Some(&ChoiceJudge {
            choice: overlong_skill,
        }),
    )
    .await
    .expect("overlong selected skill hook");
    assert_eq!(overlong_selected.record.selected_skill, None);

    let namespaced_skill = run_shadow(
        r#"{"hook_event_name":"UserPromptSubmit","prompt":"namespaced skill"}"#,
        Some("UserPromptSubmit"),
        &[skill(
            root.path(),
            "data-analytics:quality",
            "namespaced skill",
        )],
        &config,
        Some(&ChoiceJudge {
            choice: "data-analytics:quality".to_owned(),
        }),
    )
    .await
    .expect("namespaced selected skill hook");
    assert_eq!(
        namespaced_skill.record.selected_skill.as_deref(),
        Some("data-analytics:quality")
    );
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
        r#"{"hook_event_name":"SessionStart","source":"compact","session_id":"session-only"}"#,
        None,
        &[],
        &config,
        None,
    )
    .await
    .expect("session start hook");
    assert_eq!(session_start.record.source.as_deref(), Some("compact"));
    assert!(session_start.record.session_id_sha256.is_some());
    assert!(session_start.record.correlation_id_sha256.is_some());
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
        model: Some("gpt-5.6-sol".to_owned()),
        compaction_completed: true,
        compaction_duration_ms: 120,
        input_chars: 240,
        conversation_turns: 8,
        context_chars: 12_400,
        failure_recovery_required: false,
        tool_history_items: 0,
        tool_failure_count: 0,
        interrupted_turns: 0,
        recovery_turns: 0,
        recovery_completed: false,
        pre_compaction_usage: None,
        compaction_usage: None,
        post_compaction_usage: None,
        post_compaction_cost: None,
        codex_usage: None,
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
    assert_eq!(report.runs[0].model.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(report.runs[0].conversation_turns, 8);
    assert_eq!(report.runs[0].context_chars, 12_400);
    assert_eq!(report.runs[0].event_count, 2);
    let json = serde_json::to_string(&report).expect("report json");
    assert!(!json.contains("DECOY_DO_NOT_OUTPUT"));
    assert!(!json.contains("goal=keep-context"));
}

#[test]
fn conversation_compaction_rejects_token_total_overflow() {
    let usage = jevx::hooks::TokenUsageSnapshot {
        input_tokens: u64::MAX,
        total_tokens: u64::MAX,
        ..jevx::hooks::TokenUsageSnapshot::default()
    };
    let mut left = conversation_case("overflow-left", "goal=keep-context\nnext=verify");
    left.post_compaction_usage = Some(usage.clone());
    let mut right = conversation_case("overflow-right", "goal=keep-context\nnext=verify");
    right.post_compaction_usage = Some(usage);

    let error = evaluate_conversation_compaction(&[left, right])
        .expect_err("token total overflow must fail explicitly");
    assert!(error.to_string().contains("inputTokens total overflows"));

    let mut saved_left = conversation_case("saved-overflow-left", "goal=keep-context\nnext=verify");
    saved_left.pre_compaction_usage = Some(jevx::hooks::TokenUsageSnapshot {
        total_tokens: u64::MAX,
        ..jevx::hooks::TokenUsageSnapshot::default()
    });
    saved_left.post_compaction_usage = Some(jevx::hooks::TokenUsageSnapshot {
        total_tokens: 1,
        ..jevx::hooks::TokenUsageSnapshot::default()
    });
    let mut saved_right = saved_left.clone();
    saved_right.case_id = "saved-overflow-right".to_owned();
    let error = evaluate_conversation_compaction(&[saved_left, saved_right])
        .expect_err("saved token overflow must fail explicitly");
    assert!(error.to_string().contains("savedTokens total overflows"));
}

#[test]
fn conversation_compaction_records_codex_usage_and_fallback_extra_cost() {
    let mut case = conversation_case("codex-cost", "goal=keep-context\nnext=verify");
    case.codex_usage = Some(jevx::CodexUsage {
        model: Some("gpt-5.6-sol".to_owned()),
        reasoning_effort: Some("high".to_owned()),
        main_turns: Some(1),
        subagent_count: Some(2),
        input_tokens: Some(100),
        cached_input_tokens: None,
        cache_write_input_tokens: None,
        output_tokens: Some(20),
        reasoning_tokens: Some(10),
        elapsed_ms: Some(321),
        fallback_stage: Some("sol-to-terra".to_owned()),
        cost: jevx::CostEstimate::actual(
            0.42,
            Some("USD".to_owned()),
            Some("codex-fixture-1".to_owned()),
        ),
        additional_input_tokens: Some(10),
        additional_output_tokens: Some(3),
        additional_reasoning_tokens: Some(5),
        additional_cost: Some(jevx::CostEstimate::actual(
            0.07,
            Some("USD".to_owned()),
            Some("codex-fixture-1".to_owned()),
        )),
    });
    let report = evaluate_conversation_compaction(&[case]).expect("compaction evaluation");
    let usage = report.runs[0].codex_usage.as_ref().expect("codex usage");
    assert_eq!(usage.model.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(usage.additional_output_tokens, Some(3));
    assert_eq!(report.summary.codex_cost, Some(0.42));
    assert_eq!(report.summary.fallback_extra_cost, Some(0.07));
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

    let mut recovery = conversation_case("recovery-incomplete", "goal=keep-context\nnext=verify");
    recovery.failure_recovery_required = true;
    recovery.tool_history_items = 1;
    recovery.tool_failure_count = 1;
    recovery.recovery_turns = 1;
    recovery.conversation_turns = 1;
    let recovery_report =
        evaluate_conversation_compaction(&[recovery]).expect("recovery evaluation");
    assert_eq!(recovery_report.summary.passed, 0);
    assert_eq!(
        recovery_report.runs[0].error_code.as_deref(),
        Some("recovery_incomplete")
    );
}

#[test]
fn conversation_evaluation_records_tool_recovery_and_token_cache_metrics() {
    let root = tempdir().expect("tempdir");
    let input = root.path().join("resilience.jsonl");
    fs::write(
        &input,
        r#"{"caseId":"resilience-case","model":"gpt-5.6-sol","requiredFacts":["goal=keep-context","next=verify"],"followUpText":"goal=keep-context\nnext=verify\ndecoy_marker=redacted","secretMarkers":["DECOY_DO_NOT_OUTPUT"],"failureRecoveryRequired":true,"toolHistoryItems":3,"toolFailureCount":1,"interruptedTurns":1,"recoveryTurns":2,"recoveryCompleted":true,"preCompactionUsage":{"inputTokens":1000,"cachedInputTokens":400,"cacheWriteInputTokens":50,"outputTokens":80,"reasoningOutputTokens":20,"totalTokens":1100},"compactionUsage":{"inputTokens":200,"cachedInputTokens":100,"cacheWriteInputTokens":0,"outputTokens":40,"reasoningOutputTokens":10,"totalTokens":250},"postCompactionUsage":{"inputTokens":800,"cachedInputTokens":600,"cacheWriteInputTokens":0,"outputTokens":100,"reasoningOutputTokens":20,"totalTokens":920},"compactionCompleted":true,"compactionDurationMs":120,"inputChars":12400,"conversationTurns":10,"contextChars":12400,"observedEvents":["functionCallOutput","turn/interrupted","contextCompaction","turn/completed"]}"#,
    )
    .expect("write resilience fixture");

    let cases = load_conversation_cases(&input).expect("load resilience fixture");
    let report = evaluate_conversation_compaction(&cases).expect("resilience evaluation");
    let run = &report.runs[0];
    assert_eq!(run.tool_history_items, 3);
    assert_eq!(run.tool_failure_count, 1);
    assert_eq!(run.interrupted_turns, 1);
    assert_eq!(run.recovery_turns, 2);
    assert!(run.recovery_completed);
    assert_eq!(run.post_compaction_cache_hit_rate, Some(0.75));
    assert_eq!(run.post_compaction_uncached_input_tokens, Some(200));
    assert_eq!(run.post_compaction_estimated_billable_tokens, Some(300));
    assert_eq!(report.summary.recovery_completion_rate, Some(1.0));
    assert_eq!(
        report.summary.post_compaction_cache_hit_rate_p50,
        Some(0.75)
    );
    assert_eq!(report.summary.total_input_tokens, 800);
    assert_eq!(report.summary.total_cached_input_tokens, 600);
    assert_eq!(report.summary.total_cache_write_input_tokens, 0);
    assert_eq!(report.summary.total_output_tokens, 100);
    assert_eq!(report.summary.total_tokens, 920);

    let json = serde_json::to_string(&report).expect("resilience report json");
    assert!(!json.contains("DECOY_DO_NOT_OUTPUT"));
    assert!(!json.contains("goal=keep-context"));
}

#[test]
fn conversation_evaluation_rejects_invalid_token_cache_metadata() {
    let root = tempdir().expect("tempdir");
    let input = root.path().join("invalid-usage.jsonl");
    fs::write(
        &input,
        r#"{"caseId":"invalid-usage","requiredFacts":["goal=keep-context"],"followUpText":"goal=keep-context","secretMarkers":["DECOY"],"postCompactionUsage":{"inputTokens":10,"cachedInputTokens":11,"outputTokens":1,"totalTokens":12},"compactionCompleted":true,"compactionDurationMs":1,"inputChars":1,"observedEvents":["contextCompaction"]}"#,
    )
    .expect("write invalid usage fixture");

    let error = load_conversation_cases(&input).expect_err("invalid usage must fail");
    assert!(error.to_string().contains("cachedInputTokens"));
}

#[test]
fn conversation_evaluation_rejects_invalid_post_compaction_cost_metadata() {
    let mut case = conversation_case("invalid-cost", "goal=keep-context\nnext=verify");
    case.post_compaction_cost = Some(CostEstimate::actual(
        -0.1,
        Some("USD".to_owned()),
        Some("fixture".to_owned()),
    ));

    let error = evaluate_conversation_compaction(&[case])
        .expect_err("invalid post-compaction cost must fail");
    assert!(error.to_string().contains("postCompactionCost"));
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

    let invalid_tool_counts = |mut case: ConversationCompactionCase| {
        case.tool_history_items = 0;
        case.tool_failure_count = 1;
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    invalid_tool_counts(conversation_case(
        "invalid-tool-counts",
        "goal=keep-context",
    ));

    let invalid_recovery_counts = |mut case: ConversationCompactionCase| {
        case.conversation_turns = 1;
        case.recovery_turns = 2;
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    invalid_recovery_counts(conversation_case(
        "invalid-recovery-counts",
        "goal=keep-context",
    ));

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

#[test]
fn hook_correlation_analysis_groups_duplicates_without_raw_ids() {
    let root = tempdir().expect("tempdir");
    let input = root.path().join("hooks.jsonl");
    let record = |event: &str, turn: Option<&str>, elapsed: u64| {
        json!({
            "schemaVersion": 1,
            "mode": "shadow",
            "hookEventName": event,
            "trigger": if event == "SessionStart" { serde_json::Value::Null } else { json!("manual") },
            "source": if event == "SessionStart" { json!("startup") } else { serde_json::Value::Null },
            "sessionIdSha256": "session-hash-fixture",
            "turnIdSha256": turn,
            "modelSha256": "model-hash-fixture",
            "correlationIdSha256": if event == "SessionStart" { json!("session-correlation-fixture") } else { json!("turn-correlation-fixture") },
            "elapsedMs": elapsed,
        })
    };
    let lines = [
        record("UserPromptSubmit", Some("turn-hash-fixture"), 10),
        record("UserPromptSubmit", Some("turn-hash-fixture"), 11),
        record("SessionStart", None, 0),
        record("SessionStart", None, 0),
        record("PostCompact", Some("post-turn"), 12),
    ]
    .into_iter()
    .map(|value| serde_json::to_string(&value).expect("record json"))
    .collect::<Vec<_>>()
    .join("\n");
    fs::write(&input, lines).expect("write hook records");

    let records = load_hook_records(&input).expect("load hook records");
    let report = analyze_hook_correlations(&records).expect("correlation report");
    assert_eq!(report.record_count, 5);
    assert_eq!(report.duplicate_group_count, 2);
    assert_eq!(report.duplicate_record_count, 2);
    assert_eq!(report.groups[0].count, 2);
    let report_json = serde_json::to_string(&report).expect("correlation json");
    assert!(!report_json.contains("raw-session"));
    assert!(report_json.contains("session-hash-fixture"));

    let no_identity = root.path().join("hooks-without-identity.jsonl");
    fs::write(
        &no_identity,
        "{\"schemaVersion\":1,\"mode\":\"shadow\",\"hookEventName\":\"SessionStart\",\"elapsedMs\":0}\n{\"schemaVersion\":1,\"mode\":\"shadow\",\"hookEventName\":\"SessionStart\",\"elapsedMs\":0}",
    )
    .expect("write uncorrelated hook records");
    let uncorrelated = load_hook_records(&no_identity).expect("load uncorrelated records");
    let uncorrelated_report =
        analyze_hook_correlations(&uncorrelated).expect("uncorrelated report");
    assert_eq!(uncorrelated_report.duplicate_group_count, 0);
    assert_eq!(uncorrelated_report.duplicate_record_count, 0);
}

#[test]
fn hook_correlation_loader_rejects_empty_and_secret_echo() {
    let root = tempdir().expect("tempdir");
    let empty = root.path().join("empty.jsonl");
    fs::write(&empty, "\n").expect("write empty records");
    assert!(load_hook_records(&empty).is_err());

    let invalid = root.path().join("invalid.jsonl");
    fs::write(
        &invalid,
        r#"{"hookEventName":"UserPromptSubmit","prompt":"PRIVATE_RAW_INPUT"}"#,
    )
    .expect("write invalid records");
    let error = load_hook_records(&invalid).expect_err("invalid records must fail");
    assert!(!error.to_string().contains("PRIVATE_RAW_INPUT"));
    assert!(analyze_hook_correlations(&[]).is_err());

    let valid_record = || {
        json!({
            "schemaVersion": 1,
            "mode": "shadow",
            "hookEventName": "SessionStart",
            "elapsedMs": 0,
        })
    };
    for (field, value) in [
        ("schemaVersion", json!(99)),
        ("hookEventName", json!("Unknown")),
        ("mode", json!("unsafe mode")),
        ("sessionIdSha256", json!("unsafe session")),
        ("turnIdSha256", json!("unsafe turn")),
        ("modelSha256", json!("unsafe model")),
        ("correlationIdSha256", json!("unsafe correlation")),
    ] {
        let root = tempdir().expect("tempdir");
        let path = root.path().join("invalid-record.jsonl");
        let mut record = valid_record();
        record[field] = value;
        fs::write(&path, serde_json::to_string(&record).expect("record json"))
            .expect("write invalid record");
        let error = load_hook_records(&path).expect_err("unsafe record must fail");
        assert!(!error.to_string().contains("unsafe mode"));
    }
}

#[test]
fn hook_correlation_loader_migrates_legacy_unsafe_metadata() {
    let root = tempdir().expect("tempdir");
    let path = root.path().join("legacy-hook-records.jsonl");
    let record = json!({
        "schemaVersion": 1,
        "mode": "shadow",
        "hookEventName": "UserPromptSubmit",
        "trigger": " manual ",
        "source": "legacy source with spaces",
        "selectedSkill": "legacy skill 🚀",
        "elapsedMs": 1,
    });
    fs::write(&path, serde_json::to_string(&record).expect("record json")).expect("write");

    let records = load_hook_records(&path).expect("legacy record must remain readable");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].trigger.as_deref(), Some("manual"));
    assert_eq!(records[0].source, None);
    assert_eq!(records[0].selected_skill, None);
    let report = analyze_hook_correlations(&records).expect("migrated record analysis");
    assert_eq!(report.record_count, 1);
}

#[test]
fn append_shadow_record_rejects_untrusted_selected_skill() {
    let root = tempdir().expect("tempdir");
    let path = root.path().join("hook-records.jsonl");
    let record = HookShadowRecord {
        schema_version: 4,
        mode: "shadow".to_owned(),
        hook_event_name: "UserPromptSubmit".to_owned(),
        trigger: None,
        source: None,
        session_id_sha256: None,
        turn_id_sha256: None,
        model_sha256: None,
        correlation_id_sha256: None,
        prompt_sha256: None,
        prompt_chars: None,
        decision: None,
        selected_skill: Some("unsafe skill".to_owned()),
        discovery_ms: None,
        jev_response_ms: None,
        total_ms: None,
        input_tokens: None,
        output_tokens: None,
        error_code: None,
        dedupe_error: None,
        codex: None,
        dedupe_hit: false,
        dedupe_key_sha256: None,
        pre_compaction_usage: None,
        compaction_usage: None,
        post_compaction_usage: None,
        post_compaction_cost: None,
        compaction_elapsed_ms: None,
        token_savings: None,
        cost: CostSummary::default(),
        elapsed_ms: 0,
    };

    let error = append_shadow_record(&path, &record).expect_err("unsafe record must not write");
    assert!(error.to_string().contains("unsafe identifier"));
    assert!(!path.exists());
}

#[test]
fn conversation_fixture_validation_rejects_unsafe_model_and_oversized_context_metadata() {
    let invalid_model = |mut case: ConversationCompactionCase| {
        case.model = Some("model with spaces".to_owned());
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    invalid_model(conversation_case(
        "invalid-model",
        "goal=keep-context\nnext=verify",
    ));

    let oversized_turns = |mut case: ConversationCompactionCase| {
        case.conversation_turns = 1_025;
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    oversized_turns(conversation_case(
        "oversized-turns",
        "goal=keep-context\nnext=verify",
    ));

    let oversized_context = |mut case: ConversationCompactionCase| {
        case.context_chars = 10_000_001;
        assert!(evaluate_conversation_compaction(&[case]).is_err());
    };
    oversized_context(conversation_case(
        "oversized-context",
        "goal=keep-context\nnext=verify",
    ));
}
