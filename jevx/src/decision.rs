use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::JevxError;
use crate::redaction::sha256_hex;
use crate::types::{Judge, JudgeCandidate, JudgeRequest, Usage};

pub const DECISION_CONTRACT_VERSION: &str = "decision-contract.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionMode {
    Live,
    DryRun,
    Replay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Advisory,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    Accepted,
    None,
    Unknown,
    Defer,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionFailure {
    DryRun,
    MissingApiKey,
    Timeout,
    Provider,
    StateDegraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    NoneSelected,
    BelowProbabilityThreshold,
    BelowMarginThreshold,
    BelowScoreThreshold,
    LowConfidence,
    AmbiguousPredicate,
    MissingAnswer,
    UnknownChoice,
    MalformedAnswer,
    DryRun,
    MissingApiKey,
    Timeout,
    ProviderError,
    StateDegraded,
    InvalidPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PredicateCriteria {
    #[serde(rename = "true")]
    pub true_criteria: String,
    #[serde(rename = "false")]
    pub false_criteria: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum QuestionSpec {
    #[serde(rename = "choice")]
    Choice {
        instructions: String,
        criteria: BTreeMap<String, String>,
    },
    #[serde(rename = "score")]
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
    #[serde(rename = "noul")]
    Predicate {
        instructions: String,
        criteria: PredicateCriteria,
    },
}

impl QuestionSpec {
    pub fn validate(&self) -> Result<(), JevxError> {
        match self {
            Self::Choice {
                instructions,
                criteria,
            } => {
                if instructions.trim().is_empty() || criteria.is_empty() || criteria.len() > 255 {
                    return Err(JevxError::InvalidInput(
                        "choice question is empty or exceeds the option limit".to_owned(),
                    ));
                }
            }
            Self::Score {
                instructions,
                criteria,
            } => {
                if instructions.trim().is_empty() || !(2..=10).contains(&criteria.len()) {
                    return Err(JevxError::InvalidInput(
                        "score question must contain 2 to 10 levels".to_owned(),
                    ));
                }
            }
            Self::Predicate {
                instructions,
                criteria,
            } => {
                if instructions.trim().is_empty()
                    || criteria.true_criteria.trim().is_empty()
                    || criteria.false_criteria.trim().is_empty()
                {
                    return Err(JevxError::InvalidInput(
                        "predicate question requires true and false criteria".to_owned(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn is_choice(&self) -> bool {
        matches!(self, Self::Choice { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TypedAnswer {
    #[serde(rename = "choice")]
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
    },
    #[serde(rename = "score")]
    Score {
        score: f64,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    #[serde(rename = "noul")]
    Predicate { noul: f64 },
}

impl TypedAnswer {
    pub fn digest(&self) -> String {
        serde_json::to_string(self)
            .map(|encoded| sha256_hex(&encoded))
            .unwrap_or_else(|_| sha256_hex("invalid-answer"))
    }

    fn validate(&self, question: &QuestionSpec) -> Result<(), AnswerValidation> {
        match (question, self) {
            (
                QuestionSpec::Choice { criteria, .. },
                Self::Choice {
                    choice,
                    probabilities,
                    confidence,
                },
            ) => {
                if !criteria.contains_key(choice) {
                    return Err(AnswerValidation::UnknownChoice);
                }
                validate_probability_map(probabilities, criteria.keys())?;
                validate_optional_probability(*confidence)?;
            }
            (
                QuestionSpec::Score { criteria, .. },
                Self::Score {
                    score,
                    probabilities,
                    confidence,
                },
            ) => {
                if !score.is_finite() || *score < 0.0 || *score > (criteria.len() - 1) as f64 {
                    return Err(AnswerValidation::Malformed);
                }
                let indexes = criteria
                    .iter()
                    .enumerate()
                    .map(|(index, _)| index.to_string())
                    .collect::<Vec<_>>();
                validate_probability_map(probabilities, indexes.iter())?;
                validate_probability(*confidence)?;
            }
            (QuestionSpec::Predicate { .. }, Self::Predicate { noul }) => {
                validate_probability(*noul)?;
            }
            _ => return Err(AnswerValidation::WrongType),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnswerValidation {
    UnknownChoice,
    Malformed,
    WrongType,
}

fn validate_probability(value: f64) -> Result<(), AnswerValidation> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(AnswerValidation::Malformed)
    }
}

fn validate_optional_probability(value: Option<f64>) -> Result<(), AnswerValidation> {
    if let Some(value) = value {
        validate_probability(value)?;
    }
    Ok(())
}

fn validate_probability_map<'a, I>(
    probabilities: &BTreeMap<String, f64>,
    allowed: I,
) -> Result<(), AnswerValidation>
where
    I: IntoIterator<Item = &'a String>,
{
    let allowed = allowed
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if probabilities.is_empty() || probabilities.keys().any(|key| !allowed.contains(key)) {
        return Err(AnswerValidation::Malformed);
    }
    probabilities
        .values()
        .try_for_each(|value| validate_probability(*value))
}

#[derive(Debug, Clone)]
pub struct StatePlan {
    pub payload: Value,
    pub state_digest: String,
    pub state_bytes: usize,
    pub candidate_count: usize,
    pub window_count: usize,
    pub omitted: Vec<String>,
    pub redaction_reasons: Vec<String>,
    pub max_state_bytes: Option<usize>,
    pub candidate_limit: Option<usize>,
}

impl StatePlan {
    pub fn from_value(
        payload: Value,
        candidate_count: usize,
        window_count: usize,
        omitted: Vec<String>,
        redaction_reasons: Vec<String>,
    ) -> Result<Self, JevxError> {
        let encoded = serde_json::to_vec(&payload)?;
        let state_digest = sha256_hex(&String::from_utf8_lossy(&encoded));
        Ok(Self {
            payload,
            state_digest,
            state_bytes: encoded.len(),
            candidate_count,
            window_count,
            omitted,
            redaction_reasons,
            max_state_bytes: None,
            candidate_limit: None,
        })
    }

    pub fn with_budgets(mut self, max_state_bytes: usize, candidate_limit: usize) -> Self {
        self.max_state_bytes = Some(max_state_bytes);
        self.candidate_limit = Some(candidate_limit);
        self
    }

    pub fn is_degraded(&self) -> bool {
        self.window_count == 0
            || !self.omitted.is_empty()
            || self
                .max_state_bytes
                .is_some_and(|limit| self.state_bytes > limit)
            || self
                .candidate_limit
                .is_some_and(|limit| self.candidate_count > limit)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DecisionPolicy {
    #[serde(rename = "choice")]
    Choice {
        min_probability: f64,
        min_margin: f64,
        severity: Severity,
    },
    #[serde(rename = "score")]
    Score {
        min_score: f64,
        min_confidence: f64,
        severity: Severity,
    },
    #[serde(rename = "predicate")]
    Predicate {
        true_threshold: f64,
        false_threshold: f64,
        severity: Severity,
    },
}

impl DecisionPolicy {
    fn severity(&self) -> Severity {
        match self {
            Self::Choice { severity, .. }
            | Self::Score { severity, .. }
            | Self::Predicate { severity, .. } => *severity,
        }
    }

    pub fn threshold(&self) -> f64 {
        match self {
            Self::Choice {
                min_probability, ..
            } => *min_probability,
            Self::Score { min_score, .. } => *min_score,
            Self::Predicate { true_threshold, .. } => *true_threshold,
        }
    }

    pub fn validate(&self, question: &QuestionSpec) -> Result<(), JevxError> {
        question.validate()?;
        let valid = match (self, question) {
            (
                Self::Choice {
                    min_probability,
                    min_margin,
                    ..
                },
                QuestionSpec::Choice { .. },
            ) => bounded_probability(*min_probability) && bounded_probability(*min_margin),
            (
                Self::Score {
                    min_score,
                    min_confidence,
                    ..
                },
                QuestionSpec::Score { criteria, .. },
            ) => {
                min_score.is_finite()
                    && (0.0..=(criteria.len() - 1) as f64).contains(min_score)
                    && bounded_probability(*min_confidence)
            }
            (
                Self::Predicate {
                    true_threshold,
                    false_threshold,
                    ..
                },
                QuestionSpec::Predicate { .. },
            ) => {
                bounded_probability(*false_threshold)
                    && bounded_probability(*true_threshold)
                    && false_threshold < true_threshold
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(JevxError::InvalidInput(
                "decision policy is invalid for its question".to_owned(),
            ))
        }
    }
}

fn bounded_probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DecisionEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probability: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_up_probability: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionResult {
    pub status: DecisionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<TypedAnswer>,
    pub evidence: DecisionEvidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<FallbackReason>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionContract {
    pub contract_version: String,
    pub question_id: String,
    pub question_version: String,
    pub question: QuestionSpec,
    pub policy_version: String,
    pub policy: DecisionPolicy,
}

impl DecisionContract {
    pub fn choice(
        question_id: &str,
        question_version: &str,
        criteria: BTreeMap<String, String>,
        min_probability: f64,
        min_margin: f64,
    ) -> Self {
        let policy = DecisionPolicy::Choice {
            min_probability,
            min_margin,
            severity: Severity::Advisory,
        };
        Self {
            contract_version: DECISION_CONTRACT_VERSION.to_owned(),
            question_id: question_id.to_owned(),
            question_version: question_version.to_owned(),
            question: QuestionSpec::Choice {
                instructions: "Select the best matching option.".to_owned(),
                criteria,
            },
            policy_version: policy_version(question_version, &policy),
            policy,
        }
    }

    pub fn score(
        question_id: &str,
        question_version: &str,
        criteria: Vec<String>,
        min_score: f64,
        min_confidence: f64,
    ) -> Self {
        let policy = DecisionPolicy::Score {
            min_score,
            min_confidence,
            severity: Severity::Advisory,
        };
        Self {
            contract_version: DECISION_CONTRACT_VERSION.to_owned(),
            question_id: question_id.to_owned(),
            question_version: question_version.to_owned(),
            question: QuestionSpec::Score {
                instructions: "Score the state using the ordered levels.".to_owned(),
                criteria,
            },
            policy_version: policy_version(question_version, &policy),
            policy,
        }
    }

    pub fn predicate(
        question_id: &str,
        question_version: &str,
        true_criteria: &str,
        false_criteria: &str,
        true_threshold: f64,
        false_threshold: f64,
    ) -> Self {
        let policy = DecisionPolicy::Predicate {
            true_threshold,
            false_threshold,
            severity: Severity::Advisory,
        };
        Self {
            contract_version: DECISION_CONTRACT_VERSION.to_owned(),
            question_id: question_id.to_owned(),
            question_version: question_version.to_owned(),
            question: QuestionSpec::Predicate {
                instructions: "Judge the atomic predicate.".to_owned(),
                criteria: PredicateCriteria {
                    true_criteria: true_criteria.to_owned(),
                    false_criteria: false_criteria.to_owned(),
                },
            },
            policy_version: policy_version(question_version, &policy),
            policy,
        }
    }

    pub fn cache_key(&self, state_digest: &str) -> String {
        sha256_hex(&format!(
            "{}:{}:{}:{}:{}",
            self.contract_version,
            self.question_id,
            self.question_version,
            self.policy_version,
            state_digest
        ))
    }

    pub fn validate(&self) -> Result<(), JevxError> {
        if self.question_id.trim().is_empty()
            || self.question_version.trim().is_empty()
            || self.policy_version.trim().is_empty()
        {
            return Err(JevxError::InvalidInput(
                "decision contract identifiers must not be empty".to_owned(),
            ));
        }
        self.policy.validate(&self.question)
    }

    pub fn evaluate(&self, answer: Option<TypedAnswer>) -> DecisionResult {
        if self.validate().is_err() {
            return self.result(
                DecisionStatus::Unknown,
                None,
                DecisionEvidence::default(),
                Some(FallbackReason::InvalidPolicy),
                "invalid_policy",
            );
        }
        let Some(answer) = answer else {
            return self.result(
                DecisionStatus::Unknown,
                None,
                DecisionEvidence::default(),
                Some(FallbackReason::MissingAnswer),
                "answer_missing",
            );
        };

        if let Err(validation) = answer.validate(&self.question) {
            let fallback = match validation {
                AnswerValidation::UnknownChoice => FallbackReason::UnknownChoice,
                AnswerValidation::Malformed | AnswerValidation::WrongType => {
                    FallbackReason::MalformedAnswer
                }
            };
            return self.result(
                DecisionStatus::Unknown,
                None,
                DecisionEvidence::default(),
                Some(fallback),
                "answer_invalid",
            );
        }

        match (&self.policy, &self.question, &answer) {
            (
                DecisionPolicy::Choice {
                    min_probability,
                    min_margin,
                    ..
                },
                QuestionSpec::Choice { .. },
                TypedAnswer::Choice {
                    choice,
                    probabilities,
                    confidence,
                },
            ) => {
                if choice == "none" {
                    return self.result(
                        DecisionStatus::None,
                        Some(answer.clone()),
                        DecisionEvidence {
                            probability: probabilities.get(choice).copied(),
                            confidence: *confidence,
                            ..DecisionEvidence::default()
                        },
                        Some(FallbackReason::NoneSelected),
                        "none_selected",
                    );
                }
                let probability = probabilities.get(choice).copied().unwrap_or_default();
                let runner_up = probabilities
                    .iter()
                    .filter(|(candidate, _)| candidate.as_str() != choice)
                    .map(|(_, probability)| *probability)
                    .max_by(f64::total_cmp)
                    .unwrap_or_default();
                let margin = probability - runner_up;
                let evidence = DecisionEvidence {
                    probability: Some(probability),
                    runner_up_probability: Some(runner_up),
                    margin: Some(margin),
                    confidence: *confidence,
                    ..DecisionEvidence::default()
                };
                if probability < *min_probability {
                    self.result(
                        DecisionStatus::Defer,
                        Some(answer.clone()),
                        evidence,
                        Some(FallbackReason::BelowProbabilityThreshold),
                        "probability_below_threshold",
                    )
                } else if margin < *min_margin {
                    self.result(
                        DecisionStatus::Defer,
                        Some(answer.clone()),
                        evidence,
                        Some(FallbackReason::BelowMarginThreshold),
                        "margin_below_threshold",
                    )
                } else {
                    self.result(
                        DecisionStatus::Accepted,
                        Some(answer.clone()),
                        evidence,
                        None,
                        "accepted",
                    )
                }
            }
            (
                DecisionPolicy::Score {
                    min_score,
                    min_confidence,
                    ..
                },
                QuestionSpec::Score { .. },
                TypedAnswer::Score {
                    score, confidence, ..
                },
            ) => {
                let evidence = DecisionEvidence {
                    confidence: Some(*confidence),
                    score: Some(*score),
                    ..DecisionEvidence::default()
                };
                if *confidence < *min_confidence {
                    self.result(
                        DecisionStatus::Defer,
                        Some(answer.clone()),
                        evidence,
                        Some(FallbackReason::LowConfidence),
                        "confidence_below_threshold",
                    )
                } else if *score < *min_score {
                    self.result(
                        DecisionStatus::None,
                        Some(answer.clone()),
                        evidence,
                        Some(FallbackReason::BelowScoreThreshold),
                        "score_below_threshold",
                    )
                } else {
                    self.result(
                        DecisionStatus::Accepted,
                        Some(answer.clone()),
                        evidence,
                        None,
                        "accepted",
                    )
                }
            }
            (
                DecisionPolicy::Predicate {
                    true_threshold,
                    false_threshold,
                    ..
                },
                QuestionSpec::Predicate { .. },
                TypedAnswer::Predicate { noul },
            ) => {
                let evidence = DecisionEvidence {
                    probability: Some(*noul),
                    ..DecisionEvidence::default()
                };
                if *noul >= *true_threshold || *noul <= *false_threshold {
                    self.result(
                        DecisionStatus::Accepted,
                        Some(answer.clone()),
                        evidence,
                        None,
                        "accepted",
                    )
                } else {
                    self.result(
                        DecisionStatus::Defer,
                        Some(answer.clone()),
                        evidence,
                        Some(FallbackReason::AmbiguousPredicate),
                        "predicate_ambiguous",
                    )
                }
            }
            _ => self.result(
                DecisionStatus::Unknown,
                None,
                DecisionEvidence::default(),
                Some(FallbackReason::MalformedAnswer),
                "policy_question_mismatch",
            ),
        }
    }

    pub fn failure(&self, failure: DecisionFailure) -> DecisionResult {
        let (fallback, reason) = match failure {
            DecisionFailure::DryRun => (FallbackReason::DryRun, "dry_run"),
            DecisionFailure::MissingApiKey => (FallbackReason::MissingApiKey, "missing_api_key"),
            DecisionFailure::Timeout => (FallbackReason::Timeout, "timeout"),
            DecisionFailure::Provider => (FallbackReason::ProviderError, "provider_error"),
            DecisionFailure::StateDegraded => (FallbackReason::StateDegraded, "state_degraded"),
        };
        let status = if matches!(failure, DecisionFailure::StateDegraded) {
            DecisionStatus::Degraded
        } else {
            DecisionStatus::Defer
        };
        self.result(
            status,
            None,
            DecisionEvidence::default(),
            Some(fallback),
            reason,
        )
    }

    fn result(
        &self,
        status: DecisionStatus,
        answer: Option<TypedAnswer>,
        evidence: DecisionEvidence,
        fallback: Option<FallbackReason>,
        reason: &str,
    ) -> DecisionResult {
        DecisionResult {
            status,
            answer,
            evidence,
            fallback,
            reason: reason.to_owned(),
        }
    }

    pub fn severity(&self) -> Severity {
        self.policy.severity()
    }
}

fn policy_version(question_version: &str, policy: &DecisionPolicy) -> String {
    let digest_input = serde_json::to_string(policy).unwrap_or_else(|_| format!("{policy:?}"));
    format!("{question_version}.policy.v1.{}", sha256_hex(&digest_input))
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionRequest {
    pub state: String,
    pub questions: BTreeMap<String, QuestionSpec>,
}

#[derive(Debug, Clone)]
pub struct DecisionResponse {
    pub answers: BTreeMap<String, TypedAnswer>,
    pub response_ms: u64,
    pub usage: Option<Usage>,
    pub calls: u32,
    pub retries: u32,
}

#[async_trait]
pub trait DecisionJudge: Send + Sync {
    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionResponse, JevxError>;
}

pub struct LegacyJudgeAdapter<'a, J: Judge + ?Sized> {
    judge: &'a J,
}

impl<'a, J: Judge + ?Sized> LegacyJudgeAdapter<'a, J> {
    pub fn new(judge: &'a J) -> Self {
        Self { judge }
    }
}

#[async_trait]
impl<J: Judge + ?Sized> DecisionJudge for LegacyJudgeAdapter<'_, J> {
    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionResponse, JevxError> {
        let Some((question_id, question)) = request.questions.iter().next() else {
            return Err(JevxError::InvalidInput(
                "decision request must contain a question".to_owned(),
            ));
        };
        let QuestionSpec::Choice { criteria, .. } = question else {
            return Err(JevxError::InvalidInput(
                "legacy Judge adapter only supports Choice".to_owned(),
            ));
        };
        let candidates = criteria
            .iter()
            .filter(|(id, _)| id.as_str() != "none")
            .map(|(id, description)| JudgeCandidate {
                id: id.clone(),
                name: id.clone(),
                description: description.clone(),
            })
            .collect::<Vec<_>>();
        let evaluation = self
            .judge
            .evaluate_with_metrics(JudgeRequest {
                state: request.state,
                candidates,
            })
            .await?;
        let response = evaluation.response;
        let answer = response.choice.map(|choice| TypedAnswer::Choice {
            choice,
            probabilities: response.probabilities,
            confidence: None,
        });
        Ok(DecisionResponse {
            answers: answer
                .map(|answer| BTreeMap::from([(question_id.clone(), answer)]))
                .unwrap_or_default(),
            response_ms: response.response_ms,
            usage: response.usage,
            calls: evaluation.calls.max(1),
            retries: evaluation.retries,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheLookup {
    Miss,
    Hit,
}

pub trait DecisionCache: Send + Sync {
    fn get(&self, key: &str) -> Option<TypedAnswer>;
    fn put(&self, key: &str, answer: &TypedAnswer);
}

#[derive(Debug, Default)]
pub struct InMemoryDecisionCache {
    capacity: usize,
    values: Mutex<HashMap<String, TypedAnswer>>,
}

impl InMemoryDecisionCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            values: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, key: &str) -> Option<TypedAnswer> {
        <Self as DecisionCache>::get(self, key)
    }

    pub fn put(&self, key: &str, answer: &TypedAnswer) {
        <Self as DecisionCache>::put(self, key, answer)
    }
}

impl DecisionCache for InMemoryDecisionCache {
    fn get(&self, key: &str) -> Option<TypedAnswer> {
        self.values
            .lock()
            .ok()
            .and_then(|values| values.get(key).cloned())
    }

    fn put(&self, key: &str, answer: &TypedAnswer) {
        if self.capacity == 0 {
            return;
        }
        if let Ok(mut values) = self.values.lock() {
            if values.len() >= self.capacity
                && !values.contains_key(key)
                && let Some(oldest) = values.keys().next().cloned()
            {
                values.remove(&oldest);
            }
            values.insert(key.to_owned(), answer.clone());
        }
    }
}

pub struct DecisionExecutionOptions {
    pub mode: DecisionMode,
    pub cache: Option<Arc<dyn DecisionCache>>,
}

impl Default for DecisionExecutionOptions {
    fn default() -> Self {
        Self {
            mode: DecisionMode::Live,
            cache: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecisionExecution {
    pub result: DecisionResult,
    pub mode: DecisionMode,
    pub response_ms: u64,
    pub usage: Option<Usage>,
    pub calls: u32,
    pub retries: u32,
    pub cache_hit: bool,
    pub error_code: Option<String>,
    pub(crate) error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
struct ExecutionMetrics {
    response_ms: u64,
    calls: u32,
    retries: u32,
}

pub async fn execute_contract(
    contract: &DecisionContract,
    state: &StatePlan,
    judge: Option<&dyn DecisionJudge>,
    options: &DecisionExecutionOptions,
) -> DecisionExecution {
    if contract.validate().is_err() {
        return DecisionExecution {
            result: contract.evaluate(None),
            mode: options.mode,
            response_ms: 0,
            usage: None,
            calls: 0,
            retries: 0,
            cache_hit: false,
            error_code: Some("invalid_input".to_owned()),
            error_message: Some("decision contract is invalid".to_owned()),
        };
    }
    if state.is_degraded() {
        return DecisionExecution::failed(
            contract,
            options.mode,
            DecisionFailure::StateDegraded,
            Some("state_degraded"),
            None,
        );
    }
    if matches!(options.mode, DecisionMode::DryRun) {
        return DecisionExecution::failed(
            contract,
            options.mode,
            DecisionFailure::DryRun,
            None,
            None,
        );
    }
    if matches!(options.mode, DecisionMode::Replay) {
        return DecisionExecution::failed(
            contract,
            options.mode,
            DecisionFailure::Provider,
            Some("replay_answer_missing"),
            None,
        );
    }

    let cache_key = contract.cache_key(&state.state_digest);
    if let Some(cache) = &options.cache
        && let Some(answer) = cache.get(&cache_key)
    {
        return DecisionExecution {
            result: contract.evaluate(Some(answer)),
            mode: options.mode,
            response_ms: 0,
            usage: None,
            calls: 0,
            retries: 0,
            cache_hit: true,
            error_code: None,
            error_message: None,
        };
    }

    let Some(judge) = judge else {
        return DecisionExecution::failed(
            contract,
            options.mode,
            DecisionFailure::MissingApiKey,
            Some("missing_api_key"),
            Some("AI_GATEWAY_API_KEY is not configured".to_owned()),
        );
    };
    let request = DecisionRequest {
        // serde_json::Value has no fallible user-defined serializer; StatePlan already owns the
        // bounded/redacted payload before this point.
        state: serde_json::to_string(&state.payload).expect("StatePlan payload must serialize"),
        questions: BTreeMap::from([(contract.question_id.clone(), contract.question.clone())]),
    };
    match judge.evaluate(request).await {
        Ok(response) => {
            let answer = response.answers.get(&contract.question_id).cloned();
            if let (Some(cache), Some(answer)) = (&options.cache, answer.as_ref()) {
                cache.put(&cache_key, answer);
            }
            DecisionExecution {
                result: contract.evaluate(answer),
                mode: options.mode,
                response_ms: response.response_ms,
                usage: response.usage,
                calls: response.calls.max(1),
                retries: response.retries,
                cache_hit: false,
                error_code: None,
                error_message: None,
            }
        }
        Err(error) => {
            let (failure, code, metrics) = match &error {
                JevxError::MissingApiKey => {
                    (DecisionFailure::MissingApiKey, "missing_api_key", None)
                }
                JevxError::Timeout => (DecisionFailure::Timeout, "timeout", None),
                JevxError::Provider(_) => (DecisionFailure::Provider, "provider_error", None),
                JevxError::ProviderWithMetrics {
                    calls,
                    retries,
                    response_ms,
                    ..
                } => (
                    DecisionFailure::Provider,
                    "provider_error",
                    Some(ExecutionMetrics {
                        response_ms: *response_ms,
                        calls: *calls,
                        retries: *retries,
                    }),
                ),
                JevxError::InvalidInput(_) | JevxError::Json(_) | JevxError::Yaml(_) => {
                    (DecisionFailure::StateDegraded, "invalid_input", None)
                }
                JevxError::Io(_) => (DecisionFailure::Provider, "io_error", None),
            };
            if let Some(metrics) = metrics {
                DecisionExecution::failed_with_metrics(
                    contract,
                    options.mode,
                    failure,
                    Some(code),
                    Some(error.to_string()),
                    metrics,
                )
            } else {
                DecisionExecution::failed(
                    contract,
                    options.mode,
                    failure,
                    Some(code),
                    Some(error.to_string()),
                )
            }
        }
    }
}

pub fn replay_contract(
    contract: &DecisionContract,
    state: &StatePlan,
    answer: Option<TypedAnswer>,
) -> DecisionExecution {
    if state.is_degraded() {
        return DecisionExecution::failed(
            contract,
            DecisionMode::Replay,
            DecisionFailure::StateDegraded,
            Some("state_degraded"),
            None,
        );
    }
    DecisionExecution {
        result: contract.evaluate(answer),
        mode: DecisionMode::Replay,
        response_ms: 0,
        usage: None,
        calls: 0,
        retries: 0,
        cache_hit: false,
        error_code: None,
        error_message: None,
    }
}

impl DecisionExecution {
    fn failed(
        contract: &DecisionContract,
        mode: DecisionMode,
        failure: DecisionFailure,
        error_code: Option<&str>,
        error_message: Option<String>,
    ) -> Self {
        Self::failed_with_metrics(
            contract,
            mode,
            failure,
            error_code,
            error_message,
            ExecutionMetrics::default(),
        )
    }

    fn failed_with_metrics(
        contract: &DecisionContract,
        mode: DecisionMode,
        failure: DecisionFailure,
        error_code: Option<&str>,
        error_message: Option<String>,
        metrics: ExecutionMetrics,
    ) -> Self {
        Self {
            result: contract.failure(failure),
            mode,
            response_ms: metrics.response_ms,
            usage: None,
            calls: metrics.calls,
            retries: metrics.retries,
            cache_hit: false,
            error_code: error_code.map(str::to_owned),
            error_message,
        }
    }
}
