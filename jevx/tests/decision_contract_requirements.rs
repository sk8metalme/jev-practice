use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use jevx::decision::{
    DecisionContract, DecisionFailure, DecisionJudge, DecisionMode, DecisionPolicy, DecisionStatus,
    FallbackReason, InMemoryDecisionCache, LegacyJudgeAdapter, QuestionSpec, Severity, StatePlan,
    TypedAnswer, execute_contract,
};
use jevx::recorder::{
    DecisionReceipt, DecisionRecorder, JsonlDecisionRecorder, read_decision_receipts,
    read_decision_stats, replay_receipt,
};
use jevx::{
    CandidateDecision, Config, DecisionExecutionOptions, DecisionRequest, DecisionResponse,
    JevxError, Judge, JudgeRequest, JudgeResponse, SkillRecord, SuggestInput, Usage,
    suggest_with_judge,
};
use serde_json::json;
use tempfile::tempdir;

struct StubJudge {
    response: DecisionResponse,
}

struct RecordingLegacyJudge {
    response: JudgeResponse,
    calls: Arc<AtomicUsize>,
    states: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone, Copy)]
enum DecisionErrorKind {
    MissingApiKey,
    Timeout,
    Provider,
    InvalidInput,
    Io,
    Json,
    Yaml,
}

struct ErrorDecisionJudge {
    kind: DecisionErrorKind,
}

#[async_trait]
impl DecisionJudge for StubJudge {
    async fn evaluate(&self, _request: DecisionRequest) -> Result<DecisionResponse, JevxError> {
        Ok(self.response.clone())
    }
}

#[async_trait]
impl Judge for RecordingLegacyJudge {
    async fn evaluate(&self, request: JudgeRequest) -> Result<JudgeResponse, JevxError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.states.lock().expect("states lock").push(request.state);
        Ok(self.response.clone())
    }
}

#[async_trait]
impl DecisionJudge for ErrorDecisionJudge {
    async fn evaluate(&self, _request: DecisionRequest) -> Result<DecisionResponse, JevxError> {
        Err(match self.kind {
            DecisionErrorKind::MissingApiKey => JevxError::MissingApiKey,
            DecisionErrorKind::Timeout => JevxError::Timeout,
            DecisionErrorKind::Provider => JevxError::Provider("provider".to_owned()),
            DecisionErrorKind::InvalidInput => JevxError::InvalidInput("invalid".to_owned()),
            DecisionErrorKind::Io => JevxError::Io(std::io::Error::other("io")),
            DecisionErrorKind::Json => {
                JevxError::Json(serde_json::from_str::<serde_json::Value>("{").unwrap_err())
            }
            DecisionErrorKind::Yaml => {
                JevxError::Yaml(serde_yaml::from_str::<serde_yaml::Value>("[").unwrap_err())
            }
        })
    }
}

fn skill(root: &Path, id: &str, description: &str) -> SkillRecord {
    SkillRecord::new(
        id.to_owned(),
        description.to_owned(),
        root.join(id).join("SKILL.md"),
        "test".to_owned(),
    )
}

fn choice_contract() -> DecisionContract {
    DecisionContract::choice(
        "skill",
        "skill-choice.v1",
        BTreeMap::from([
            ("pdf".to_owned(), "PDF work".to_owned()),
            ("testing".to_owned(), "Test work".to_owned()),
            ("none".to_owned(), "No candidate fits".to_owned()),
        ]),
        0.60,
        0.10,
    )
}

fn state() -> StatePlan {
    StatePlan::from_value(
        json!({"prompt":"redacted prompt","candidates":["pdf"]}),
        1,
        1,
        Vec::new(),
        vec!["secret_pattern".to_owned()],
    )
    .expect("state")
}

#[test]
fn choice_gate_is_code_owned_and_fail_closed() {
    let contract = choice_contract();

    let accepted = contract.evaluate(Some(TypedAnswer::Choice {
        choice: "pdf".to_owned(),
        probabilities: BTreeMap::from([
            ("pdf".to_owned(), 0.87),
            ("testing".to_owned(), 0.08),
            ("none".to_owned(), 0.05),
        ]),
        confidence: Some(0.87),
    }));
    assert_eq!(accepted.status, DecisionStatus::Accepted);

    let below_margin = contract.evaluate(Some(TypedAnswer::Choice {
        choice: "pdf".to_owned(),
        probabilities: BTreeMap::from([
            ("pdf".to_owned(), 0.70),
            ("testing".to_owned(), 0.65),
            ("none".to_owned(), 0.05),
        ]),
        confidence: Some(0.70),
    }));
    assert_eq!(below_margin.status, DecisionStatus::Defer);

    let unknown = contract.evaluate(Some(TypedAnswer::Choice {
        choice: "unknown".to_owned(),
        probabilities: BTreeMap::from([("unknown".to_owned(), 0.99)]),
        confidence: Some(0.99),
    }));
    assert_eq!(unknown.status, DecisionStatus::Unknown);
    assert!(unknown.answer.is_none());

    let none = contract.evaluate(Some(TypedAnswer::Choice {
        choice: "none".to_owned(),
        probabilities: BTreeMap::from([("none".to_owned(), 0.99)]),
        confidence: Some(0.99),
    }));
    assert_eq!(none.status, DecisionStatus::None);
}

#[tokio::test]
async fn typed_contract_validates_all_shapes_and_failure_reasons() {
    let choice = choice_contract();
    assert!(choice.question.is_choice());
    assert!(
        !QuestionSpec::Score {
            instructions: "levels".to_owned(),
            criteria: vec!["low".to_owned(), "high".to_owned()],
        }
        .is_choice()
    );
    assert!(
        QuestionSpec::Choice {
            instructions: String::new(),
            criteria: BTreeMap::new(),
        }
        .validate()
        .is_err()
    );
    assert!(
        QuestionSpec::Score {
            instructions: "score".to_owned(),
            criteria: vec!["only".to_owned()],
        }
        .validate()
        .is_err()
    );
    assert!(
        QuestionSpec::Score {
            instructions: "score".to_owned(),
            criteria: (0..11).map(|index| index.to_string()).collect(),
        }
        .validate()
        .is_err()
    );
    assert!(
        QuestionSpec::Predicate {
            instructions: "predicate".to_owned(),
            criteria: jevx::PredicateCriteria {
                true_criteria: String::new(),
                false_criteria: "false".to_owned(),
            },
        }
        .validate()
        .is_err()
    );

    let score = DecisionContract::score(
        "severity",
        "impact.v1",
        vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()],
        1.5,
        0.6,
    );
    assert!(score.question.validate().is_ok());
    assert_eq!(score.policy.threshold(), 1.5);
    assert_eq!(score.severity(), Severity::Advisory);
    let predicate = DecisionContract::predicate("danger", "danger.v1", "true", "false", 0.75, 0.25);
    assert_eq!(predicate.policy.threshold(), 0.75);
    assert_eq!(predicate.severity(), Severity::Advisory);
    assert_eq!(choice.policy.threshold(), 0.6);
    assert!(choice.validate().is_ok());
    assert_eq!(
        DecisionPolicy::Predicate {
            true_threshold: 0.8,
            false_threshold: 0.2,
            severity: Severity::High,
        }
        .threshold(),
        0.8
    );

    let malformed_probability = choice.evaluate(Some(TypedAnswer::Choice {
        choice: "pdf".to_owned(),
        probabilities: BTreeMap::new(),
        confidence: Some(0.9),
    }));
    assert_eq!(malformed_probability.status, DecisionStatus::Unknown);
    assert_eq!(
        malformed_probability.fallback,
        Some(FallbackReason::MalformedAnswer)
    );
    let invalid_probability = choice.evaluate(Some(TypedAnswer::Choice {
        choice: "pdf".to_owned(),
        probabilities: BTreeMap::from([("pdf".to_owned(), f64::NAN)]),
        confidence: Some(0.9),
    }));
    assert_eq!(invalid_probability.status, DecisionStatus::Unknown);
    let wrong_type = choice.evaluate(Some(TypedAnswer::Score {
        score: 1.0,
        probabilities: BTreeMap::from([("0".to_owned(), 1.0)]),
        confidence: 0.9,
    }));
    assert_eq!(wrong_type.status, DecisionStatus::Unknown);

    let invalid_score = score.evaluate(Some(TypedAnswer::Score {
        score: f64::NAN,
        probabilities: BTreeMap::from([("0".to_owned(), 1.0)]),
        confidence: 0.9,
    }));
    assert_eq!(invalid_score.status, DecisionStatus::Unknown);
    let low_confidence = score.evaluate(Some(TypedAnswer::Score {
        score: 2.0,
        probabilities: BTreeMap::from([("2".to_owned(), 1.0)]),
        confidence: 0.5,
    }));
    assert_eq!(low_confidence.status, DecisionStatus::Defer);
    let low_score = score.evaluate(Some(TypedAnswer::Score {
        score: 0.0,
        probabilities: BTreeMap::from([("0".to_owned(), 1.0)]),
        confidence: 0.9,
    }));
    assert_eq!(low_score.status, DecisionStatus::None);
    let invalid_predicate = predicate.evaluate(Some(TypedAnswer::Predicate { noul: f64::NAN }));
    assert_eq!(invalid_predicate.status, DecisionStatus::Unknown);

    assert_eq!(
        choice.failure(DecisionFailure::DryRun).status,
        DecisionStatus::Defer
    );
    assert_eq!(
        choice.failure(DecisionFailure::Timeout).status,
        DecisionStatus::Defer
    );
    assert_eq!(
        choice.failure(DecisionFailure::Provider).fallback,
        Some(FallbackReason::ProviderError)
    );
    let mut mismatched = choice.clone();
    mismatched.policy = DecisionPolicy::Score {
        min_score: 1.0,
        min_confidence: 0.5,
        severity: Severity::Advisory,
    };
    let mismatch = mismatched.evaluate(Some(TypedAnswer::Choice {
        choice: "pdf".to_owned(),
        probabilities: BTreeMap::from([("pdf".to_owned(), 0.9)]),
        confidence: Some(0.9),
    }));
    assert_eq!(mismatch.status, DecisionStatus::Unknown);

    let mut invalid_policy = choice.clone();
    invalid_policy.policy = DecisionPolicy::Choice {
        min_probability: f64::NAN,
        min_margin: -0.1,
        severity: Severity::Advisory,
    };
    assert!(invalid_policy.validate().is_err());
    let invalid_result = invalid_policy.evaluate(Some(TypedAnswer::Choice {
        choice: "pdf".to_owned(),
        probabilities: BTreeMap::from([("pdf".to_owned(), 1.0)]),
        confidence: Some(1.0),
    }));
    assert_eq!(invalid_result.status, DecisionStatus::Unknown);
    assert_eq!(invalid_result.fallback, Some(FallbackReason::InvalidPolicy));
    let invalid_execution = execute_contract(
        &invalid_policy,
        &state(),
        Some(&StubJudge {
            response: DecisionResponse {
                answers: BTreeMap::new(),
                response_ms: 0,
                usage: None,
                calls: 1,
                retries: 0,
            },
        }),
        &DecisionExecutionOptions::default(),
    )
    .await;
    assert_eq!(
        invalid_execution.error_code.as_deref(),
        Some("invalid_input")
    );

    let invalid_predicate =
        DecisionContract::predicate("danger", "danger.v1", "true", "false", 0.2, 0.8);
    assert!(invalid_predicate.validate().is_err());
}

#[test]
fn score_and_predicate_have_typed_safe_boundaries() {
    let score = DecisionContract::score(
        "severity",
        "impact.v1",
        vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()],
        1.5,
        0.60,
    );
    let score_result = score.evaluate(Some(TypedAnswer::Score {
        score: 2.0,
        confidence: 0.90,
        probabilities: BTreeMap::from([
            ("0".to_owned(), 0.05),
            ("1".to_owned(), 0.10),
            ("2".to_owned(), 0.85),
        ]),
    }));
    assert_eq!(score_result.status, DecisionStatus::Accepted);

    let predicate = DecisionContract::predicate(
        "dangerous",
        "danger.v1",
        "danger present",
        "danger absent",
        0.75,
        0.25,
    );
    let ambiguous = predicate.evaluate(Some(TypedAnswer::Predicate { noul: 0.50 }));
    assert_eq!(ambiguous.status, DecisionStatus::Defer);
    let accepted_false = predicate.evaluate(Some(TypedAnswer::Predicate { noul: 0.10 }));
    assert_eq!(accepted_false.status, DecisionStatus::Accepted);
}

#[tokio::test]
async fn execution_replay_and_cache_never_call_jev_for_replay_or_hit() {
    let root = tempdir().expect("tempdir");
    let contract = choice_contract();
    let state = state();
    let answer = TypedAnswer::Choice {
        choice: "pdf".to_owned(),
        probabilities: BTreeMap::from([("pdf".to_owned(), 0.90)]),
        confidence: Some(0.90),
    };
    let judge = StubJudge {
        response: DecisionResponse {
            answers: BTreeMap::from([("skill".to_owned(), answer.clone())]),
            response_ms: 12,
            usage: Some(Usage {
                input_tokens: 10,
                output_tokens: 2,
            }),
            calls: 1,
            retries: 0,
        },
    };
    let cache = Arc::new(InMemoryDecisionCache::new(4));
    let options = DecisionExecutionOptions {
        mode: DecisionMode::Live,
        cache: Some(cache.clone()),
    };
    let first = execute_contract(&contract, &state, Some(&judge), &options).await;
    assert_eq!(first.result.status, DecisionStatus::Accepted);
    cache.put(&contract.cache_key(&state.state_digest), &answer);
    let cached = execute_contract(&contract, &state, Some(&judge), &options).await;
    assert!(cached.cache_hit);
    assert_eq!(cached.calls, 0);

    let receipt = DecisionReceipt::from_execution(&contract, &state, &first, 1.0, 1.0);
    let replay = replay_receipt(&receipt, &contract, &state).expect("replay");
    assert_eq!(replay.result.status, first.result.status);
    assert_eq!(replay.mode, DecisionMode::Replay);
    assert_eq!(replay.calls, 0);
    let stricter = DecisionContract::choice(
        "skill",
        "skill-choice.v1",
        BTreeMap::from([
            ("pdf".to_owned(), "PDF work".to_owned()),
            ("testing".to_owned(), "Test work".to_owned()),
            ("none".to_owned(), "No candidate fits".to_owned()),
        ]),
        0.95,
        0.10,
    );
    assert_ne!(contract.policy_version, stricter.policy_version);
    assert_eq!(
        replay_receipt(&receipt, &stricter, &state)
            .expect("policy mismatch replay")
            .result
            .status,
        DecisionStatus::Degraded
    );
    let mut unsupported_schema = receipt.clone();
    unsupported_schema.schema_version = 99;
    assert!(matches!(
        replay_receipt(&unsupported_schema, &contract, &state),
        Err(JevxError::InvalidInput(message)) if message.contains("schema")
    ));
    let mut invalid_digest = receipt.clone();
    invalid_digest.answer_digest = Some("tampered-answer".to_owned());
    assert_eq!(
        replay_receipt(&invalid_digest, &contract, &state)
            .expect("answer digest mismatch replay")
            .result
            .status,
        DecisionStatus::Degraded
    );
    let mut invalid_result = receipt.clone();
    invalid_result.reason = "tampered-reason".to_owned();
    assert_eq!(
        replay_receipt(&invalid_result, &contract, &state)
            .expect("result mismatch replay")
            .result
            .status,
        DecisionStatus::Degraded
    );
    let mismatch = replay_receipt(
        &receipt,
        &contract,
        &StatePlan::from_value(json!({"different":true}), 1, 1, Vec::new(), Vec::new())
            .expect("different state"),
    )
    .expect("mismatch replay");
    assert_eq!(mismatch.result.status, DecisionStatus::Degraded);
    assert_eq!(mismatch.error_code.as_deref(), Some("replay_mismatch"));

    let recorder = JsonlDecisionRecorder::new(root.path().join("decisions.jsonl"));
    assert!(recorder.path().ends_with("decisions.jsonl"));
    recorder.record(&receipt).expect("record");
    let stats = read_decision_stats(&root.path().join("decisions.jsonl")).expect("stats");
    assert_eq!(stats.events, 1);
    assert_eq!(stats.accepted, 1);
    assert_eq!(
        read_decision_receipts(&root.path().join("decisions.jsonl"))
            .unwrap()
            .len(),
        1
    );
    let json = std::fs::read_to_string(root.path().join("decisions.jsonl")).expect("jsonl");
    assert!(!json.contains("redacted prompt"));
    assert!(!json.contains("secret-value"));
}

#[tokio::test]
async fn execution_modes_fail_closed_and_legacy_adapter_rejects_unsupported_questions() {
    let contract = choice_contract();
    let state = state();
    let answer = TypedAnswer::Choice {
        choice: "pdf".to_owned(),
        probabilities: BTreeMap::from([("pdf".to_owned(), 0.9)]),
        confidence: Some(0.9),
    };
    let judge = StubJudge {
        response: DecisionResponse {
            answers: BTreeMap::from([("skill".to_owned(), answer)]),
            response_ms: 1,
            usage: None,
            calls: 1,
            retries: 0,
        },
    };
    let dry = execute_contract(
        &contract,
        &state,
        Some(&judge),
        &DecisionExecutionOptions {
            mode: DecisionMode::DryRun,
            cache: None,
        },
    )
    .await;
    assert_eq!(dry.result.status, DecisionStatus::Defer);
    assert_eq!(dry.result.fallback, Some(FallbackReason::DryRun));
    let replay = execute_contract(
        &contract,
        &state,
        Some(&judge),
        &DecisionExecutionOptions {
            mode: DecisionMode::Replay,
            cache: None,
        },
    )
    .await;
    assert_eq!(replay.error_code.as_deref(), Some("replay_answer_missing"));
    let missing = execute_contract(
        &contract,
        &state,
        None,
        &DecisionExecutionOptions::default(),
    )
    .await;
    assert_eq!(missing.result.fallback, Some(FallbackReason::MissingApiKey));
    let missing_receipt = DecisionReceipt::from_execution(&contract, &state, &missing, 1.0, 1.0);
    let missing_replay =
        replay_receipt(&missing_receipt, &contract, &state).expect("missing key replay");
    assert_eq!(missing_replay.result.status, DecisionStatus::Defer);
    assert_eq!(
        missing_replay.result.fallback,
        Some(FallbackReason::MissingApiKey)
    );
    assert_eq!(
        missing_replay.error_code.as_deref(),
        Some("missing_api_key")
    );
    let mut accepted_without_answer = missing_receipt.clone();
    accepted_without_answer.decision = DecisionStatus::Accepted;
    assert_eq!(
        replay_receipt(&accepted_without_answer, &contract, &state)
            .expect("invalid failure receipt replay")
            .result
            .status,
        DecisionStatus::Degraded
    );
    let degraded_state =
        StatePlan::from_value(json!({"prompt":"bounded"}), 0, 0, Vec::new(), Vec::new())
            .expect("degraded state");
    let degraded = execute_contract(
        &contract,
        &degraded_state,
        Some(&judge),
        &DecisionExecutionOptions::default(),
    )
    .await;
    assert_eq!(degraded.result.status, DecisionStatus::Degraded);

    let legacy_judge = RecordingLegacyJudge {
        response: JudgeResponse::selected("pdf", 0.9, 1, None),
        calls: Arc::new(AtomicUsize::new(0)),
        states: Arc::new(Mutex::new(Vec::new())),
    };
    let adapter = LegacyJudgeAdapter::new(&legacy_judge);
    let empty_request = DecisionRequest {
        state: "{}".to_owned(),
        questions: BTreeMap::new(),
    };
    assert!(
        jevx::decision::DecisionJudge::evaluate(&adapter, empty_request)
            .await
            .is_err()
    );
    let score_request = DecisionRequest {
        state: "{}".to_owned(),
        questions: BTreeMap::from([(
            "score".to_owned(),
            QuestionSpec::Score {
                instructions: "score".to_owned(),
                criteria: vec!["low".to_owned(), "high".to_owned()],
            },
        )]),
    };
    assert!(
        jevx::decision::DecisionJudge::evaluate(&adapter, score_request)
            .await
            .is_err()
    );

    let cache = InMemoryDecisionCache::new(1);
    cache.put("one", &TypedAnswer::Predicate { noul: 0.9 });
    cache.put("two", &TypedAnswer::Predicate { noul: 0.1 });
    assert!(cache.get("one").is_none());
    assert!(cache.get("two").is_some());
    let empty_cache = InMemoryDecisionCache::new(0);
    empty_cache.put("one", &TypedAnswer::Predicate { noul: 0.9 });
    assert!(empty_cache.get("one").is_none());
}

#[tokio::test]
async fn execution_maps_all_provider_failures_without_accepting() {
    let contract = choice_contract();
    let state = state();
    for kind in [
        DecisionErrorKind::MissingApiKey,
        DecisionErrorKind::Timeout,
        DecisionErrorKind::Provider,
        DecisionErrorKind::InvalidInput,
        DecisionErrorKind::Io,
        DecisionErrorKind::Json,
        DecisionErrorKind::Yaml,
    ] {
        let judge = ErrorDecisionJudge { kind };
        let execution = execute_contract(
            &contract,
            &state,
            Some(&judge),
            &DecisionExecutionOptions::default(),
        )
        .await;
        assert_ne!(execution.result.status, DecisionStatus::Accepted);
        assert!(execution.error_code.is_some());
    }
    let degraded_state =
        StatePlan::from_value(json!({}), 0, 0, Vec::new(), Vec::new()).expect("degraded state");
    let replay = jevx::replay_contract(&contract, &degraded_state, None);
    assert_eq!(replay.result.status, DecisionStatus::Degraded);
}

#[test]
fn degraded_state_is_not_accepted_and_receipt_is_metadata_only() {
    let contract = choice_contract();
    let degraded = StatePlan::from_value(
        json!({"prompt":"secret raw prompt"}),
        0,
        1,
        vec!["state_bytes_limit".to_owned()],
        Vec::new(),
    )
    .expect("state");
    let result = contract.failure(jevx::DecisionFailure::StateDegraded);
    assert_eq!(result.status, DecisionStatus::Degraded);
    let receipt = DecisionReceipt::from_result(&contract, &degraded, &result, None, 0, 0, false, 0);
    let json = serde_json::to_string(&receipt).expect("receipt json");
    assert!(!json.contains("secret raw prompt"));
    assert!(json.contains("state_bytes_limit"));
}

#[tokio::test]
async fn ranking_records_safe_receipts_and_uses_process_cache() {
    let root = tempdir().expect("tempdir");
    let mut config = Config::for_test(root.path().join("data"));
    config.telemetry_enabled = true;
    config.cache_capacity = 2;
    let calls = Arc::new(AtomicUsize::new(0));
    let states = Arc::new(Mutex::new(Vec::new()));
    let judge = RecordingLegacyJudge {
        response: JudgeResponse {
            choice: Some("pdf".to_owned()),
            probabilities: BTreeMap::from([("pdf".to_owned(), 0.9), ("none".to_owned(), 0.1)]),
            response_ms: 12,
            usage: Some(Usage {
                input_tokens: 10,
                output_tokens: 2,
            }),
        },
        calls: calls.clone(),
        states: states.clone(),
    };
    let input = SuggestInput::new(
        "api_key=secret-value でPDFを確認".to_owned(),
        std::path::PathBuf::from("/tmp/api_key=secret-cwd/repo"),
    );
    let skills = vec![skill(
        root.path(),
        "pdf",
        "PDF documents api_key=skill-secret",
    )];
    let first = suggest_with_judge(input.clone(), skills.clone(), &config, &judge)
        .await
        .expect("first suggestion");
    let second = suggest_with_judge(input, skills, &config, &judge)
        .await
        .expect("cached suggestion");
    assert_eq!(first.decision, CandidateDecision::Selected);
    assert_eq!(second.decision, CandidateDecision::Selected);
    assert_eq!(second.metrics.cache_hit, Some(true));
    assert_eq!(second.metrics.jev_response_ms, 0);
    assert_eq!(second.metrics.relative_cost, Some(0.0));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(states.lock().expect("states lock").iter().all(|state| {
        !state.contains("secret-value")
            && !state.contains("secret-cwd")
            && !state.contains("skill-secret")
    }));

    let receipts = read_decision_receipts(&config.decision_receipt_path).expect("receipts");
    assert_eq!(receipts.len(), 2);
    assert!(receipts[1].cache_hit);
    assert_eq!(receipts[1].relative_cost, Some(0.0));
    assert_eq!(receipts[0].relative_cost, Some(12.0));
    assert!(receipts[0].state_bytes > 0);
    assert_eq!(receipts[0].max_state_bytes, Some(32_000));
    assert_eq!(receipts[0].candidate_limit, Some(32));
    let json = std::fs::read_to_string(&config.decision_receipt_path).expect("receipt jsonl");
    assert!(!json.contains("secret-value"));
    assert!(!json.contains("secret-cwd"));
    assert!(!json.contains("skill-secret"));
}

#[tokio::test]
async fn ranking_fails_closed_when_state_window_is_degraded() {
    let root = tempdir().expect("tempdir");
    let mut config = Config::for_test(root.path().join("data"));
    config.max_candidates = 1;
    let calls = Arc::new(AtomicUsize::new(0));
    let judge = RecordingLegacyJudge {
        response: JudgeResponse::selected("pdf", 0.99, 1, None),
        calls: calls.clone(),
        states: Arc::new(Mutex::new(Vec::new())),
    };
    let result = suggest_with_judge(
        SuggestInput::new("PDF".to_owned(), root.path().to_path_buf()),
        vec![
            skill(root.path(), "pdf", "PDF documents"),
            skill(root.path(), "testing", "Run tests"),
        ],
        &config,
        &judge,
    )
    .await
    .expect_err("degraded state must not auto-select");
    assert!(matches!(result, JevxError::InvalidInput(message) if message.contains("degraded")));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let mut byte_config = Config::for_test(root.path().join("bytes-data"));
    byte_config.max_state_bytes = 1;
    let byte_result = suggest_with_judge(
        SuggestInput::new("PDF".to_owned(), root.path().to_path_buf()),
        vec![skill(root.path(), "pdf", "PDF documents")],
        &byte_config,
        &judge,
    )
    .await
    .expect_err("state byte overflow must defer");
    assert!(
        matches!(byte_result, JevxError::InvalidInput(message) if message.contains("degraded"))
    );

    let over_budget =
        StatePlan::from_value(json!({"prompt":"oversized"}), 2, 1, Vec::new(), Vec::new())
            .expect("state")
            .with_budgets(1, 1);
    assert!(over_budget.is_degraded());
}

#[test]
fn decision_stats_count_fallbacks_and_validate_receipt_schema() {
    let root = tempdir().expect("tempdir");
    let path = root.path().join("decisions.jsonl");
    let missing = root.path().join("missing.jsonl");
    assert!(
        read_decision_receipts(&missing)
            .expect("missing receipts")
            .is_empty()
    );
    assert_eq!(
        read_decision_stats(&missing).expect("missing stats").events,
        0
    );
    let contract = choice_contract();
    let state = state();
    let accepted = contract.evaluate(Some(TypedAnswer::Choice {
        choice: "pdf".to_owned(),
        probabilities: BTreeMap::from([("pdf".to_owned(), 0.9)]),
        confidence: Some(0.9),
    }));
    let deferred = contract.evaluate(Some(TypedAnswer::Choice {
        choice: "pdf".to_owned(),
        probabilities: BTreeMap::from([("pdf".to_owned(), 0.5), ("none".to_owned(), 0.5)]),
        confidence: Some(0.5),
    }));
    let recorder = JsonlDecisionRecorder::new(path.clone());
    recorder
        .record(&DecisionReceipt::from_result(
            &contract,
            &state,
            &accepted,
            Some((4, 1)),
            1,
            0,
            false,
            10,
        ))
        .expect("accepted receipt");
    recorder
        .record(&DecisionReceipt::from_result(
            &contract,
            &state,
            &deferred,
            Some((4, 1)),
            1,
            1,
            false,
            20,
        ))
        .expect("deferred receipt");
    let none = contract.evaluate(Some(TypedAnswer::Choice {
        choice: "none".to_owned(),
        probabilities: BTreeMap::from([("none".to_owned(), 0.9)]),
        confidence: Some(0.9),
    }));
    let unknown = contract.evaluate(Some(TypedAnswer::Choice {
        choice: "unknown".to_owned(),
        probabilities: BTreeMap::from([("unknown".to_owned(), 0.9)]),
        confidence: Some(0.9),
    }));
    let degraded = contract.failure(DecisionFailure::StateDegraded);
    for (result, cache_hit) in [(none, true), (unknown, false), (degraded, false)] {
        recorder
            .record(&DecisionReceipt::from_result(
                &contract, &state, &result, None, 0, 0, cache_hit, 30,
            ))
            .expect("status receipt");
    }
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open receipt")
        .write_all(b"\n")
        .expect("blank receipt line");
    let stats = read_decision_stats(&path).expect("stats");
    assert_eq!(stats.events, 5);
    assert_eq!(stats.none, 1);
    assert_eq!(stats.unknown, 1);
    assert_eq!(stats.defer, 1);
    assert_eq!(stats.degraded, 1);
    assert_eq!(stats.fallback_rate, Some(0.8));
    assert_eq!(stats.cache_hits, 1);
    assert_eq!(stats.retries, 1);
    assert_eq!(stats.average_relative_cost, Some(10.0 / 3.0));
    assert_eq!(stats.usage_events, 2);

    let mut invalid = serde_json::to_value(read_decision_receipts(&path).unwrap()[0].clone())
        .expect("receipt value");
    invalid["schemaVersion"] = serde_json::json!(99);
    std::fs::write(&path, format!("{}\n", invalid)).expect("write invalid receipt");
    assert!(matches!(
        read_decision_receipts(&path),
        Err(JevxError::InvalidInput(message)) if message.contains("schema")
    ));
}

#[test]
fn question_wire_shape_is_typed_and_serializable() {
    let question = QuestionSpec::Score {
        instructions: "How severe?".to_owned(),
        criteria: vec!["low".to_owned(), "high".to_owned()],
    };
    let value = serde_json::to_value(question).expect("question");
    assert_eq!(value["type"], "score");
    assert_eq!(value["criteria"][0], "low");
}

#[tokio::test]
async fn baseline_receipts_are_generated_and_replayable() {
    let contract = DecisionContract::choice(
        "skill",
        "skill-choice.v1",
        BTreeMap::from([
            ("none".to_owned(), "No candidate fits".to_owned()),
            ("pdf".to_owned(), "PDF work".to_owned()),
        ]),
        0.60,
        0.10,
    );
    let eval_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("evals");
    let receipts = read_decision_receipts(&eval_dir.join("decision-contract-baseline.jsonl"))
        .expect("baseline receipts");
    let states = std::fs::read_to_string(eval_dir.join("decision-contract-baseline-state.jsonl"))
        .expect("baseline states")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("state json"))
        .collect::<Vec<_>>();
    let answers = [
        TypedAnswer::Choice {
            choice: "pdf".to_owned(),
            probabilities: BTreeMap::from([("none".to_owned(), 0.1), ("pdf".to_owned(), 0.9)]),
            confidence: Some(0.9),
        },
        TypedAnswer::Choice {
            choice: "none".to_owned(),
            probabilities: BTreeMap::from([("none".to_owned(), 0.9), ("pdf".to_owned(), 0.1)]),
            confidence: Some(0.9),
        },
    ];
    let usages = [Some((10, 2)), None];
    let latencies = [12, 8];
    assert_eq!(receipts.len(), answers.len());
    assert_eq!(states.len(), answers.len());
    for ((receipt, state_spec), (answer, (usage, latency))) in receipts
        .iter()
        .zip(states)
        .zip(answers.into_iter().zip(usages.into_iter().zip(latencies)))
    {
        let state = StatePlan::from_value(
            state_spec["payload"].clone(),
            state_spec["candidateCount"]
                .as_u64()
                .expect("candidate count") as usize,
            state_spec["windowCount"].as_u64().expect("window count") as usize,
            state_spec["omitted"]
                .as_array()
                .expect("omitted")
                .iter()
                .map(|value| value.as_str().expect("omitted value").to_owned())
                .collect(),
            state_spec["redactionReasons"]
                .as_array()
                .expect("redaction reasons")
                .iter()
                .map(|value| value.as_str().expect("redaction value").to_owned())
                .collect(),
        )
        .expect("state")
        .with_budgets(
            state_spec["maxStateBytes"]
                .as_u64()
                .expect("state byte limit") as usize,
            state_spec["candidateLimit"]
                .as_u64()
                .expect("candidate limit") as usize,
        );
        let execution = execute_contract(
            &contract,
            &state,
            Some(&StubJudge {
                response: DecisionResponse {
                    answers: BTreeMap::from([("skill".to_owned(), answer)]),
                    response_ms: latency,
                    usage: usage.map(|(input_tokens, output_tokens)| Usage {
                        input_tokens,
                        output_tokens,
                    }),
                    calls: 1,
                    retries: 0,
                },
            }),
            &DecisionExecutionOptions::default(),
        )
        .await;
        let generated = DecisionReceipt::from_execution(&contract, &state, &execution, 1.0, 1.0);
        assert_eq!(
            serde_json::to_value(receipt).expect("fixture receipt json"),
            serde_json::to_value(&generated).expect("generated receipt json")
        );
        let replay = replay_receipt(receipt, &contract, &state).expect("baseline replay");
        assert_eq!(replay.result, execution.result);
        assert_eq!(replay.mode, DecisionMode::Replay);
    }
}
