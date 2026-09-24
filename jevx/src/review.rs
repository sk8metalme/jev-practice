//! 毎ターンの意味レビューと、その安全な観測契約。
//!
//! 構文解析・差分抽出・秘密値の除外はローカルコードが担当する。Jevには、明示的な
//! `allow_content`がある場合だけredact済み本文を渡し、最終的なstatusとfix適用可否は
//! 常にコード側で決める。

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Config;
use crate::cost::{CodexUsage, CostAccumulator, CostEstimate, CostStatus, CostSummary, total_cost};
use crate::decision::{
    DecisionContract, DecisionJudge, DecisionRequest, DecisionStatus, QuestionSpec, TypedAnswer,
};
use crate::error::JevxError;
use crate::redaction::{redact, sha256_hex};
use crate::route::{RouteDecision, RouteEvidence, RouteStatus, route_contract, route_result};
use crate::storage::append_json_line;

pub const REVIEW_SCHEMA_VERSION: u8 = 2;
pub const REVIEW_CONTRACT_VERSION: &str = "review-contract.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewTarget {
    Prompt,
    Plan,
    Diff,
    FinalAnswer,
    Turn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewCategory {
    JapaneseClarity,
    TextContradiction,
    CodeContradiction,
    CommentImplementationDrift,
}

impl ReviewCategory {
    pub const ALL: [Self; 4] = [
        Self::JapaneseClarity,
        Self::TextContradiction,
        Self::CodeContradiction,
        Self::CommentImplementationDrift,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::JapaneseClarity => "japanese_clarity",
            Self::TextContradiction => "text_contradiction",
            Self::CodeContradiction => "code_contradiction",
            Self::CommentImplementationDrift => "comment_implementation_drift",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::JapaneseClarity => "日本語のわかりにくい表現",
            Self::TextContradiction => "文章内の矛盾",
            Self::CodeContradiction => "コード内の意味的矛盾",
            Self::CommentImplementationDrift => "コメントと実装の乖離",
        }
    }

    fn is_code_related(self) -> bool {
        matches!(
            self,
            Self::CodeContradiction | Self::CommentImplementationDrift
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Completed,
    None,
    Unknown,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    pub schema_version: u8,
    pub target: ReviewTarget,
    pub content_digest: String,
    pub content_chars: usize,
    pub content_included: bool,
    #[serde(skip_serializing, skip_deserializing)]
    pub content: Option<String>,
    /// Jevへ送らない場合も、redact済みの決定的なローカル検出には使える。
    #[serde(skip_serializing, skip_deserializing)]
    pub local_content: Option<String>,
    pub cwd_digest: Option<String>,
    pub task_id_sha256: Option<String>,
    pub session_id_sha256: Option<String>,
    pub turn_id_sha256: Option<String>,
    pub file_count: usize,
    pub skill_body_included: bool,
    pub settings_included: bool,
}

impl ReviewRequest {
    pub fn from_content(
        target: ReviewTarget,
        content: Option<&str>,
        cwd: Option<&Path>,
        allow_content: bool,
    ) -> Self {
        let raw = content.unwrap_or_default();
        let redacted = redact(raw);
        Self {
            schema_version: REVIEW_SCHEMA_VERSION,
            target,
            content_digest: sha256_hex(raw),
            content_chars: raw.chars().count(),
            content_included: allow_content && !raw.is_empty(),
            content: (allow_content && !raw.is_empty()).then_some(redacted.clone()),
            local_content: (!raw.is_empty()).then_some(redacted),
            cwd_digest: cwd.map(|path| sha256_hex(&redact(&path.display().to_string()))),
            task_id_sha256: None,
            session_id_sha256: None,
            turn_id_sha256: None,
            file_count: 0,
            skill_body_included: false,
            settings_included: false,
        }
    }

    pub fn with_metadata(
        mut self,
        file_count: usize,
        skill_body_included: bool,
        settings_included: bool,
    ) -> Self {
        self.file_count = file_count;
        self.skill_body_included = skill_body_included;
        self.settings_included = settings_included;
        self
    }

    pub fn with_identifiers(
        mut self,
        task_id: Option<&str>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Self {
        self.task_id_sha256 = task_id.map(sha256_hex);
        self.session_id_sha256 = session_id.map(sha256_hex);
        self.turn_id_sha256 = turn_id.map(sha256_hex);
        self
    }

    pub fn digest(&self) -> String {
        serde_json::to_string(self)
            .map(|value| sha256_hex(&value))
            .unwrap_or_else(|_| sha256_hex("invalid-review-request"))
    }

    pub fn state_plan(&self, config: &Config) -> Result<crate::decision::StatePlan, JevxError> {
        let payload = json!({
            "target": self.target,
            "content": self.content,
            "contentDigest": self.content_digest,
            "contentChars": self.content_chars,
            "cwdDigest": self.cwd_digest,
            "taskIdSha256": self.task_id_sha256,
            "sessionIdSha256": self.session_id_sha256,
            "turnIdSha256": self.turn_id_sha256,
            "fileCount": self.file_count,
            "skillBodyIncluded": self.skill_body_included,
            "settingsIncluded": self.settings_included,
        });
        Ok(
            crate::decision::StatePlan::from_value(payload, 0, 1, Vec::new(), Vec::new())?
                .with_budgets(config.max_state_bytes, config.max_candidates),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFinding {
    pub id: String,
    pub category: ReviewCategory,
    pub severity: crate::decision::Severity,
    pub confidence: Option<f64>,
    pub source: String,
    pub message: String,
    pub evidence_digest: String,
    pub fixable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixOperation {
    pub path: String,
    pub expected_sha256: String,
    pub replacement_sha256: String,
    #[serde(skip_serializing, skip_deserializing)]
    pub replacement: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixStatus {
    NotRequested,
    Blocked,
    Applied,
    Partial,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixPlan {
    pub schema_version: u8,
    pub requested: bool,
    pub operations: Vec<FixOperation>,
    pub status: FixStatus,
    pub reason: String,
}

impl FixPlan {
    pub fn not_requested() -> Self {
        Self {
            schema_version: REVIEW_SCHEMA_VERSION,
            requested: false,
            operations: Vec::new(),
            status: FixStatus::NotRequested,
            reason: "auto_fix_not_requested".to_owned(),
        }
    }

    pub fn requested(operations: Vec<FixOperation>) -> Self {
        Self {
            schema_version: REVIEW_SCHEMA_VERSION,
            requested: true,
            operations,
            status: FixStatus::Blocked,
            reason: "awaiting_code_side_review_gate".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixApplication {
    pub status: FixStatus,
    pub applied_count: usize,
    pub skipped_count: usize,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewReceipt {
    pub schema_version: u8,
    pub review_contract_version: String,
    pub request_digest: String,
    pub target: ReviewTarget,
    pub content_digest: String,
    pub content_chars: usize,
    pub task_id_sha256: Option<String>,
    pub session_id_sha256: Option<String>,
    pub turn_id_sha256: Option<String>,
    pub status: ReviewStatus,
    pub finding_count: usize,
    pub route: RouteDecision,
    pub latency_ms: u64,
    pub calls: u32,
    pub retries: u32,
    pub cache_hit: Option<bool>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub fallback: Option<String>,
    #[serde(
        rename = "codexUsage",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub codex_usage: Option<CodexUsage>,
    pub cost: CostSummary,
    pub fix_status: FixStatus,
    pub replay_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewResponse {
    pub schema_version: u8,
    pub status: ReviewStatus,
    pub target: ReviewTarget,
    pub findings: Vec<ReviewFinding>,
    pub route: RouteDecision,
    pub fix_plan: FixPlan,
    pub receipt: ReviewReceipt,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStats {
    pub events: usize,
    pub completed: usize,
    pub none: usize,
    pub unknown: usize,
    pub degraded: usize,
    pub failed: usize,
    pub findings: usize,
    pub route_applied: usize,
    pub route_degraded: usize,
    pub route_failed: usize,
    pub latency_ms_p50: Option<u64>,
    pub latency_ms_p95: Option<u64>,
    pub jev_cost: Option<f64>,
    pub codex_cost: Option<f64>,
    pub total_cost: Option<f64>,
    pub average_total_cost: Option<f64>,
    pub successful_review_cost: Option<f64>,
    pub successful_fix_cost: Option<f64>,
    pub fallback_extra_cost: Option<f64>,
    pub calls: u64,
    pub retries: u64,
    pub cache_attempts: usize,
    pub cache_hits: usize,
    pub cache_hit_rate: Option<f64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub additional_input_tokens: Option<u64>,
    pub additional_output_tokens: Option<u64>,
    pub additional_reasoning_tokens: Option<u64>,
    pub cost_status_counts: BTreeMap<String, usize>,
    pub cost_by_task: BTreeMap<String, CostUnitSummary>,
    pub cost_by_turn: BTreeMap<String, CostUnitSummary>,
    pub cost_by_session: BTreeMap<String, CostUnitSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostUnitSummary {
    pub events: usize,
    pub jev: Option<f64>,
    pub codex: Option<f64>,
    pub total: Option<f64>,
    pub fallback_extra: Option<f64>,
    pub unknown_cost_events: usize,
    pub unavailable_cost_events: usize,
}

#[derive(Debug, Default)]
struct CostUnitAccumulator {
    events: usize,
    jev: CostAccumulator,
    codex: CostAccumulator,
    total: CostAccumulator,
    fallback_extra: CostAccumulator,
    unknown_cost_events: usize,
    unavailable_cost_events: usize,
}

impl CostUnitAccumulator {
    fn into_summary(self) -> CostUnitSummary {
        CostUnitSummary {
            events: self.events,
            jev: self.jev.amount(),
            codex: self.codex.amount(),
            total: self.total.amount(),
            fallback_extra: self.fallback_extra.amount(),
            unknown_cost_events: self.unknown_cost_events,
            unavailable_cost_events: self.unavailable_cost_events,
        }
    }
}

pub fn append_review_receipt(path: &Path, receipt: &ReviewReceipt) -> Result<(), JevxError> {
    append_json_line(path, receipt)
}

pub fn read_review_stats(path: &Path) -> Result<ReviewStats, JevxError> {
    if !path.exists() {
        return Ok(ReviewStats::default());
    }
    let file = fs::File::open(path)?;
    let mut stats = ReviewStats::default();
    let mut latencies = Vec::new();
    let mut jev_cost = CostAccumulator::default();
    let mut codex_cost = CostAccumulator::default();
    let mut total_cost_value = CostAccumulator::default();
    let mut successful_review_cost = CostAccumulator::default();
    let mut successful_fix_cost = CostAccumulator::default();
    let mut fallback_extra_cost = CostAccumulator::default();
    let mut input_tokens = 0_u64;
    let mut output_tokens = 0_u64;
    let mut input_tokens_seen = false;
    let mut output_tokens_seen = false;
    let mut additional_input_tokens = 0_u64;
    let mut additional_output_tokens = 0_u64;
    let mut additional_reasoning_tokens = 0_u64;
    let mut additional_input_tokens_seen = false;
    let mut additional_output_tokens_seen = false;
    let mut additional_reasoning_tokens_seen = false;
    let mut cost_by_task = BTreeMap::new();
    let mut cost_by_turn = BTreeMap::new();
    let mut cost_by_session = BTreeMap::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let receipt: ReviewReceipt = serde_json::from_str(&line)?;
        stats.events += 1;
        match receipt.status {
            ReviewStatus::Completed => stats.completed += 1,
            ReviewStatus::None => stats.none += 1,
            ReviewStatus::Unknown => stats.unknown += 1,
            ReviewStatus::Degraded => stats.degraded += 1,
            ReviewStatus::Failed => stats.failed += 1,
        }
        stats.findings += receipt.finding_count;
        match receipt.route.status {
            RouteStatus::Applied => stats.route_applied += 1,
            RouteStatus::Degraded => stats.route_degraded += 1,
            RouteStatus::Failed => stats.route_failed += 1,
        }
        latencies.push(receipt.latency_ms);
        stats.calls += u64::from(receipt.calls);
        stats.retries += u64::from(receipt.retries);
        if let Some(cache_hit) = receipt.cache_hit {
            stats.cache_attempts += 1;
            stats.cache_hits += usize::from(cache_hit);
        }
        if let Some(value) = receipt.input_tokens {
            input_tokens = input_tokens.saturating_add(value);
            input_tokens_seen = true;
        }
        if let Some(value) = receipt.output_tokens {
            output_tokens = output_tokens.saturating_add(value);
            output_tokens_seen = true;
        }
        for (name, estimate) in [
            ("jev", &receipt.cost.jev),
            ("codex", &receipt.cost.codex),
            ("total", &receipt.cost.total),
        ] {
            let status = match estimate.status {
                CostStatus::Available => "available",
                CostStatus::Unknown => "unknown",
                CostStatus::Unavailable => "unavailable",
            };
            *stats
                .cost_status_counts
                .entry(format!("{name}:{status}"))
                .or_insert(0) += 1;
        }
        jev_cost.add(&receipt.cost.jev);
        codex_cost.add(&receipt.cost.codex);
        total_cost_value.add(&receipt.cost.total);
        if let Some(usage) = &receipt.codex_usage {
            if let Some(value) = usage.additional_input_tokens {
                additional_input_tokens = additional_input_tokens.saturating_add(value);
                additional_input_tokens_seen = true;
            }
            if let Some(value) = usage.additional_output_tokens {
                additional_output_tokens = additional_output_tokens.saturating_add(value);
                additional_output_tokens_seen = true;
            }
            if let Some(value) = usage.additional_reasoning_tokens {
                additional_reasoning_tokens = additional_reasoning_tokens.saturating_add(value);
                additional_reasoning_tokens_seen = true;
            }
            if let Some(amount) = usage.additional_cost.as_ref() {
                fallback_extra_cost.add(amount);
            }
        }
        if matches!(receipt.status, ReviewStatus::Completed | ReviewStatus::None) {
            successful_review_cost.add(&receipt.cost.total);
        }
        if receipt.fix_status == FixStatus::Applied {
            successful_fix_cost.add(&receipt.cost.total);
        }
        add_unit_cost(
            &mut cost_by_task,
            receipt.task_id_sha256.as_deref(),
            &receipt.cost,
            receipt.codex_usage.as_ref(),
        );
        add_unit_cost(
            &mut cost_by_turn,
            receipt.turn_id_sha256.as_deref(),
            &receipt.cost,
            receipt.codex_usage.as_ref(),
        );
        add_unit_cost(
            &mut cost_by_session,
            receipt.session_id_sha256.as_deref(),
            &receipt.cost,
            receipt.codex_usage.as_ref(),
        );
    }
    stats.latency_ms_p50 = percentile(&latencies, 50);
    stats.latency_ms_p95 = percentile(&latencies, 95);
    stats.jev_cost = jev_cost.amount();
    stats.codex_cost = codex_cost.amount();
    stats.total_cost = total_cost_value.amount();
    stats.average_total_cost = total_cost_value
        .amount()
        .map(|amount| amount / stats.events as f64);
    stats.successful_review_cost = successful_review_cost.amount();
    stats.successful_fix_cost = successful_fix_cost.amount();
    stats.fallback_extra_cost = fallback_extra_cost.amount();
    stats.cache_hit_rate =
        (stats.cache_attempts > 0).then(|| stats.cache_hits as f64 / stats.cache_attempts as f64);
    stats.input_tokens = input_tokens_seen.then_some(input_tokens);
    stats.output_tokens = output_tokens_seen.then_some(output_tokens);
    stats.additional_input_tokens = additional_input_tokens_seen.then_some(additional_input_tokens);
    stats.additional_output_tokens =
        additional_output_tokens_seen.then_some(additional_output_tokens);
    stats.additional_reasoning_tokens =
        additional_reasoning_tokens_seen.then_some(additional_reasoning_tokens);
    stats.cost_by_task = cost_by_task
        .into_iter()
        .map(|(id, accumulator)| (id, accumulator.into_summary()))
        .collect();
    stats.cost_by_turn = cost_by_turn
        .into_iter()
        .map(|(id, accumulator)| (id, accumulator.into_summary()))
        .collect();
    stats.cost_by_session = cost_by_session
        .into_iter()
        .map(|(id, accumulator)| (id, accumulator.into_summary()))
        .collect();
    Ok(stats)
}

fn add_unit_cost(
    units: &mut BTreeMap<String, CostUnitAccumulator>,
    id: Option<&str>,
    cost: &CostSummary,
    codex_usage: Option<&CodexUsage>,
) {
    let Some(id) = id else {
        return;
    };
    let entry = units.entry(id.to_owned()).or_default();
    entry.events += 1;
    entry.jev.add(&cost.jev);
    entry.codex.add(&cost.codex);
    entry.total.add(&cost.total);
    if let Some(estimate) = codex_usage.and_then(|usage| usage.additional_cost.as_ref()) {
        entry.fallback_extra.add(estimate);
    }
    if matches!(cost.total.status, CostStatus::Unknown) {
        entry.unknown_cost_events += 1;
    } else if matches!(cost.total.status, CostStatus::Unavailable) {
        entry.unavailable_cost_events += 1;
    }
}

#[derive(Debug, Clone)]
struct ReviewExecution {
    status: ReviewStatus,
    calls: u32,
    retries: u32,
    cache_hit: Option<bool>,
    response_ms: u64,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reported_jev_cost: Option<CostEstimate>,
    fallback: Option<&'static str>,
}

pub fn review_contracts() -> BTreeMap<String, DecisionContract> {
    let mut contracts = BTreeMap::new();
    for category in ReviewCategory::ALL {
        let mut contract = DecisionContract::predicate(
            &format!("review.{}", category.key()),
            "review-category.v1",
            &format!(
                "true: {} is present in the supplied state",
                category.label()
            ),
            &format!(
                "false: {} is not present in the supplied state",
                category.label()
            ),
            0.80,
            0.20,
        );
        if let QuestionSpec::Predicate { instructions, .. } = &mut contract.question {
            *instructions = format!(
                "{}について、入力stateに意味的な問題がある確率を判定してください。",
                category.label()
            );
        }
        contracts.insert(contract.question_id.clone(), contract);
    }
    contracts.insert("route".to_owned(), route_contract());
    contracts
}

pub async fn review_with_optional_judge(
    request: &ReviewRequest,
    config: &Config,
    judge: Option<&dyn DecisionJudge>,
    codex_usage: Option<&CodexUsage>,
    route_evidence: Option<&RouteEvidence>,
    fix_plan: FixPlan,
) -> Result<ReviewResponse, JevxError> {
    let started = Instant::now();
    validate_codex_usage(codex_usage)?;
    let local_findings = request
        .local_content
        .as_deref()
        .map(|content| local_findings(request.target, content))
        .unwrap_or_default();

    if !request.content_included {
        return Ok(build_response(
            request,
            local_findings,
            RouteDecision::failed("content_opt_in_required"),
            fix_plan_blocked(fix_plan, "content_opt_in_required"),
            ReviewExecution {
                status: ReviewStatus::Degraded,
                calls: 0,
                retries: 0,
                cache_hit: None,
                response_ms: 0,
                input_tokens: None,
                output_tokens: None,
                reported_jev_cost: None,
                fallback: Some("content_opt_in_required"),
            },
            codex_usage,
            config,
            started,
        ));
    }

    let state = request.state_plan(config)?;
    if state.is_degraded() {
        return Ok(build_response(
            request,
            local_findings,
            RouteDecision::failed("state_degraded"),
            fix_plan_blocked(fix_plan, "state_degraded"),
            ReviewExecution {
                status: ReviewStatus::Degraded,
                calls: 0,
                retries: 0,
                cache_hit: None,
                response_ms: 0,
                input_tokens: None,
                output_tokens: None,
                reported_jev_cost: None,
                fallback: Some("state_degraded"),
            },
            codex_usage,
            config,
            started,
        ));
    }

    let Some(judge) = judge else {
        return Ok(build_response(
            request,
            local_findings,
            RouteDecision::failed("missing_api_key"),
            fix_plan_blocked(fix_plan, "missing_api_key"),
            ReviewExecution {
                status: ReviewStatus::Degraded,
                calls: 0,
                retries: 0,
                cache_hit: None,
                response_ms: 0,
                input_tokens: None,
                output_tokens: None,
                reported_jev_cost: None,
                fallback: Some("missing_api_key"),
            },
            codex_usage,
            config,
            started,
        ));
    };

    let contracts = review_contracts();
    let questions = contracts
        .iter()
        .map(|(id, contract)| (id.clone(), contract.question.clone()))
        .collect::<BTreeMap<_, _>>();
    let decision = match judge
        .evaluate(DecisionRequest {
            state: serde_json::to_string(&state.payload)?,
            questions,
        })
        .await
    {
        Ok(decision) => decision,
        Err(error) => {
            let (response_ms, calls, retries, cache_hit, fallback) = match &error {
                JevxError::ProviderWithMetrics {
                    response_ms,
                    calls,
                    retries,
                    ..
                } => (
                    *response_ms,
                    *calls,
                    *retries,
                    Some(false),
                    Some("provider_error"),
                ),
                JevxError::Timeout => (0, 0, 0, Some(false), Some("timeout")),
                JevxError::TimeoutWithMetrics {
                    calls,
                    retries,
                    response_ms,
                } => (*response_ms, *calls, *retries, Some(false), Some("timeout")),
                JevxError::MissingApiKey => (0, 0, 0, None, Some("missing_api_key")),
                JevxError::InvalidInput(_) => (0, 0, 0, Some(false), Some("contract_failure")),
                JevxError::Provider(_)
                | JevxError::Io(_)
                | JevxError::Json(_)
                | JevxError::Yaml(_) => (0, 0, 0, Some(false), Some("provider_error")),
            };
            return Ok(build_response(
                request,
                local_findings,
                RouteDecision::failed(fallback.unwrap_or("provider_error")),
                fix_plan_blocked(fix_plan, fallback.unwrap_or("provider_error")),
                ReviewExecution {
                    status: ReviewStatus::Failed,
                    calls,
                    retries,
                    cache_hit,
                    response_ms,
                    input_tokens: None,
                    output_tokens: None,
                    reported_jev_cost: None,
                    fallback,
                },
                codex_usage,
                config,
                started,
            ));
        }
    };

    let mut findings = local_findings;
    let mut unresolved = false;
    for category in ReviewCategory::ALL {
        let id = format!("review.{}", category.key());
        let contract = &contracts[&id];
        let result = contract.evaluate(decision.answers.get(&id).cloned());
        if !matches!(result.status, DecisionStatus::Accepted) {
            unresolved = true;
        }
        if let Some(TypedAnswer::Predicate { noul }) = decision.answers.get(&id)
            && *noul >= 0.80
        {
            findings.push(jev_finding(category, *noul));
        }
    }

    let route_contract = &contracts["route"];
    let route_result_value = route_contract.evaluate(decision.answers.get("route").cloned());
    let route = if matches!(route_result_value.status, DecisionStatus::Accepted) {
        route_result(&route_result_value, route_evidence)
    } else {
        unresolved = true;
        RouteDecision::failed(route_result_value.reason)
    };
    let status = if unresolved {
        ReviewStatus::Degraded
    } else if findings.is_empty() {
        ReviewStatus::None
    } else {
        ReviewStatus::Completed
    };
    let execution = ReviewExecution {
        status,
        calls: decision.calls,
        retries: decision.retries,
        cache_hit: Some(false),
        response_ms: decision.response_ms,
        input_tokens: decision.usage.as_ref().map(|usage| usage.input_tokens),
        output_tokens: decision.usage.as_ref().map(|usage| usage.output_tokens),
        reported_jev_cost: decision
            .usage
            .as_ref()
            .and_then(|usage| usage.cost.as_ref().filter(|cost| cost.is_valid()).cloned()),
        fallback: unresolved.then_some("low_confidence_or_contract_failure"),
    };
    Ok(build_response(
        request,
        findings,
        route,
        fix_plan,
        execution,
        codex_usage,
        config,
        started,
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_response(
    request: &ReviewRequest,
    findings: Vec<ReviewFinding>,
    route: RouteDecision,
    fix_plan: FixPlan,
    execution: ReviewExecution,
    codex_usage: Option<&CodexUsage>,
    config: &Config,
    started: Instant,
) -> ReviewResponse {
    let cost = if known_no_external_review(&execution) {
        let jev = CostEstimate::actual(0.0, None, None);
        let codex = codex_usage
            .map(|usage| usage.cost.clone())
            .unwrap_or_else(|| CostEstimate::actual(0.0, None, None));
        CostSummary {
            total: total_cost(&jev, &codex),
            jev,
            codex,
        }
    } else {
        review_cost(
            config,
            execution.input_tokens,
            execution.output_tokens,
            codex_usage,
            execution.reported_jev_cost.as_ref(),
        )
    };
    let status = execution.status;
    let replay_id = sha256_hex(&format!(
        "{}:{}:{}:{}",
        REVIEW_CONTRACT_VERSION,
        request.digest(),
        status_label(status),
        findings
            .iter()
            .map(|finding| finding.evidence_digest.as_str())
            .collect::<Vec<_>>()
            .join(":")
    ));
    let receipt = ReviewReceipt {
        schema_version: REVIEW_SCHEMA_VERSION,
        review_contract_version: REVIEW_CONTRACT_VERSION.to_owned(),
        request_digest: request.digest(),
        target: request.target,
        content_digest: request.content_digest.clone(),
        content_chars: request.content_chars,
        task_id_sha256: request.task_id_sha256.clone(),
        session_id_sha256: request.session_id_sha256.clone(),
        turn_id_sha256: request.turn_id_sha256.clone(),
        status,
        finding_count: findings.len(),
        route: route.clone(),
        latency_ms: if execution.response_ms > 0 {
            execution.response_ms
        } else {
            elapsed_ms(started)
        },
        calls: execution.calls,
        retries: execution.retries,
        cache_hit: execution.cache_hit,
        input_tokens: execution.input_tokens,
        output_tokens: execution.output_tokens,
        fallback: execution.fallback.map(str::to_owned),
        codex_usage: codex_usage.cloned(),
        cost,
        fix_status: fix_plan.status,
        replay_id,
    };
    ReviewResponse {
        schema_version: REVIEW_SCHEMA_VERSION,
        status,
        target: request.target,
        findings,
        route,
        fix_plan,
        receipt,
    }
}

fn known_no_external_review(execution: &ReviewExecution) -> bool {
    matches!(
        execution.fallback,
        Some("content_opt_in_required" | "state_degraded" | "missing_api_key")
    )
}

fn review_cost(
    config: &Config,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    codex_usage: Option<&CodexUsage>,
    reported_jev_cost: Option<&CostEstimate>,
) -> CostSummary {
    let jev = reported_jev_cost.cloned().unwrap_or_else(|| {
        config
            .jev_pricing()
            .estimate_two_part(input_tokens, output_tokens, false)
    });
    let codex = codex_usage
        .map(|usage| usage.cost.clone())
        .unwrap_or_else(CostEstimate::unavailable);
    CostSummary {
        total: total_cost(&jev, &codex),
        jev,
        codex,
    }
}

fn validate_codex_usage(usage: Option<&CodexUsage>) -> Result<(), JevxError> {
    let Some(usage) = usage else {
        return Ok(());
    };
    for (label, amount) in [
        ("cost", usage.cost.amount),
        (
            "additionalCost",
            usage
                .additional_cost
                .as_ref()
                .and_then(|estimate| estimate.amount),
        ),
    ] {
        if amount.is_some_and(|amount| !amount.is_finite() || amount < 0.0) {
            return Err(JevxError::InvalidInput(format!(
                "codexUsage {label} must be a finite non-negative amount"
            )));
        }
    }
    if !usage.is_valid() {
        return Err(JevxError::InvalidInput(
            "codexUsage contains invalid metadata or cost".to_owned(),
        ));
    }
    Ok(())
}

fn fix_plan_blocked(mut plan: FixPlan, reason: &str) -> FixPlan {
    if plan.requested {
        plan.status = FixStatus::Blocked;
        plan.reason = reason.to_owned();
    }
    plan
}

fn status_label(status: ReviewStatus) -> &'static str {
    match status {
        ReviewStatus::Completed => "completed",
        ReviewStatus::None => "none",
        ReviewStatus::Unknown => "unknown",
        ReviewStatus::Degraded => "degraded",
        ReviewStatus::Failed => "failed",
    }
}

fn local_findings(target: ReviewTarget, content: &str) -> Vec<ReviewFinding> {
    let lower = content.to_ascii_lowercase();
    let mut findings = Vec::new();
    if ["適宜", "いい感じ", "適切に", "なるべく", "など"]
        .iter()
        .any(|phrase| content.contains(phrase))
    {
        findings.push(local_finding(
            ReviewCategory::JapaneseClarity,
            "ambiguous_expression",
            crate::decision::Severity::Low,
            true,
        ));
    }
    // 「必ず」と「しない」の共起だけでは、別の命題を誤って矛盾扱いする。
    // 同一命題への否定を構文的に確定できない意味判定はJevへ委ねる。
    if target != ReviewTarget::Prompt
        && (lower.contains("must return true") && lower.contains("return false")
            || lower.contains("always true") && lower.contains("return false")
            || lower.contains("always returns true") && lower.contains("return false"))
    {
        findings.push(local_finding(
            ReviewCategory::CodeContradiction,
            "return_contract_conflict",
            crate::decision::Severity::High,
            true,
        ));
    }
    let comment_claim = content.lines().any(|line| {
        let line = line.trim().to_ascii_lowercase();
        (line.starts_with("//") || line.starts_with("#"))
            && (line.contains("returns true") || line.contains("常に") || line.contains("always"))
    });
    if target != ReviewTarget::Prompt && comment_claim && lower.contains("return false") {
        findings.push(local_finding(
            ReviewCategory::CommentImplementationDrift,
            "comment_return_contract_conflict",
            crate::decision::Severity::Medium,
            true,
        ));
    }
    findings
}

fn local_finding(
    category: ReviewCategory,
    code: &str,
    severity: crate::decision::Severity,
    fixable: bool,
) -> ReviewFinding {
    ReviewFinding {
        id: format!("local:{}", category.key()),
        category,
        severity,
        confidence: Some(1.0),
        source: "local".to_owned(),
        message: format!("{}を確認してください（{code}）。", category.label()),
        evidence_digest: sha256_hex(&format!("{}:{code}", category.key())),
        fixable,
    }
}

fn jev_finding(category: ReviewCategory, confidence: f64) -> ReviewFinding {
    ReviewFinding {
        id: format!("jev:{}", category.key()),
        category,
        severity: if category.is_code_related() {
            crate::decision::Severity::Medium
        } else {
            crate::decision::Severity::Low
        },
        confidence: Some(confidence),
        source: "jev".to_owned(),
        message: format!("{}の意味的な問題候補があります。", category.label()),
        evidence_digest: sha256_hex(&format!("jev:{}:{confidence:.6}", category.key())),
        fixable: category.is_code_related(),
    }
}

pub fn fix_operations_from_json(
    value: Option<&serde_json::Value>,
) -> Result<Vec<FixOperation>, JevxError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let operations = value
        .as_array()
        .ok_or_else(|| JevxError::InvalidInput("fixes must be an array".to_owned()))?;
    let mut result = Vec::with_capacity(operations.len());
    for operation in operations {
        let path = operation
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| JevxError::InvalidInput("fix path is required".to_owned()))?;
        let expected_sha256 = operation
            .get("expectedSha256")
            .or_else(|| operation.get("expected_sha256"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| JevxError::InvalidInput("fix expectedSha256 is required".to_owned()))?;
        let replacement = operation
            .get("replacement")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| JevxError::InvalidInput("fix replacement is required".to_owned()))?;
        result.push(FixOperation {
            path: path.to_owned(),
            expected_sha256: expected_sha256.to_owned(),
            replacement_sha256: sha256_hex(replacement),
            replacement: Some(replacement.to_owned()),
        });
    }
    Ok(result)
}

pub fn apply_fix_plan(
    plan: &FixPlan,
    workspace: &Path,
    enabled: bool,
    review_status: ReviewStatus,
    findings: &[ReviewFinding],
) -> FixApplication {
    if !enabled || !plan.requested {
        return FixApplication {
            status: FixStatus::NotRequested,
            applied_count: 0,
            skipped_count: 0,
            reason: "auto_fix_not_enabled".to_owned(),
        };
    }
    if !matches!(review_status, ReviewStatus::Completed)
        || findings.iter().any(|finding| {
            finding
                .confidence
                .is_none_or(|confidence| confidence < 0.80)
        })
    {
        return FixApplication {
            status: FixStatus::Blocked,
            applied_count: 0,
            skipped_count: plan.operations.len(),
            reason: "review_not_high_confidence_or_completed".to_owned(),
        };
    }
    if plan.operations.is_empty() {
        return FixApplication {
            status: FixStatus::Skipped,
            applied_count: 0,
            skipped_count: 0,
            reason: "no_fix_operations".to_owned(),
        };
    }

    let mut applied_count = 0;
    let mut skipped_count = 0;
    let mut mismatch = false;
    for operation in &plan.operations {
        if !safe_fix_path(&operation.path) {
            skipped_count += 1;
            continue;
        }
        let Some(path) = safe_workspace_file(workspace, &operation.path) else {
            skipped_count += 1;
            continue;
        };
        let Ok(current) = fs::read(&path) else {
            skipped_count += 1;
            continue;
        };
        let current_text = String::from_utf8_lossy(&current);
        if sha256_hex(&current_text) != operation.expected_sha256 {
            skipped_count += 1;
            mismatch = true;
            continue;
        }
        let Some(replacement) = operation.replacement.as_deref() else {
            skipped_count += 1;
            continue;
        };
        if sha256_hex(replacement) != operation.replacement_sha256 {
            skipped_count += 1;
            continue;
        }
        if write_without_following_symlink(&path, replacement).is_ok() {
            applied_count += 1;
        } else {
            skipped_count += 1;
        }
    }
    let status = if applied_count == plan.operations.len() {
        FixStatus::Applied
    } else if applied_count > 0 {
        FixStatus::Partial
    } else if mismatch {
        FixStatus::Skipped
    } else {
        FixStatus::Failed
    };
    FixApplication {
        status,
        applied_count,
        skipped_count,
        reason: if mismatch {
            "expected_hash_mismatch_prevented_overwrite".to_owned()
        } else {
            "fix_application_finished_without_backup".to_owned()
        },
    }
}

fn safe_workspace_file(workspace: &Path, relative: &str) -> Option<PathBuf> {
    if !safe_fix_path(relative) {
        return None;
    }
    let root = fs::canonicalize(workspace).ok()?;
    let path = root.join(relative);
    let mut current = root.clone();
    for component in Path::new(relative).components() {
        if let Component::Normal(value) = component {
            current.push(value);
            let metadata = fs::symlink_metadata(&current).ok()?;
            if metadata.file_type().is_symlink() {
                return None;
            }
        }
    }
    let canonical = fs::canonicalize(&path).ok()?;
    canonical.starts_with(&root).then_some(path)
}

#[cfg(unix)]
fn write_without_following_symlink(path: &Path, replacement: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.write_all(replacement.as_bytes())
}

#[cfg(not(unix))]
fn write_without_following_symlink(path: &Path, replacement: &str) -> std::io::Result<()> {
    fs::write(path, replacement)
}

fn safe_fix_path(path: &str) -> bool {
    let path = Path::new(path);
    if path.is_absolute() || path.as_os_str().is_empty() {
        return false;
    }
    let mut has_normal = false;
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_string_lossy();
                if value == ".git" || value == "legacy" || value.starts_with('.') {
                    return false;
                }
                has_normal = true;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    let file = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    has_normal
        && ![
            ".env",
            ".env.local",
            ".env.production",
            "id_rsa",
            "credentials",
        ]
        .contains(&file)
        && !file.ends_with(".pem")
        && !file.ends_with(".key")
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn percentile(values: &[u64], percentile: usize) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() * percentile).div_ceil(100).saturating_sub(1);
    sorted.get(rank).copied()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::decision::{
        DecisionEvidence, DecisionRequest, DecisionResponse, DecisionResult, TypedAnswer,
    };
    use crate::error::JevxError;
    use crate::route::{RouteStatus, route_evidence_from_payload};
    use async_trait::async_trait;
    use tempfile::tempdir;

    #[derive(Clone, Copy)]
    enum JudgeMode {
        Clean,
        MissingAnswers,
        Timeout,
        TimeoutWithMetrics,
        ProviderWithMetrics,
        Provider,
        MissingApiKey,
        InvalidInput,
        Io,
        Json,
        Yaml,
    }

    struct StubJudge {
        mode: JudgeMode,
    }

    #[async_trait]
    impl DecisionJudge for StubJudge {
        async fn evaluate(&self, _request: DecisionRequest) -> Result<DecisionResponse, JevxError> {
            match self.mode {
                JudgeMode::Timeout => Err(JevxError::Timeout),
                JudgeMode::TimeoutWithMetrics => Err(JevxError::TimeoutWithMetrics {
                    calls: 3,
                    retries: 2,
                    response_ms: 15,
                }),
                JudgeMode::ProviderWithMetrics => Err(JevxError::ProviderWithMetrics {
                    message: "fixture provider error".to_owned(),
                    calls: 2,
                    retries: 1,
                    response_ms: 12,
                }),
                JudgeMode::Provider => {
                    Err(JevxError::Provider("fixture provider error".to_owned()))
                }
                JudgeMode::MissingApiKey => Err(JevxError::MissingApiKey),
                JudgeMode::InvalidInput => {
                    Err(JevxError::InvalidInput("fixture contract error".to_owned()))
                }
                JudgeMode::Io => Err(JevxError::Io(std::io::Error::other("fixture io error"))),
                JudgeMode::Json => Err(JevxError::Json(
                    serde_json::from_str::<serde_json::Value>("[")
                        .expect_err("invalid json fixture"),
                )),
                JudgeMode::Yaml => Err(JevxError::Yaml(
                    serde_yaml::from_str::<serde_yaml::Value>("[")
                        .expect_err("invalid yaml fixture"),
                )),
                JudgeMode::MissingAnswers => Ok(DecisionResponse {
                    answers: BTreeMap::new(),
                    response_ms: 1,
                    usage: None,
                    calls: 1,
                    retries: 0,
                }),
                JudgeMode::Clean => {
                    let mut answers = BTreeMap::new();
                    for id in ReviewCategory::ALL {
                        answers.insert(
                            format!("review.{}", id.key()),
                            TypedAnswer::Predicate { noul: 0.1 },
                        );
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
                        response_ms: 4,
                        usage: Some(crate::types::Usage {
                            input_tokens: 10,
                            output_tokens: 2,
                            cost: None,
                        }),
                        calls: 1,
                        retries: 0,
                    })
                }
            }
        }
    }

    #[test]
    fn request_never_serializes_body_without_explicit_opt_in() {
        let request = ReviewRequest::from_content(
            ReviewTarget::Prompt,
            Some("秘密 secret=fixture-only"),
            Some(Path::new("/workspace")),
            false,
        );
        assert!(!request.content_included);
        assert!(request.content.is_none());
        assert_eq!(request.local_content.as_deref(), Some("秘密 <redacted>"));
        let json = serde_json::to_string(&request).expect("request json");
        assert!(!json.contains("fixture-only"));
        assert!(json.contains("\"contentIncluded\":false"));
    }

    #[test]
    fn local_four_category_fixture_is_reproducible() {
        let content = "適宜対応する。必須だが不要でもある。\n// always returns true\nreturn false;";
        let findings = local_findings(ReviewTarget::Diff, content);
        let categories = findings
            .iter()
            .map(|finding| finding.category)
            .collect::<Vec<_>>();
        assert!(categories.contains(&ReviewCategory::JapaneseClarity));
        assert!(!categories.contains(&ReviewCategory::TextContradiction));
        assert!(categories.contains(&ReviewCategory::CodeContradiction));
        assert!(categories.contains(&ReviewCategory::CommentImplementationDrift));
    }

    #[test]
    fn local_text_contradiction_detection_defers_without_same_proposition_proof() {
        let findings = local_findings(
            ReviewTarget::Turn,
            "必須だが不要でもある。\n別の話として必ず確認する。",
        );
        assert!(
            !findings
                .iter()
                .any(|finding| finding.category == ReviewCategory::TextContradiction)
        );
    }

    #[test]
    fn request_metadata_state_and_digest_are_stable_without_body() {
        let config = Config::for_test(PathBuf::from("/tmp/jevx-review-test"));
        let request = ReviewRequest::from_content(
            ReviewTarget::Turn,
            Some("safe content"),
            Some(Path::new("/workspace")),
            true,
        )
        .with_metadata(100, true, true)
        .with_identifiers(Some("task"), Some("session"), Some("turn"));
        let state = request.state_plan(&config).expect("state");
        assert!(!state.is_degraded());
        assert_eq!(state.candidate_count, 0);
        assert_eq!(request.file_count, 100);
        assert!(request.skill_body_included);
        assert!(request.settings_included);
        assert_eq!(
            request.task_id_sha256.as_deref(),
            Some(sha256_hex("task").as_str())
        );
        assert_eq!(request.digest(), request.digest());
        let serialized = serde_json::to_string(&request).expect("request json");
        assert!(!serialized.contains("safe content"));
    }

    #[test]
    fn fix_requires_high_confidence_and_expected_hash() {
        let root = tempdir().expect("tempdir");
        let path = root.path().join("src.txt");
        std::fs::write(&path, "old").expect("write");
        let operation = FixOperation {
            path: "src.txt".to_owned(),
            expected_sha256: sha256_hex("different"),
            replacement_sha256: sha256_hex("new"),
            replacement: Some("new".to_owned()),
        };
        let plan = FixPlan::requested(vec![operation]);
        let finding = local_finding(
            ReviewCategory::TextContradiction,
            "fixture",
            crate::decision::Severity::Medium,
            true,
        );
        let result = apply_fix_plan(
            &plan,
            root.path(),
            true,
            ReviewStatus::Completed,
            &[finding],
        );
        assert_eq!(result.status, FixStatus::Skipped);
        assert_eq!(std::fs::read_to_string(path).expect("read"), "old");
    }

    #[test]
    fn fix_excludes_legacy_git_and_secret_paths() {
        for path in [
            "legacy/file.rs",
            ".git/config",
            ".env",
            "id_rsa",
            "credentials",
            "secret.key",
            "key.pem",
            "../outside",
            "/absolute/path",
            "",
        ] {
            assert!(!safe_fix_path(path), "{path} should be excluded");
        }
        assert!(safe_fix_path("src/file.rs"));
        assert!(safe_fix_path("./src/./file.rs"));
    }

    #[cfg(unix)]
    #[test]
    fn fix_rejects_symlinked_files_and_parent_directories() {
        use std::os::unix::fs::symlink;

        let workspace = tempdir().expect("workspace");
        let outside = tempdir().expect("outside");
        let outside_file = outside.path().join("target.txt");
        std::fs::write(&outside_file, "old").expect("outside file");
        symlink(&outside_file, workspace.path().join("link.txt")).expect("file symlink");

        let finding = local_finding(
            ReviewCategory::CodeContradiction,
            "fixture",
            crate::decision::Severity::High,
            true,
        );
        let operation = FixOperation {
            path: "link.txt".to_owned(),
            expected_sha256: sha256_hex("old"),
            replacement_sha256: sha256_hex("new"),
            replacement: Some("new".to_owned()),
        };
        let result = apply_fix_plan(
            &FixPlan::requested(vec![operation]),
            workspace.path(),
            true,
            ReviewStatus::Completed,
            std::slice::from_ref(&finding),
        );
        assert_eq!(result.status, FixStatus::Failed);
        assert_eq!(
            std::fs::read_to_string(&outside_file).expect("outside read"),
            "old"
        );

        let outside_dir = outside.path().join("directory");
        std::fs::create_dir(&outside_dir).expect("outside directory");
        let nested = outside_dir.join("nested.txt");
        std::fs::write(&nested, "old").expect("nested file");
        symlink(&outside_dir, workspace.path().join("link-dir")).expect("directory symlink");
        let operation = FixOperation {
            path: "link-dir/nested.txt".to_owned(),
            expected_sha256: sha256_hex("old"),
            replacement_sha256: sha256_hex("new"),
            replacement: Some("new".to_owned()),
        };
        let result = apply_fix_plan(
            &FixPlan::requested(vec![operation]),
            workspace.path(),
            true,
            ReviewStatus::Completed,
            std::slice::from_ref(&finding),
        );
        assert_eq!(result.status, FixStatus::Failed);
        assert_eq!(std::fs::read_to_string(nested).expect("nested read"), "old");
    }

    #[test]
    fn fix_parser_and_application_fail_closed_on_invalid_plans() {
        assert!(fix_operations_from_json(None).expect("empty").is_empty());
        assert!(fix_operations_from_json(Some(&json!({}))).is_err());
        assert!(fix_operations_from_json(Some(&json!([{}]))).is_err());
        assert!(fix_operations_from_json(Some(&json!([{"path":"a"}]))).is_err());
        assert!(
            fix_operations_from_json(Some(&json!([{
                "path":"a",
                "expected_sha256":"hash"
            }])))
            .is_err()
        );
        let operations = fix_operations_from_json(Some(&json!([{
            "path":"src.txt",
            "expectedSha256":"old",
            "replacement":"new"
        }])))
        .expect("operation");
        assert_eq!(operations[0].replacement_sha256, sha256_hex("new"));

        let root = tempdir().expect("workspace");
        assert_eq!(
            apply_fix_plan(
                &FixPlan::not_requested(),
                root.path(),
                false,
                ReviewStatus::None,
                &[]
            )
            .status,
            FixStatus::NotRequested
        );
        assert_eq!(
            apply_fix_plan(
                &FixPlan::requested(Vec::new()),
                root.path(),
                true,
                ReviewStatus::None,
                &[]
            )
            .status,
            FixStatus::Blocked
        );
        assert_eq!(
            apply_fix_plan(
                &FixPlan::requested(Vec::new()),
                root.path(),
                true,
                ReviewStatus::Completed,
                &[]
            )
            .status,
            FixStatus::Skipped
        );
        let low_confidence = local_finding(
            ReviewCategory::CodeContradiction,
            "low",
            crate::decision::Severity::High,
            true,
        );
        let mut low_confidence = low_confidence;
        low_confidence.confidence = Some(0.2);
        assert_eq!(
            apply_fix_plan(
                &FixPlan::requested(Vec::new()),
                root.path(),
                true,
                ReviewStatus::Completed,
                &[low_confidence]
            )
            .status,
            FixStatus::Blocked
        );
    }

    #[test]
    fn fix_application_rejects_unsafe_missing_and_invalid_replacements() {
        let root = tempdir().expect("workspace");
        let path = root.path().join("src.txt");
        std::fs::write(&path, "old").expect("write");
        let finding = local_finding(
            ReviewCategory::CodeContradiction,
            "fixture",
            crate::decision::Severity::High,
            true,
        );
        let expected = sha256_hex("old");
        let invalid_operations = vec![
            FixOperation {
                path: "../outside".to_owned(),
                expected_sha256: expected.clone(),
                replacement_sha256: sha256_hex("new"),
                replacement: Some("new".to_owned()),
            },
            FixOperation {
                path: "missing.txt".to_owned(),
                expected_sha256: expected.clone(),
                replacement_sha256: sha256_hex("new"),
                replacement: Some("new".to_owned()),
            },
            FixOperation {
                path: "src.txt".to_owned(),
                expected_sha256: expected.clone(),
                replacement_sha256: sha256_hex("new"),
                replacement: None,
            },
            FixOperation {
                path: "src.txt".to_owned(),
                expected_sha256: expected,
                replacement_sha256: sha256_hex("different"),
                replacement: Some("new".to_owned()),
            },
        ];
        let result = apply_fix_plan(
            &FixPlan::requested(invalid_operations),
            root.path(),
            true,
            ReviewStatus::Completed,
            std::slice::from_ref(&finding),
        );
        assert_eq!(result.status, FixStatus::Failed);
        assert_eq!(result.skipped_count, 4);
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "old");

        let partial = apply_fix_plan(
            &FixPlan::requested(vec![
                FixOperation {
                    path: "src.txt".to_owned(),
                    expected_sha256: sha256_hex("old"),
                    replacement_sha256: sha256_hex("new"),
                    replacement: Some("new".to_owned()),
                },
                FixOperation {
                    path: ".env".to_owned(),
                    expected_sha256: sha256_hex("old"),
                    replacement_sha256: sha256_hex("new"),
                    replacement: Some("new".to_owned()),
                },
            ]),
            root.path(),
            true,
            ReviewStatus::Completed,
            &[finding],
        );
        assert_eq!(partial.status, FixStatus::Partial);
        assert_eq!(partial.applied_count, 1);
    }

    #[tokio::test]
    async fn review_keeps_missing_state_judge_and_answer_failures_explicit() {
        let mut config = Config::for_test(PathBuf::from("/tmp/jevx-review-test"));
        let request = ReviewRequest::from_content(ReviewTarget::Diff, Some("safe"), None, true);
        config.max_state_bytes = 1;
        let state_degraded = review_with_optional_judge(
            &request,
            &config,
            Some(&StubJudge {
                mode: JudgeMode::Clean,
            }),
            None,
            None,
            FixPlan::not_requested(),
        )
        .await
        .expect("state response");
        assert_eq!(state_degraded.status, ReviewStatus::Degraded);
        assert_eq!(
            state_degraded.receipt.fallback.as_deref(),
            Some("state_degraded")
        );

        config.max_state_bytes = 32_000;
        let missing_judge = review_with_optional_judge(
            &request,
            &config,
            None,
            None,
            None,
            FixPlan::not_requested(),
        )
        .await
        .expect("missing judge response");
        assert_eq!(missing_judge.status, ReviewStatus::Degraded);
        assert_eq!(
            missing_judge.receipt.fallback.as_deref(),
            Some("missing_api_key")
        );
        assert_eq!(missing_judge.receipt.cost.total.amount, Some(0.0));
        assert_eq!(
            missing_judge.receipt.cost.total.basis,
            crate::cost::CostBasis::Actual
        );

        let clean = review_with_optional_judge(
            &request,
            &config,
            Some(&StubJudge {
                mode: JudgeMode::Clean,
            }),
            None,
            None,
            FixPlan::not_requested(),
        )
        .await
        .expect("clean response");
        assert_eq!(clean.status, ReviewStatus::None);
        assert_eq!(clean.route.status, RouteStatus::Degraded);
        assert_eq!(clean.receipt.cache_hit, Some(false));

        let unresolved = review_with_optional_judge(
            &request,
            &config,
            Some(&StubJudge {
                mode: JudgeMode::MissingAnswers,
            }),
            None,
            None,
            FixPlan::not_requested(),
        )
        .await
        .expect("unresolved response");
        assert_eq!(unresolved.status, ReviewStatus::Degraded);
        assert_eq!(unresolved.route.status, RouteStatus::Failed);
    }

    #[tokio::test]
    async fn review_records_timeout_and_provider_metrics_without_fixing() {
        let config = Config::for_test(PathBuf::from("/tmp/jevx-review-test"));
        let request =
            ReviewRequest::from_content(ReviewTarget::Diff, Some("return false"), None, true);
        for (mode, fallback) in [
            (JudgeMode::Timeout, "timeout"),
            (JudgeMode::TimeoutWithMetrics, "timeout"),
            (JudgeMode::ProviderWithMetrics, "provider_error"),
        ] {
            let response = review_with_optional_judge(
                &request,
                &config,
                Some(&StubJudge { mode }),
                None,
                None,
                FixPlan::requested(Vec::new()),
            )
            .await
            .expect("failed response");
            assert_eq!(response.status, ReviewStatus::Failed);
            assert_eq!(response.receipt.fallback.as_deref(), Some(fallback));
            assert_eq!(response.fix_plan.status, FixStatus::Blocked);
            if matches!(mode, JudgeMode::TimeoutWithMetrics) {
                assert_eq!(response.receipt.calls, 3);
                assert_eq!(response.receipt.retries, 2);
                assert_eq!(response.receipt.latency_ms, 15);
            }
        }
    }

    #[tokio::test]
    async fn review_maps_all_provider_error_shapes_to_safe_fallbacks() {
        let config = Config::for_test(PathBuf::from("/tmp/jevx-review-test"));
        let request =
            ReviewRequest::from_content(ReviewTarget::Diff, Some("return false"), None, true);
        for (mode, fallback) in [
            (JudgeMode::Provider, "provider_error"),
            (JudgeMode::MissingApiKey, "missing_api_key"),
            (JudgeMode::InvalidInput, "contract_failure"),
            (JudgeMode::Io, "provider_error"),
            (JudgeMode::Json, "provider_error"),
            (JudgeMode::Yaml, "provider_error"),
        ] {
            let response = review_with_optional_judge(
                &request,
                &config,
                Some(&StubJudge { mode }),
                None,
                None,
                FixPlan::not_requested(),
            )
            .await
            .expect("safe response");
            assert_eq!(response.status, ReviewStatus::Failed);
            assert_eq!(response.receipt.fallback.as_deref(), Some(fallback));
        }
    }

    #[tokio::test]
    async fn review_rejects_negative_codex_cost_metadata() {
        let config = Config::for_test(PathBuf::from("/tmp/jevx-review-test"));
        let request =
            ReviewRequest::from_content(ReviewTarget::Diff, Some("return false"), None, true);
        let error = review_with_optional_judge(
            &request,
            &config,
            None,
            Some(&CodexUsage {
                cost: CostEstimate::actual(
                    -0.1,
                    Some("USD".to_owned()),
                    Some("fixture".to_owned()),
                ),
                ..CodexUsage::default()
            }),
            None,
            FixPlan::not_requested(),
        )
        .await
        .expect_err("negative cost must be rejected");
        assert!(error.to_string().contains("finite non-negative"));
    }

    #[tokio::test]
    async fn review_rejects_unsafe_codex_cost_metadata() {
        let config = Config::for_test(PathBuf::from("/tmp/jevx-review-test"));
        let request =
            ReviewRequest::from_content(ReviewTarget::Diff, Some("return false"), None, true);
        let error = review_with_optional_judge(
            &request,
            &config,
            None,
            Some(&CodexUsage {
                model: Some("model\nsecret".to_owned()),
                cost: CostEstimate::actual(0.1, Some("USD".to_owned()), Some("fixture".to_owned())),
                ..CodexUsage::default()
            }),
            None,
            FixPlan::not_requested(),
        )
        .await
        .expect_err("unsafe metadata must be rejected");
        assert!(error.to_string().contains("invalid metadata"));
    }

    #[test]
    fn review_stats_cover_status_route_and_unknown_cost_dimensions() {
        let root = tempdir().expect("data");
        let missing = root.path().join("missing.jsonl");
        assert_eq!(read_review_stats(&missing).expect("empty stats").events, 0);

        let path = root.path().join("reviews.jsonl");
        let blank = root.path().join("blank.jsonl");
        std::fs::write(&blank, "\n").expect("blank receipt file");
        assert_eq!(read_review_stats(&blank).expect("blank stats").events, 0);

        let request = ReviewRequest::from_content(ReviewTarget::Turn, Some("fixture"), None, false);
        let mut none = fixture_receipt(&request, ReviewStatus::None, RouteStatus::Applied);
        none.cost = CostSummary {
            jev: CostEstimate::available(0.1, Some("USD".to_owned()), Some("fixture".to_owned())),
            codex: CostEstimate::available(0.2, Some("USD".to_owned()), Some("fixture".to_owned())),
            total: CostEstimate::available(0.3, Some("USD".to_owned()), Some("fixture".to_owned())),
        };
        let unknown = fixture_receipt(&request, ReviewStatus::Unknown, RouteStatus::Degraded);
        let mut unknown = unknown;
        unknown.cost = CostSummary {
            jev: CostEstimate::unknown(Some("USD".to_owned()), Some("fixture".to_owned())),
            codex: CostEstimate::unknown(Some("USD".to_owned()), Some("fixture".to_owned())),
            total: CostEstimate::unknown(Some("USD".to_owned()), Some("fixture".to_owned())),
        };
        let failed = fixture_receipt(&request, ReviewStatus::Failed, RouteStatus::Failed);
        append_review_receipt(&path, &none).expect("none receipt");
        append_review_receipt(&path, &unknown).expect("unknown receipt");
        append_review_receipt(&path, &failed).expect("failed receipt");
        let stats = read_review_stats(&path).expect("stats");
        assert_eq!(stats.none, 1);
        assert_eq!(stats.unknown, 1);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.route_applied, 1);
        assert_eq!(stats.route_degraded, 1);
        assert_eq!(stats.route_failed, 1);
        assert_eq!(stats.cost_status_counts.get("total:available"), Some(&1));
        assert_eq!(stats.cost_status_counts.get("total:unknown"), Some(&1));
        assert_eq!(stats.cost_status_counts.get("total:unavailable"), Some(&1));

        let mut units = BTreeMap::new();
        add_unit_cost(
            &mut units,
            Some("unknown"),
            &CostSummary {
                total: CostEstimate::unknown(Some("USD".to_owned()), Some("fixture".to_owned())),
                ..CostSummary::default()
            },
            None,
        );
        add_unit_cost(
            &mut units,
            Some("unavailable"),
            &CostSummary::default(),
            None,
        );
        add_unit_cost(&mut units, None, &CostSummary::default(), None);
        assert_eq!(units["unknown"].unknown_cost_events, 1);
        assert_eq!(units["unavailable"].unavailable_cost_events, 1);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let read_only_path = root.path().join("read-only.txt");
            std::fs::write(&read_only_path, "old").expect("read-only fixture");
            std::fs::set_permissions(&read_only_path, std::fs::Permissions::from_mode(0o444))
                .expect("read-only permissions");
            let result = apply_fix_plan(
                &FixPlan::requested(vec![FixOperation {
                    path: "read-only.txt".to_owned(),
                    expected_sha256: sha256_hex("old"),
                    replacement_sha256: sha256_hex("new"),
                    replacement: Some("new".to_owned()),
                }]),
                root.path(),
                true,
                ReviewStatus::Completed,
                &[],
            );
            assert_eq!(result.status, FixStatus::Failed);
            assert_eq!(result.skipped_count, 1);
            std::fs::set_permissions(&read_only_path, std::fs::Permissions::from_mode(0o644))
                .expect("restore permissions");
        }
    }

    fn fixture_receipt(
        request: &ReviewRequest,
        status: ReviewStatus,
        route_status: RouteStatus,
    ) -> ReviewReceipt {
        let mut route = RouteDecision::failed("fixture");
        route.status = route_status;
        ReviewReceipt {
            schema_version: REVIEW_SCHEMA_VERSION,
            review_contract_version: REVIEW_CONTRACT_VERSION.to_owned(),
            request_digest: request.digest(),
            target: request.target,
            content_digest: request.content_digest.clone(),
            content_chars: request.content_chars,
            task_id_sha256: None,
            session_id_sha256: None,
            turn_id_sha256: None,
            status,
            finding_count: 0,
            route,
            latency_ms: 1,
            calls: 0,
            retries: 0,
            cache_hit: None,
            input_tokens: None,
            output_tokens: None,
            fallback: Some("fixture".to_owned()),
            codex_usage: None,
            cost: CostSummary::default(),
            fix_status: FixStatus::NotRequested,
            replay_id: sha256_hex("fixture"),
        }
    }

    #[test]
    fn review_contracts_are_typed_and_include_route() {
        let contracts = review_contracts();
        assert_eq!(contracts.len(), 5);
        assert!(contracts.contains_key("review.japanese_clarity"));
        assert!(contracts.contains_key("route"));
        let answer = TypedAnswer::Predicate { noul: 0.9 };
        let result = contracts["review.japanese_clarity"].evaluate(Some(answer));
        assert_eq!(result.status, DecisionStatus::Accepted);
    }

    #[test]
    fn response_replay_id_does_not_contain_review_body() {
        let config = Config::for_test(PathBuf::from("/tmp/jevx-review-test"));
        let request = ReviewRequest::from_content(
            ReviewTarget::Diff,
            Some("secret=fixture-only"),
            None,
            false,
        );
        let response = build_response(
            &request,
            Vec::new(),
            RouteDecision::failed("degraded"),
            FixPlan::not_requested(),
            ReviewExecution {
                status: ReviewStatus::Degraded,
                calls: 0,
                retries: 0,
                cache_hit: None,
                response_ms: 0,
                input_tokens: None,
                output_tokens: None,
                reported_jev_cost: None,
                fallback: Some("content_opt_in_required"),
            },
            None,
            &config,
            Instant::now(),
        );
        let json = serde_json::to_string(&response).expect("response json");
        assert!(!json.contains("fixture-only"));
    }

    #[test]
    fn route_predicate_helpers_cover_answer_evidence() {
        let result = DecisionResult {
            status: DecisionStatus::Accepted,
            answer: Some(TypedAnswer::Score {
                score: 1.0,
                probabilities: BTreeMap::from([
                    ("0".to_owned(), 0.1),
                    ("1".to_owned(), 0.8),
                    ("2".to_owned(), 0.1),
                ]),
                confidence: 0.9,
            }),
            evidence: DecisionEvidence {
                score: Some(1.0),
                confidence: Some(0.9),
                ..DecisionEvidence::default()
            },
            fallback: None,
            reason: "accepted".to_owned(),
        };
        let route = route_result(
            &result,
            route_evidence_from_payload(Some("terra"), Some("high")).as_ref(),
        );
        assert_eq!(route.status, RouteStatus::Applied);
        assert_eq!(route.model, Some(crate::route::ModelFamily::Terra));
    }

    #[test]
    fn local_and_helper_branches_are_explicit() {
        assert!(local_findings(ReviewTarget::Prompt, "plain").is_empty());
        assert!(!local_findings(ReviewTarget::Diff, "must return true\nreturn false").is_empty());
        assert_eq!(status_label(ReviewStatus::Completed), "completed");
        assert_eq!(status_label(ReviewStatus::None), "none");
        assert_eq!(status_label(ReviewStatus::Unknown), "unknown");
        assert_eq!(status_label(ReviewStatus::Degraded), "degraded");
        assert_eq!(status_label(ReviewStatus::Failed), "failed");
        assert_eq!(percentile(&[], 50), None);
        assert_eq!(percentile(&[20, 10, 30], 50), Some(20));
        assert_eq!(percentile(&[20, 10, 30], 95), Some(30));
        assert_eq!(
            fix_plan_blocked(FixPlan::not_requested(), "test").status,
            FixStatus::NotRequested
        );
        assert_eq!(
            fix_plan_blocked(FixPlan::requested(Vec::new()), "test").status,
            FixStatus::Blocked
        );
    }
}
