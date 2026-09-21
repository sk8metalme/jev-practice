use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

use serde::Serialize;
use serde_json::Value;

use crate::Config;
use crate::error::JevxError;
use crate::ranking::suggest_with_judge;
use crate::redaction::{redact, sha256_hex};
use crate::types::{CandidateDecision, Judge, SkillRecord, SuggestInput};

const HOOK_SCHEMA_VERSION: u8 = 1;
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

#[derive(Debug, Clone, Serialize)]
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
    #[serde(rename = "elapsedMs")]
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookShadowResult {
    pub response: HookResponse,
    pub record: HookShadowRecord,
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
    #[serde(rename = "eventCount")]
    pub event_count: usize,
    #[serde(rename = "requiredFactCount")]
    pub required_fact_count: usize,
    #[serde(rename = "retainedRequiredFacts")]
    pub retained_required_facts: usize,
    #[serde(rename = "secretLeaks")]
    pub secret_leaks: usize,
    #[serde(rename = "inputChars")]
    pub input_chars: usize,
    #[serde(rename = "outputChars")]
    pub output_chars: usize,
    #[serde(rename = "durationMs")]
    pub duration_ms: u64,
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
    #[serde(rename = "durationMsP50")]
    pub duration_ms_p50: Option<u64>,
    #[serde(rename = "durationMsP95")]
    pub duration_ms_p95: Option<u64>,
    #[serde(rename = "outputCharsP50")]
    pub output_chars_p50: Option<u64>,
    #[serde(rename = "outputCharsP95")]
    pub output_chars_p95: Option<u64>,
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
    let event = payload
        .get("hook_event_name")
        .or_else(|| payload.get("event"))
        .and_then(Value::as_str)
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
    let mut record = HookShadowRecord {
        schema_version: HOOK_SCHEMA_VERSION,
        mode: "shadow".to_owned(),
        hook_event_name: event.to_owned(),
        trigger: payload
            .get("trigger")
            .and_then(Value::as_str)
            .map(str::to_owned),
        source: payload
            .get("source")
            .and_then(Value::as_str)
            .map(str::to_owned),
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
        elapsed_ms: 0,
    };

    if event == "UserPromptSubmit" {
        let Some(prompt) = prompt.map(str::trim).filter(|value| !value.is_empty()) else {
            record.error_code = Some("invalid_input".to_owned());
            record.elapsed_ms = elapsed_ms(started);
            return Ok(shadow_result(record));
        };
        let Some(judge) = judge else {
            record.error_code = Some("missing_api_key".to_owned());
            record.elapsed_ms = elapsed_ms(started);
            return Ok(shadow_result(record));
        };
        let cwd = payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let input = SuggestInput::new(prompt.to_owned(), cwd);
        match suggest_with_judge(input, skills.to_vec(), config, judge).await {
            Ok(result) => {
                record.decision = Some(result.decision);
                record.selected_skill = result.selected.map(|candidate| candidate.id);
                record.discovery_ms = Some(result.metrics.discovery_ms);
                record.jev_response_ms = Some(result.metrics.jev_response_ms);
                record.total_ms = Some(result.metrics.total_ms);
                record.input_tokens = result.metrics.input_tokens;
                record.output_tokens = result.metrics.output_tokens;
            }
            Err(error) => record.error_code = Some(error_code(&error).to_owned()),
        }
    }
    record.elapsed_ms = elapsed_ms(started);
    Ok(shadow_result(record))
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, record)?;
    file.write_all(b"\n")?;
    Ok(())
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
    let secret_leaks = runs.iter().map(|run| run.secret_leaks).sum();
    Ok(CompactionEvaluationReport {
        schema_version: HOOK_SCHEMA_VERSION,
        mode: "shadow".to_owned(),
        scenario: "synthetic-codex-compaction-v1".to_owned(),
        run_count,
        runs,
        summary: CompactionSummary {
            passed,
            retention_rate: ratio(retained_facts, required_facts),
            secret_leaks,
            error_rate: ratio(run_count.saturating_sub(passed), run_count),
            duration_ms_p50: percentile(&durations, 50),
            duration_ms_p95: percentile(&durations, 95),
            output_chars_p50: percentile(&output_chars, 50),
            output_chars_p95: percentile(&output_chars, 95),
        },
    })
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
        event_count: 3,
        required_fact_count: REQUIRED_FACTS.len(),
        retained_required_facts,
        secret_leaks,
        input_chars: input.chars().count(),
        output_chars: compacted.chars().count(),
        duration_ms: elapsed_ms(started),
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

fn percentile(values: &[u64], percentile: usize) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
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
        JevxError::Timeout => "timeout",
        JevxError::Io(_) => "io_error",
        JevxError::Json(_) => "json_error",
        JevxError::Yaml(_) => "yaml_error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_metrics_and_error_codes_cover_empty_paths() {
        assert!(percentile(&[], 50).is_none());
        assert_eq!(percentile(&[30, 10, 20], 50), Some(20));
        let errors = [
            JevxError::InvalidInput("x".to_owned()),
            JevxError::MissingApiKey,
            JevxError::Provider("x".to_owned()),
            JevxError::Timeout,
            JevxError::Io(std::io::Error::other("x")),
            JevxError::Json(serde_json::from_str::<Value>("{").expect_err("json")),
            JevxError::Yaml(serde_yaml::from_str::<Value>("[").expect_err("yaml")),
        ];
        assert_eq!(errors.iter().map(error_code).count(), 7);
    }
}
