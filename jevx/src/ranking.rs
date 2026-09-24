use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use crate::Config;
use crate::cost::{CostEstimate, CostSummary, total_cost};
use crate::decision::{
    DecisionCache, DecisionContract, DecisionExecution, DecisionExecutionOptions, DecisionJudge,
    DecisionStatus, FallbackReason, InMemoryDecisionCache, LegacyJudgeAdapter, QuestionSpec,
    StatePlan, TypedAnswer, execute_contract,
};
use crate::error::JevxError;
use crate::recorder::{DecisionReceipt, DecisionRecorder, JsonlDecisionRecorder};
use crate::redaction::redact;
use crate::types::{
    CandidateDecision, CandidateResult, Judge, JudgeCandidate, Metrics, SUGGESTION_SCHEMA_VERSION,
    SkillRecord, SuggestInput, SuggestionResult,
};

#[derive(Debug, Clone)]
pub struct LocalRanking {
    pub candidates: Vec<(i32, SkillRecord)>,
    pub discovery_ms: u64,
    pub omitted_candidates: usize,
}

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
            schema_version: SUGGESTION_SCHEMA_VERSION,
            decision: CandidateDecision::Explicit,
            selected: Some(CandidateResult::from_skill(skill, 1_000, Some(1.0))),
            candidates: vec![CandidateResult::from_skill(skill, 1_000, Some(1.0))],
            metrics: Metrics {
                discovery_ms,
                total_ms: discovery_ms,
                candidate_count: 1,
                cost: Some(CostSummary::no_external_call()),
                ..Metrics::default()
            },
            reason_code: Some("explicit_skill".to_owned()),
            mode: "shadow".to_owned(),
        });
    }

    let ranking = rank_candidates(prompt, &skills, config);
    let candidates = ranking.candidates;
    let discovery_ms = ranking.discovery_ms;

    if candidates.is_empty() {
        return Ok(SuggestionResult {
            schema_version: SUGGESTION_SCHEMA_VERSION,
            decision: CandidateDecision::NoCandidates,
            selected: None,
            candidates: Vec::new(),
            metrics: Metrics {
                discovery_ms,
                total_ms: discovery_ms,
                candidate_count: 0,
                cost: Some(CostSummary::no_external_call()),
                ..Metrics::default()
            },
            reason_code: Some("no_candidates".to_owned()),
            mode: "shadow".to_owned(),
        });
    }

    let mut candidate_redacted = false;
    let judge_candidates = candidates
        .iter()
        .map(|(_, skill)| {
            let description = redact(&skill.description);
            candidate_redacted |= description != skill.description;
            JudgeCandidate {
                id: skill.id.clone(),
                name: skill.name.clone(),
                description,
            }
        })
        .collect::<Vec<_>>();
    let state = build_state_plan(
        &input,
        prompt,
        &judge_candidates,
        candidate_redacted,
        ranking.omitted_candidates,
        config,
    );
    let contract = skill_contract(&judge_candidates, config);
    let adapter = judge.map(LegacyJudgeAdapter::new);
    let decision_judge = adapter
        .as_ref()
        .map(|adapter| adapter as &dyn DecisionJudge);
    let options = DecisionExecutionOptions {
        mode: crate::decision::DecisionMode::Live,
        cache: process_cache(config),
    };
    let execution = execute_contract(&contract, &state, decision_judge, &options).await;
    record_execution(config, &contract, &state, &execution)?;
    if let Some(error) = execution_error(&execution) {
        return Err(error);
    }

    let probabilities = match execution.result.answer.as_ref() {
        Some(TypedAnswer::Choice { probabilities, .. }) => probabilities.clone(),
        _ => BTreeMap::new(),
    };
    let choice = match execution.result.answer.as_ref() {
        Some(TypedAnswer::Choice { choice, .. }) => Some(choice.clone()),
        _ => None,
    };
    let mut result_candidates = candidates
        .iter()
        .map(|(score, skill)| {
            CandidateResult::from_skill(skill, *score, probabilities.get(&skill.id).copied())
        })
        .collect::<Vec<_>>();

    let mut decision = CandidateDecision::None;
    let mut selected = None;
    let mut reason_code = fallback_code(execution.result.fallback);
    if matches!(execution.result.status, DecisionStatus::Accepted)
        && let Some(choice) = choice.as_deref()
        && choice != "none"
    {
        if let Some((score, skill)) = candidates.iter().find(|(_, skill)| skill.id == choice) {
            decision = CandidateDecision::Selected;
            selected = Some(CandidateResult::from_skill(
                skill,
                *score,
                execution
                    .result
                    .evidence
                    .probability
                    .or_else(|| probabilities.get(choice).copied()),
            ));
            reason_code = None;
        } else {
            reason_code = Some("unknown_choice".to_owned());
        }
    }

    result_candidates.sort_by(|left, right| {
        right
            .probability
            .unwrap_or_default()
            .total_cmp(&left.probability.unwrap_or_default())
            .then_with(|| left.name.cmp(&right.name))
    });
    let response_ms = execution.response_ms;
    let total_ms = elapsed_ms(started).max(discovery_ms.saturating_add(response_ms));
    let usage = execution.usage;
    let jev_cost = usage
        .as_ref()
        .and_then(|usage| usage.cost.as_ref().filter(|cost| cost.is_valid()).cloned())
        .unwrap_or_else(|| {
            config.jev_pricing().estimate_two_part(
                usage.as_ref().map(|usage| usage.input_tokens),
                usage.as_ref().map(|usage| usage.output_tokens),
                execution.cache_hit,
            )
        });
    let codex_cost = CostEstimate::unavailable();
    let cost = CostSummary {
        total: total_cost(&jev_cost, &codex_cost),
        jev: jev_cost,
        codex: codex_cost,
    };
    let relative_cost = if execution.cache_hit {
        Some(0.0)
    } else {
        usage
            .as_ref()
            .filter(|_| {
                config.input_cost_weight.is_finite()
                    && config.output_cost_weight.is_finite()
                    && config.input_cost_weight >= 0.0
                    && config.output_cost_weight >= 0.0
            })
            .map(|usage| {
                usage.input_tokens as f64 * config.input_cost_weight
                    + usage.output_tokens as f64 * config.output_cost_weight
            })
    };
    Ok(SuggestionResult {
        schema_version: SUGGESTION_SCHEMA_VERSION,
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
            decision_calls: Some(execution.calls),
            decision_retries: Some(execution.retries),
            cache_hit: Some(execution.cache_hit),
            relative_cost,
            fallback: Some(execution.result.fallback.is_some()),
            cost: Some(cost),
        },
        reason_code,
        mode: "shadow".to_owned(),
    })
}

pub fn rank_candidates(prompt: &str, skills: &[SkillRecord], config: &Config) -> LocalRanking {
    let started = Instant::now();
    let mut candidates = skills
        .iter()
        .map(|skill| (local_score(prompt, skill), skill.clone()))
        .collect::<Vec<_>>();
    candidates.sort_by(|(left_score, left), (right_score, right)| {
        right_score.cmp(left_score).then_with(|| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        })
    });
    let total = candidates.len();
    candidates.truncate(config.max_candidates);
    LocalRanking {
        omitted_candidates: total.saturating_sub(candidates.len()),
        candidates,
        discovery_ms: elapsed_ms(started),
    }
}

fn skill_contract(candidates: &[JudgeCandidate], config: &Config) -> DecisionContract {
    let criteria = candidates
        .iter()
        .map(|candidate| (candidate.id.clone(), candidate.description.clone()))
        .chain([(
            "none".to_owned(),
            "候補Skillのどれも現在の依頼に適合しない".to_owned(),
        )])
        .collect::<BTreeMap<_, _>>();
    let mut contract = DecisionContract::choice(
        "skill",
        "skill-choice.v1",
        criteria,
        config.min_probability,
        config.min_margin,
    );
    if let QuestionSpec::Choice { instructions, .. } = &mut contract.question {
        *instructions =
            "現在の依頼に最も適したSkillを1つ選んでください。適合するSkillがなければnoneを選んでください。"
                .to_owned();
    }
    contract
}

fn build_state_plan(
    input: &SuggestInput,
    prompt: &str,
    candidates: &[JudgeCandidate],
    candidate_redacted: bool,
    omitted_candidates: usize,
    config: &Config,
) -> StatePlan {
    let redacted_prompt = redact(prompt);
    let cwd = input.cwd.display().to_string();
    let redacted_cwd = crate::redaction::redact_path(&input.cwd);
    let mut redaction_reasons = Vec::new();
    if redacted_prompt != prompt {
        redaction_reasons.push("prompt_secret_pattern".to_owned());
    }
    if redacted_cwd != cwd {
        redaction_reasons.push("cwd_secret_pattern".to_owned());
    }
    if candidate_redacted {
        redaction_reasons.push("candidate_secret_pattern".to_owned());
    }
    let safe_candidates = candidates
        .iter()
        .map(|candidate| {
            let description = redact(&candidate.description);
            serde_json::json!({
                "id": candidate.id,
                "name": candidate.name,
                "description": description,
            })
        })
        .collect::<Vec<_>>();
    let mut omitted = Vec::new();
    if omitted_candidates > 0 {
        omitted.push("candidate_window_overflow".to_owned());
    }
    let payload = serde_json::json!({
        "prompt": redacted_prompt,
        "cwd": redacted_cwd,
        "candidates": safe_candidates,
    });
    let mut plan = StatePlan::from_value(
        payload,
        candidates.len() + omitted_candidates,
        1,
        omitted,
        redaction_reasons,
    )
    .expect("redacted StatePlan payload must serialize")
    .with_budgets(config.max_state_bytes, config.max_candidates);
    if plan.state_bytes > config.max_state_bytes {
        plan.omitted.push("state_bytes_limit".to_owned());
        plan.payload = serde_json::json!({
            "prompt": truncate_text(&redacted_prompt, config.max_state_bytes.saturating_sub(128)),
            "candidates": [],
        });
        let encoded = serde_json::to_vec(&plan.payload).expect("bounded StatePlan must serialize");
        plan.state_bytes = encoded.len();
        plan.state_digest = crate::redaction::sha256_hex(&String::from_utf8_lossy(&encoded));
    }
    plan
}

fn process_cache(config: &Config) -> Option<Arc<dyn DecisionCache>> {
    if config.cache_capacity == 0 {
        return None;
    }
    static CACHES: OnceLock<Mutex<HashMap<usize, Arc<InMemoryDecisionCache>>>> = OnceLock::new();
    let caches = CACHES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut caches = caches.lock().ok()?;
    let cache = caches
        .entry(config.cache_capacity)
        .or_insert_with(|| Arc::new(InMemoryDecisionCache::new(config.cache_capacity)));
    let cache = Arc::clone(&*cache);
    let cache: Arc<dyn DecisionCache> = cache;
    Some(cache)
}

fn record_execution(
    config: &Config,
    contract: &DecisionContract,
    state: &StatePlan,
    execution: &DecisionExecution,
) -> Result<(), JevxError> {
    if !config.telemetry_enabled {
        return Ok(());
    }
    let recorder = JsonlDecisionRecorder::new(config.decision_receipt_path.clone());
    let receipt = DecisionReceipt::from_execution_with_pricing(
        contract,
        state,
        execution,
        config.input_cost_weight,
        config.output_cost_weight,
        &config.jev_pricing(),
    );
    recorder.record(&receipt)
}

fn execution_error(execution: &DecisionExecution) -> Option<JevxError> {
    let code = execution.error_code.as_deref()?;
    Some(match code {
        "missing_api_key" => JevxError::MissingApiKey,
        "timeout" => JevxError::Timeout,
        "state_degraded" | "invalid_input" | "replay_mismatch" => {
            JevxError::InvalidInput("decision state is degraded or invalid".to_owned())
        }
        "provider_error" | "io_error" => JevxError::Provider(
            execution
                .error_message
                .clone()
                .unwrap_or_else(|| "Jevの評価に失敗しました。".to_owned()),
        ),
        _ => JevxError::Provider("Jevの評価に失敗しました。".to_owned()),
    })
}

fn fallback_code(fallback: Option<FallbackReason>) -> Option<String> {
    fallback.map(|fallback| {
        match fallback {
            FallbackReason::NoneSelected => "none_selected",
            FallbackReason::BelowProbabilityThreshold => "below_probability_threshold",
            FallbackReason::BelowMarginThreshold => "below_margin_threshold",
            FallbackReason::BelowScoreThreshold => "below_score_threshold",
            FallbackReason::LowConfidence => "low_confidence",
            FallbackReason::AmbiguousPredicate => "ambiguous_predicate",
            FallbackReason::MissingAnswer => "missing_choice",
            FallbackReason::UnknownChoice => "unknown_choice",
            FallbackReason::MalformedAnswer => "malformed_answer",
            FallbackReason::DryRun => "dry_run",
            FallbackReason::MissingApiKey => "missing_api_key",
            FallbackReason::Timeout => "timeout",
            FallbackReason::ProviderError => "provider_error",
            FallbackReason::StateDegraded => "state_degraded",
            FallbackReason::InvalidPolicy => "invalid_policy",
        }
        .to_owned()
    })
}

fn truncate_text(value: &str, max_bytes: usize) -> String {
    const MARKER: &str = "<truncated>";
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes.saturating_sub(MARKER.len()).min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], MARKER)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::{DecisionEvidence, DecisionMode, DecisionResult};

    #[test]
    fn fallback_codes_and_truncation_are_deterministic() {
        let fallbacks = [
            FallbackReason::NoneSelected,
            FallbackReason::BelowProbabilityThreshold,
            FallbackReason::BelowMarginThreshold,
            FallbackReason::BelowScoreThreshold,
            FallbackReason::LowConfidence,
            FallbackReason::AmbiguousPredicate,
            FallbackReason::MissingAnswer,
            FallbackReason::UnknownChoice,
            FallbackReason::MalformedAnswer,
            FallbackReason::DryRun,
            FallbackReason::MissingApiKey,
            FallbackReason::Timeout,
            FallbackReason::ProviderError,
            FallbackReason::StateDegraded,
            FallbackReason::InvalidPolicy,
        ];
        for fallback in fallbacks {
            assert!(fallback_code(Some(fallback)).is_some());
        }
        assert!(fallback_code(None).is_none());
        assert_eq!(truncate_text("short", 32), "short");
        assert!(truncate_text("日本語の長い状態", 4).ends_with("<truncated>"));
    }

    #[test]
    fn execution_error_mapping_is_safe_for_unknown_and_missing_messages() {
        let execution = |code: Option<&str>, message: Option<&str>| DecisionExecution {
            result: DecisionResult {
                status: DecisionStatus::Defer,
                answer: None,
                evidence: DecisionEvidence::default(),
                fallback: Some(FallbackReason::ProviderError),
                reason: "provider_error".to_owned(),
            },
            mode: DecisionMode::Live,
            response_ms: 0,
            usage: None,
            calls: 0,
            retries: 0,
            cache_hit: false,
            error_code: code.map(str::to_owned),
            error_message: message.map(str::to_owned),
        };
        assert!(execution_error(&execution(None, None)).is_none());
        assert!(matches!(
            execution_error(&execution(Some("unknown"), None)),
            Some(JevxError::Provider(message)) if message.contains("評価")
        ));
        assert!(matches!(
            execution_error(&execution(Some("provider_error"), None)),
            Some(JevxError::Provider(message)) if message.contains("評価")
        ));
        assert!(matches!(
            execution_error(&execution(Some("provider_error"), Some("safe failure"))),
            Some(JevxError::Provider(message)) if message == "safe failure"
        ));
        assert!(matches!(
            execution_error(&execution(Some("missing_api_key"), None)),
            Some(JevxError::MissingApiKey)
        ));
        assert!(matches!(
            execution_error(&execution(Some("timeout"), None)),
            Some(JevxError::Timeout)
        ));
        assert!(matches!(
            execution_error(&execution(Some("state_degraded"), None)),
            Some(JevxError::InvalidInput(_))
        ));
    }
}
