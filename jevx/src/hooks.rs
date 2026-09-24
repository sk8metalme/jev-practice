use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Config;
use crate::cost::{CodexUsage, CostAccumulator, CostEstimate, CostStatus, CostSummary, total_cost};
use crate::error::JevxError;
use crate::hook_dedupe::{CachedHookDecision, DedupeStore};
use crate::ranking::suggest_with_judge;
use crate::redaction::{redact, sha256_hex};
use crate::storage::append_json_line;
use crate::types::{CandidateDecision, Judge, SkillRecord, SuggestInput};

pub const HOOK_SCHEMA_VERSION: u8 = 3;
const LEGACY_HOOK_SCHEMA_VERSION: u8 = 1;
const PREVIOUS_HOOK_SCHEMA_VERSION: u8 = 2;
const REQUIRED_FACTS: [&str; 3] = [
    "task_id=compact-fixture-1",
    "acceptance=preserve-tests",
    "next=run-cargo-test",
];
const SENSITIVE_FIXTURE_MARKERS: [&str; 2] = ["api_key=fixture-only", "secret=fixture-only"];

#[derive(Debug, Clone, Serialize)]
pub struct HookResponse {
    #[serde(rename = "continue")]
    pub continue_running: bool,
    #[serde(rename = "suppressOutput")]
    pub suppress_output: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookShadowRecord {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub mode: String,
    #[serde(rename = "hookEventName")]
    pub hook_event_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(rename = "sessionIdSha256", skip_serializing_if = "Option::is_none")]
    pub session_id_sha256: Option<String>,
    #[serde(rename = "turnIdSha256", skip_serializing_if = "Option::is_none")]
    pub turn_id_sha256: Option<String>,
    #[serde(rename = "modelSha256", skip_serializing_if = "Option::is_none")]
    pub model_sha256: Option<String>,
    #[serde(
        rename = "correlationIdSha256",
        skip_serializing_if = "Option::is_none"
    )]
    pub correlation_id_sha256: Option<String>,
    #[serde(rename = "promptSha256", skip_serializing_if = "Option::is_none")]
    pub prompt_sha256: Option<String>,
    #[serde(rename = "promptChars", skip_serializing_if = "Option::is_none")]
    pub prompt_chars: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<CandidateDecision>,
    #[serde(rename = "selectedSkill", skip_serializing_if = "Option::is_none")]
    pub selected_skill: Option<String>,
    #[serde(rename = "discoveryMs", skip_serializing_if = "Option::is_none")]
    pub discovery_ms: Option<u64>,
    #[serde(rename = "jevResponseMs", skip_serializing_if = "Option::is_none")]
    pub jev_response_ms: Option<u64>,
    #[serde(rename = "totalMs", skip_serializing_if = "Option::is_none")]
    pub total_ms: Option<u64>,
    #[serde(rename = "inputTokens", skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(rename = "outputTokens", skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(rename = "errorCode", skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexUsage>,
    #[serde(rename = "dedupeHit", default)]
    pub dedupe_hit: bool,
    #[serde(rename = "dedupeKeySha256", skip_serializing_if = "Option::is_none")]
    pub dedupe_key_sha256: Option<String>,
    #[serde(rename = "preCompactionUsage", skip_serializing_if = "Option::is_none")]
    pub pre_compaction_usage: Option<TokenUsageSnapshot>,
    #[serde(rename = "compactionUsage", skip_serializing_if = "Option::is_none")]
    pub compaction_usage: Option<TokenUsageSnapshot>,
    #[serde(
        rename = "postCompactionUsage",
        skip_serializing_if = "Option::is_none"
    )]
    pub post_compaction_usage: Option<TokenUsageSnapshot>,
    #[serde(rename = "postCompactionCost", skip_serializing_if = "Option::is_none")]
    pub post_compaction_cost: Option<CostEstimate>,
    #[serde(
        rename = "compactionElapsedMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub compaction_elapsed_ms: Option<u64>,
    #[serde(rename = "tokenSavings", skip_serializing_if = "Option::is_none")]
    pub token_savings: Option<TokenSavings>,
    #[serde(default)]
    pub cost: CostSummary,
    #[serde(rename = "elapsedMs")]
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookShadowResult {
    pub response: HookResponse,
    pub record: HookShadowRecord,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookCorrelationReport {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub mode: String,
    #[serde(rename = "recordCount")]
    pub record_count: usize,
    #[serde(rename = "duplicateGroupCount")]
    pub duplicate_group_count: usize,
    #[serde(rename = "duplicateRecordCount")]
    pub duplicate_record_count: usize,
    #[serde(rename = "eventCounts")]
    pub event_counts: BTreeMap<String, usize>,
    pub groups: Vec<HookCorrelationGroup>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookCorrelationGroup {
    #[serde(
        rename = "correlationIdSha256",
        skip_serializing_if = "Option::is_none"
    )]
    pub correlation_id_sha256: Option<String>,
    #[serde(rename = "sessionIdSha256", skip_serializing_if = "Option::is_none")]
    pub session_id_sha256: Option<String>,
    #[serde(rename = "turnIdSha256", skip_serializing_if = "Option::is_none")]
    pub turn_id_sha256: Option<String>,
    #[serde(rename = "modelSha256", skip_serializing_if = "Option::is_none")]
    pub model_sha256: Option<String>,
    #[serde(rename = "hookEventName")]
    pub hook_event_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub count: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementStatus {
    Measured,
    Estimated,
    Unavailable,
    Degraded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TokenSavings {
    pub before_tokens: Option<u64>,
    pub after_tokens: Option<u64>,
    pub saved_tokens: Option<u64>,
    pub reduction_rate: Option<f64>,
    pub source: String,
    pub status: MeasurementStatus,
}

impl TokenSavings {
    pub fn from_snapshots(
        before: Option<&TokenUsageSnapshot>,
        after: Option<&TokenUsageSnapshot>,
        source: &str,
    ) -> Self {
        let before_tokens = before
            .filter(|usage| usage.has_total_tokens())
            .map(|usage| usage.total_tokens);
        let after_tokens = after
            .filter(|usage| usage.has_total_tokens())
            .map(|usage| usage.total_tokens);
        let (saved_tokens, reduction_rate, status) = match (before_tokens, after_tokens) {
            (Some(before), Some(after)) if after <= before => {
                let saved = before - after;
                let rate = (before > 0).then_some(saved as f64 / before as f64);
                (Some(saved), rate, MeasurementStatus::Measured)
            }
            (Some(_), Some(_)) => (None, None, MeasurementStatus::Degraded),
            _ => (None, None, MeasurementStatus::Unavailable),
        };
        Self {
            before_tokens,
            after_tokens,
            saved_tokens,
            reduction_rate,
            source: source.to_owned(),
            status,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageSnapshot {
    #[serde(rename = "inputTokens", alias = "input_tokens", default)]
    pub input_tokens: u64,
    #[serde(rename = "cachedInputTokens", alias = "cached_input_tokens", default)]
    pub cached_input_tokens: u64,
    #[serde(
        rename = "cacheWriteInputTokens",
        alias = "cache_write_input_tokens",
        default
    )]
    pub cache_write_input_tokens: u64,
    #[serde(rename = "outputTokens", alias = "output_tokens", default)]
    pub output_tokens: u64,
    #[serde(
        rename = "reasoningOutputTokens",
        alias = "reasoning_output_tokens",
        default
    )]
    pub reasoning_output_tokens: u64,
    #[serde(rename = "totalTokens", alias = "total_tokens", default)]
    pub total_tokens: u64,
}

impl TokenUsageSnapshot {
    fn validate(&self, context: &str) -> Result<(), JevxError> {
        if self.cached_input_tokens > self.input_tokens {
            return Err(JevxError::InvalidInput(format!(
                "{context}: cachedInputTokens must not exceed inputTokens"
            )));
        }
        Ok(())
    }

    fn cache_hit_rate(&self) -> Option<f64> {
        (self.input_tokens > 0).then(|| self.cached_input_tokens as f64 / self.input_tokens as f64)
    }

    fn uncached_input_tokens(&self) -> u64 {
        self.input_tokens.saturating_sub(self.cached_input_tokens)
    }

    fn estimated_billable_tokens(&self) -> u64 {
        self.uncached_input_tokens()
            .saturating_add(self.output_tokens)
    }

    fn has_total_tokens(&self) -> bool {
        self.total_tokens > 0
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HookStatsReport {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub mode: String,
    #[serde(rename = "recordCount")]
    pub record_count: usize,
    #[serde(rename = "eventCounts")]
    pub event_counts: BTreeMap<String, usize>,
    #[serde(rename = "decisionCounts")]
    pub decision_counts: BTreeMap<String, usize>,
    #[serde(rename = "errorCount")]
    pub error_count: usize,
    #[serde(rename = "dedupeHitCount")]
    pub dedupe_hit_count: usize,
    #[serde(rename = "dedupeRate")]
    pub dedupe_rate: Option<f64>,
    #[serde(rename = "latencyMsP50")]
    pub latency_ms_p50: Option<u64>,
    #[serde(rename = "latencyMsP95")]
    pub latency_ms_p95: Option<u64>,
    #[serde(rename = "dedupeLatencyMsP50")]
    pub dedupe_latency_ms_p50: Option<u64>,
    #[serde(rename = "dedupeLatencyMsP95")]
    pub dedupe_latency_ms_p95: Option<u64>,
    #[serde(rename = "usageMeasuredRecords")]
    pub usage_measured_records: usize,
    #[serde(rename = "tokenSavings")]
    pub token_savings: TokenSavingsSummary,
    #[serde(rename = "jevCost")]
    pub jev_cost: Option<f64>,
    #[serde(rename = "codexCost")]
    pub codex_cost: Option<f64>,
    #[serde(rename = "totalCost")]
    pub total_cost: Option<f64>,
    #[serde(rename = "costStatusCounts")]
    pub cost_status_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TokenSavingsSummary {
    #[serde(rename = "measuredRecords")]
    pub measured_records: usize,
    #[serde(rename = "beforeTokens")]
    pub before_tokens: Option<u64>,
    #[serde(rename = "afterTokens")]
    pub after_tokens: Option<u64>,
    #[serde(rename = "savedTokens")]
    pub saved_tokens: Option<u64>,
    #[serde(rename = "reductionRate")]
    pub reduction_rate: Option<f64>,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct CorrelationKey {
    correlation_id_sha256: Option<String>,
    session_id_sha256: Option<String>,
    turn_id_sha256: Option<String>,
    model_sha256: Option<String>,
    hook_event_name: String,
    trigger: Option<String>,
    source: Option<String>,
    unkeyed_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompactionEvaluationReport {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub mode: String,
    pub scenario: String,
    #[serde(rename = "runCount")]
    pub run_count: usize,
    pub runs: Vec<CompactionRun>,
    pub summary: CompactionSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompactionRun {
    pub run: usize,
    #[serde(rename = "caseId", skip_serializing_if = "Option::is_none")]
    pub case_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "eventCount")]
    pub event_count: usize,
    #[serde(rename = "observedEvents", skip_serializing_if = "Vec::is_empty")]
    pub observed_events: Vec<String>,
    #[serde(rename = "compactionCompleted")]
    pub compaction_completed: bool,
    #[serde(rename = "requiredFactCount")]
    pub required_fact_count: usize,
    #[serde(rename = "retainedRequiredFacts")]
    pub retained_required_facts: usize,
    #[serde(rename = "secretLeaks")]
    pub secret_leaks: usize,
    #[serde(rename = "inputChars")]
    pub input_chars: usize,
    #[serde(rename = "conversationTurns")]
    pub conversation_turns: usize,
    #[serde(rename = "contextChars")]
    pub context_chars: usize,
    #[serde(rename = "outputChars")]
    pub output_chars: usize,
    #[serde(rename = "durationMs")]
    pub duration_ms: u64,
    #[serde(rename = "failureRecoveryRequired")]
    pub failure_recovery_required: bool,
    #[serde(rename = "toolHistoryItems")]
    pub tool_history_items: usize,
    #[serde(rename = "toolFailureCount")]
    pub tool_failure_count: usize,
    #[serde(rename = "interruptedTurns")]
    pub interrupted_turns: usize,
    #[serde(rename = "recoveryTurns")]
    pub recovery_turns: usize,
    #[serde(rename = "recoveryCompleted")]
    pub recovery_completed: bool,
    #[serde(rename = "preCompactionUsage", skip_serializing_if = "Option::is_none")]
    pub pre_compaction_usage: Option<TokenUsageSnapshot>,
    #[serde(rename = "compactionUsage", skip_serializing_if = "Option::is_none")]
    pub compaction_usage: Option<TokenUsageSnapshot>,
    #[serde(
        rename = "postCompactionUsage",
        skip_serializing_if = "Option::is_none"
    )]
    pub post_compaction_usage: Option<TokenUsageSnapshot>,
    #[serde(
        rename = "postCompactionCacheHitRate",
        skip_serializing_if = "Option::is_none"
    )]
    pub post_compaction_cache_hit_rate: Option<f64>,
    #[serde(
        rename = "postCompactionUncachedInputTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub post_compaction_uncached_input_tokens: Option<u64>,
    #[serde(
        rename = "postCompactionEstimatedBillableTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub post_compaction_estimated_billable_tokens: Option<u64>,
    #[serde(rename = "tokenSavings", skip_serializing_if = "Option::is_none")]
    pub token_savings: Option<TokenSavings>,
    #[serde(rename = "codexUsage", skip_serializing_if = "Option::is_none")]
    pub codex_usage: Option<CodexUsage>,
    #[serde(default)]
    pub cost: CostSummary,
    #[serde(rename = "errorCode", skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompactionSummary {
    pub passed: usize,
    #[serde(rename = "retentionRate")]
    pub retention_rate: Option<f64>,
    #[serde(rename = "secretLeaks")]
    pub secret_leaks: usize,
    #[serde(rename = "errorRate")]
    pub error_rate: Option<f64>,
    #[serde(rename = "compactionCompletionRate")]
    pub compaction_completion_rate: Option<f64>,
    #[serde(rename = "durationMsP50")]
    pub duration_ms_p50: Option<u64>,
    #[serde(rename = "durationMsP95")]
    pub duration_ms_p95: Option<u64>,
    #[serde(rename = "outputCharsP50")]
    pub output_chars_p50: Option<u64>,
    #[serde(rename = "outputCharsP95")]
    pub output_chars_p95: Option<u64>,
    #[serde(rename = "recoveryCompletionRate")]
    pub recovery_completion_rate: Option<f64>,
    #[serde(rename = "toolHistoryItems")]
    pub tool_history_items: usize,
    #[serde(rename = "toolFailureCount")]
    pub tool_failure_count: usize,
    #[serde(rename = "interruptedTurns")]
    pub interrupted_turns: usize,
    #[serde(rename = "recoveryTurns")]
    pub recovery_turns: usize,
    #[serde(rename = "usageMeasuredRuns")]
    pub usage_measured_runs: usize,
    #[serde(rename = "tokenSavingsMeasuredRuns")]
    pub token_savings_measured_runs: usize,
    #[serde(rename = "totalSavedTokens")]
    pub total_saved_tokens: Option<u64>,
    #[serde(rename = "tokenReductionRate")]
    pub token_reduction_rate: Option<f64>,
    #[serde(rename = "totalInputTokens")]
    pub total_input_tokens: u64,
    #[serde(rename = "totalCachedInputTokens")]
    pub total_cached_input_tokens: u64,
    #[serde(rename = "totalCacheWriteInputTokens")]
    pub total_cache_write_input_tokens: u64,
    #[serde(rename = "totalOutputTokens")]
    pub total_output_tokens: u64,
    #[serde(rename = "totalTokens")]
    pub total_tokens: u64,
    #[serde(rename = "postCompactionCacheHitRateP50")]
    pub post_compaction_cache_hit_rate_p50: Option<f64>,
    #[serde(rename = "postCompactionCacheHitRateP95")]
    pub post_compaction_cache_hit_rate_p95: Option<f64>,
    #[serde(rename = "postCompactionUncachedInputTokensP50")]
    pub post_compaction_uncached_input_tokens_p50: Option<u64>,
    #[serde(rename = "postCompactionUncachedInputTokensP95")]
    pub post_compaction_uncached_input_tokens_p95: Option<u64>,
    #[serde(rename = "postCompactionEstimatedBillableTokensP50")]
    pub post_compaction_estimated_billable_tokens_p50: Option<u64>,
    #[serde(rename = "postCompactionEstimatedBillableTokensP95")]
    pub post_compaction_estimated_billable_tokens_p95: Option<u64>,
    #[serde(rename = "jevCost")]
    pub jev_cost: Option<f64>,
    #[serde(rename = "codexCost")]
    pub codex_cost: Option<f64>,
    #[serde(rename = "totalCost")]
    pub total_cost: Option<f64>,
    #[serde(rename = "fallbackExtraCost")]
    pub fallback_extra_cost: Option<f64>,
    #[serde(rename = "costStatusCounts")]
    pub cost_status_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationCompactionCase {
    pub case_id: String,
    pub required_facts: Vec<String>,
    pub follow_up_text: String,
    pub secret_markers: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub compaction_completed: bool,
    pub compaction_duration_ms: u64,
    pub input_chars: usize,
    #[serde(default)]
    pub conversation_turns: usize,
    #[serde(default)]
    pub context_chars: usize,
    #[serde(default)]
    pub failure_recovery_required: bool,
    #[serde(default)]
    pub tool_history_items: usize,
    #[serde(default)]
    pub tool_failure_count: usize,
    #[serde(default)]
    pub interrupted_turns: usize,
    #[serde(default)]
    pub recovery_turns: usize,
    #[serde(default)]
    pub recovery_completed: bool,
    #[serde(default)]
    pub pre_compaction_usage: Option<TokenUsageSnapshot>,
    #[serde(default)]
    pub compaction_usage: Option<TokenUsageSnapshot>,
    #[serde(default)]
    pub post_compaction_usage: Option<TokenUsageSnapshot>,
    #[serde(rename = "postCompactionCost", default)]
    pub post_compaction_cost: Option<CostEstimate>,
    #[serde(rename = "codexUsage", default)]
    pub codex_usage: Option<CodexUsage>,
    pub observed_events: Vec<String>,
}

pub async fn run_shadow(
    input: &str,
    expected_event: Option<&str>,
    skills: &[SkillRecord],
    config: &Config,
    judge: Option<&dyn Judge>,
) -> Result<HookShadowResult, JevxError> {
    let started = Instant::now();
    let payload: Value = serde_json::from_str(input)?;
    let event = hook_event_name(&payload)
        .ok_or_else(|| JevxError::InvalidInput("hook event is required".to_owned()))?;
    if !matches!(
        event,
        "SessionStart" | "PreCompact" | "PostCompact" | "UserPromptSubmit"
    ) {
        return Err(JevxError::InvalidInput(format!(
            "unsupported hook event: {event}"
        )));
    }
    if let Some(expected_event) = expected_event
        && event != expected_event
    {
        return Err(JevxError::InvalidInput(format!(
            "hook event mismatch: expected {expected_event}, got {event}"
        )));
    }

    let prompt = payload.get("prompt").and_then(Value::as_str);
    let session_id = payload.get("session_id").and_then(Value::as_str);
    let turn_id = payload.get("turn_id").and_then(Value::as_str);
    let model = payload.get("model").and_then(Value::as_str);
    let codex = parse_codex_usage(&payload)?;
    let pre_compaction_usage = parse_token_usage(&payload, "preCompactionUsage")?;
    let compaction_usage = parse_token_usage(&payload, "compactionUsage")?;
    let post_compaction_usage = parse_token_usage(&payload, "postCompactionUsage")?;
    let post_compaction_cost = parse_post_compaction_cost(&payload)?;
    let token_savings = TokenSavings::from_snapshots(
        pre_compaction_usage.as_ref(),
        post_compaction_usage.as_ref(),
        "hook_payload",
    );
    let correlation_id_sha256 = match (session_id, turn_id) {
        (Some(session_id), Some(turn_id)) => {
            let value = format!("{session_id}:{turn_id}");
            Some(sha256_hex(&value))
        }
        (Some(session_id), None) => Some(sha256_hex(session_id)),
        _ => None,
    };
    let mut record = HookShadowRecord {
        schema_version: HOOK_SCHEMA_VERSION,
        mode: "shadow".to_owned(),
        hook_event_name: event.to_owned(),
        trigger: sanitized_identifier(payload.get("trigger").and_then(Value::as_str), 32),
        source: sanitized_identifier(payload.get("source").and_then(Value::as_str), 64),
        session_id_sha256: session_id.map(sha256_hex),
        turn_id_sha256: turn_id.map(sha256_hex),
        model_sha256: model.map(sha256_hex),
        correlation_id_sha256,
        prompt_sha256: prompt.map(sha256_hex),
        prompt_chars: prompt.map(|value| value.chars().count()),
        decision: None,
        selected_skill: None,
        discovery_ms: None,
        jev_response_ms: None,
        total_ms: None,
        input_tokens: None,
        output_tokens: None,
        error_code: None,
        codex,
        dedupe_hit: false,
        dedupe_key_sha256: None,
        pre_compaction_usage,
        compaction_usage,
        post_compaction_usage,
        post_compaction_cost,
        compaction_elapsed_ms: payload.get("compactionElapsedMs").and_then(Value::as_u64),
        token_savings: None,
        cost: CostSummary::default(),
        elapsed_ms: 0,
    };
    record.token_savings = match (
        record.pre_compaction_usage.as_ref(),
        record.post_compaction_usage.as_ref(),
    ) {
        (Some(_), Some(_)) => Some(token_savings),
        _ => None,
    };

    if event != "UserPromptSubmit" {
        record.cost.jev = CostEstimate::actual(0.0, None, None);
    }

    if event == "UserPromptSubmit" {
        let cwd = payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let dedupe_key = user_prompt_dedupe_key(&record, &cwd, skills, config);
        record.dedupe_key_sha256 = dedupe_key.clone();
        let dedupe_store = config.telemetry_path.parent().map(DedupeStore::new);
        let mut dedupe_error = false;
        if let (Some(key), Some(store)) = (dedupe_key.as_deref(), dedupe_store.as_ref()) {
            match store.lookup(key) {
                Ok(Some(cached)) => {
                    record.dedupe_hit = true;
                    record.decision = Some(cached.decision);
                    record.selected_skill = cached.selected_skill;
                    record.cost.jev = CostEstimate::actual(0.0, None, None);
                    apply_provider_cost(&mut record);
                    record.elapsed_ms = elapsed_ms(started);
                    return Ok(shadow_result(record));
                }
                Ok(None) => {}
                Err(_) => dedupe_error = true,
            }
        }
        let Some(prompt) = prompt.map(str::trim).filter(|value| !value.is_empty()) else {
            record.cost.jev = CostEstimate::actual(0.0, None, None);
            record.error_code = Some(
                if dedupe_error {
                    "dedupe_error"
                } else {
                    "invalid_input"
                }
                .to_owned(),
            );
            record.elapsed_ms = elapsed_ms(started);
            return Ok(shadow_result(record));
        };
        let Some(judge) = judge else {
            record.cost.jev = CostEstimate::actual(0.0, None, None);
            record.error_code = Some(
                if dedupe_error {
                    "dedupe_error"
                } else {
                    "missing_api_key"
                }
                .to_owned(),
            );
            record.elapsed_ms = elapsed_ms(started);
            return Ok(shadow_result(record));
        };
        let input = SuggestInput::new(prompt.to_owned(), cwd);
        match suggest_with_judge(input, skills.to_vec(), config, judge).await {
            Ok(result) => {
                let decision = result.decision.clone();
                record.decision = Some(decision.clone());
                record.selected_skill = result
                    .selected
                    .and_then(|candidate| sanitized_skill_identifier(Some(&candidate.id), 128));
                record.discovery_ms = Some(result.metrics.discovery_ms);
                record.jev_response_ms = Some(result.metrics.jev_response_ms);
                record.total_ms = Some(result.metrics.total_ms);
                record.input_tokens = result.metrics.input_tokens;
                record.output_tokens = result.metrics.output_tokens;
                if let Some(cost) = result.metrics.cost {
                    record.cost = cost;
                }
                if dedupe_error {
                    record.error_code = Some("dedupe_error".to_owned());
                }
                if result.metrics.fallback != Some(true)
                    && !matches!(decision, CandidateDecision::Error)
                    && let (Some(key), Some(store)) = (dedupe_key.as_deref(), dedupe_store.as_ref())
                    && store
                        .insert(CachedHookDecision {
                            key_sha256: key.to_owned(),
                            created_at_ms: 0,
                            decision,
                            selected_skill: record.selected_skill.clone(),
                            discovery_ms: record.discovery_ms,
                            jev_response_ms: record.jev_response_ms,
                            total_ms: record.total_ms,
                            input_tokens: record.input_tokens,
                            output_tokens: record.output_tokens,
                        })
                        .is_err()
                {
                    record.error_code = Some("dedupe_error".to_owned());
                }
            }
            Err(error) => {
                record.error_code = Some(
                    if dedupe_error {
                        "dedupe_error"
                    } else {
                        error_code(&error)
                    }
                    .to_owned(),
                )
            }
        }
    }
    apply_provider_cost(&mut record);
    record.elapsed_ms = elapsed_ms(started);
    Ok(shadow_result(record))
}

fn parse_token_usage(
    payload: &Value,
    field: &str,
) -> Result<Option<TokenUsageSnapshot>, JevxError> {
    let Some(value) = payload.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let mut usage: TokenUsageSnapshot = serde_json::from_value(value.clone())
        .map_err(|_| JevxError::InvalidInput(format!("{field} must be a valid object")))?;
    if usage.cached_input_tokens == 0 {
        usage.cached_input_tokens = value
            .get("input_tokens_details")
            .or_else(|| value.get("inputTokensDetails"))
            .and_then(Value::as_object)
            .and_then(|details| {
                details
                    .get("cached_tokens")
                    .or_else(|| details.get("cachedTokens"))
            })
            .and_then(Value::as_u64)
            .unwrap_or(0);
    }
    if usage.cache_write_input_tokens == 0 {
        usage.cache_write_input_tokens = value
            .get("input_tokens_details")
            .or_else(|| value.get("inputTokensDetails"))
            .and_then(Value::as_object)
            .and_then(|details| {
                details
                    .get("cache_write_tokens")
                    .or_else(|| details.get("cacheWriteTokens"))
            })
            .and_then(Value::as_u64)
            .unwrap_or(0);
    }
    if usage.reasoning_output_tokens == 0 {
        usage.reasoning_output_tokens = value
            .get("output_tokens_details")
            .or_else(|| value.get("outputTokensDetails"))
            .and_then(Value::as_object)
            .and_then(|details| {
                details
                    .get("reasoning_tokens")
                    .or_else(|| details.get("reasoningTokens"))
            })
            .and_then(Value::as_u64)
            .unwrap_or(0);
    }
    usage.validate(field)?;
    Ok(Some(usage))
}

fn parse_post_compaction_cost(payload: &Value) -> Result<Option<CostEstimate>, JevxError> {
    let Some(value) = payload.get("postCompactionCost") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let cost: CostEstimate = serde_json::from_value(value.clone()).map_err(|_| {
        JevxError::InvalidInput("postCompactionCost must be a valid object".to_owned())
    })?;
    if !cost.is_valid() {
        return Err(JevxError::InvalidInput(
            "postCompactionCost contains invalid cost metadata".to_owned(),
        ));
    }
    Ok(Some(cost))
}

fn apply_provider_cost(record: &mut HookShadowRecord) {
    if let Some(cost) = &record.post_compaction_cost {
        record.cost.codex = cost.clone();
        record.cost.total = total_cost(&record.cost.jev, &record.cost.codex);
    } else if let Some(codex) = &record.codex {
        record.cost.codex = codex.cost.clone();
        record.cost.total = total_cost(&record.cost.jev, &record.cost.codex);
    }
}

pub(crate) fn hook_event_name(payload: &Value) -> Option<&str> {
    payload
        .get("hook_event_name")
        .or_else(|| payload.get("event"))
        .and_then(Value::as_str)
}

fn user_prompt_dedupe_key(
    record: &HookShadowRecord,
    cwd: &std::path::Path,
    skills: &[SkillRecord],
    config: &Config,
) -> Option<String> {
    let (Some(session), Some(turn), Some(prompt)) = (
        record.session_id_sha256.as_deref(),
        record.turn_id_sha256.as_deref(),
        record.prompt_sha256.as_deref(),
    ) else {
        return None;
    };
    let value = [
        session,
        turn,
        record.model_sha256.as_deref().unwrap_or(""),
        &record.hook_event_name,
        record.trigger.as_deref().unwrap_or(""),
        record.source.as_deref().unwrap_or(""),
        prompt,
        &sha256_hex(cwd.to_string_lossy().as_ref()),
        &skills_digest(skills),
        &config_digest(config),
    ]
    .join("\u{1f}");
    Some(sha256_hex(&value))
}

fn skills_digest(skills: &[SkillRecord]) -> String {
    let mut descriptors = skills
        .iter()
        .map(|skill| {
            format!(
                "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
                skill.id,
                skill.name,
                skill.description,
                skill.path.to_string_lossy(),
                skill.source
            )
        })
        .collect::<Vec<_>>();
    descriptors.sort();
    sha256_hex(&descriptors.join("\u{1e}"))
}

fn config_digest(config: &Config) -> String {
    sha256_hex(&format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{:.17}\u{1f}{:.17}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{:.17}\u{1f}{:.17}\u{1f}{}\u{1f}{}",
        config.endpoint,
        config.timeout.as_millis(),
        config.max_candidates,
        config.min_probability,
        config.min_margin,
        config.max_state_bytes,
        config.max_retries,
        config.retry_backoff_ms,
        config.cache_capacity,
        config.input_cost_weight,
        config.output_cost_weight,
        config.price_currency,
        config.price_version,
    ))
}

fn parse_codex_usage(payload: &Value) -> Result<Option<CodexUsage>, JevxError> {
    let Some(value) = payload
        .get("codexUsage")
        .or_else(|| payload.get("codex"))
        .or_else(|| payload.get("usage"))
    else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let mut usage: CodexUsage = serde_json::from_value(value.clone())
        .map_err(|_| JevxError::InvalidInput("codexUsage must be a valid object".to_owned()))?;
    if usage.cached_input_tokens.is_none() {
        usage.cached_input_tokens = nested_usage_token(
            value,
            "input_tokens_details",
            "inputTokensDetails",
            "cached_tokens",
            "cachedTokens",
        );
    }
    if usage.cache_write_input_tokens.is_none() {
        usage.cache_write_input_tokens = nested_usage_token(
            value,
            "input_tokens_details",
            "inputTokensDetails",
            "cache_write_tokens",
            "cacheWriteTokens",
        );
    }
    if usage.reasoning_tokens.is_none() {
        usage.reasoning_tokens = nested_usage_token(
            value,
            "output_tokens_details",
            "outputTokensDetails",
            "reasoning_tokens",
            "reasoningTokens",
        );
    }
    usage.model = sanitized_identifier(usage.model.as_deref(), 128);
    usage.reasoning_effort = sanitized_identifier(usage.reasoning_effort.as_deref(), 32);
    usage.fallback_stage = sanitized_identifier(usage.fallback_stage.as_deref(), 64);
    if !usage.is_valid() {
        return Err(JevxError::InvalidInput(
            "codexUsage contains invalid cost metadata".to_owned(),
        ));
    }
    Ok(Some(usage))
}

fn nested_usage_token(
    value: &Value,
    snake_section: &str,
    camel_section: &str,
    snake_field: &str,
    camel_field: &str,
) -> Option<u64> {
    value
        .get(snake_section)
        .or_else(|| value.get(camel_section))
        .and_then(Value::as_object)
        .and_then(|details| {
            details
                .get(snake_field)
                .or_else(|| details.get(camel_field))
        })
        .and_then(Value::as_u64)
}

fn shadow_result(record: HookShadowRecord) -> HookShadowResult {
    HookShadowResult {
        response: HookResponse {
            continue_running: true,
            suppress_output: true,
        },
        record,
    }
}

pub fn append_shadow_record(path: &Path, record: &HookShadowRecord) -> Result<(), JevxError> {
    validate_hook_record(record, "hook record")?;
    append_json_line(path, record)
}

pub fn load_hook_records(path: &Path) -> Result<Vec<HookShadowRecord>, JevxError> {
    let content = fs::read_to_string(path)?;
    let mut records = Vec::new();
    for (line_number, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut record = serde_json::from_str::<HookShadowRecord>(line).map_err(|_| {
            JevxError::InvalidInput(format!("invalid hook record at line {}", line_number + 1))
        })?;
        normalize_loaded_hook_record(&mut record, &format!("line {}", line_number + 1))?;
        records.push(record);
    }
    if records.is_empty() {
        return Err(JevxError::InvalidInput(
            "hook records must contain at least one record".to_owned(),
        ));
    }
    Ok(records)
}

pub fn analyze_hook_correlations(
    records: &[HookShadowRecord],
) -> Result<HookCorrelationReport, JevxError> {
    if records.is_empty() {
        return Err(JevxError::InvalidInput(
            "hook records must contain at least one record".to_owned(),
        ));
    }

    let mut event_counts = BTreeMap::new();
    let mut groups = BTreeMap::<CorrelationKey, usize>::new();
    for (index, record) in records.iter().enumerate() {
        validate_hook_record(record, "hook record")?;
        *event_counts
            .entry(record.hook_event_name.clone())
            .or_insert(0) += 1;
        let correlation_id_sha256 = record.correlation_id_sha256.clone();
        let session_id_sha256 = record.session_id_sha256.clone();
        let turn_id_sha256 = record.turn_id_sha256.clone();
        let model_sha256 = record.model_sha256.clone();
        let has_identity = correlation_id_sha256.is_some()
            || session_id_sha256.is_some()
            || turn_id_sha256.is_some();
        let key = CorrelationKey {
            correlation_id_sha256,
            session_id_sha256,
            turn_id_sha256,
            model_sha256,
            hook_event_name: record.hook_event_name.clone(),
            trigger: record.trigger.clone(),
            source: record.source.clone(),
            unkeyed_index: (!has_identity).then_some(index),
        };
        *groups.entry(key).or_insert(0) += 1;
    }

    let mut groups = groups
        .into_iter()
        .map(|(key, count)| HookCorrelationGroup {
            correlation_id_sha256: key.correlation_id_sha256,
            session_id_sha256: key.session_id_sha256,
            turn_id_sha256: key.turn_id_sha256,
            model_sha256: key.model_sha256,
            hook_event_name: key.hook_event_name,
            trigger: key.trigger,
            source: key.source,
            count,
        })
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.hook_event_name.cmp(&right.hook_event_name))
            .then_with(|| left.trigger.cmp(&right.trigger))
            .then_with(|| left.source.cmp(&right.source))
    });
    let duplicate_group_count = groups.iter().filter(|group| group.count > 1).count();
    let duplicate_record_count = groups
        .iter()
        .filter(|group| group.count > 1)
        .map(|group| group.count - 1)
        .sum();

    Ok(HookCorrelationReport {
        schema_version: HOOK_SCHEMA_VERSION,
        mode: "live".to_owned(),
        record_count: records.len(),
        duplicate_group_count,
        duplicate_record_count,
        event_counts,
        groups,
    })
}

pub fn analyze_hook_stats(records: &[HookShadowRecord]) -> Result<HookStatsReport, JevxError> {
    if records.is_empty() {
        return Err(JevxError::InvalidInput(
            "hook records must contain at least one record".to_owned(),
        ));
    }
    let mut event_counts = BTreeMap::new();
    let mut decision_counts = BTreeMap::new();
    let mut cost_status_counts = BTreeMap::new();
    let mut latencies = Vec::with_capacity(records.len());
    let mut dedupe_latencies = Vec::new();
    let mut dedupe_hit_count = 0;
    let mut dedupe_target_count = 0;
    let mut error_count = 0;
    let mut usage_measured_records = 0;
    let mut measured_savings = Vec::new();
    let mut jev_cost = CostAccumulator::default();
    let mut codex_cost = CostAccumulator::default();
    let mut total_cost_value = CostAccumulator::default();

    for record in records {
        validate_hook_record(record, "hook record")?;
        *event_counts
            .entry(record.hook_event_name.clone())
            .or_insert(0) += 1;
        if let Some(decision) = &record.decision {
            *decision_counts
                .entry(decision_label(decision).to_owned())
                .or_insert(0) += 1;
        }
        if record.error_code.is_some() {
            error_count += 1;
        }
        latencies.push(record.elapsed_ms);
        if record.dedupe_hit {
            dedupe_hit_count += 1;
            dedupe_latencies.push(record.elapsed_ms);
        }
        if record.hook_event_name == "UserPromptSubmit" && record.dedupe_key_sha256.is_some() {
            dedupe_target_count += 1;
        }
        if record
            .codex
            .as_ref()
            .is_some_and(CodexUsage::has_usage_tokens)
        {
            usage_measured_records += 1;
        }
        if let Some(savings) = &record.token_savings
            && savings.status == MeasurementStatus::Measured
        {
            measured_savings.push(savings);
        }
        jev_cost.add(&record.cost.jev);
        codex_cost.add(&record.cost.codex);
        total_cost_value.add(&record.cost.total);
        for (name, estimate) in [
            ("jev", &record.cost.jev),
            ("codex", &record.cost.codex),
            ("total", &record.cost.total),
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
    }

    let before_tokens = measured_savings
        .iter()
        .filter_map(|savings| savings.before_tokens)
        .try_fold(0_u64, |sum, value| sum.checked_add(value));
    let after_tokens = measured_savings
        .iter()
        .filter_map(|savings| savings.after_tokens)
        .try_fold(0_u64, |sum, value| sum.checked_add(value));
    let saved_tokens = measured_savings
        .iter()
        .filter_map(|savings| savings.saved_tokens)
        .try_fold(0_u64, |sum, value| sum.checked_add(value));
    let before_tokens = (!measured_savings.is_empty())
        .then_some(before_tokens)
        .flatten();
    let after_tokens = (!measured_savings.is_empty())
        .then_some(after_tokens)
        .flatten();
    let saved_tokens = (!measured_savings.is_empty())
        .then_some(saved_tokens)
        .flatten();
    let token_reduction_rate = match (before_tokens, saved_tokens) {
        (Some(before), Some(saved)) if before > 0 => Some(saved as f64 / before as f64),
        _ => None,
    };

    Ok(HookStatsReport {
        schema_version: HOOK_SCHEMA_VERSION,
        mode: "live".to_owned(),
        record_count: records.len(),
        event_counts,
        decision_counts,
        error_count,
        dedupe_hit_count,
        dedupe_rate: ratio(dedupe_hit_count, dedupe_target_count),
        latency_ms_p50: percentile(&latencies, 50),
        latency_ms_p95: percentile(&latencies, 95),
        dedupe_latency_ms_p50: percentile(&dedupe_latencies, 50),
        dedupe_latency_ms_p95: percentile(&dedupe_latencies, 95),
        usage_measured_records,
        token_savings: TokenSavingsSummary {
            measured_records: measured_savings.len(),
            before_tokens,
            after_tokens,
            saved_tokens,
            reduction_rate: token_reduction_rate,
        },
        jev_cost: jev_cost.amount(),
        codex_cost: codex_cost.amount(),
        total_cost: total_cost_value.amount(),
        cost_status_counts,
    })
}

pub fn write_compaction_report(
    path: &Path,
    report: &CompactionEvaluationReport,
) -> Result<(), JevxError> {
    fs::write(path, serde_json::to_vec_pretty(report)?)?;
    Ok(())
}

pub fn compact_evaluation(run_count: usize) -> Result<CompactionEvaluationReport, JevxError> {
    if run_count == 0 {
        return Err(JevxError::InvalidInput(
            "compaction runs must be greater than zero".to_owned(),
        ));
    }
    let mut runs = Vec::with_capacity(run_count);
    for run in 1..=run_count {
        runs.push(compact_once(run));
    }
    Ok(build_compaction_report(
        "shadow",
        "synthetic-codex-compaction-v1",
        runs,
    ))
}

pub fn load_conversation_cases(path: &Path) -> Result<Vec<ConversationCompactionCase>, JevxError> {
    let content = fs::read_to_string(path)?;
    let mut cases = Vec::new();
    for (line_number, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let case = serde_json::from_str::<ConversationCompactionCase>(line).map_err(|_| {
            JevxError::InvalidInput(format!(
                "invalid conversation fixture at line {}",
                line_number + 1
            ))
        })?;
        validate_conversation_case(&case, &format!("line {}", line_number + 1))?;
        cases.push(case);
    }
    if cases.is_empty() {
        return Err(JevxError::InvalidInput(
            "conversation fixture must contain at least one case".to_owned(),
        ));
    }
    Ok(cases)
}

pub fn evaluate_conversation_compaction(
    cases: &[ConversationCompactionCase],
) -> Result<CompactionEvaluationReport, JevxError> {
    if cases.is_empty() {
        return Err(JevxError::InvalidInput(
            "conversation cases must contain at least one case".to_owned(),
        ));
    }
    for case in cases {
        validate_conversation_case(case, "conversation case")?;
    }
    let runs = cases
        .iter()
        .enumerate()
        .map(|(index, case)| conversation_once(index + 1, case))
        .collect();
    Ok(build_compaction_report(
        "live",
        "real-conversation-codex-v1",
        runs,
    ))
}

fn validate_conversation_case(
    case: &ConversationCompactionCase,
    context: &str,
) -> Result<(), JevxError> {
    if !safe_identifier(&case.case_id, 64) {
        return Err(JevxError::InvalidInput(format!(
            "{context}: caseId must be a short ASCII identifier"
        )));
    }
    if case.required_facts.is_empty() || case.required_facts.len() > 32 {
        return Err(JevxError::InvalidInput(format!(
            "{context}: requiredFacts must contain 1..32 items"
        )));
    }
    if case
        .required_facts
        .iter()
        .any(|fact| fact.is_empty() || fact.chars().count() > 512)
    {
        return Err(JevxError::InvalidInput(format!(
            "{context}: requiredFacts contains an invalid item"
        )));
    }
    if case.secret_markers.is_empty() || case.secret_markers.len() > 32 {
        return Err(JevxError::InvalidInput(format!(
            "{context}: secretMarkers must contain 1..32 items"
        )));
    }
    if case
        .model
        .as_deref()
        .is_some_and(|model| !safe_identifier(model, 128))
    {
        return Err(JevxError::InvalidInput(format!(
            "{context}: model must be a short ASCII identifier"
        )));
    }
    if case
        .secret_markers
        .iter()
        .any(|marker| marker.is_empty() || marker.chars().count() > 512)
    {
        return Err(JevxError::InvalidInput(format!(
            "{context}: secretMarkers contains an invalid item"
        )));
    }
    if case.follow_up_text.chars().count() > 1_000_000 {
        return Err(JevxError::InvalidInput(format!(
            "{context}: followUpText is too large"
        )));
    }
    if case.input_chars > 1_000_000 {
        return Err(JevxError::InvalidInput(format!(
            "{context}: inputChars is too large"
        )));
    }
    if case.conversation_turns > 1_024 {
        return Err(JevxError::InvalidInput(format!(
            "{context}: conversationTurns is too large"
        )));
    }
    if case.context_chars > 10_000_000 {
        return Err(JevxError::InvalidInput(format!(
            "{context}: contextChars is too large"
        )));
    }
    if case.tool_failure_count > case.tool_history_items {
        return Err(JevxError::InvalidInput(format!(
            "{context}: toolFailureCount must not exceed toolHistoryItems"
        )));
    }
    if case.recovery_turns > case.conversation_turns {
        return Err(JevxError::InvalidInput(format!(
            "{context}: recoveryTurns must not exceed conversationTurns"
        )));
    }
    for (label, usage) in [
        ("preCompactionUsage", case.pre_compaction_usage.as_ref()),
        ("compactionUsage", case.compaction_usage.as_ref()),
        ("postCompactionUsage", case.post_compaction_usage.as_ref()),
    ] {
        if let Some(usage) = usage {
            usage.validate(&format!("{context}: {label}"))?;
        }
    }
    if let Some(cost) = &case.post_compaction_cost
        && !cost.is_valid()
    {
        return Err(JevxError::InvalidInput(format!(
            "{context}: postCompactionCost has invalid status, amount, or metadata"
        )));
    }
    if let Some(usage) = &case.codex_usage
        && !usage.is_valid()
    {
        return Err(JevxError::InvalidInput(format!(
            "{context}: codexUsage contains unsafe metadata"
        )));
    }
    if case.observed_events.is_empty() || case.observed_events.len() > 32 {
        return Err(JevxError::InvalidInput(format!(
            "{context}: observedEvents must contain 1..32 items"
        )));
    }
    if case
        .observed_events
        .iter()
        .any(|event| !safe_identifier(event, 64))
    {
        return Err(JevxError::InvalidInput(format!(
            "{context}: observedEvents contains an invalid item"
        )));
    }
    Ok(())
}

fn validate_hook_record(record: &HookShadowRecord, context: &str) -> Result<(), JevxError> {
    if record.schema_version != HOOK_SCHEMA_VERSION {
        return Err(JevxError::InvalidInput(format!(
            "{context}: unsupported hook record schema"
        )));
    }
    if !matches!(
        record.hook_event_name.as_str(),
        "SessionStart" | "PreCompact" | "PostCompact" | "UserPromptSubmit"
    ) {
        return Err(JevxError::InvalidInput(format!(
            "{context}: unsupported hook event"
        )));
    }
    if !safe_identifier(&record.mode, 32)
        || record
            .trigger
            .as_deref()
            .is_some_and(|value| !safe_identifier(value, 32))
        || record
            .source
            .as_deref()
            .is_some_and(|value| !safe_identifier(value, 64))
        || record
            .selected_skill
            .as_deref()
            .is_some_and(|value| !safe_skill_identifier(value, 128))
        || record
            .session_id_sha256
            .as_deref()
            .is_some_and(|value| !safe_identifier(value, 128))
        || record
            .turn_id_sha256
            .as_deref()
            .is_some_and(|value| !safe_identifier(value, 128))
        || record
            .model_sha256
            .as_deref()
            .is_some_and(|value| !safe_identifier(value, 128))
        || record
            .correlation_id_sha256
            .as_deref()
            .is_some_and(|value| !safe_identifier(value, 128))
        || record
            .dedupe_key_sha256
            .as_deref()
            .is_some_and(|value| !safe_identifier(value, 128))
        || record.codex.as_ref().is_some_and(|codex| !codex.is_valid())
        || record
            .pre_compaction_usage
            .as_ref()
            .is_some_and(|usage| usage.validate("hook record preCompactionUsage").is_err())
        || record
            .compaction_usage
            .as_ref()
            .is_some_and(|usage| usage.validate("hook record compactionUsage").is_err())
        || record
            .post_compaction_usage
            .as_ref()
            .is_some_and(|usage| usage.validate("hook record postCompactionUsage").is_err())
        || record
            .post_compaction_cost
            .as_ref()
            .is_some_and(|cost| !cost.is_valid())
        || record
            .token_savings
            .as_ref()
            .is_some_and(|savings| !safe_identifier(&savings.source, 32))
    {
        return Err(JevxError::InvalidInput(format!(
            "{context}: hook record contains an unsafe identifier"
        )));
    }
    Ok(())
}

fn normalize_loaded_hook_record(
    record: &mut HookShadowRecord,
    context: &str,
) -> Result<(), JevxError> {
    if record.schema_version == LEGACY_HOOK_SCHEMA_VERSION
        || record.schema_version == PREVIOUS_HOOK_SCHEMA_VERSION
    {
        record.schema_version = HOOK_SCHEMA_VERSION;
    }
    if record.schema_version == HOOK_SCHEMA_VERSION {
        record.trigger = sanitized_identifier(record.trigger.as_deref(), 32);
        record.source = sanitized_identifier(record.source.as_deref(), 64);
        record.selected_skill = sanitized_skill_identifier(record.selected_skill.as_deref(), 128);
        if let Some(codex) = &mut record.codex {
            codex.model = sanitized_identifier(codex.model.as_deref(), 128);
            codex.reasoning_effort = sanitized_identifier(codex.reasoning_effort.as_deref(), 32);
            codex.fallback_stage = sanitized_identifier(codex.fallback_stage.as_deref(), 64);
        }
    }
    validate_hook_record(record, context)
}

fn safe_identifier(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max_chars
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-/.".contains(character))
}

fn safe_skill_identifier(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max_chars
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-/.:".contains(character))
}

fn sanitized_identifier(value: Option<&str>, max_chars: usize) -> Option<String> {
    let value = value?.trim();
    safe_identifier(value, max_chars).then(|| value.to_owned())
}

fn sanitized_skill_identifier(value: Option<&str>, max_chars: usize) -> Option<String> {
    let value = value?.trim();
    safe_skill_identifier(value, max_chars).then(|| value.to_owned())
}

fn conversation_once(run: usize, case: &ConversationCompactionCase) -> CompactionRun {
    let retained_required_facts = case
        .required_facts
        .iter()
        .filter(|fact| fact_is_retained(&case.follow_up_text, fact))
        .count();
    let secret_leaks = case
        .secret_markers
        .iter()
        .filter(|marker| case.follow_up_text.contains(*marker))
        .count();
    let error_code = if !case.compaction_completed {
        Some("compaction_incomplete".to_owned())
    } else if case.failure_recovery_required && !case.recovery_completed {
        Some("recovery_incomplete".to_owned())
    } else if retained_required_facts != case.required_facts.len() {
        Some("required_fact_lost".to_owned())
    } else {
        None
    };
    let post_compaction_cache_hit_rate = case
        .post_compaction_usage
        .as_ref()
        .and_then(TokenUsageSnapshot::cache_hit_rate);
    let post_compaction_uncached_input_tokens = case
        .post_compaction_usage
        .as_ref()
        .map(TokenUsageSnapshot::uncached_input_tokens);
    let post_compaction_estimated_billable_tokens = case
        .post_compaction_usage
        .as_ref()
        .map(TokenUsageSnapshot::estimated_billable_tokens);
    let token_savings = match (
        case.pre_compaction_usage.as_ref(),
        case.post_compaction_usage.as_ref(),
    ) {
        (Some(before), Some(after)) => Some(TokenSavings::from_snapshots(
            Some(before),
            Some(after),
            "fixture",
        )),
        _ => None,
    };
    let codex_cost = case
        .post_compaction_cost
        .clone()
        .or_else(|| case.codex_usage.as_ref().map(|usage| usage.cost.clone()))
        .unwrap_or_else(CostEstimate::unavailable);
    let cost = CostSummary {
        jev: CostEstimate::unavailable(),
        total: total_cost(&CostEstimate::unavailable(), &codex_cost),
        codex: codex_cost,
    };
    CompactionRun {
        run,
        case_id: Some(case.case_id.clone()),
        model: case.model.clone(),
        event_count: case.observed_events.len(),
        observed_events: case.observed_events.clone(),
        compaction_completed: case.compaction_completed,
        required_fact_count: case.required_facts.len(),
        retained_required_facts,
        secret_leaks,
        input_chars: case.input_chars,
        conversation_turns: case.conversation_turns,
        context_chars: case.context_chars,
        output_chars: case.follow_up_text.chars().count(),
        duration_ms: case.compaction_duration_ms,
        failure_recovery_required: case.failure_recovery_required,
        tool_history_items: case.tool_history_items,
        tool_failure_count: case.tool_failure_count,
        interrupted_turns: case.interrupted_turns,
        recovery_turns: case.recovery_turns,
        recovery_completed: case.recovery_completed,
        pre_compaction_usage: case.pre_compaction_usage.clone(),
        compaction_usage: case.compaction_usage.clone(),
        post_compaction_usage: case.post_compaction_usage.clone(),
        post_compaction_cache_hit_rate,
        post_compaction_uncached_input_tokens,
        post_compaction_estimated_billable_tokens,
        token_savings,
        codex_usage: case.codex_usage.clone(),
        cost,
        error_code,
    }
}

fn fact_is_retained(response: &str, fact: &str) -> bool {
    if response.contains(fact) {
        return true;
    }
    let Some((key, value)) = fact.split_once('=') else {
        return false;
    };
    response.contains(&format!("{key}: {value}")) || response.contains(&format!("{key}:{value}"))
}

fn build_compaction_report(
    mode: &str,
    scenario: &str,
    runs: Vec<CompactionRun>,
) -> CompactionEvaluationReport {
    let run_count = runs.len();
    let passed = runs
        .iter()
        .filter(|run| run.error_code.is_none() && run.secret_leaks == 0)
        .filter(|run| run.retained_required_facts == run.required_fact_count)
        .count();
    let required_facts = runs
        .iter()
        .map(|run| run.required_fact_count)
        .sum::<usize>();
    let retained_facts = runs
        .iter()
        .map(|run| run.retained_required_facts)
        .sum::<usize>();
    let durations = runs.iter().map(|run| run.duration_ms).collect::<Vec<_>>();
    let output_chars = runs
        .iter()
        .map(|run| run.output_chars as u64)
        .collect::<Vec<_>>();
    let recovery_required_runs = runs
        .iter()
        .filter(|run| run.failure_recovery_required)
        .count();
    let recovery_completed_runs = runs
        .iter()
        .filter(|run| run.failure_recovery_required && run.recovery_completed)
        .count();
    let post_compaction_usage = runs
        .iter()
        .filter_map(|run| run.post_compaction_usage.as_ref())
        .collect::<Vec<_>>();
    let post_compaction_cache_hit_rates = runs
        .iter()
        .filter_map(|run| run.post_compaction_cache_hit_rate)
        .collect::<Vec<_>>();
    let post_compaction_uncached_input_tokens = runs
        .iter()
        .filter_map(|run| run.post_compaction_uncached_input_tokens)
        .collect::<Vec<_>>();
    let post_compaction_estimated_billable_tokens = runs
        .iter()
        .filter_map(|run| run.post_compaction_estimated_billable_tokens)
        .collect::<Vec<_>>();
    let token_savings = runs
        .iter()
        .filter_map(|run| run.token_savings.as_ref())
        .filter(|savings| savings.status == MeasurementStatus::Measured)
        .collect::<Vec<_>>();
    let total_saved_tokens = (!token_savings.is_empty())
        .then(|| {
            token_savings
                .iter()
                .filter_map(|savings| savings.saved_tokens)
                .try_fold(0_u64, |sum, value| sum.checked_add(value))
        })
        .flatten();
    let total_before_tokens = (!token_savings.is_empty())
        .then(|| {
            token_savings
                .iter()
                .filter_map(|savings| savings.before_tokens)
                .try_fold(0_u64, |sum, value| sum.checked_add(value))
        })
        .flatten();
    let token_reduction_rate = match (total_before_tokens, total_saved_tokens) {
        (Some(before), Some(saved)) if before > 0 => Some(saved as f64 / before as f64),
        _ => None,
    };
    let mut jev_cost = CostAccumulator::default();
    let mut codex_cost = CostAccumulator::default();
    let mut total_cost_value = CostAccumulator::default();
    let mut fallback_extra_cost = CostAccumulator::default();
    let mut cost_status_counts = BTreeMap::new();
    for run in &runs {
        let cost = &run.cost;
        for (name, estimate) in [
            ("jev", &cost.jev),
            ("codex", &cost.codex),
            ("total", &cost.total),
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
        jev_cost.add(&cost.jev);
        codex_cost.add(&cost.codex);
        total_cost_value.add(&cost.total);
        if let Some(estimate) = run
            .codex_usage
            .as_ref()
            .and_then(|usage| usage.additional_cost.as_ref())
        {
            fallback_extra_cost.add(estimate);
        }
    }
    let secret_leaks = runs.iter().map(|run| run.secret_leaks).sum();
    let completed_runs = runs.iter().filter(|run| run.compaction_completed).count();
    let summary = CompactionSummary {
        passed,
        retention_rate: ratio(retained_facts, required_facts),
        secret_leaks,
        error_rate: ratio(run_count.saturating_sub(passed), run_count),
        compaction_completion_rate: ratio(completed_runs, run_count),
        duration_ms_p50: percentile(&durations, 50),
        duration_ms_p95: percentile(&durations, 95),
        output_chars_p50: percentile(&output_chars, 50),
        output_chars_p95: percentile(&output_chars, 95),
        recovery_completion_rate: ratio(recovery_completed_runs, recovery_required_runs),
        tool_history_items: runs.iter().map(|run| run.tool_history_items).sum(),
        tool_failure_count: runs.iter().map(|run| run.tool_failure_count).sum(),
        interrupted_turns: runs.iter().map(|run| run.interrupted_turns).sum(),
        recovery_turns: runs.iter().map(|run| run.recovery_turns).sum(),
        usage_measured_runs: post_compaction_usage.len(),
        token_savings_measured_runs: token_savings.len(),
        total_saved_tokens,
        token_reduction_rate,
        total_input_tokens: post_compaction_usage
            .iter()
            .map(|usage| usage.input_tokens)
            .sum(),
        total_cached_input_tokens: post_compaction_usage
            .iter()
            .map(|usage| usage.cached_input_tokens)
            .sum(),
        total_cache_write_input_tokens: post_compaction_usage
            .iter()
            .map(|usage| usage.cache_write_input_tokens)
            .sum(),
        total_output_tokens: post_compaction_usage
            .iter()
            .map(|usage| usage.output_tokens)
            .sum(),
        total_tokens: post_compaction_usage
            .iter()
            .map(|usage| usage.total_tokens)
            .sum(),
        post_compaction_cache_hit_rate_p50: percentile_f64(&post_compaction_cache_hit_rates, 50),
        post_compaction_cache_hit_rate_p95: percentile_f64(&post_compaction_cache_hit_rates, 95),
        post_compaction_uncached_input_tokens_p50: percentile(
            &post_compaction_uncached_input_tokens,
            50,
        ),
        post_compaction_uncached_input_tokens_p95: percentile(
            &post_compaction_uncached_input_tokens,
            95,
        ),
        post_compaction_estimated_billable_tokens_p50: percentile(
            &post_compaction_estimated_billable_tokens,
            50,
        ),
        post_compaction_estimated_billable_tokens_p95: percentile(
            &post_compaction_estimated_billable_tokens,
            95,
        ),
        jev_cost: jev_cost.amount(),
        codex_cost: codex_cost.amount(),
        total_cost: total_cost_value.amount(),
        fallback_extra_cost: fallback_extra_cost.amount(),
        cost_status_counts,
    };
    CompactionEvaluationReport {
        schema_version: HOOK_SCHEMA_VERSION,
        mode: mode.to_owned(),
        scenario: scenario.to_owned(),
        run_count,
        runs,
        summary,
    }
}

fn compact_once(run: usize) -> CompactionRun {
    let started = Instant::now();
    let input = synthetic_transcript();
    let compacted = redact(&input);
    let retained_required_facts = REQUIRED_FACTS
        .iter()
        .filter(|fact| compacted.contains(*fact))
        .count();
    let secret_leaks = SENSITIVE_FIXTURE_MARKERS
        .iter()
        .filter(|marker| compacted.contains(*marker))
        .count();
    let error_code = (retained_required_facts != REQUIRED_FACTS.len())
        .then_some("required_fact_lost".to_owned());
    CompactionRun {
        run,
        case_id: None,
        model: None,
        event_count: 3,
        observed_events: Vec::new(),
        compaction_completed: true,
        required_fact_count: REQUIRED_FACTS.len(),
        retained_required_facts,
        secret_leaks,
        input_chars: input.chars().count(),
        conversation_turns: 0,
        context_chars: input.chars().count(),
        output_chars: compacted.chars().count(),
        duration_ms: elapsed_ms(started),
        failure_recovery_required: false,
        tool_history_items: 0,
        tool_failure_count: 0,
        interrupted_turns: 0,
        recovery_turns: 0,
        recovery_completed: false,
        pre_compaction_usage: None,
        compaction_usage: None,
        post_compaction_usage: None,
        post_compaction_cache_hit_rate: None,
        post_compaction_uncached_input_tokens: None,
        post_compaction_estimated_billable_tokens: None,
        token_savings: None,
        codex_usage: None,
        cost: CostSummary::default(),
        error_code,
    }
}

fn synthetic_transcript() -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        REQUIRED_FACTS[0],
        "noise=fixture-only",
        REQUIRED_FACTS[1],
        SENSITIVE_FIXTURE_MARKERS[0],
        REQUIRED_FACTS[2],
        SENSITIVE_FIXTURE_MARKERS[1]
    )
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

fn decision_label(decision: &CandidateDecision) -> &'static str {
    match decision {
        CandidateDecision::Selected => "selected",
        CandidateDecision::Explicit => "explicit",
        CandidateDecision::None => "none",
        CandidateDecision::NoCandidates => "no_candidates",
        CandidateDecision::Error => "error",
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

fn percentile_f64(values: &[f64], percentile: usize) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (sorted.len() * percentile).div_ceil(100).saturating_sub(1);
    sorted.get(rank).copied()
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn error_code(error: &JevxError) -> &'static str {
    match error {
        JevxError::InvalidInput(_) => "invalid_input",
        JevxError::MissingApiKey => "missing_api_key",
        JevxError::Provider(_) => "provider_error",
        JevxError::ProviderWithMetrics { .. } => "provider_error",
        JevxError::Timeout => "timeout",
        JevxError::TimeoutWithMetrics { .. } => "timeout",
        JevxError::Io(_) => "io_error",
        JevxError::Json(_) => "json_error",
        JevxError::Yaml(_) => "yaml_error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(event: &str) -> HookShadowRecord {
        HookShadowRecord {
            schema_version: HOOK_SCHEMA_VERSION,
            mode: "shadow".to_owned(),
            hook_event_name: event.to_owned(),
            trigger: None,
            source: None,
            session_id_sha256: None,
            turn_id_sha256: None,
            model_sha256: None,
            correlation_id_sha256: None,
            prompt_sha256: None,
            prompt_chars: None,
            decision: None,
            selected_skill: None,
            discovery_ms: None,
            jev_response_ms: None,
            total_ms: None,
            input_tokens: None,
            output_tokens: None,
            error_code: None,
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
            elapsed_ms: 1,
        }
    }

    #[test]
    fn helper_metrics_and_error_codes_cover_empty_paths() {
        assert!(percentile(&[], 50).is_none());
        assert_eq!(percentile(&[30, 10, 20], 50), Some(20));
        let errors = [
            JevxError::InvalidInput("x".to_owned()),
            JevxError::MissingApiKey,
            JevxError::Provider("x".to_owned()),
            JevxError::ProviderWithMetrics {
                message: "x".to_owned(),
                calls: 1,
                retries: 0,
                response_ms: 2,
            },
            JevxError::Timeout,
            JevxError::TimeoutWithMetrics {
                calls: 1,
                retries: 0,
                response_ms: 2,
            },
            JevxError::Io(std::io::Error::other("x")),
            JevxError::Json(serde_json::from_str::<Value>("{").expect_err("json")),
            JevxError::Yaml(serde_yaml::from_str::<Value>("[").expect_err("yaml")),
        ];
        assert_eq!(errors.iter().map(error_code).count(), 9);
    }

    #[test]
    fn payload_parsers_and_measurements_keep_invalid_states_explicit() {
        let before = TokenUsageSnapshot {
            total_tokens: 10,
            input_tokens: 10,
            ..TokenUsageSnapshot::default()
        };
        let after = TokenUsageSnapshot {
            total_tokens: 11,
            input_tokens: 11,
            ..TokenUsageSnapshot::default()
        };
        assert_eq!(
            TokenSavings::from_snapshots(Some(&before), Some(&after), "test").status,
            MeasurementStatus::Degraded
        );

        let payload = serde_json::json!({
            "preCompactionUsage": {
                "input_tokens": 100,
                "total_tokens": 120,
                "input_tokens_details": {
                    "cached_tokens": 30,
                    "cache_write_tokens": 4
                },
                "output_tokens_details": {"reasoning_tokens": 8}
            }
        });
        let usage = parse_token_usage(&payload, "preCompactionUsage")
            .expect("nested usage")
            .expect("usage present");
        assert_eq!(usage.cached_input_tokens, 30);
        assert_eq!(usage.cache_write_input_tokens, 4);
        assert_eq!(usage.reasoning_output_tokens, 8);

        let responses_usage = serde_json::json!({
            "usage": {
                "input_tokens": 100,
                "output_tokens": 20,
                "input_tokens_details": {
                    "cached_tokens": 30,
                    "cache_write_tokens": 4
                },
                "output_tokens_details": {"reasoning_tokens": 8}
            }
        });
        let codex_usage = parse_codex_usage(&responses_usage)
            .expect("responses usage")
            .expect("responses usage present");
        assert_eq!(codex_usage.input_tokens, Some(100));
        assert_eq!(codex_usage.output_tokens, Some(20));
        assert_eq!(codex_usage.cached_input_tokens, Some(30));
        assert_eq!(codex_usage.cache_write_input_tokens, Some(4));
        assert_eq!(codex_usage.reasoning_tokens, Some(8));

        let invalid_usage = serde_json::json!({
            "preCompactionUsage": {"inputTokens": 1, "cachedInputTokens": 2}
        });
        assert!(parse_token_usage(&invalid_usage, "preCompactionUsage").is_err());
        assert!(
            parse_token_usage(
                &serde_json::json!({"preCompactionUsage": null}),
                "preCompactionUsage"
            )
            .expect("null usage")
            .is_none()
        );

        let valid_cost = serde_json::json!({
            "postCompactionCost": {
                "amount": 0.25,
                "currency": "USD",
                "priceVersion": "provider-usage",
                "status": "available",
                "basis": "actual"
            }
        });
        assert!(
            parse_post_compaction_cost(&valid_cost)
                .expect("valid cost")
                .is_some()
        );
        let invalid_cost = serde_json::json!({
            "postCompactionCost": {
                "amount": -1.0,
                "status": "available",
                "basis": "actual"
            }
        });
        assert!(parse_post_compaction_cost(&invalid_cost).is_err());
        let malformed_cost = serde_json::json!({"postCompactionCost": "not-an-object"});
        assert!(parse_post_compaction_cost(&malformed_cost).is_err());
        assert!(
            parse_post_compaction_cost(&serde_json::json!({"postCompactionCost": null}))
                .expect("null cost")
                .is_none()
        );

        let invalid_codex = serde_json::json!({
            "codexUsage": {
                "cost": {"amount": -1.0, "status": "available", "basis": "actual"}
            }
        });
        assert!(parse_codex_usage(&invalid_codex).is_err());
        assert!(
            parse_codex_usage(&serde_json::json!({"codexUsage": null}))
                .expect("null codex")
                .is_none()
        );
    }

    #[test]
    fn stats_cover_all_decisions_errors_usage_and_cost_statuses() {
        let decisions = [
            CandidateDecision::Selected,
            CandidateDecision::Explicit,
            CandidateDecision::None,
            CandidateDecision::NoCandidates,
            CandidateDecision::Error,
        ];
        let mut records = decisions
            .into_iter()
            .map(|decision| {
                let mut value = record("UserPromptSubmit");
                value.decision = Some(decision);
                value
            })
            .collect::<Vec<_>>();
        records[0].dedupe_hit = true;
        records[0].dedupe_key_sha256 = Some("dedupe-key".to_owned());
        records[0].error_code = Some("provider_error".to_owned());
        records[0].codex = Some(CodexUsage {
            input_tokens: Some(100),
            output_tokens: Some(10),
            ..CodexUsage::default()
        });
        records[1].codex = Some(CodexUsage::default());
        records[0].token_savings = Some(TokenSavings::from_snapshots(
            Some(&TokenUsageSnapshot {
                total_tokens: 100,
                ..TokenUsageSnapshot::default()
            }),
            Some(&TokenUsageSnapshot {
                total_tokens: 50,
                ..TokenUsageSnapshot::default()
            }),
            "test",
        ));
        records[0].cost.jev = CostEstimate::actual(0.1, None, None);
        records[0].cost.codex = CostEstimate::unknown(None, None);
        records[0].cost.total = CostEstimate::unavailable();

        let report = analyze_hook_stats(&records).expect("stats");
        assert_eq!(report.decision_counts.len(), 5);
        assert_eq!(report.error_count, 1);
        assert_eq!(report.dedupe_hit_count, 1);
        assert_eq!(report.dedupe_rate, Some(1.0));
        assert_eq!(report.usage_measured_records, 1);
        assert_eq!(report.token_savings.measured_records, 1);
        assert_eq!(report.cost_status_counts["jev:available"], 1);
        assert_eq!(report.cost_status_counts["codex:unknown"], 1);
        assert_eq!(report.cost_status_counts["total:unavailable"], 5);
        assert!(analyze_hook_stats(&[]).is_err());

        let mut priced = record("PostCompact");
        priced.cost = CostSummary {
            jev: CostEstimate::actual(
                0.1,
                Some("USD".to_owned()),
                Some("provider-usage".to_owned()),
            ),
            codex: CostEstimate::actual(
                0.2,
                Some("USD".to_owned()),
                Some("provider-usage".to_owned()),
            ),
            total: CostEstimate::available(
                0.3,
                Some("USD".to_owned()),
                Some("provider-usage".to_owned()),
            ),
        };
        let priced_report = analyze_hook_stats(&[priced]).expect("priced stats");
        assert_eq!(priced_report.jev_cost, Some(0.1));
        assert_eq!(priced_report.codex_cost, Some(0.2));
        assert_eq!(priced_report.total_cost, Some(0.3));
    }

    #[test]
    fn normalization_and_conversation_validation_remain_safe() {
        let mut value = record("SessionStart");
        value.schema_version = PREVIOUS_HOOK_SCHEMA_VERSION;
        value.trigger = Some("unsafe trigger".to_owned());
        value.source = Some("unsafe source".to_owned());
        value.codex = Some(CodexUsage {
            model: Some("unsafe model".to_owned()),
            reasoning_effort: Some("unsafe reasoning".to_owned()),
            fallback_stage: Some("unsafe fallback".to_owned()),
            ..CodexUsage::default()
        });
        normalize_loaded_hook_record(&mut value, "test").expect("sanitize legacy record");
        assert_eq!(value.schema_version, HOOK_SCHEMA_VERSION);
        assert!(value.trigger.is_none());
        assert!(value.codex.as_ref().expect("codex").model.is_none());

        let mut case = ConversationCompactionCase {
            case_id: "case".to_owned(),
            required_facts: vec!["fact".to_owned()],
            follow_up_text: "follow-up".to_owned(),
            secret_markers: vec!["secret".to_owned()],
            model: None,
            compaction_completed: true,
            compaction_duration_ms: 1,
            input_chars: 1,
            conversation_turns: 1,
            context_chars: 1,
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
            codex_usage: Some(CodexUsage {
                cost: CostEstimate::available(-1.0, None, None),
                ..CodexUsage::default()
            }),
            observed_events: vec!["PostCompact".to_owned()],
        };
        assert!(validate_conversation_case(&case, "case").is_err());
        case.codex_usage = None;
        assert!(validate_conversation_case(&case, "case").is_ok());
        let report = evaluate_conversation_compaction(std::slice::from_ref(&case))
            .expect("compaction report without usage");
        assert_eq!(report.summary.token_savings_measured_runs, 0);
        assert_eq!(report.summary.total_saved_tokens, None);

        assert_eq!(decision_label(&CandidateDecision::Selected), "selected");
        assert_eq!(decision_label(&CandidateDecision::Explicit), "explicit");
        assert_eq!(decision_label(&CandidateDecision::None), "none");
        assert_eq!(
            decision_label(&CandidateDecision::NoCandidates),
            "no_candidates"
        );
        assert_eq!(decision_label(&CandidateDecision::Error), "error");
    }
}
