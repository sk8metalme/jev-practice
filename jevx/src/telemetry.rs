use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::JevxError;
use crate::redaction::sha256_hex;
use crate::storage::append_json_line;
use crate::types::{CandidateDecision, CandidateResult, Metrics};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "promptSha256")]
    pub prompt_sha256: String,
    #[serde(rename = "promptChars")]
    pub prompt_chars: usize,
    pub decision: CandidateDecision,
    #[serde(rename = "selectedSkill", skip_serializing_if = "Option::is_none")]
    pub selected_skill: Option<String>,
    pub metrics: Metrics,
}

impl TelemetryEvent {
    pub fn from_result(
        prompt: &str,
        decision: &CandidateDecision,
        selected: Option<&CandidateResult>,
        metrics: &Metrics,
    ) -> Self {
        Self {
            schema_version: 1,
            prompt_sha256: sha256_hex(prompt),
            prompt_chars: prompt.chars().count(),
            decision: decision.clone(),
            selected_skill: selected.map(|candidate| candidate.id.clone()),
            metrics: metrics.clone(),
        }
    }
}

pub fn append_telemetry(path: &Path, event: &TelemetryEvent) -> Result<(), JevxError> {
    append_json_line(path, event)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    pub events: usize,
    pub selected: usize,
    pub none: usize,
    pub errors: usize,
    #[serde(rename = "selectedRate")]
    pub selected_rate: Option<f64>,
    #[serde(rename = "noneRate")]
    pub none_rate: Option<f64>,
    #[serde(rename = "errorRate")]
    pub error_rate: Option<f64>,
    #[serde(rename = "averageJevResponseMs")]
    pub average_jev_response_ms: f64,
    #[serde(rename = "jevResponseMsP50")]
    pub jev_response_ms_p50: Option<u64>,
    #[serde(rename = "jevResponseMsP95")]
    pub jev_response_ms_p95: Option<u64>,
    #[serde(rename = "totalMsP50")]
    pub total_ms_p50: Option<u64>,
    #[serde(rename = "totalMsP95")]
    pub total_ms_p95: Option<u64>,
    #[serde(rename = "averageInputTokens")]
    pub average_input_tokens: Option<f64>,
    #[serde(rename = "averageOutputTokens")]
    pub average_output_tokens: Option<f64>,
    #[serde(rename = "usageEvents")]
    pub usage_events: usize,
}

pub fn read_stats(path: &Path) -> Result<Stats, JevxError> {
    if !path.exists() {
        return Ok(Stats::default());
    }
    let file = fs::File::open(path)?;
    let mut stats = Stats::default();
    let mut total_response_ms = 0.0_f64;
    let mut response_times = Vec::new();
    let mut total_times = Vec::new();
    let mut input_tokens = Vec::new();
    let mut output_tokens = Vec::new();
    let mut usage_events = 0_usize;
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let event: TelemetryEvent = serde_json::from_str(&line)?;
        stats.events += 1;
        match event.decision {
            CandidateDecision::Selected | CandidateDecision::Explicit => stats.selected += 1,
            CandidateDecision::None | CandidateDecision::NoCandidates => stats.none += 1,
            CandidateDecision::Error => stats.errors += 1,
        }
        total_response_ms += event.metrics.jev_response_ms as f64;
        if event.metrics.jev_response_ms > 0 {
            response_times.push(event.metrics.jev_response_ms);
        }
        total_times.push(event.metrics.total_ms);
        if let Some(tokens) = event.metrics.input_tokens {
            input_tokens.push(tokens);
        }
        if let Some(tokens) = event.metrics.output_tokens {
            output_tokens.push(tokens);
        }
        if event.metrics.input_tokens.is_some() || event.metrics.output_tokens.is_some() {
            usage_events += 1;
        }
    }
    if stats.events > 0 {
        stats.average_jev_response_ms = total_response_ms / stats.events as f64;
        stats.selected_rate = Some(stats.selected as f64 / stats.events as f64);
        stats.none_rate = Some(stats.none as f64 / stats.events as f64);
        stats.error_rate = Some(stats.errors as f64 / stats.events as f64);
    }
    stats.jev_response_ms_p50 = percentile(&response_times, 50);
    stats.jev_response_ms_p95 = percentile(&response_times, 95);
    stats.total_ms_p50 = percentile(&total_times, 50);
    stats.total_ms_p95 = percentile(&total_times, 95);
    stats.average_input_tokens = average(&input_tokens);
    stats.average_output_tokens = average(&output_tokens);
    stats.usage_events = usage_events;
    Ok(stats)
}

fn average(values: &[u64]) -> Option<f64> {
    (!values.is_empty())
        .then(|| values.iter().map(|&value| value as f64).sum::<f64>() / values.len() as f64)
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
