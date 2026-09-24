use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cost::{
    CostAccumulator, CostEstimate, CostStatus, CostSummary, TokenPricing, total_cost,
};
use crate::decision::{
    DecisionContract, DecisionExecution, DecisionFailure, DecisionMode, DecisionResult,
    DecisionStatus, StatePlan, TypedAnswer, replay_contract,
};
use crate::error::JevxError;
use crate::redaction::sha256_hex;
use crate::storage::append_json_line;

pub const RECEIPT_SCHEMA_VERSION: u8 = 2;
const LEGACY_RECEIPT_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionReceipt {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub mode: DecisionMode,
    #[serde(rename = "contractVersion")]
    pub contract_version: String,
    #[serde(rename = "questionId")]
    pub question_id: String,
    #[serde(rename = "questionVersion")]
    pub question_version: String,
    #[serde(rename = "policyVersion")]
    pub policy_version: String,
    #[serde(rename = "stateDigest")]
    pub state_digest: String,
    #[serde(rename = "stateBytes", default)]
    pub state_bytes: usize,
    #[serde(rename = "maxStateBytes", skip_serializing_if = "Option::is_none")]
    pub max_state_bytes: Option<usize>,
    #[serde(rename = "candidateCount")]
    pub candidate_count: usize,
    #[serde(rename = "candidateLimit", skip_serializing_if = "Option::is_none")]
    pub candidate_limit: Option<usize>,
    #[serde(rename = "windowCount")]
    pub window_count: usize,
    pub omitted: Vec<String>,
    #[serde(rename = "redactionReasons")]
    pub redaction_reasons: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<TypedAnswer>,
    #[serde(rename = "answerDigest", skip_serializing_if = "Option::is_none")]
    pub answer_digest: Option<String>,
    pub decision: DecisionStatus,
    pub evidence: crate::decision::DecisionEvidence,
    pub severity: crate::decision::Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<crate::decision::FallbackReason>,
    pub reason: String,
    pub calls: u32,
    pub retries: u32,
    #[serde(rename = "cacheHit")]
    pub cache_hit: bool,
    #[serde(rename = "latencyMs")]
    pub latency_ms: u64,
    #[serde(rename = "inputTokens", skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(rename = "outputTokens", skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(rename = "relativeCost", skip_serializing_if = "Option::is_none")]
    pub relative_cost: Option<f64>,
    #[serde(default)]
    pub cost: CostSummary,
    #[serde(rename = "replayId")]
    pub replay_id: String,
    #[serde(rename = "errorCode", skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

impl DecisionReceipt {
    pub fn from_execution(
        contract: &DecisionContract,
        state: &StatePlan,
        execution: &DecisionExecution,
        input_weight: f64,
        output_weight: f64,
    ) -> Self {
        Self::from_result_with_execution(
            contract,
            state,
            &execution.result,
            execution.mode,
            execution.usage.as_ref().map(|usage| usage.input_tokens),
            execution.usage.as_ref().map(|usage| usage.output_tokens),
            execution.calls,
            execution.retries,
            execution.cache_hit,
            execution.response_ms,
            execution.error_code.clone(),
            execution
                .usage
                .as_ref()
                .and_then(|usage| usage.cost.as_ref().filter(|cost| cost.is_valid()).cloned()),
            input_weight,
            output_weight,
            &TokenPricing::default(),
        )
    }

    pub fn from_execution_with_pricing(
        contract: &DecisionContract,
        state: &StatePlan,
        execution: &DecisionExecution,
        input_weight: f64,
        output_weight: f64,
        pricing: &TokenPricing,
    ) -> Self {
        Self::from_result_with_execution(
            contract,
            state,
            &execution.result,
            execution.mode,
            execution.usage.as_ref().map(|usage| usage.input_tokens),
            execution.usage.as_ref().map(|usage| usage.output_tokens),
            execution.calls,
            execution.retries,
            execution.cache_hit,
            execution.response_ms,
            execution.error_code.clone(),
            execution
                .usage
                .as_ref()
                .and_then(|usage| usage.cost.as_ref().filter(|cost| cost.is_valid()).cloned()),
            input_weight,
            output_weight,
            pricing,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_result(
        contract: &DecisionContract,
        state: &StatePlan,
        result: &DecisionResult,
        usage: Option<(u64, u64)>,
        calls: u32,
        retries: u32,
        cache_hit: bool,
        latency_ms: u64,
    ) -> Self {
        Self::from_result_with_execution(
            contract,
            state,
            result,
            DecisionMode::Live,
            usage.map(|usage| usage.0),
            usage.map(|usage| usage.1),
            calls,
            retries,
            cache_hit,
            latency_ms,
            None,
            None,
            1.0,
            1.0,
            &TokenPricing::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_result_with_execution(
        contract: &DecisionContract,
        state: &StatePlan,
        result: &DecisionResult,
        mode: DecisionMode,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        calls: u32,
        retries: u32,
        cache_hit: bool,
        latency_ms: u64,
        error_code: Option<String>,
        reported_jev_cost: Option<CostEstimate>,
        input_weight: f64,
        output_weight: f64,
        pricing: &TokenPricing,
    ) -> Self {
        let answer = result.answer.clone();
        let answer_digest = answer.as_ref().map(TypedAnswer::digest);
        let replay_id = replay_id(contract, state, answer_digest.as_deref());
        let relative_cost = if cache_hit {
            Some(0.0)
        } else {
            match (input_tokens, output_tokens) {
                (Some(input), Some(output))
                    if input_weight.is_finite()
                        && output_weight.is_finite()
                        && input_weight >= 0.0
                        && output_weight >= 0.0 =>
                {
                    Some(input as f64 * input_weight + output as f64 * output_weight)
                }
                _ => None,
            }
        };
        let jev_cost = reported_jev_cost
            .unwrap_or_else(|| pricing.estimate_two_part(input_tokens, output_tokens, cache_hit));
        let codex_cost = CostEstimate::unavailable();
        let cost = CostSummary {
            total: total_cost(&jev_cost, &codex_cost),
            jev: jev_cost,
            codex: codex_cost,
        };
        Self {
            schema_version: RECEIPT_SCHEMA_VERSION,
            mode,
            contract_version: contract.contract_version.clone(),
            question_id: contract.question_id.clone(),
            question_version: contract.question_version.clone(),
            policy_version: contract.policy_version.clone(),
            state_digest: state.state_digest.clone(),
            state_bytes: state.state_bytes,
            max_state_bytes: state.max_state_bytes,
            candidate_count: state.candidate_count,
            candidate_limit: state.candidate_limit,
            window_count: state.window_count,
            omitted: state.omitted.clone(),
            redaction_reasons: state.redaction_reasons.clone(),
            answer,
            answer_digest,
            decision: result.status,
            evidence: result.evidence.clone(),
            severity: contract.severity(),
            fallback: result.fallback,
            reason: result.reason.clone(),
            calls,
            retries,
            cache_hit,
            latency_ms,
            input_tokens,
            output_tokens,
            relative_cost,
            cost,
            replay_id,
            error_code,
        }
    }
}

pub trait DecisionRecorder: Send + Sync {
    fn record(&self, receipt: &DecisionReceipt) -> Result<(), JevxError>;
}

#[derive(Debug, Clone)]
pub struct JsonlDecisionRecorder {
    path: PathBuf,
}

impl JsonlDecisionRecorder {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl DecisionRecorder for JsonlDecisionRecorder {
    fn record(&self, receipt: &DecisionReceipt) -> Result<(), JevxError> {
        append_json_line(&self.path, receipt)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DecisionStats {
    pub events: usize,
    pub accepted: usize,
    pub none: usize,
    pub unknown: usize,
    pub defer: usize,
    pub degraded: usize,
    #[serde(rename = "fallbackRate")]
    pub fallback_rate: Option<f64>,
    #[serde(rename = "cacheHits")]
    pub cache_hits: usize,
    pub retries: usize,
    #[serde(rename = "latencyMsP50")]
    pub latency_ms_p50: Option<u64>,
    #[serde(rename = "latencyMsP95")]
    pub latency_ms_p95: Option<u64>,
    #[serde(rename = "averageRelativeCost")]
    pub average_relative_cost: Option<f64>,
    #[serde(rename = "usageEvents")]
    pub usage_events: usize,
    #[serde(rename = "jevCost")]
    pub jev_cost: Option<f64>,
    #[serde(rename = "codexCost")]
    pub codex_cost: Option<f64>,
    #[serde(rename = "totalCost")]
    pub total_cost: Option<f64>,
    #[serde(rename = "costStatusCounts")]
    pub cost_status_counts: std::collections::BTreeMap<String, usize>,
}

pub fn read_decision_stats(path: &Path) -> Result<DecisionStats, JevxError> {
    let mut stats = DecisionStats::default();
    let mut latencies = Vec::new();
    let mut costs = Vec::new();
    let mut jev_cost = CostAccumulator::default();
    let mut codex_cost = CostAccumulator::default();
    let mut total_cost = CostAccumulator::default();
    let mut cost_status_counts = std::collections::BTreeMap::new();
    let receipts = read_decision_receipts(path)?;
    let mut fallback_count = 0_usize;
    for receipt in receipts {
        stats.events += 1;
        match receipt.decision {
            DecisionStatus::Accepted => stats.accepted += 1,
            DecisionStatus::None => stats.none += 1,
            DecisionStatus::Unknown => stats.unknown += 1,
            DecisionStatus::Defer => stats.defer += 1,
            DecisionStatus::Degraded => stats.degraded += 1,
        }
        if receipt.fallback.is_some() {
            fallback_count += 1;
        }
        if receipt.cache_hit {
            stats.cache_hits += 1;
        }
        stats.retries += receipt.retries as usize;
        latencies.push(receipt.latency_ms);
        if let Some(cost) = receipt.relative_cost {
            costs.push(cost);
        }
        if receipt.input_tokens.is_some() || receipt.output_tokens.is_some() {
            stats.usage_events += 1;
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
            *cost_status_counts
                .entry(format!("{name}:{status}"))
                .or_insert(0) += 1;
        }
        jev_cost.add(&receipt.cost.jev);
        codex_cost.add(&receipt.cost.codex);
        total_cost.add(&receipt.cost.total);
    }
    stats.latency_ms_p50 = percentile(&latencies, 50);
    stats.latency_ms_p95 = percentile(&latencies, 95);
    stats.fallback_rate = (stats.events > 0).then(|| fallback_count as f64 / stats.events as f64);
    stats.average_relative_cost =
        (!costs.is_empty()).then(|| costs.iter().sum::<f64>() / costs.len() as f64);
    stats.jev_cost = jev_cost.amount();
    stats.codex_cost = codex_cost.amount();
    stats.total_cost = total_cost.amount();
    stats.cost_status_counts = cost_status_counts;
    Ok(stats)
}

pub fn read_decision_receipts(path: &Path) -> Result<Vec<DecisionReceipt>, JevxError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)?;
    let mut receipts = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let receipt: DecisionReceipt = serde_json::from_str(&line)?;
        if !matches!(
            receipt.schema_version,
            LEGACY_RECEIPT_SCHEMA_VERSION | RECEIPT_SCHEMA_VERSION
        ) {
            return Err(JevxError::InvalidInput(
                "unsupported decision receipt schema".to_owned(),
            ));
        }
        receipts.push(receipt);
    }
    Ok(receipts)
}

pub fn replay_receipt(
    receipt: &DecisionReceipt,
    contract: &DecisionContract,
    state: &StatePlan,
) -> Result<DecisionExecution, JevxError> {
    if !matches!(
        receipt.schema_version,
        LEGACY_RECEIPT_SCHEMA_VERSION | RECEIPT_SCHEMA_VERSION
    ) {
        return Err(JevxError::InvalidInput(
            "unsupported decision receipt schema".to_owned(),
        ));
    }
    if receipt.contract_version != contract.contract_version
        || receipt.question_id != contract.question_id
        || receipt.question_version != contract.question_version
        || receipt.policy_version != contract.policy_version
        || receipt.state_digest != state.state_digest
        || (receipt.state_bytes != 0 && receipt.state_bytes != state.state_bytes)
        || receipt.candidate_count != state.candidate_count
        || receipt.window_count != state.window_count
        || receipt.omitted != state.omitted
        || receipt.redaction_reasons != state.redaction_reasons
        || (receipt.max_state_bytes.is_some() && receipt.max_state_bytes != state.max_state_bytes)
        || (receipt.candidate_limit.is_some() && receipt.candidate_limit != state.candidate_limit)
    {
        return Ok(replay_mismatch(contract));
    }
    let answer_digest = receipt.answer.as_ref().map(TypedAnswer::digest);
    if receipt.answer_digest != answer_digest
        || receipt.replay_id != replay_id(contract, state, answer_digest.as_deref())
    {
        return Ok(replay_mismatch(contract));
    }

    if receipt.answer.is_some() {
        let execution = replay_contract(contract, state, receipt.answer.clone());
        let recorded_result = DecisionResult {
            status: receipt.decision,
            answer: receipt.answer.clone(),
            evidence: receipt.evidence.clone(),
            fallback: receipt.fallback,
            reason: receipt.reason.clone(),
        };
        if execution.result != recorded_result || execution.error_code.is_some() {
            return Ok(replay_mismatch(contract));
        }
        return Ok(execution);
    }

    if !matches!(
        receipt.decision,
        DecisionStatus::Unknown | DecisionStatus::Defer | DecisionStatus::Degraded
    ) || receipt.fallback.is_none()
    {
        return Ok(replay_mismatch(contract));
    }
    Ok(DecisionExecution {
        result: DecisionResult {
            status: receipt.decision,
            answer: None,
            evidence: receipt.evidence.clone(),
            fallback: receipt.fallback,
            reason: receipt.reason.clone(),
        },
        mode: DecisionMode::Replay,
        response_ms: 0,
        usage: None,
        calls: 0,
        retries: 0,
        cache_hit: false,
        error_code: receipt.error_code.clone(),
        error_message: None,
    })
}

fn replay_id(
    contract: &DecisionContract,
    state: &StatePlan,
    answer_digest: Option<&str>,
) -> String {
    sha256_hex(&format!(
        "{}:{}:{}:{}:{}",
        contract.contract_version,
        contract.question_id,
        contract.policy_version,
        state.state_digest,
        answer_digest.unwrap_or("none")
    ))
}

fn replay_mismatch(contract: &DecisionContract) -> DecisionExecution {
    DecisionExecution {
        result: contract.failure(DecisionFailure::StateDegraded),
        mode: DecisionMode::Replay,
        response_ms: 0,
        usage: None,
        calls: 0,
        retries: 0,
        cache_hit: false,
        error_code: Some("replay_mismatch".to_owned()),
        error_message: None,
    }
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
