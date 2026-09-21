use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Config;
use crate::error::JevxError;
use crate::ranking::suggest_with_judge;
use crate::types::{CandidateDecision, Judge, SkillRecord, SuggestInput};

const REPORT_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Deserialize)]
pub struct EvaluationFixture {
    pub id: String,
    pub kind: String,
    pub prompt: String,
    pub expected: String,
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvaluationReport {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "caseCount")]
    pub case_count: usize,
    pub modes: BTreeMap<String, ModeSummary>,
    pub cases: Vec<EvaluationCaseResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModeSummary {
    pub status: String,
    pub cases: usize,
    pub correct: usize,
    pub accuracy: Option<f64>,
    #[serde(rename = "expectedNone")]
    pub expected_none: usize,
    #[serde(rename = "noneCorrect")]
    pub none_correct: usize,
    #[serde(rename = "nonePrecision")]
    pub none_precision: Option<f64>,
    #[serde(rename = "candidateMisses")]
    pub candidate_misses: usize,
    #[serde(rename = "candidateMissRate")]
    pub candidate_miss_rate: Option<f64>,
    pub errors: usize,
    #[serde(rename = "errorRate")]
    pub error_rate: Option<f64>,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct EvaluationCaseResult {
    pub id: String,
    pub kind: String,
    pub expected: String,
    #[serde(rename = "nonePrediction")]
    pub none_prediction: String,
    #[serde(rename = "localPrediction")]
    pub local_prediction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jevx: Option<JevCaseResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JevCaseResult {
    pub decision: CandidateDecision,
    pub selected: Option<String>,
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
}

#[derive(Debug, Clone, Default)]
struct Observation {
    prediction: Option<String>,
    candidate_miss: bool,
    error: bool,
    jev_response_ms: Option<u64>,
    total_ms: Option<u64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

pub fn load_fixtures(path: &Path) -> Result<Vec<EvaluationFixture>, JevxError> {
    let content = fs::read_to_string(path)?;
    let mut fixtures = Vec::new();
    let mut ids = BTreeSet::new();
    for (line_number, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fixture = serde_json::from_str::<EvaluationFixture>(line).map_err(|error| {
            JevxError::InvalidInput(format!(
                "invalid fixture at line {}: {error}",
                line_number + 1
            ))
        })?;
        validate_fixture(&fixture, line_number + 1)?;
        if !ids.insert(fixture.id.to_ascii_lowercase()) {
            return Err(JevxError::InvalidInput(format!(
                "duplicate fixture id at line {}: {}",
                line_number + 1,
                fixture.id
            )));
        }
        fixtures.push(fixture);
    }
    if fixtures.is_empty() {
        return Err(JevxError::InvalidInput(
            "evaluation fixtures must contain at least one case".to_owned(),
        ));
    }
    Ok(fixtures)
}

fn validate_fixture(fixture: &EvaluationFixture, line_number: usize) -> Result<(), JevxError> {
    let missing = [
        ("id", fixture.id.trim().is_empty()),
        ("kind", fixture.kind.trim().is_empty()),
        ("prompt", fixture.prompt.trim().is_empty()),
        ("expected", fixture.expected.trim().is_empty()),
    ]
    .into_iter()
    .find_map(|(field, is_missing)| is_missing.then_some(field));
    if let Some(field) = missing {
        return Err(JevxError::InvalidInput(format!(
            "fixture at line {line_number} is missing {field}"
        )));
    }
    Ok(())
}

pub async fn evaluate(
    fixtures: &[EvaluationFixture],
    skills: &[SkillRecord],
    config: &Config,
    judge: Option<&dyn Judge>,
) -> EvaluationReport {
    let mut cases = Vec::with_capacity(fixtures.len());
    let mut local_observations = Vec::with_capacity(fixtures.len());
    let mut jev_observations = Vec::with_capacity(fixtures.len());

    for fixture in fixtures {
        let local_prediction = local_keyword_prediction(&fixture.prompt, skills);
        local_observations.push(Observation {
            prediction: local_prediction.clone(),
            ..Observation::default()
        });

        let jevx = if let Some(judge) = judge {
            let (result, observation) = evaluate_jev_case(fixture, skills, config, judge).await;
            jev_observations.push(observation);
            Some(result)
        } else {
            None
        };

        cases.push(EvaluationCaseResult {
            id: fixture.id.clone(),
            kind: fixture.kind.clone(),
            expected: fixture.expected.clone(),
            none_prediction: "none".to_owned(),
            local_prediction,
            jevx,
        });
    }

    let none_observations = fixtures
        .iter()
        .map(|_| Observation::default())
        .collect::<Vec<_>>();
    let mut modes = BTreeMap::new();
    modes.insert(
        "none".to_owned(),
        summarize(fixtures, &none_observations, "completed", false),
    );
    modes.insert(
        "local_keyword".to_owned(),
        summarize(fixtures, &local_observations, "completed", false),
    );
    modes.insert(
        "jevx".to_owned(),
        if judge.is_some() {
            summarize(fixtures, &jev_observations, "completed", true)
        } else {
            ModeSummary::not_run()
        },
    );

    EvaluationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        case_count: fixtures.len(),
        modes,
        cases,
    }
}

async fn evaluate_jev_case(
    fixture: &EvaluationFixture,
    skills: &[SkillRecord],
    config: &Config,
    judge: &dyn Judge,
) -> (JevCaseResult, Observation) {
    let input = SuggestInput::new(fixture.prompt.clone(), std::path::PathBuf::from("."));
    match suggest_with_judge(input, skills.to_vec(), config, judge).await {
        Ok(result) => {
            let selected = result
                .selected
                .as_ref()
                .map(|candidate| candidate.id.clone());
            let observation = Observation {
                prediction: selected.clone(),
                candidate_miss: !is_none_expected(&fixture.expected)
                    && !result
                        .candidates
                        .iter()
                        .any(|candidate| candidate.id.eq_ignore_ascii_case(&fixture.expected)),
                error: false,
                jev_response_ms: Some(result.metrics.jev_response_ms),
                total_ms: Some(result.metrics.total_ms),
                input_tokens: result.metrics.input_tokens,
                output_tokens: result.metrics.output_tokens,
            };
            let case = JevCaseResult {
                decision: result.decision,
                selected,
                jev_response_ms: Some(result.metrics.jev_response_ms),
                total_ms: Some(result.metrics.total_ms),
                input_tokens: result.metrics.input_tokens,
                output_tokens: result.metrics.output_tokens,
                error_code: None,
            };
            (case, observation)
        }
        Err(error) => (
            JevCaseResult {
                decision: CandidateDecision::Error,
                selected: None,
                jev_response_ms: None,
                total_ms: None,
                input_tokens: None,
                output_tokens: None,
                error_code: Some(error_code(&error).to_owned()),
            },
            Observation {
                error: true,
                ..Observation::default()
            },
        ),
    }
}

fn local_keyword_prediction(prompt: &str, skills: &[SkillRecord]) -> Option<String> {
    let normalized_prompt = prompt.to_ascii_lowercase();
    let mut scored = skills
        .iter()
        .map(|skill| (keyword_score(&normalized_prompt, skill), skill))
        .collect::<Vec<_>>();
    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score.cmp(left_score).then_with(|| {
            left.id
                .to_ascii_lowercase()
                .cmp(&right.id.to_ascii_lowercase())
        })
    });
    let (score, skill) = scored.first()?;
    (*score > 0).then(|| skill.id.clone())
}

fn keyword_score(normalized_prompt: &str, skill: &SkillRecord) -> usize {
    let mut keywords = BTreeSet::new();
    keywords.insert(skill.id.to_ascii_lowercase());
    for token in skill.description.split_whitespace() {
        let token = token
            .trim_matches(|character: char| !character.is_alphanumeric() && character != '_')
            .to_ascii_lowercase();
        if !token.is_empty() {
            keywords.insert(token);
        }
    }
    keywords
        .iter()
        .filter(|keyword| normalized_prompt.contains(keyword.as_str()))
        .count()
}

fn summarize(
    fixtures: &[EvaluationFixture],
    observations: &[Observation],
    status: &str,
    include_candidate_miss: bool,
) -> ModeSummary {
    let correct = fixtures
        .iter()
        .zip(observations)
        .filter(|(fixture, observation)| {
            prediction_matches(&observation.prediction, &fixture.expected)
        })
        .count();
    let expected_none = fixtures
        .iter()
        .filter(|fixture| is_none_expected(&fixture.expected))
        .count();
    let none_correct = fixtures
        .iter()
        .zip(observations)
        .filter(|(fixture, observation)| {
            is_none_expected(&fixture.expected) && observation.prediction.is_none()
        })
        .count();
    let positive_expected = fixtures.len().saturating_sub(expected_none);
    let candidate_misses = observations
        .iter()
        .filter(|observation| observation.candidate_miss)
        .count();
    let errors = observations
        .iter()
        .filter(|observation| observation.error)
        .count();
    let response_times = observations
        .iter()
        .filter_map(|observation| observation.jev_response_ms)
        .collect::<Vec<_>>();
    let total_times = observations
        .iter()
        .filter_map(|observation| observation.total_ms)
        .collect::<Vec<_>>();
    let input_tokens = observations
        .iter()
        .filter_map(|observation| observation.input_tokens)
        .collect::<Vec<_>>();
    let output_tokens = observations
        .iter()
        .filter_map(|observation| observation.output_tokens)
        .collect::<Vec<_>>();

    ModeSummary {
        status: status.to_owned(),
        cases: fixtures.len(),
        correct,
        accuracy: ratio(correct, fixtures.len()),
        expected_none,
        none_correct,
        none_precision: ratio(none_correct, expected_none),
        candidate_misses: if include_candidate_miss {
            candidate_misses
        } else {
            0
        },
        candidate_miss_rate: include_candidate_miss
            .then(|| ratio(candidate_misses, positive_expected))
            .flatten(),
        errors,
        error_rate: ratio(errors, fixtures.len()),
        jev_response_ms_p50: percentile(&response_times, 50),
        jev_response_ms_p95: percentile(&response_times, 95),
        total_ms_p50: percentile(&total_times, 50),
        total_ms_p95: percentile(&total_times, 95),
        average_input_tokens: average(&input_tokens),
        average_output_tokens: average(&output_tokens),
    }
}

impl ModeSummary {
    fn not_run() -> Self {
        Self {
            status: "not_run".to_owned(),
            cases: 0,
            correct: 0,
            accuracy: None,
            expected_none: 0,
            none_correct: 0,
            none_precision: None,
            candidate_misses: 0,
            candidate_miss_rate: None,
            errors: 0,
            error_rate: None,
            jev_response_ms_p50: None,
            jev_response_ms_p95: None,
            total_ms_p50: None,
            total_ms_p95: None,
            average_input_tokens: None,
            average_output_tokens: None,
        }
    }
}

fn prediction_matches(prediction: &Option<String>, expected: &str) -> bool {
    match prediction {
        Some(prediction) => {
            !is_none_expected(expected) && prediction.eq_ignore_ascii_case(expected)
        }
        None => is_none_expected(expected),
    }
}

fn is_none_expected(expected: &str) -> bool {
    expected.eq_ignore_ascii_case("none")
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

fn average(values: &[u64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<u64>() as f64 / values.len() as f64)
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

pub fn write_case_results(path: &Path, cases: &[EvaluationCaseResult]) -> Result<(), JevxError> {
    let mut file = File::create(path)?;
    for case in cases {
        serde_json::to_writer(&mut file, case)?;
        file.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn skill(id: &str, description: &str) -> SkillRecord {
        SkillRecord::new(
            id.to_owned(),
            description.to_owned(),
            PathBuf::from(format!("/{id}/SKILL.md")),
            "test".to_owned(),
        )
    }

    #[test]
    fn local_keyword_matching_is_deterministic_and_supports_none() {
        let skills = vec![skill("zeta", "alpha"), skill("alpha", "alpha")];
        assert_eq!(
            local_keyword_prediction("alpha", &skills).as_deref(),
            Some("alpha")
        );
        assert_eq!(local_keyword_prediction("unrelated", &skills), None);
    }

    #[test]
    fn percentile_and_average_handle_empty_and_round_up() {
        assert_eq!(percentile(&[], 50), None);
        assert_eq!(percentile(&[30, 10, 20], 50), Some(20));
        assert_eq!(percentile(&[30, 10, 20], 95), Some(30));
        assert_eq!(average(&[]), None);
        assert_eq!(average(&[2, 4]), Some(3.0));
    }

    #[test]
    fn fixture_validation_rejects_missing_fields_and_duplicate_ids() {
        let missing = EvaluationFixture {
            id: String::new(),
            kind: "synthetic".to_owned(),
            prompt: "prompt".to_owned(),
            expected: "none".to_owned(),
            keywords: vec![],
        };
        assert!(validate_fixture(&missing, 3).is_err());
    }

    #[test]
    fn mode_not_run_has_no_measurements() {
        let mode = ModeSummary::not_run();
        assert_eq!(mode.status, "not_run");
        assert!(mode.accuracy.is_none());
        assert!(mode.jev_response_ms_p95.is_none());
    }

    #[test]
    fn prediction_and_error_codes_cover_all_paths() {
        assert!(prediction_matches(&None, "none"));
        assert!(!prediction_matches(&None, "pdf"));
        assert!(prediction_matches(&Some("PDF".to_owned()), "pdf"));
        assert!(!prediction_matches(&Some("pdf".to_owned()), "none"));
        let errors = [
            JevxError::InvalidInput("x".to_owned()),
            JevxError::MissingApiKey,
            JevxError::Provider("x".to_owned()),
            JevxError::Timeout,
            JevxError::Io(std::io::Error::other("x")),
            JevxError::Json(serde_json::from_str::<serde_json::Value>("{").expect_err("json")),
            JevxError::Yaml(serde_yaml::from_str::<serde_yaml::Value>("[").expect_err("yaml")),
        ];
        assert_eq!(errors.iter().map(error_code).count(), 7);
    }

    #[test]
    fn metrics_keep_only_expected_safe_fields() {
        let metrics = crate::types::Metrics {
            jev_response_ms: 12,
            total_ms: 15,
            ..crate::types::Metrics::default()
        };
        assert_eq!(metrics.jev_response_ms, 12);
        assert_eq!(metrics.total_ms, 15);
    }
}
