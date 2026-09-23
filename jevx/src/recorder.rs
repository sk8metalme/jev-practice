use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::decision::{
    DecisionContract, DecisionExecution, DecisionFailure, DecisionMode, DecisionResult,
    DecisionStatus, StatePlan, TypedAnswer, replay_contract,
};
use crate::error::JevxError;
use crate::redaction::sha256_hex;
use crate::storage::append_json_line;

const RECEIPT_SCHEMA_VERSION: u8 = 1;

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
    #[serde(rename = "candidateCount")]
    pub candidate_count: usize,
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
            input_weight,
            output_weight,
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
            1.0,
            1.0,
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
        input_weight: f64,
        output_weight: f64,
    ) -> Self {
        let answer = result.answer.clone();
        let answer_digest = answer.as_ref().map(TypedAnswer::digest);
        let replay_id = sha256_hex(&format!(
            "{}:{}:{}:{}:{}",
            contract.contract_version,
            contract.question_id,
            contract.policy_version,
            state.state_digest,
            answer_digest.as_deref().unwrap_or("none")
        ));
        let relative_cost = match (input_tokens, output_tokens) {
            (Some(input), Some(output))
                if input_weight.is_finite()
                    && output_weight.is_finite()
                    && input_weight >= 0.0
                    && output_weight >= 0.0 =>
            {
                Some(input as f64 * input_weight + output as f64 * output_weight)
            }
            _ => None,
        };
        Self {
            schema_version: RECEIPT_SCHEMA_VERSION,
            mode,
            contract_version: contract.contract_version.clone(),
            question_id: contract.question_id.clone(),
            question_version: contract.question_version.clone(),
            policy_version: contract.policy_version.clone(),
            state_digest: state.state_digest.clone(),
            candidate_count: state.candidate_count,
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
}

pub fn read_decision_stats(path: &Path) -> Result<DecisionStats, JevxError> {
    let mut stats = DecisionStats::default();
    let mut latencies = Vec::new();
    let mut costs = Vec::new();
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
    }
    stats.latency_ms_p50 = percentile(&latencies, 50);
    stats.latency_ms_p95 = percentile(&latencies, 95);
    stats.fallback_rate = (stats.events > 0).then(|| fallback_count as f64 / stats.events as f64);
    stats.average_relative_cost =
        (!costs.is_empty()).then(|| costs.iter().sum::<f64>() / costs.len() as f64);
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
        if receipt.schema_version != RECEIPT_SCHEMA_VERSION {
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
    if receipt.contract_version != contract.contract_version
        || receipt.question_id != contract.question_id
        || receipt.question_version != contract.question_version
        || receipt.policy_version != contract.policy_version
        || receipt.state_digest != state.state_digest
    {
        return Ok(DecisionExecution {
            result: contract.failure(DecisionFailure::StateDegraded),
            mode: DecisionMode::Replay,
            response_ms: 0,
            usage: None,
            calls: 0,
            retries: 0,
            cache_hit: false,
            error_code: Some("replay_mismatch".to_owned()),
            error_message: None,
        });
    }
    Ok(replay_contract(contract, state, receipt.answer.clone()))
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
