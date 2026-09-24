use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use jevx::decision::{DecisionRequest, DecisionResponse, TypedAnswer};
use jevx::review::{
    FixPlan, FixStatus, ReviewRequest, ReviewStatus, ReviewTarget, apply_fix_plan,
    review_with_optional_judge,
};
use jevx::route::{ModelFamily, ReasoningLevel, RouteStatus, route_evidence_from_payload};
use jevx::{
    Config, CostEstimate, CostStatus, DecisionJudge, JevxError, Severity, Usage, read_review_stats,
    total_cost,
};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

struct ReviewJudge {
    seen_state: Arc<Mutex<Option<String>>>,
    fail: Option<JevxError>,
}

#[async_trait]
impl DecisionJudge for ReviewJudge {
    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionResponse, JevxError> {
        if let Some(error) = &self.fail {
            return Err(match error {
                JevxError::Timeout => JevxError::Timeout,
                JevxError::Provider(message) => JevxError::Provider(message.clone()),
                _ => JevxError::InvalidInput("fixture failure".to_owned()),
            });
        }
        *self.seen_state.lock().expect("state lock") = Some(request.state);
        let mut answers = BTreeMap::new();
        for id in [
            "review.japanese_clarity",
            "review.text_contradiction",
            "review.code_contradiction",
            "review.comment_implementation_drift",
        ] {
            answers.insert(id.to_owned(), TypedAnswer::Predicate { noul: 0.9 });
        }
        answers.insert(
            "route".to_owned(),
            TypedAnswer::Score {
                score: 1.0,
                probabilities: BTreeMap::from([
                    ("0".to_owned(), 0.1),
                    ("1".to_owned(), 0.8),
                    ("2".to_owned(), 0.1),
                ]),
                confidence: 0.9,
            },
        );
        Ok(DecisionResponse {
            answers,
            response_ms: 23,
            usage: Some(Usage {
                input_tokens: 100,
                output_tokens: 20,
                cost: Some(CostEstimate::available(
                    0.00014,
                    Some("USD".to_owned()),
                    Some("test".to_owned()),
                )),
            }),
            calls: 1,
            retries: 0,
        })
    }
}

fn hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[tokio::test]
async fn content_requires_explicit_opt_in_and_is_safe_by_default() {
    let config = Config::for_test(PathBuf::from("/tmp/jevx-review-requirement"));
    let request = ReviewRequest::from_content(
        ReviewTarget::Turn,
        Some("適宜対応 secret=fixture-only"),
        None,
        false,
    );
    let response = review_with_optional_judge(
        &request,
        &config,
        None,
        None,
        None,
        FixPlan::not_requested(),
    )
    .await
    .expect("review response");
    assert_eq!(response.status, ReviewStatus::Degraded);
    assert_eq!(
        response.receipt.fallback.as_deref(),
        Some("content_opt_in_required")
    );
    assert!(
        response
            .findings
            .iter()
            .any(|finding| finding.category == jevx::ReviewCategory::JapaneseClarity)
    );
    let serialized = serde_json::to_string(&response).expect("json");
    assert!(!serialized.contains("fixture-only"));
}

#[tokio::test]
async fn no_external_review_keeps_known_codex_cost_separate_from_zero_jev() {
    let config = Config::for_test(PathBuf::from("/tmp/jevx-review-requirement"));
    let request = ReviewRequest::from_content(
        ReviewTarget::Turn,
        Some("適宜対応 secret=fixture-only"),
        None,
        false,
    );
    let codex = jevx::CodexUsage {
        cost: CostEstimate::actual(0.42, Some("USD".to_owned()), Some("test".to_owned())),
        ..jevx::CodexUsage::default()
    };
    let response = review_with_optional_judge(
        &request,
        &config,
        None,
        Some(&codex),
        None,
        FixPlan::not_requested(),
    )
    .await
    .expect("review response");

    assert_eq!(response.receipt.cost.jev.amount, Some(0.0));
    assert_eq!(response.receipt.cost.codex.amount, Some(0.42));
    assert_eq!(response.receipt.cost.total.amount, Some(0.42));
    assert_eq!(response.receipt.cost.codex.status, CostStatus::Available);
    assert_eq!(response.receipt.cost.total.status, CostStatus::Available);
}

#[tokio::test]
async fn one_typed_jev_request_carries_findings_route_and_both_cost_sides() {
    let config = Config::for_test(PathBuf::from("/tmp/jevx-review-requirement"));
    let seen_state = Arc::new(Mutex::new(None));
    let judge = ReviewJudge {
        seen_state: Arc::clone(&seen_state),
        fail: None,
    };
    let request = ReviewRequest::from_content(
        ReviewTarget::Diff,
        Some("適宜対応 secret=fixture-only"),
        None,
        true,
    );
    let codex = jevx::CodexUsage {
        model: Some("gpt-5.6-terra".to_owned()),
        reasoning_effort: Some("high".to_owned()),
        additional_input_tokens: Some(3),
        additional_output_tokens: Some(2),
        additional_reasoning_tokens: Some(1),
        cost: CostEstimate::available(0.42, Some("USD".to_owned()), Some("test".to_owned())),
        additional_cost: Some(CostEstimate::actual(
            0.07,
            Some("USD".to_owned()),
            Some("test".to_owned()),
        )),
        ..jevx::CodexUsage::default()
    };
    let evidence = route_evidence_from_payload(Some("terra"), Some("high"));
    let response = review_with_optional_judge(
        &request,
        &config,
        Some(&judge),
        Some(&codex),
        evidence.as_ref(),
        FixPlan::not_requested(),
    )
    .await
    .expect("review response");
    assert_eq!(response.status, ReviewStatus::Completed);
    assert_eq!(response.route.status, RouteStatus::Applied);
    assert_eq!(response.route.model, Some(ModelFamily::Terra));
    assert_eq!(response.route.reasoning, Some(ReasoningLevel::High));
    assert_eq!(response.receipt.calls, 1);
    assert_eq!(
        response.receipt.cost.jev.status,
        jevx::CostStatus::Available
    );
    assert_eq!(response.receipt.cost.codex.amount, Some(0.42));
    let recorded_codex = response
        .receipt
        .codex_usage
        .as_ref()
        .expect("codex receipt");
    assert_eq!(recorded_codex.additional_output_tokens, Some(2));
    assert_eq!(
        recorded_codex
            .additional_cost
            .as_ref()
            .and_then(|cost| cost.amount),
        Some(0.07)
    );
    assert_eq!(
        response.receipt.cost.total.status,
        jevx::CostStatus::Available
    );
    assert!((response.receipt.cost.total.amount.expect("total") - 0.42014).abs() < 1e-9);
    let state = seen_state
        .lock()
        .expect("state lock")
        .clone()
        .expect("state");
    assert!(!state.contains("fixture-only"));
}

#[tokio::test]
async fn provider_failure_blocks_fix_and_route_application() {
    let config = Config::for_test(PathBuf::from("/tmp/jevx-review-requirement"));
    let judge = ReviewJudge {
        seen_state: Arc::new(Mutex::new(None)),
        fail: Some(JevxError::Timeout),
    };
    let request = ReviewRequest::from_content(ReviewTarget::Diff, Some("return false"), None, true);
    let response = review_with_optional_judge(
        &request,
        &config,
        Some(&judge),
        None,
        None,
        FixPlan::requested(Vec::new()),
    )
    .await
    .expect("safe response");
    assert_eq!(response.status, ReviewStatus::Failed);
    assert_eq!(response.route.status, RouteStatus::Failed);
    assert_eq!(response.fix_plan.status, FixStatus::Blocked);
    assert!(response.receipt.cost.total.amount.is_none());
}

#[test]
fn successful_fix_is_workspace_local_and_creates_no_backup() {
    let root = tempdir().expect("workspace");
    let path = root.path().join("src.txt");
    std::fs::write(&path, "old").expect("write");
    let operation = jevx::FixOperation {
        path: "src.txt".to_owned(),
        expected_sha256: hash("old"),
        replacement_sha256: hash("new"),
        replacement: Some("new".to_owned()),
    };
    let finding = jevx::ReviewFinding {
        id: "fixture".to_owned(),
        category: jevx::ReviewCategory::CodeContradiction,
        severity: Severity::Medium,
        confidence: Some(0.95),
        source: "jev".to_owned(),
        message: "fixture".to_owned(),
        evidence_digest: hash("evidence"),
        fixable: true,
    };
    let result = apply_fix_plan(
        &FixPlan::requested(vec![operation]),
        root.path(),
        true,
        ReviewStatus::Completed,
        &[finding],
    );
    assert_eq!(result.status, FixStatus::Applied);
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "new");
    assert!(!root.path().join("src.txt.jevx.bak").exists());
}

#[test]
fn review_stats_keep_unknown_cost_distinct_from_zero() {
    let root = tempdir().expect("data");
    let path = root.path().join("reviews.jsonl");
    let request = ReviewRequest::from_content(ReviewTarget::Plan, Some("plan"), None, false);
    let response = fixture_response(&request);
    jevx::append_review_receipt(&path, &response.receipt).expect("receipt");
    let stats = read_review_stats(&path).expect("stats");
    assert_eq!(stats.events, 1);
    assert_eq!(stats.total_cost, None);
    assert_eq!(stats.cost_status_counts.get("total:unavailable"), Some(&1));
}

#[test]
fn review_stats_group_safe_costs_by_task_turn_and_session() {
    let root = tempdir().expect("data");
    let path = root.path().join("reviews.jsonl");
    let request = ReviewRequest::from_content(ReviewTarget::Turn, Some("review"), None, false)
        .with_identifiers(Some("task-1"), Some("session-1"), Some("turn-1"));
    let mut response = fixture_response(&request);
    response.receipt.status = ReviewStatus::Completed;
    response.receipt.fix_status = FixStatus::Applied;
    response.receipt.calls = 2;
    response.receipt.retries = 1;
    response.receipt.cache_hit = Some(false);
    response.receipt.input_tokens = Some(4);
    response.receipt.output_tokens = Some(5);
    let jev = CostEstimate::available(0.10, Some("USD".to_owned()), Some("fixture".to_owned()));
    let codex = CostEstimate::actual(0.20, Some("USD".to_owned()), Some("fixture".to_owned()));
    response.receipt.cost = jevx::CostSummary {
        jev: jev.clone(),
        codex: codex.clone(),
        total: total_cost(&jev, &codex),
    };
    response.receipt.codex_usage = Some(jevx::CodexUsage {
        additional_input_tokens: Some(3),
        additional_output_tokens: Some(2),
        additional_reasoning_tokens: Some(1),
        additional_cost: Some(CostEstimate::actual(
            0.07,
            Some("USD".to_owned()),
            Some("fixture".to_owned()),
        )),
        ..jevx::CodexUsage::default()
    });
    jevx::append_review_receipt(&path, &response.receipt).expect("receipt");
    let stats = read_review_stats(&path).expect("stats");
    assert_eq!(stats.calls, 2);
    assert_eq!(stats.retries, 1);
    assert_eq!(stats.cache_hit_rate, Some(0.0));
    assert_eq!(stats.input_tokens, Some(4));
    assert_eq!(stats.output_tokens, Some(5));
    assert!((stats.successful_review_cost.expect("review cost") - 0.3).abs() < 1e-9);
    assert!((stats.successful_fix_cost.expect("fix cost") - 0.3).abs() < 1e-9);
    assert_eq!(stats.additional_input_tokens, Some(3));
    assert_eq!(stats.additional_output_tokens, Some(2));
    assert_eq!(stats.additional_reasoning_tokens, Some(1));
    assert_eq!(stats.fallback_extra_cost, Some(0.07));
    let task = stats.cost_by_task.get(&hash("task-1")).expect("task cost");
    assert!((task.total.expect("task total") - 0.3).abs() < 1e-9);
    assert!((task.fallback_extra.expect("fallback cost") - 0.07).abs() < 1e-9);
    assert_eq!(task.unknown_cost_events, 0);
    assert_eq!(stats.cost_by_turn.len(), 1);
    assert_eq!(stats.cost_by_session.len(), 1);
    assert_eq!(response.receipt.cost.codex.status, CostStatus::Available);
    assert_eq!(response.receipt.cost.codex.basis, jevx::CostBasis::Actual);
}

fn fixture_response(request: &ReviewRequest) -> jevx::ReviewResponse {
    let receipt = jevx::ReviewReceipt {
        schema_version: jevx::REVIEW_SCHEMA_VERSION,
        review_contract_version: jevx::REVIEW_CONTRACT_VERSION.to_owned(),
        request_digest: request.digest(),
        target: request.target,
        content_digest: request.content_digest.clone(),
        content_chars: request.content_chars,
        task_id_sha256: request.task_id_sha256.clone(),
        session_id_sha256: request.session_id_sha256.clone(),
        turn_id_sha256: request.turn_id_sha256.clone(),
        status: ReviewStatus::Degraded,
        finding_count: 0,
        route: jevx::RouteDecision::failed("fixture"),
        latency_ms: 1,
        calls: 0,
        retries: 0,
        cache_hit: None,
        input_tokens: None,
        output_tokens: None,
        fallback: Some("content_opt_in_required".to_owned()),
        codex_usage: None,
        cost: jevx::CostSummary::default(),
        fix_status: FixStatus::NotRequested,
        replay_id: hash("receipt"),
    };
    jevx::ReviewResponse {
        schema_version: jevx::REVIEW_SCHEMA_VERSION,
        status: ReviewStatus::Degraded,
        target: request.target,
        findings: Vec::new(),
        route: jevx::RouteDecision::failed("fixture"),
        fix_plan: FixPlan::not_requested(),
        receipt,
    }
}
