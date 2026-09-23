use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::thread;

use async_trait::async_trait;
use jevx::decision::{DecisionRequest, QuestionSpec};
use jevx::{
    CandidateDecision, Config, CostEstimate, CostStatus, CostSummary, GatewayJudge, Judge,
    JudgeRequest, JudgeResponse, SkillRecord, SuggestInput, TelemetryEvent, Usage,
    append_telemetry, read_decision_receipts, read_stats, suggest_with_judge,
    suggest_with_optional_judge,
};
use tempfile::tempdir;

struct StubJudge {
    response: JudgeResponse,
}

#[async_trait]
impl Judge for StubJudge {
    async fn evaluate(&self, _request: JudgeRequest) -> Result<JudgeResponse, jevx::JevxError> {
        Ok(self.response.clone())
    }
}

struct ErrorJudge;

#[async_trait]
impl Judge for ErrorJudge {
    async fn evaluate(&self, _request: JudgeRequest) -> Result<JudgeResponse, jevx::JevxError> {
        Err(jevx::JevxError::Provider("stub failure".to_owned()))
    }
}

fn skill(root: &Path, name: &str, description: &str) -> SkillRecord {
    SkillRecord::new(
        name.to_owned(),
        description.to_owned(),
        root.join(name).join("SKILL.md"),
        "project".to_owned(),
    )
}

#[tokio::test]
async fn suggestion_returns_selected_candidate_and_metrics() {
    let root = tempdir().expect("tempdir");
    let skills = vec![
        skill(root.path(), "pdf", "Merge and inspect PDF documents"),
        skill(root.path(), "testing", "Write and run tests"),
    ];
    let judge = StubJudge {
        response: JudgeResponse::selected("pdf", 0.87, 111, Some((12, 3))),
    };
    let config = Config::for_test(root.path().to_path_buf());

    let result = suggest_with_judge(
        SuggestInput::new(
            "Merge these PDF files".to_owned(),
            root.path().to_path_buf(),
        ),
        skills,
        &config,
        &judge,
    )
    .await
    .expect("suggestion");

    assert_eq!(result.decision, CandidateDecision::Selected);
    assert_eq!(
        result.selected.as_ref().map(|item| item.name.as_str()),
        Some("pdf")
    );
    assert_eq!(result.metrics.jev_response_ms, 111);
    assert_eq!(result.metrics.input_tokens, Some(12));
    assert_eq!(result.metrics.output_tokens, Some(3));
    assert!(result.metrics.total_ms >= result.metrics.jev_response_ms);
    assert_eq!(result.mode, "shadow");
}

#[tokio::test]
async fn low_probability_is_explicitly_not_recommended() {
    let root = tempdir().expect("tempdir");
    let judge = StubJudge {
        response: JudgeResponse::selected("pdf", 0.59, 20, None),
    };
    let result = suggest_with_judge(
        SuggestInput::new("Do something".to_owned(), root.path().to_path_buf()),
        vec![skill(root.path(), "pdf", "PDF work")],
        &Config::for_test(root.path().to_path_buf()),
        &judge,
    )
    .await
    .expect("suggestion");

    assert_eq!(result.decision, CandidateDecision::None);
    assert!(result.selected.is_none());
    assert_eq!(
        result.reason_code.as_deref(),
        Some("below_probability_threshold")
    );
}

#[tokio::test]
async fn ranking_handles_empty_prompt_missing_skill_and_no_candidates() {
    let root = tempdir().expect("tempdir");
    let judge = StubJudge {
        response: JudgeResponse::selected("pdf", 0.9, 1, None),
    };
    let config = Config::for_test(root.path().to_path_buf());

    let empty = suggest_with_judge(
        SuggestInput::new("  ".to_owned(), root.path().to_path_buf()),
        vec![],
        &config,
        &judge,
    )
    .await
    .expect_err("empty prompt");
    assert!(matches!(empty, jevx::JevxError::InvalidInput(message) if message.contains("empty")));

    let mut missing = SuggestInput::new("run tests".to_owned(), root.path().to_path_buf());
    missing.explicit_skill = Some("missing".to_owned());
    let error = suggest_with_judge(missing, vec![], &config, &judge)
        .await
        .expect_err("missing skill");
    assert!(
        matches!(error, jevx::JevxError::InvalidInput(message) if message.contains("not found"))
    );

    let no_candidates = suggest_with_judge(
        SuggestInput::new("run tests".to_owned(), root.path().to_path_buf()),
        vec![],
        &config,
        &judge,
    )
    .await
    .expect("no candidates");
    assert_eq!(no_candidates.decision, CandidateDecision::NoCandidates);
    assert_eq!(no_candidates.reason_code.as_deref(), Some("no_candidates"));
}

#[tokio::test]
async fn ranking_handles_margin_none_unknown_missing_and_judge_errors() {
    let root = tempdir().expect("tempdir");
    let skills = vec![
        skill(root.path(), "pdf", "PDF documents"),
        skill(root.path(), "testing", "Run tests"),
    ];
    let config = Config::for_test(root.path().to_path_buf());

    let margin_judge = StubJudge {
        response: JudgeResponse {
            choice: Some("pdf".to_owned()),
            probabilities: [("pdf".to_owned(), 0.7), ("testing".to_owned(), 0.65)]
                .into_iter()
                .collect(),
            response_ms: 1,
            usage: None,
        },
    };
    let margin = suggest_with_judge(
        SuggestInput::new("PDF documents".to_owned(), root.path().to_path_buf()),
        skills.clone(),
        &config,
        &margin_judge,
    )
    .await
    .expect("margin");
    assert_eq!(
        margin.reason_code.as_deref(),
        Some("below_margin_threshold")
    );

    let none_judge = StubJudge {
        response: JudgeResponse {
            choice: Some("none".to_owned()),
            probabilities: [("none".to_owned(), 0.99)].into_iter().collect(),
            response_ms: 1,
            usage: None,
        },
    };
    let none = suggest_with_judge(
        SuggestInput::new("unrelated".to_owned(), root.path().to_path_buf()),
        skills.clone(),
        &config,
        &none_judge,
    )
    .await
    .expect("none");
    assert_eq!(none.reason_code.as_deref(), Some("none_selected"));

    let unknown_judge = StubJudge {
        response: JudgeResponse {
            choice: Some("unknown".to_owned()),
            probabilities: [("unknown".to_owned(), 0.99)].into_iter().collect(),
            response_ms: 1,
            usage: None,
        },
    };
    let unknown = suggest_with_judge(
        SuggestInput::new("unrelated".to_owned(), root.path().to_path_buf()),
        skills.clone(),
        &config,
        &unknown_judge,
    )
    .await
    .expect("unknown");
    assert_eq!(unknown.reason_code.as_deref(), Some("unknown_choice"));

    let missing_judge = StubJudge {
        response: JudgeResponse {
            choice: None,
            probabilities: Default::default(),
            response_ms: 1,
            usage: None,
        },
    };
    let missing = suggest_with_judge(
        SuggestInput::new("unrelated".to_owned(), root.path().to_path_buf()),
        skills.clone(),
        &config,
        &missing_judge,
    )
    .await
    .expect("missing");
    assert_eq!(missing.reason_code.as_deref(), Some("missing_choice"));

    let propagated = suggest_with_judge(
        SuggestInput::new("unrelated".to_owned(), root.path().to_path_buf()),
        skills,
        &config,
        &ErrorJudge,
    )
    .await
    .expect_err("judge error");
    assert!(propagated.to_string().contains("stub failure"));
}

#[tokio::test]
async fn ranking_uses_stable_name_tiebreakers() {
    let root = tempdir().expect("tempdir");
    let judge = StubJudge {
        response: JudgeResponse {
            choice: Some("none".to_owned()),
            probabilities: [("none".to_owned(), 0.99)].into_iter().collect(),
            response_ms: 1,
            usage: None,
        },
    };
    let result = suggest_with_judge(
        SuggestInput::new("unrelated".to_owned(), root.path().to_path_buf()),
        vec![
            skill(root.path(), "zeta", "alpha"),
            skill(root.path(), "alpha", "beta"),
        ],
        &Config::for_test(root.path().to_path_buf()),
        &judge,
    )
    .await
    .expect("ranking");
    assert_eq!(result.candidates[0].name, "alpha");
}

#[tokio::test]
async fn explicit_skill_bypasses_jev() {
    let root = tempdir().expect("tempdir");
    let judge = StubJudge {
        response: JudgeResponse::selected("testing", 0.99, 999, None),
    };
    let mut input = SuggestInput::new("Run the test suite".to_owned(), root.path().to_path_buf());
    input.explicit_skill = Some("pdf".to_owned());

    let result = suggest_with_judge(
        input,
        vec![skill(root.path(), "pdf", "PDF work")],
        &Config::for_test(root.path().to_path_buf()),
        &judge,
    )
    .await
    .expect("suggestion");

    assert_eq!(result.decision, CandidateDecision::Explicit);
    assert_eq!(result.metrics.jev_response_ms, 0);
    assert_eq!(
        result.selected.as_ref().map(|item| item.name.as_str()),
        Some("pdf")
    );
}

#[tokio::test]
async fn optional_judge_allows_explicit_skill_without_a_provider() {
    let root = tempdir().expect("tempdir");
    let mut input = SuggestInput::new("use pdf".to_owned(), root.path().to_path_buf());
    input.explicit_skill = Some("pdf".to_owned());
    let result = suggest_with_optional_judge::<StubJudge>(
        input,
        vec![skill(root.path(), "pdf", "PDF work")],
        &Config::for_test(root.path().to_path_buf()),
        None,
    )
    .await
    .expect("explicit");
    assert_eq!(result.decision, CandidateDecision::Explicit);
}

#[tokio::test]
async fn optional_judge_requires_a_provider_for_automatic_selection() {
    let root = tempdir().expect("tempdir");
    let error = suggest_with_optional_judge::<StubJudge>(
        SuggestInput::new("use pdf".to_owned(), root.path().to_path_buf()),
        vec![skill(root.path(), "pdf", "PDF work")],
        &Config::for_test(root.path().to_path_buf()),
        None,
    )
    .await
    .expect_err("provider required");
    assert!(matches!(error, jevx::JevxError::MissingApiKey));
}

#[test]
fn discovery_reads_frontmatter_and_skips_invalid_files() {
    let root = tempdir().expect("tempdir");
    let good = root.path().join("pdf");
    fs::create_dir_all(&good).expect("mkdir");
    fs::write(
        good.join("SKILL.md"),
        "---\nname: pdf\ndescription: Merge PDF files\n---\nInstructions\n",
    )
    .expect("write skill");
    let bad = root.path().join("broken");
    fs::create_dir_all(&bad).expect("mkdir");
    fs::write(bad.join("SKILL.md"), "not frontmatter").expect("write broken skill");

    let found = jevx::discover_skills(&[root.path().to_path_buf()]).expect("discover");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "pdf");
    assert_eq!(found[0].source, "project");
}

#[test]
fn redaction_and_telemetry_do_not_keep_raw_prompt() {
    let secret = "Use token=sk-live-abcdefghijklmnopqrstuvwxyz123456";
    let redacted = jevx::redact(secret);
    assert!(!redacted.contains("sk-live-"));
    assert!(redacted.contains("<redacted>"));

    let event = TelemetryEvent::from_result(
        "prompt text",
        &CandidateDecision::None,
        None,
        &jevx::Metrics::default(),
    );
    let json = serde_json::to_string(&event).expect("json");
    assert!(!json.contains("prompt text"));
    assert!(json.contains("promptSha256"));
}

#[test]
fn output_json_has_stable_schema_and_response_speed_name() {
    let result = jevx::SuggestionResult::none("none");
    let json = serde_json::to_value(result).expect("json");
    assert_eq!(json["schemaVersion"], 2);
    assert!(json.get("metrics").is_some());
    assert!(json["metrics"].get("jevResponseMs").is_some());
}

#[test]
fn config_uses_jevx_home_and_default_thresholds() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().to_path_buf());
    assert_eq!(config.max_candidates, 32);
    assert_eq!(config.min_probability, 0.60);
    assert_eq!(config.min_margin, 0.10);
    assert_eq!(config.telemetry_path, root.path().join("events.jsonl"));
}

#[test]
fn candidate_local_score_prefers_name_match() {
    let root = tempdir().expect("tempdir");
    let named = skill(root.path(), "pdf", "A document helper");
    let described = skill(root.path(), "documents", "Merge and inspect PDF documents");
    assert!(jevx::local_score("pdf merge", &named) > jevx::local_score("pdf merge", &described));
}

#[test]
fn local_score_covers_exact_description_and_short_bigram_paths() {
    let root = tempdir().expect("tempdir");
    let exact = skill(root.path(), "PDF", "A document helper");
    let described = skill(root.path(), "documents", "Merge and inspect PDF documents");
    let short = skill(root.path(), "x", "zzz");
    assert!(jevx::local_score("pdf", &exact) >= 1_000);
    assert!(jevx::local_score("merge and inspect", &described) >= 60);
    assert_eq!(jevx::local_score("a", &short), 0);
}

#[test]
fn json_input_round_trip_is_supported() {
    let input =
        SuggestInput::from_json(r#"{"prompt":"run tests","cwd":"/tmp/project"}"#).expect("input");
    assert_eq!(input.prompt, "run tests");
    assert_eq!(input.cwd, std::path::PathBuf::from("/tmp/project"));
}

#[test]
fn json_input_rejects_empty_and_malformed_payloads() {
    let empty = SuggestInput::from_json(r#"{"prompt":"  "}"#).expect_err("empty");
    assert!(matches!(empty, jevx::JevxError::InvalidInput(message) if message.contains("empty")));
    let malformed = SuggestInput::from_json("not json").expect_err("malformed");
    assert!(matches!(malformed, jevx::JevxError::Json(_)));
}

#[test]
fn serialized_types_accept_aliases_and_preserve_optional_fields() {
    let input = SuggestInput::from_json(r#"{"prompt":"run tests","explicit_skill":"testing"}"#)
        .expect("input");
    assert_eq!(input.cwd, std::path::PathBuf::from("."));
    assert_eq!(input.explicit_skill.as_deref(), Some("testing"));
    let usage: Usage =
        serde_json::from_str(r#"{"input_tokens":4,"output_tokens":2}"#).expect("usage");
    assert_eq!(
        usage,
        Usage {
            input_tokens: 4,
            output_tokens: 2
        }
    );
    let response = JudgeResponse::selected("none", 0.9, 1, None);
    assert_eq!(response.choice.as_deref(), Some("none"));
}

#[test]
fn selection_result_can_be_shared_across_threads() {
    let result = Arc::new(jevx::SuggestionResult::none("none"));
    let clone = Arc::clone(&result);
    assert_eq!(clone.decision, CandidateDecision::None);
}

#[test]
fn errors_display_and_from_conversions_are_stable() {
    let io_error: jevx::JevxError = std::io::Error::other("disk").into();
    let json_error: jevx::JevxError = serde_json::from_str::<serde_json::Value>("{")
        .expect_err("json")
        .into();
    let yaml_error: jevx::JevxError = serde_yaml::from_str::<serde_yaml::Value>("[")
        .expect_err("yaml")
        .into();
    assert!(io_error.to_string().contains("I/O error"));
    assert!(json_error.to_string().contains("JSON error"));
    assert!(yaml_error.to_string().contains("YAML error"));
    assert_eq!(
        jevx::JevxError::Timeout.to_string(),
        "Jev request timed out"
    );
    assert_eq!(
        jevx::JevxError::MissingApiKey.to_string(),
        "AI_GATEWAY_API_KEY is not configured"
    );
    assert_eq!(
        jevx::JevxError::Provider("provider".to_owned()).to_string(),
        "provider"
    );
    assert_eq!(
        jevx::JevxError::ProviderWithMetrics {
            message: "provider".to_owned(),
            calls: 2,
            retries: 1,
            response_ms: 4,
        }
        .to_string(),
        "provider"
    );
    assert_eq!(
        jevx::JevxError::InvalidInput("invalid".to_owned()).to_string(),
        "invalid"
    );
}

#[test]
fn telemetry_is_appendable_and_stats_are_aggregated() {
    let root = tempdir().expect("tempdir");
    let path = root.path().join("events.jsonl");
    let selected = TelemetryEvent::from_result(
        "one",
        &CandidateDecision::Selected,
        None,
        &jevx::Metrics {
            jev_response_ms: 10,
            ..jevx::Metrics::default()
        },
    );
    let none = TelemetryEvent::from_result(
        "two",
        &CandidateDecision::None,
        None,
        &jevx::Metrics {
            jev_response_ms: 20,
            ..jevx::Metrics::default()
        },
    );
    append_telemetry(&path, &selected).expect("append selected");
    append_telemetry(&path, &none).expect("append none");

    let stats = read_stats(&path).expect("stats");
    assert_eq!(stats.events, 2);
    assert_eq!(stats.selected, 1);
    assert_eq!(stats.none, 1);
    assert_eq!(stats.average_jev_response_ms, 15.0);
}

#[test]
fn telemetry_aggregates_costs_and_keeps_unknown_distinct_from_zero() {
    let root = tempdir().expect("tempdir");
    let path = root.path().join("events.jsonl");
    let cost = CostSummary {
        jev: CostEstimate::available(0.25, Some("USD".to_owned()), Some("fixture-1".to_owned())),
        codex: CostEstimate::unknown(Some("USD".to_owned()), Some("fixture-1".to_owned())),
        total: CostEstimate::unknown(Some("USD".to_owned()), Some("fixture-1".to_owned())),
    };
    let event = TelemetryEvent::from_result(
        "cost-prompt",
        &CandidateDecision::Selected,
        None,
        &jevx::Metrics {
            cost: Some(cost),
            ..jevx::Metrics::default()
        },
    );
    append_telemetry(&path, &event).expect("append cost event");

    let stats = read_stats(&path).expect("stats");
    assert_eq!(stats.jev_cost, Some(0.25));
    assert_eq!(stats.codex_cost, None);
    assert_eq!(stats.total_cost, None);
    assert_eq!(stats.cost_status_counts.get("codex:unknown"), Some(&1));
    assert_eq!(
        CostStatus::Unknown,
        event.metrics.cost.expect("cost").codex.status
    );
}

#[test]
fn telemetry_handles_missing_empty_and_error_events() {
    let root = tempdir().expect("tempdir");
    let missing = root.path().join("missing.jsonl");
    assert_eq!(read_stats(&missing).expect("missing").events, 0);
    let path = root.path().join("events.jsonl");
    let error = TelemetryEvent::from_result(
        "error",
        &CandidateDecision::Error,
        None,
        &jevx::Metrics::default(),
    );
    append_telemetry(&path, &error).expect("append");
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open")
        .write_all(b"\n")
        .expect("blank line");
    let stats = read_stats(&path).expect("stats");
    assert_eq!(stats.errors, 1);
}

#[test]
fn telemetry_reports_open_error_for_an_empty_path() {
    let event = TelemetryEvent::from_result(
        "prompt",
        &CandidateDecision::None,
        None,
        &jevx::Metrics::default(),
    );
    assert!(append_telemetry(Path::new(""), &event).is_err());
}

#[test]
fn telemetry_keeps_explicit_and_selected_skill_ids() {
    let root = tempdir().expect("tempdir");
    let candidate = SkillRecord::new(
        "pdf".to_owned(),
        "PDF".to_owned(),
        root.path().join("SKILL.md"),
        "project".to_owned(),
    );
    let selected = jevx::CandidateResult::from_skill(&candidate, 1, Some(0.9));
    let event = TelemetryEvent::from_result(
        "prompt",
        &CandidateDecision::Selected,
        Some(&selected),
        &jevx::Metrics::default(),
    );
    assert_eq!(event.selected_skill.as_deref(), Some("pdf"));
}

#[test]
fn duplicate_skill_names_use_higher_priority_root() {
    let root = tempdir().expect("tempdir");
    let project = root.path().join("project");
    let user = root.path().join("user");
    for (directory, description) in [(&project, "project version"), (&user, "user version")] {
        let skill_dir = directory.join("same");
        fs::create_dir_all(&skill_dir).expect("mkdir");
        fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: same\ndescription: {description}\n---\n"),
        )
        .expect("write skill");
    }
    let roots = [
        jevx::SkillRoot::new(project, "project".to_owned(), 0),
        jevx::SkillRoot::new(user, "user".to_owned(), 1),
    ];
    let skills = jevx::discover_skill_roots(&roots).expect("discover");
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].description, "project version");
}

#[test]
fn invalid_frontmatter_fields_are_skipped() {
    let root = tempdir().expect("tempdir");
    let path = root.path().join("SKILL.md");
    fs::write(&path, "---\nname: only-name\n---\n").expect("write");
    assert!(
        jevx::parse_skill_file(&path, "project")
            .expect("parse")
            .is_none()
    );
    fs::write(&path, "---\ndescription: only-description\n---\n").expect("write");
    assert!(
        jevx::parse_skill_file(&path, "project")
            .expect("parse")
            .is_none()
    );
    fs::write(&path, "---\nname: missing-end\ndescription: broken\n").expect("write");
    assert!(
        jevx::parse_skill_file(&path, "project")
            .expect("parse")
            .is_none()
    );
    fs::write(&path, "---\nname: [broken\ndescription: yaml\n---\n").expect("write");
    assert!(matches!(
        jevx::parse_skill_file(&path, "project"),
        Err(jevx::JevxError::Yaml(_))
    ));
}

#[test]
fn discovery_skips_hidden_and_target_directories_and_missing_roots() {
    let root = tempdir().expect("tempdir");
    for directory in [root.path().join(".hidden"), root.path().join("target")] {
        let skill_dir = directory.join("ignored");
        fs::create_dir_all(&skill_dir).expect("mkdir");
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: ignored\ndescription: ignored\n---\n",
        )
        .expect("write");
    }
    let file = root.path().join("SKILL.md");
    fs::write(&file, "---\nname: direct\ndescription: direct\n---\n").expect("write");
    let found = jevx::discover_skills(&[
        root.path().join("does-not-exist"),
        file.clone(),
        root.path().to_path_buf(),
    ])
    .expect("discover");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "direct");
}

#[test]
fn redaction_handles_multiple_secret_shapes() {
    let value = "api_key=abc secret=xyz password=pwd Bearer abc token: Bearer separated-token-bearer password Bearer separated-password-bearer sk-test tsk-test explain basic auth API_KEY: separated-api password: separated-password Authorization: Bearer separated-bearer Authorization: Basic separated-basic password = separated-space \"token\":\"json-secret\" Authorization:Bearer embedded-bearer Authorization=Basic embedded-basic";
    let redacted = jevx::redact(value);
    for secret in [
        "abc",
        "xyz",
        "pwd",
        "separated-token-bearer",
        "separated-password-bearer",
        "separated-api",
        "separated-password",
        "separated-bearer",
        "separated-basic",
        "separated-space",
        "json-secret",
        "embedded-bearer",
        "embedded-basic",
        "sk-test",
        "tsk-test",
    ] {
        assert!(!redacted.contains(secret), "secret leaked: {secret}");
    }
    assert!(redacted.contains("explain basic auth"));
    assert!(redacted.matches("<redacted>").count() >= 8);
}

#[tokio::test]
async fn gateway_judge_parses_successful_gateway_response() {
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", server.local_addr().expect("address"));
    let handle = thread::spawn(move || {
        let (mut stream, _) = server.accept().expect("accept");
        read_request(&mut stream);
        let body = r#"{"answers":{"skill":{"choice":"pdf","probabilities":{"pdf":0.9,"none":0.1}}},"usage":{"inputTokens":5,"outputTokens":2}}"#;
        write_http_response(&mut stream, "200 OK", body);
    });
    let mut config = Config::for_test(tempdir().expect("tempdir").path().to_path_buf());
    config.endpoint = endpoint;
    let judge = GatewayJudge::from_config(&config).expect("judge");
    let response = judge
        .evaluate(JudgeRequest {
            state: "{}".to_owned(),
            candidates: vec![jevx::JudgeCandidate {
                id: "pdf".to_owned(),
                name: "pdf".to_owned(),
                description: "PDF".to_owned(),
            }],
        })
        .await
        .expect("response");
    handle.join().expect("server");
    assert_eq!(response.choice.as_deref(), Some("pdf"));
    assert_eq!(response.usage.expect("usage").input_tokens, 5);
}

#[tokio::test]
async fn gateway_judge_retries_retryable_status_and_reports_retry_count() {
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", server.local_addr().expect("address"));
    let handle = thread::spawn(move || {
        let (mut first, _) = server.accept().expect("first accept");
        read_request(&mut first);
        write_http_response(
            &mut first,
            "429 Too Many Requests",
            r#"{"error":{"message":"retry"}}"#,
        );
        let (mut second, _) = server.accept().expect("second accept");
        read_request(&mut second);
        write_http_response(
            &mut second,
            "200 OK",
            r#"{"answers":{"skill":{"choice":"pdf","probabilities":{"pdf":0.9,"none":0.1}}}}"#,
        );
    });
    let mut config = Config::for_test(tempdir().expect("tempdir").path().to_path_buf());
    config.endpoint = endpoint;
    config.max_retries = 1;
    config.retry_backoff_ms = 0;
    let judge = GatewayJudge::from_config(&config).expect("judge");
    let response = jevx::Judge::evaluate_with_metrics(
        &judge,
        JudgeRequest {
            state: "{}".to_owned(),
            candidates: vec![jevx::JudgeCandidate {
                id: "pdf".to_owned(),
                name: "pdf".to_owned(),
                description: "PDF".to_owned(),
            }],
        },
    )
    .await
    .expect("retried response");
    handle.join().expect("server");
    assert_eq!(response.retries, 1);
    assert_eq!(response.calls, 2);
    assert_eq!(response.response.choice.as_deref(), Some("pdf"));
}

#[tokio::test]
async fn gateway_retries_stay_within_the_request_timeout_budget() {
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", server.local_addr().expect("address"));
    let handle = thread::spawn(move || {
        let (mut stream, _) = server.accept().expect("accept");
        read_request(&mut stream);
        write_http_response(
            &mut stream,
            "429 Too Many Requests",
            r#"{"error":{"message":"slow down"}}"#,
        );
    });
    let mut config = Config::for_test(tempdir().expect("tempdir").path().to_path_buf());
    config.endpoint = endpoint;
    config.max_retries = 3;
    config.retry_backoff_ms = 1_000;
    config.timeout = std::time::Duration::from_millis(300);
    let judge = GatewayJudge::from_config(&config).expect("judge");
    let started = std::time::Instant::now();
    let error = jevx::Judge::evaluate_with_metrics(
        &judge,
        JudgeRequest {
            state: "{}".to_owned(),
            candidates: vec![jevx::JudgeCandidate {
                id: "pdf".to_owned(),
                name: "pdf".to_owned(),
                description: "PDF".to_owned(),
            }],
        },
    )
    .await
    .expect_err("retry would exceed the deadline");
    handle.join().expect("server");
    assert!(
        started.elapsed() < std::time::Duration::from_millis(900),
        "the whole call must respect the request timeout, took {:?}",
        started.elapsed()
    );
    match error {
        jevx::JevxError::ProviderWithMetrics { calls, retries, .. } => {
            assert_eq!(calls, 1);
            assert_eq!(retries, 0);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn gateway_retry_failure_preserves_attempt_metrics_in_the_receipt() {
    let root = tempdir().expect("tempdir");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", server.local_addr().expect("address"));
    let handle = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = server.accept().expect("accept");
            read_request(&mut stream);
            write_http_response(
                &mut stream,
                "429 Too Many Requests",
                r#"{"error":{"message":"retry exhausted"}}"#,
            );
        }
    });
    let mut config = Config::for_test(root.path().join("data"));
    config.endpoint = endpoint;
    config.max_retries = 1;
    config.retry_backoff_ms = 0;
    config.telemetry_enabled = true;
    let judge = GatewayJudge::from_config(&config).expect("judge");
    let result = suggest_with_judge(
        SuggestInput::new("PDFを確認したい".to_owned(), root.path().to_path_buf()),
        vec![skill(root.path(), "pdf", "PDF")],
        &config,
        &judge,
    )
    .await;
    handle.join().expect("server");
    assert!(result.is_err());
    let receipts = read_decision_receipts(&config.decision_receipt_path).expect("receipts");
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].calls, 2);
    assert_eq!(receipts[0].retries, 1);
    assert_eq!(receipts[0].error_code.as_deref(), Some("provider_error"));
}

#[tokio::test]
async fn gateway_judge_parses_score_predicate_and_rejects_malformed_typed_answers() {
    let cases = [
        (
            QuestionSpec::Score {
                instructions: "score".to_owned(),
                criteria: vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()],
            },
            r#"{"answers":{"severity":{"score":2.0,"probabilities":{"0":0.1,"1":0.2,"2":0.7},"confidence":0.9}}}"#,
            true,
        ),
        (
            QuestionSpec::Predicate {
                instructions: "predicate".to_owned(),
                criteria: jevx::PredicateCriteria {
                    true_criteria: "true".to_owned(),
                    false_criteria: "false".to_owned(),
                },
            },
            r#"{"answers":{"danger":{"noul":0.9}}}"#,
            true,
        ),
        (
            QuestionSpec::Choice {
                instructions: "choice".to_owned(),
                criteria: BTreeMap::from([("pdf".to_owned(), "PDF".to_owned())]),
            },
            r#"{"answers":{"skill":{"probabilities":{"pdf":1.0}}}}"#,
            false,
        ),
        (
            QuestionSpec::Choice {
                instructions: "choice".to_owned(),
                criteria: BTreeMap::from([("pdf".to_owned(), "PDF".to_owned())]),
            },
            r#"{"answers":{"skill":{"choice":"pdf"}}}"#,
            false,
        ),
        (
            QuestionSpec::Choice {
                instructions: "choice".to_owned(),
                criteria: BTreeMap::from([("pdf".to_owned(), "PDF".to_owned())]),
            },
            r#"{"answers":{"skill":{"choice":"pdf","probabilities":{"pdf":0.7,"none":"invalid"}}}}"#,
            false,
        ),
        (
            QuestionSpec::Score {
                instructions: "score".to_owned(),
                criteria: vec!["low".to_owned(), "high".to_owned()],
            },
            r#"{"answers":{"severity":{"score":1.0,"confidence":0.9}}}"#,
            false,
        ),
    ];
    for (question, body, succeeds) in cases {
        let server = TcpListener::bind("127.0.0.1:0").expect("bind");
        let endpoint = format!("http://{}", server.local_addr().expect("address"));
        let body = body.to_owned();
        let handle = thread::spawn(move || {
            let (mut stream, _) = server.accept().expect("accept");
            read_request(&mut stream);
            write_http_response(&mut stream, "200 OK", &body);
        });
        let mut config = Config::for_test(tempdir().expect("tempdir").path().to_path_buf());
        config.endpoint = endpoint;
        let judge = GatewayJudge::from_config(&config).expect("judge");
        let question_id = match &question {
            QuestionSpec::Score { .. } => "severity",
            QuestionSpec::Predicate { .. } => "danger",
            QuestionSpec::Choice { .. } => "skill",
        };
        let result = jevx::decision::DecisionJudge::evaluate(
            &judge,
            DecisionRequest {
                state: "{}".to_owned(),
                questions: BTreeMap::from([(question_id.to_owned(), question)]),
            },
        )
        .await;
        handle.join().expect("server");
        assert_eq!(result.is_ok(), succeeds);
    }

    let config = Config::for_test(tempdir().expect("tempdir").path().to_path_buf());
    let judge = GatewayJudge::from_config(&config).expect("judge");
    let invalid_question = jevx::decision::DecisionJudge::evaluate(
        &judge,
        DecisionRequest {
            state: "{}".to_owned(),
            questions: BTreeMap::from([(
                "severity".to_owned(),
                QuestionSpec::Score {
                    instructions: "score".to_owned(),
                    criteria: vec!["only".to_owned()],
                },
            )]),
        },
    )
    .await;
    assert!(matches!(
        invalid_question,
        Err(jevx::JevxError::InvalidInput(message)) if message.contains("2 to 10")
    ));
}

#[tokio::test]
async fn gateway_judge_handles_malformed_and_default_error_payloads() {
    for (status, body, expected) in [
        ("200 OK", "not json", "読み取れませんでした"),
        (
            "500 Internal Server Error",
            r#"{"error":{}}"#,
            "評価に失敗しました",
        ),
    ] {
        let server = TcpListener::bind("127.0.0.1:0").expect("bind");
        let endpoint = format!("http://{}", server.local_addr().expect("address"));
        let status = status.to_owned();
        let body = body.to_owned();
        let handle = thread::spawn(move || {
            let (mut stream, _) = server.accept().expect("accept");
            read_request(&mut stream);
            write_http_response(&mut stream, &status, &body);
        });
        let mut config = Config::for_test(tempdir().expect("tempdir").path().to_path_buf());
        config.endpoint = endpoint;
        let judge = GatewayJudge::from_config(&config).expect("judge");
        let error = judge
            .evaluate(JudgeRequest {
                state: "{}".to_owned(),
                candidates: vec![],
            })
            .await
            .expect_err("error");
        handle.join().expect("server");
        assert!(error.to_string().contains(expected));
    }
}

#[tokio::test]
async fn gateway_judge_records_response_read_failure_as_provider_error() {
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", server.local_addr().expect("address"));
    let handle = thread::spawn(move || {
        let (mut stream, _) = server.accept().expect("accept");
        read_request(&mut stream);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}",
            )
            .expect("write truncated response");
    });
    let mut config = Config::for_test(tempdir().expect("tempdir").path().to_path_buf());
    config.endpoint = endpoint;
    let judge = GatewayJudge::from_config(&config).expect("judge");
    let error = judge
        .evaluate(JudgeRequest {
            state: "{}".to_owned(),
            candidates: vec![],
        })
        .await
        .expect_err("truncated response should fail");
    handle.join().expect("server");
    assert!(matches!(
        error,
        jevx::JevxError::ProviderWithMetrics { calls: 1, .. }
    ));
}

#[tokio::test]
async fn gateway_judge_reports_error_payloads() {
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", server.local_addr().expect("address"));
    let handle = thread::spawn(move || {
        let (mut stream, _) = server.accept().expect("accept");
        read_request(&mut stream);
        let body = r#"{"error":{"message":"rate limited"}}"#;
        write_http_response(&mut stream, "429 Too Many Requests", body);
    });
    let mut config = Config::for_test(tempdir().expect("tempdir").path().to_path_buf());
    config.endpoint = endpoint;
    let judge = GatewayJudge::from_config(&config).expect("judge");
    let error = judge
        .evaluate(JudgeRequest {
            state: "{}".to_owned(),
            candidates: vec![],
        })
        .await
        .expect_err("error");
    handle.join().expect("server");
    assert!(error.to_string().contains("rate limited"));
}

#[tokio::test]
async fn gateway_judge_rejects_missing_skill_answer() {
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", server.local_addr().expect("address"));
    let handle = thread::spawn(move || {
        let (mut stream, _) = server.accept().expect("accept");
        read_request(&mut stream);
        write_http_response(&mut stream, "200 OK", r#"{"answers":{}}"#);
    });
    let mut config = Config::for_test(tempdir().expect("tempdir").path().to_path_buf());
    config.endpoint = endpoint;
    let judge = GatewayJudge::from_config(&config).expect("judge");
    let error = judge
        .evaluate(JudgeRequest {
            state: "{}".to_owned(),
            candidates: vec![],
        })
        .await
        .expect_err("error");
    handle.join().expect("server");
    assert!(error.to_string().contains("skill"));
}

#[test]
fn binary_reports_missing_key_as_json_error() {
    let binary = env!("CARGO_BIN_EXE_jevx");
    let root = tempdir().expect("tempdir");
    let eval_skills = Path::new(env!("CARGO_MANIFEST_DIR")).join("evals/skills");
    let output = Command::new(binary)
        .args(["skills", "suggest", "--prompt", "run tests", "--json"])
        .arg("--skill-dir")
        .arg(eval_skills)
        .env("AI_GATEWAY_API_KEY", "")
        .env("JEVX_HOME", root.path())
        // Receipts are written only when telemetry is on; do not depend on the caller's environment.
        .env("JEVX_TELEMETRY", "1")
        .output()
        .expect("run binary");
    assert_eq!(output.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(json["error"]["code"], "missing_api_key");
    let receipts = jevx::read_decision_receipts(&root.path().join("decisions.jsonl"))
        .expect("decision receipts");
    assert!(receipts.iter().any(|receipt| {
        receipt.decision == jevx::DecisionStatus::Defer
            && receipt.fallback == Some(jevx::FallbackReason::MissingApiKey)
    }));
}

#[test]
fn binary_reports_invalid_input_on_stderr() {
    let binary = env!("CARGO_BIN_EXE_jevx");
    let output = Command::new(binary)
        .args(["skills", "suggest"])
        .env("AI_GATEWAY_API_KEY", "")
        .output()
        .expect("run binary");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("one of --prompt, --stdin, or --input-json is required"));
}

fn write_http_response(stream: &mut std::net::TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("write response");
}

fn read_request(stream: &mut std::net::TcpStream) {
    let mut request = [0_u8; 4096];
    loop {
        let read = stream.read(&mut request).expect("read request");
        if read == 0
            || request[..read]
                .windows(4)
                .any(|window| window == b"\r\n\r\n")
        {
            break;
        }
    }
}
