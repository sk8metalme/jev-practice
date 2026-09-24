use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::cost::{CostEstimate, CostSummary};
use crate::error::JevxError;

pub const SUGGESTION_SCHEMA_VERSION: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub source: String,
}

impl SkillRecord {
    pub fn new(id: String, description: String, path: PathBuf, source: String) -> Self {
        Self {
            name: id.clone(),
            id,
            description,
            path,
            source,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestInput {
    pub prompt: String,
    pub cwd: PathBuf,
    pub explicit_skill: Option<String>,
}

impl SuggestInput {
    pub fn new(prompt: String, cwd: PathBuf) -> Self {
        Self {
            prompt,
            cwd,
            explicit_skill: None,
        }
    }

    pub fn from_json(value: &str) -> Result<Self, JevxError> {
        #[derive(Deserialize)]
        struct Input {
            prompt: String,
            cwd: Option<PathBuf>,
            explicit_skill: Option<String>,
        }

        let input: Input = serde_json::from_str(value)?;
        let prompt = input.prompt.trim().to_owned();
        if prompt.is_empty() {
            return Err(JevxError::InvalidInput(
                "prompt must not be empty".to_owned(),
            ));
        }
        Ok(Self {
            prompt,
            cwd: input.cwd.unwrap_or_else(|| PathBuf::from(".")),
            explicit_skill: input.explicit_skill,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDecision {
    Selected,
    Explicit,
    None,
    NoCandidates,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateResult {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub source: String,
    pub local_score: i32,
    pub probability: Option<f64>,
}

impl CandidateResult {
    pub fn from_skill(skill: &SkillRecord, local_score: i32, probability: Option<f64>) -> Self {
        Self {
            id: skill.id.clone(),
            name: skill.name.clone(),
            description: skill.description.clone(),
            path: skill.path.clone(),
            source: skill.source.clone(),
            local_score,
            probability,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(rename = "inputTokens", alias = "input_tokens")]
    pub input_tokens: u64,
    #[serde(rename = "outputTokens", alias = "output_tokens")]
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostEstimate>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    #[serde(rename = "discoveryMs")]
    pub discovery_ms: u64,
    #[serde(rename = "jevResponseMs")]
    pub jev_response_ms: u64,
    #[serde(rename = "totalMs")]
    pub total_ms: u64,
    #[serde(rename = "candidateCount")]
    pub candidate_count: usize,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestionResult {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub decision: CandidateDecision,
    pub selected: Option<CandidateResult>,
    pub candidates: Vec<CandidateResult>,
    pub metrics: Metrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    pub mode: String,
}

impl SuggestionResult {
    pub fn none(reason: &str) -> Self {
        Self {
            schema_version: SUGGESTION_SCHEMA_VERSION,
            decision: CandidateDecision::None,
            selected: None,
            candidates: Vec::new(),
            metrics: Metrics::default(),
            reason_code: Some(reason.to_owned()),
            mode: "shadow".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct JudgeCandidate {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JudgeRequest {
    pub state: String,
    pub candidates: Vec<JudgeCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeResponse {
    pub choice: Option<String>,
    pub probabilities: std::collections::BTreeMap<String, f64>,
    pub response_ms: u64,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JudgeEvaluation {
    pub response: JudgeResponse,
    pub calls: u32,
    pub retries: u32,
}

impl JudgeResponse {
    pub fn selected(
        choice: &str,
        probability: f64,
        response_ms: u64,
        usage: Option<(u64, u64)>,
    ) -> Self {
        let mut probabilities = std::collections::BTreeMap::new();
        probabilities.insert(choice.to_owned(), probability);
        Self {
            choice: Some(choice.to_owned()),
            probabilities,
            response_ms,
            usage: usage.map(|(input_tokens, output_tokens)| Usage {
                input_tokens,
                output_tokens,
                cost: None,
            }),
        }
    }
}

#[async_trait]
pub trait Judge: Send + Sync {
    async fn evaluate(&self, request: JudgeRequest) -> Result<JudgeResponse, JevxError>;

    async fn evaluate_with_metrics(
        &self,
        request: JudgeRequest,
    ) -> Result<JudgeEvaluation, JevxError> {
        let response = self.evaluate(request).await?;
        Ok(JudgeEvaluation {
            response,
            calls: 1,
            retries: 0,
        })
    }
}
