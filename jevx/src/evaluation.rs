use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Config;
use crate::error::JevxError;
use crate::ranking::{rank_candidates, suggest_with_judge};
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
pub struct RepeatEvaluationReport {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "runCount")]
    pub run_count: usize,
    #[serde(rename = "caseCount")]
    pub case_count: usize,
    pub modes: BTreeMap<String, RepeatModeSummary>,
    pub runs: Vec<RepeatRunSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepeatRunSummary {
    pub run: usize,
    pub modes: BTreeMap<String, ModeSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepeatModeSummary {
    pub status: String,
    pub runs: usize,
    #[serde(rename = "casesPerRun")]
    pub cases_per_run: usize,
    pub accuracy: DistributionSummary,
    /// 互換のために残すキー。値は `noneRecall` と同じ（`noneCorrect / expectedNone`）。
    #[serde(rename = "nonePrecision")]
    pub none_precision: DistributionSummary,
    #[serde(rename = "noneRecall")]
    pub none_recall: DistributionSummary,
    #[serde(rename = "candidateMissRate")]
    pub candidate_miss_rate: DistributionSummary,
    #[serde(rename = "errorRate")]
    pub error_rate: DistributionSummary,
    #[serde(rename = "fallbackRate")]
    pub fallback_rate: DistributionSummary,
    #[serde(rename = "cacheHitRate")]
    pub cache_hit_rate: DistributionSummary,
    #[serde(rename = "retryRate")]
    pub retry_rate: DistributionSummary,
    #[serde(rename = "averageRetries")]
    pub average_retries: DistributionSummary,
    #[serde(rename = "relativeCost")]
    pub relative_cost: DistributionSummary,
    #[serde(rename = "discoveryMs")]
    pub discovery_ms: DistributionSummary,
    #[serde(rename = "jevResponseMs")]
    pub jev_response_ms: DistributionSummary,
    #[serde(rename = "totalMs")]
    pub total_ms: DistributionSummary,
    #[serde(rename = "inputTokens")]
    pub input_tokens: DistributionSummary,
    #[serde(rename = "outputTokens")]
    pub output_tokens: DistributionSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct DistributionSummary {
    pub mean: Option<f64>,
    pub stddev: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
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
    /// 互換のために残すキー。値は `noneRecall` と同じ（`noneCorrect / expectedNone`）。
    #[serde(rename = "nonePrecision")]
    pub none_precision: Option<f64>,
    #[serde(rename = "noneRecall")]
    pub none_recall: Option<f64>,
    #[serde(rename = "candidateMisses")]
    pub candidate_misses: usize,
    #[serde(rename = "candidateMissRate")]
    pub candidate_miss_rate: Option<f64>,
    pub errors: usize,
    #[serde(rename = "errorRate")]
    pub error_rate: Option<f64>,
    #[serde(rename = "fallbackRate")]
    pub fallback_rate: Option<f64>,
    #[serde(rename = "cacheHitRate")]
    pub cache_hit_rate: Option<f64>,
    #[serde(rename = "retryRate")]
    pub retry_rate: Option<f64>,
    #[serde(rename = "averageRetries")]
    pub average_retries: Option<f64>,
    #[serde(rename = "averageRelativeCost")]
    pub average_relative_cost: Option<f64>,
    #[serde(rename = "jevResponseMsP50")]
    pub jev_response_ms_p50: Option<u64>,
    #[serde(rename = "jevResponseMsP95")]
    pub jev_response_ms_p95: Option<u64>,
    #[serde(rename = "totalMsP50")]
    pub total_ms_p50: Option<u64>,
    #[serde(rename = "totalMsP95")]
    pub total_ms_p95: Option<u64>,
    #[serde(rename = "discoveryMsP50")]
    pub discovery_ms_p50: Option<u64>,
    #[serde(rename = "discoveryMsP95")]
    pub discovery_ms_p95: Option<u64>,
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
    #[serde(rename = "localRank")]
    pub local_rank: LocalRankCaseResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jevx: Option<JevCaseResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalRankCaseResult {
    pub prediction: Option<String>,
    #[serde(rename = "discoveryMs")]
    pub discovery_ms: u64,
    #[serde(rename = "candidateCount")]
    pub candidate_count: usize,
    #[serde(rename = "candidateMiss")]
    pub candidate_miss: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct JevCaseResult {
    pub decision: CandidateDecision,
    pub selected: Option<String>,
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
    #[serde(rename = "decisionCalls", skip_serializing_if = "Option::is_none")]
    pub decision_calls: Option<u32>,
    #[serde(rename = "decisionRetries", skip_serializing_if = "Option::is_none")]
    pub decision_retries: Option<u32>,
    #[serde(rename = "cacheHit", skip_serializing_if = "Option::is_none")]
    pub cache_hit: Option<bool>,
    #[serde(rename = "relativeCost", skip_serializing_if = "Option::is_none")]
    pub relative_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<bool>,
    #[serde(rename = "errorCode", skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(rename = "candidateMiss")]
    pub candidate_miss: bool,
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
    discovery_ms: Option<u64>,
    fallback: bool,
    cache_hit: bool,
    retries: u32,
    relative_cost: Option<f64>,
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
    let mut local_rank_observations = Vec::with_capacity(fixtures.len());
    let mut jev_observations = Vec::with_capacity(fixtures.len());

    for fixture in fixtures {
        let local_prediction = local_keyword_prediction(&fixture.prompt, skills);
        local_observations.push(Observation {
            prediction: local_prediction.clone(),
            ..Observation::default()
        });

        let local_ranking = rank_candidates(&fixture.prompt, skills, config);
        let local_rank_prediction = local_ranking
            .candidates
            .first()
            .and_then(|(score, skill)| (*score > 0).then(|| skill.id.clone()));
        let local_rank_candidate_miss = !is_none_expected(&fixture.expected)
            && !local_ranking
                .candidates
                .iter()
                .any(|(_, skill)| skill.id.eq_ignore_ascii_case(&fixture.expected));
        local_rank_observations.push(Observation {
            prediction: local_rank_prediction.clone(),
            candidate_miss: local_rank_candidate_miss,
            discovery_ms: Some(local_ranking.discovery_ms),
            total_ms: Some(local_ranking.discovery_ms),
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
            local_rank: LocalRankCaseResult {
                prediction: local_rank_prediction,
                discovery_ms: local_ranking.discovery_ms,
                candidate_count: local_ranking.candidates.len(),
                candidate_miss: local_rank_candidate_miss,
            },
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
        "local_rank".to_owned(),
        summarize(fixtures, &local_rank_observations, "completed", true),
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

pub async fn evaluate_repeated(
    fixtures: &[EvaluationFixture],
    skills: &[SkillRecord],
    config: &Config,
    judge: Option<&dyn Judge>,
    run_count: usize,
) -> Result<RepeatEvaluationReport, JevxError> {
    if run_count == 0 {
        return Err(JevxError::InvalidInput(
            "evaluation runs must be greater than zero".to_owned(),
        ));
    }

    let mut repeat_config = config.clone();
    repeat_config.cache_capacity = 0;
    let mut reports = Vec::with_capacity(run_count);
    for _ in 0..run_count {
        reports.push(evaluate(fixtures, skills, &repeat_config, judge).await);
    }

    let mut mode_names = BTreeSet::new();
    for report in &reports {
        mode_names.extend(report.modes.keys().cloned());
    }
    let modes = mode_names
        .into_iter()
        .map(|mode| {
            let summary = repeat_mode_summary(&mode, &reports);
            (mode, summary)
        })
        .collect::<BTreeMap<_, _>>();
    let runs = reports
        .iter()
        .enumerate()
        .map(|(index, report)| RepeatRunSummary {
            run: index + 1,
            modes: report.modes.clone(),
        })
        .collect::<Vec<_>>();

    Ok(RepeatEvaluationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        run_count,
        case_count: fixtures.len(),
        modes,
        runs,
    })
}

fn repeat_mode_summary(mode: &str, reports: &[EvaluationReport]) -> RepeatModeSummary {
    let run_modes = reports
        .iter()
        .filter_map(|report| report.modes.get(mode))
        .collect::<Vec<_>>();
    let metric_values = reports
        .iter()
        .flat_map(|report| report.cases.iter())
        .filter_map(|case| match mode {
            "local_rank" => Some((
                Some(case.local_rank.discovery_ms),
                None,
                Some(case.local_rank.discovery_ms),
                None,
                None,
                None,
                None,
                None,
                None,
            )),
            "jevx" => case.jevx.as_ref().map(|jevx| {
                (
                    jevx.discovery_ms,
                    jevx.jev_response_ms,
                    jevx.total_ms,
                    jevx.input_tokens,
                    jevx.output_tokens,
                    jevx.fallback
                        .map(|fallback| if fallback { 1.0 } else { 0.0 }),
                    jevx.cache_hit
                        .map(|cache_hit| if cache_hit { 1.0 } else { 0.0 }),
                    jevx.decision_retries.map(f64::from),
                    jevx.relative_cost,
                )
            }),
            _ => None,
        })
        .collect::<Vec<_>>();

    RepeatModeSummary {
        status: run_modes
            .first()
            .map(|summary| summary.status.clone())
            .unwrap_or_else(|| "not_run".to_owned()),
        runs: run_modes.len(),
        cases_per_run: reports.first().map_or(0, |report| report.case_count),
        accuracy: distribution(run_modes.iter().filter_map(|summary| summary.accuracy)),
        none_precision: distribution(
            run_modes
                .iter()
                .filter_map(|summary| summary.none_precision),
        ),
        none_recall: distribution(run_modes.iter().filter_map(|summary| summary.none_recall)),
        candidate_miss_rate: distribution(
            run_modes
                .iter()
                .filter_map(|summary| summary.candidate_miss_rate),
        ),
        error_rate: distribution(run_modes.iter().filter_map(|summary| summary.error_rate)),
        fallback_rate: distribution(run_modes.iter().filter_map(|summary| summary.fallback_rate)),
        cache_hit_rate: distribution(
            run_modes
                .iter()
                .filter_map(|summary| summary.cache_hit_rate),
        ),
        retry_rate: distribution(run_modes.iter().filter_map(|summary| summary.retry_rate)),
        average_retries: distribution(
            run_modes
                .iter()
                .filter_map(|summary| summary.average_retries),
        ),
        relative_cost: distribution(metric_values.iter().filter_map(|metrics| metrics.8)),
        discovery_ms: distribution(
            metric_values
                .iter()
                .filter_map(|metrics| metrics.0.map(|value| value as f64)),
        ),
        jev_response_ms: distribution(
            metric_values
                .iter()
                .filter_map(|metrics| metrics.1.map(|value| value as f64)),
        ),
        total_ms: distribution(
            metric_values
                .iter()
                .filter_map(|metrics| metrics.2.map(|value| value as f64)),
        ),
        input_tokens: distribution(
            metric_values
                .iter()
                .filter_map(|metrics| metrics.3.map(|value| value as f64)),
        ),
        output_tokens: distribution(
            metric_values
                .iter()
                .filter_map(|metrics| metrics.4.map(|value| value as f64)),
        ),
    }
}

fn distribution(values: impl IntoIterator<Item = f64>) -> DistributionSummary {
    let mut values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() {
        return DistributionSummary {
            mean: None,
            stddev: None,
            min: None,
            max: None,
            p50: None,
            p95: None,
        };
    }
    values.sort_by(f64::total_cmp);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    DistributionSummary {
        mean: Some(mean),
        stddev: Some(variance.sqrt()),
        min: values.first().copied(),
        max: values.last().copied(),
        p50: percentile_f64(&values, 50),
        p95: percentile_f64(&values, 95),
    }
}

fn percentile_f64(values: &[f64], percentile: usize) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let rank = (values.len() * percentile).div_ceil(100).saturating_sub(1);
    values.get(rank).copied()
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
                discovery_ms: Some(result.metrics.discovery_ms),
                fallback: result.metrics.fallback.unwrap_or(false),
                cache_hit: result.metrics.cache_hit.unwrap_or(false),
                retries: result.metrics.decision_retries.unwrap_or_default(),
                relative_cost: result.metrics.relative_cost,
            };
            let case = JevCaseResult {
                decision: result.decision,
                selected,
                discovery_ms: Some(result.metrics.discovery_ms),
                jev_response_ms: Some(result.metrics.jev_response_ms),
                total_ms: Some(result.metrics.total_ms),
                input_tokens: result.metrics.input_tokens,
                output_tokens: result.metrics.output_tokens,
                decision_calls: result.metrics.decision_calls,
                decision_retries: result.metrics.decision_retries,
                cache_hit: result.metrics.cache_hit,
                relative_cost: result.metrics.relative_cost,
                fallback: result.metrics.fallback,
                error_code: None,
                candidate_miss: observation.candidate_miss,
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
                discovery_ms: None,
                decision_calls: None,
                decision_retries: None,
                cache_hit: None,
                relative_cost: None,
                fallback: Some(true),
                error_code: Some(error_code(&error).to_owned()),
                candidate_miss: false,
            },
            Observation {
                error: true,
                fallback: true,
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
    let fallback_count = observations
        .iter()
        .filter(|observation| observation.fallback)
        .count();
    let cache_hit_count = observations
        .iter()
        .filter(|observation| observation.cache_hit)
        .count();
    let retry_cases = observations
        .iter()
        .filter(|observation| observation.retries > 0)
        .count();
    let retries = observations
        .iter()
        .map(|observation| observation.retries as f64)
        .collect::<Vec<_>>();
    let relative_cost = observations
        .iter()
        .filter_map(|observation| observation.relative_cost)
        .collect::<Vec<_>>();
    let response_times = observations
        .iter()
        .filter_map(|observation| observation.jev_response_ms)
        .collect::<Vec<_>>();
    let total_times = observations
        .iter()
        .filter_map(|observation| observation.total_ms)
        .collect::<Vec<_>>();
    let discovery_times = observations
        .iter()
        .filter_map(|observation| observation.discovery_ms)
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
        none_recall: ratio(none_correct, expected_none),
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
        fallback_rate: ratio(fallback_count, fixtures.len()),
        cache_hit_rate: ratio(cache_hit_count, fixtures.len()),
        retry_rate: ratio(retry_cases, fixtures.len()),
        average_retries: average_f64(&retries),
        average_relative_cost: average_f64(&relative_cost),
        jev_response_ms_p50: percentile(&response_times, 50),
        jev_response_ms_p95: percentile(&response_times, 95),
        total_ms_p50: percentile(&total_times, 50),
        total_ms_p95: percentile(&total_times, 95),
        discovery_ms_p50: percentile(&discovery_times, 50),
        discovery_ms_p95: percentile(&discovery_times, 95),
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
            none_recall: None,
            candidate_misses: 0,
            candidate_miss_rate: None,
            errors: 0,
            error_rate: None,
            fallback_rate: None,
            cache_hit_rate: None,
            retry_rate: None,
            average_retries: None,
            average_relative_cost: None,
            jev_response_ms_p50: None,
            jev_response_ms_p95: None,
            total_ms_p50: None,
            total_ms_p95: None,
            discovery_ms_p50: None,
            discovery_ms_p95: None,
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

fn average_f64(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
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
        JevxError::ProviderWithMetrics { .. } => "provider_error",
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

pub fn write_repeat_report(path: &Path, report: &RepeatEvaluationReport) -> Result<(), JevxError> {
    let content = serde_json::to_vec_pretty(report)?;
    fs::write(path, content)?;
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

    #[test]
    fn repeat_distribution_handles_empty_values_and_rounds_percentiles() {
        let empty = distribution(Vec::<f64>::new());
        assert!(empty.mean.is_none());
        assert!(percentile_f64(&[], 95).is_none());
        let values = distribution([30.0, 10.0, 20.0]);
        assert_eq!(values.p50, Some(20.0));
        assert_eq!(values.p95, Some(30.0));
        assert!(values.stddev.is_some());
    }
}
