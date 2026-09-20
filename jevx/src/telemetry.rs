use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::JevxError;
use crate::redaction::sha256_hex;
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, event)?;
    file.write_all(b"\n")?;
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    pub events: usize,
    pub selected: usize,
    pub none: usize,
    pub errors: usize,
    #[serde(rename = "averageJevResponseMs")]
    pub average_jev_response_ms: f64,
}

pub fn read_stats(path: &Path) -> Result<Stats, JevxError> {
    if !path.exists() {
        return Ok(Stats::default());
    }
    let file = fs::File::open(path)?;
    let mut stats = Stats::default();
    let mut total_response_ms = 0_u64;
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
        total_response_ms += event.metrics.jev_response_ms;
    }
    if stats.events > 0 {
        stats.average_jev_response_ms = total_response_ms as f64 / stats.events as f64;
    }
    Ok(stats)
}
