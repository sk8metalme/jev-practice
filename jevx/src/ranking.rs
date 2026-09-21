use std::time::Instant;

use crate::Config;
use crate::error::JevxError;
use crate::redaction::redact;
use crate::types::{
    CandidateDecision, CandidateResult, Judge, JudgeCandidate, JudgeRequest, Metrics, SkillRecord,
    SuggestInput, SuggestionResult,
};

pub async fn suggest_with_judge<J: Judge + ?Sized>(
    input: SuggestInput,
    skills: Vec<SkillRecord>,
    config: &Config,
    judge: &J,
) -> Result<SuggestionResult, JevxError> {
    suggest_with_optional_judge(input, skills, config, Some(judge)).await
}

pub async fn suggest_with_optional_judge<J: Judge + ?Sized>(
    input: SuggestInput,
    skills: Vec<SkillRecord>,
    config: &Config,
    judge: Option<&J>,
) -> Result<SuggestionResult, JevxError> {
    let started = Instant::now();
    let prompt = input.prompt.trim();
    if prompt.is_empty() {
        return Err(JevxError::InvalidInput(
            "prompt must not be empty".to_owned(),
        ));
    }

    if let Some(explicit) = input.explicit_skill.as_deref() {
        let Some(skill) = skills
            .iter()
            .find(|skill| skill.name.eq_ignore_ascii_case(explicit))
        else {
            return Err(JevxError::InvalidInput(format!(
                "skill not found: {explicit}"
            )));
        };
        let discovery_ms = elapsed_ms(started);
        return Ok(SuggestionResult {
            schema_version: 1,
            decision: CandidateDecision::Explicit,
            selected: Some(CandidateResult::from_skill(skill, 1_000, Some(1.0))),
            candidates: vec![CandidateResult::from_skill(skill, 1_000, Some(1.0))],
            metrics: Metrics {
                discovery_ms,
                total_ms: discovery_ms,
                candidate_count: 1,
                ..Metrics::default()
            },
            reason_code: Some("explicit_skill".to_owned()),
            mode: "shadow".to_owned(),
        });
    }

    let mut candidates = skills
        .iter()
        .map(|skill| (local_score(prompt, skill), skill))
        .collect::<Vec<_>>();
    candidates.sort_by(|(left_score, left), (right_score, right)| {
        right_score.cmp(left_score).then_with(|| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        })
    });
    candidates.truncate(config.max_candidates);
    let discovery_ms = elapsed_ms(started);

    if candidates.is_empty() {
        return Ok(SuggestionResult {
            schema_version: 1,
            decision: CandidateDecision::NoCandidates,
            selected: None,
            candidates: Vec::new(),
            metrics: Metrics {
                discovery_ms,
                total_ms: discovery_ms,
                candidate_count: 0,
                ..Metrics::default()
            },
            reason_code: Some("no_candidates".to_owned()),
            mode: "shadow".to_owned(),
        });
    }

    let judge_candidates = candidates
        .iter()
        .map(|(_, skill)| JudgeCandidate {
            id: skill.id.clone(),
            name: skill.name.clone(),
            description: redact(&skill.description),
        })
        .collect::<Vec<_>>();
    let state = serde_json::json!({
        "prompt": redact(prompt),
        "cwd": input.cwd,
        "candidates": judge_candidates,
    });
    let response = judge
        .ok_or(JevxError::MissingApiKey)?
        .evaluate(JudgeRequest {
            state: serde_json::to_string(&state)?,
            candidates: judge_candidates,
        })
        .await?;

    let choice = response.choice.clone();
    let probabilities = response.probabilities;
    let mut result_candidates = candidates
        .iter()
        .map(|(score, skill)| {
            CandidateResult::from_skill(skill, *score, probabilities.get(&skill.id).copied())
        })
        .collect::<Vec<_>>();
    let selected_probability = choice
        .as_deref()
        .and_then(|choice| probabilities.get(choice).copied());
    let runner_up = probabilities
        .iter()
        .filter(|(id, _)| choice.as_deref() != Some(id.as_str()))
        .map(|(_, probability)| *probability)
        .max_by(f64::total_cmp);

    let mut decision = CandidateDecision::None;
    let mut selected = None;
    let mut reason_code = Some("none_selected".to_owned());
    if let Some(choice) = choice.as_deref() {
        if choice != "none" {
            if let Some(skill) = candidates.iter().find(|(_, skill)| skill.id == choice) {
                let probability = selected_probability.unwrap_or_default();
                let margin = probability - runner_up.unwrap_or_default();
                if probability >= config.min_probability && margin >= config.min_margin {
                    decision = CandidateDecision::Selected;
                    selected = Some(CandidateResult::from_skill(
                        skill.1,
                        skill.0,
                        Some(probability),
                    ));
                    reason_code = None;
                } else if probability < config.min_probability {
                    reason_code = Some("below_probability_threshold".to_owned());
                } else {
                    reason_code = Some("below_margin_threshold".to_owned());
                }
            } else {
                reason_code = Some("unknown_choice".to_owned());
            }
        }
    } else {
        reason_code = Some("missing_choice".to_owned());
    }

    result_candidates.sort_by(|left, right| {
        right
            .probability
            .unwrap_or_default()
            .total_cmp(&left.probability.unwrap_or_default())
            .then_with(|| left.name.cmp(&right.name))
    });
    let response_ms = response
        .response_ms
        .max(elapsed_ms(started).saturating_sub(discovery_ms));
    let total_ms = elapsed_ms(started).max(discovery_ms.saturating_add(response_ms));
    let usage = response.usage;
    Ok(SuggestionResult {
        schema_version: 1,
        decision,
        selected,
        candidates: result_candidates,
        metrics: Metrics {
            discovery_ms,
            jev_response_ms: response_ms,
            total_ms,
            candidate_count: candidates.len(),
            input_tokens: usage.as_ref().map(|usage| usage.input_tokens),
            output_tokens: usage.as_ref().map(|usage| usage.output_tokens),
        },
        reason_code,
        mode: "shadow".to_owned(),
    })
}

pub fn local_score(query: &str, skill: &SkillRecord) -> i32 {
    let normalized_query = normalize(query);
    let normalized_name = normalize(&skill.name);
    let normalized_description = normalize(&skill.description);
    let mut score = 0;
    if normalized_query == normalized_name {
        score += 1_000;
    }
    if normalized_query.contains(&normalized_name) || normalized_name.contains(&normalized_query) {
        score += 100;
    }
    if normalized_description.contains(&normalized_query) {
        score += 60;
    }
    let query_tokens = tokens(query);
    let name_tokens = tokens(&skill.name);
    let description_tokens = tokens(&skill.description);
    score += query_tokens
        .iter()
        .filter(|token| name_tokens.contains(token))
        .count() as i32
        * 20;
    score += query_tokens
        .iter()
        .filter(|token| description_tokens.contains(token))
        .count() as i32
        * 5;
    score + bigram_overlap(query, &skill.name) * 3
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<String>()
}

fn tokens(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(normalize)
        .collect()
}

fn bigram_overlap(left: &str, right: &str) -> i32 {
    let left = normalize(left).chars().collect::<Vec<_>>();
    let right = normalize(right).chars().collect::<Vec<_>>();
    if left.len() < 2 || right.len() < 2 {
        return 0;
    }
    let right_bigrams = right.windows(2).collect::<Vec<_>>();
    left.windows(2)
        .filter(|bigram| right_bigrams.contains(bigram))
        .count() as i32
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}
