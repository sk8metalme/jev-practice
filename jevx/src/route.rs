//! Codexのモデル／reasoning候補を、意味判定と適用結果から観測する。
//!
//! このモジュールはCodexのモデルを直接切り替えない。Hookから切替を保証できない
//! 場合は`degraded`として返し、観測された適用証拠がある場合だけ`applied`にする。

use serde::{Deserialize, Serialize};

use crate::decision::{DecisionContract, DecisionResult, DecisionStatus};

pub const ROUTE_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskDifficulty {
    Low,
    Medium,
    High,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    Luna,
    Terra,
    Sol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteStatus {
    Applied,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningLevel {
    Max,
    High,
    Medium,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteEvidence {
    pub model: Option<ModelFamily>,
    pub reasoning: Option<ReasoningLevel>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteDecision {
    pub schema_version: u8,
    pub difficulty: TaskDifficulty,
    pub model: Option<ModelFamily>,
    pub reasoning: Option<ReasoningLevel>,
    pub fallback_chain: Vec<ModelFamily>,
    pub status: RouteStatus,
    pub applied_model: Option<ModelFamily>,
    pub applied_reasoning: Option<ReasoningLevel>,
    pub confidence: Option<f64>,
    pub score: Option<f64>,
    pub reason: String,
}

impl RouteDecision {
    pub fn failed(reason: impl Into<String>) -> Self {
        Self {
            schema_version: ROUTE_SCHEMA_VERSION,
            difficulty: TaskDifficulty::Unknown,
            model: None,
            reasoning: None,
            fallback_chain: Vec::new(),
            status: RouteStatus::Failed,
            applied_model: None,
            applied_reasoning: None,
            confidence: None,
            score: None,
            reason: reason.into(),
        }
    }

    pub fn from_score(result: &DecisionResult) -> Self {
        let Some(score) = result.evidence.score else {
            return Self::failed(result.reason.clone());
        };
        let confidence = result.evidence.confidence;
        let difficulty = score_to_difficulty(score);
        let (model, reasoning, fallback_chain) = route_for(difficulty);
        let status = if matches!(result.status, DecisionStatus::Accepted) {
            RouteStatus::Degraded
        } else {
            RouteStatus::Failed
        };
        Self {
            schema_version: ROUTE_SCHEMA_VERSION,
            difficulty,
            model: Some(model),
            reasoning: Some(reasoning),
            fallback_chain,
            status,
            applied_model: None,
            applied_reasoning: None,
            confidence,
            score: Some(score),
            reason: if status == RouteStatus::Degraded {
                "hook_cannot_apply_model_route".to_owned()
            } else {
                result.reason.clone()
            },
        }
    }

    pub fn with_application(mut self, evidence: Option<&RouteEvidence>) -> Self {
        let Some(evidence) = evidence else {
            return self;
        };
        self.applied_model = evidence.model;
        self.applied_reasoning = evidence.reasoning;
        if evidence.model == self.model && evidence.reasoning == self.reasoning {
            self.status = RouteStatus::Applied;
            self.reason = "route_applied_with_hook_evidence".to_owned();
        } else {
            self.status = RouteStatus::Failed;
            self.reason = "route_application_mismatch".to_owned();
        }
        self
    }
}

pub fn route_contract() -> DecisionContract {
    DecisionContract::score(
        "route",
        "route-difficulty.v1",
        vec![
            "low: bounded, deterministic work with little context".to_owned(),
            "medium: multiple files, tradeoffs, or moderate context".to_owned(),
            "high: broad change, ambiguity, or high-risk reasoning".to_owned(),
        ],
        0.0,
        0.65,
    )
}

pub fn score_to_difficulty(score: f64) -> TaskDifficulty {
    if !score.is_finite() {
        return TaskDifficulty::Unknown;
    }
    match score.round() as i32 {
        0 => TaskDifficulty::Low,
        1 => TaskDifficulty::Medium,
        2 => TaskDifficulty::High,
        _ => TaskDifficulty::Unknown,
    }
}

pub fn route_for(difficulty: TaskDifficulty) -> (ModelFamily, ReasoningLevel, Vec<ModelFamily>) {
    match difficulty {
        TaskDifficulty::Low => (
            ModelFamily::Luna,
            ReasoningLevel::Max,
            vec![ModelFamily::Luna, ModelFamily::Terra, ModelFamily::Sol],
        ),
        TaskDifficulty::Medium => (
            ModelFamily::Terra,
            ReasoningLevel::High,
            vec![ModelFamily::Terra, ModelFamily::Sol],
        ),
        TaskDifficulty::High => (
            ModelFamily::Sol,
            ReasoningLevel::Medium,
            vec![ModelFamily::Sol],
        ),
        TaskDifficulty::Unknown => (ModelFamily::Luna, ReasoningLevel::Max, Vec::new()),
    }
}

pub fn route_evidence_from_payload(
    model: Option<&str>,
    reasoning: Option<&str>,
) -> Option<RouteEvidence> {
    let model = model.and_then(parse_model);
    let reasoning = reasoning.and_then(parse_reasoning);
    (model.is_some() || reasoning.is_some()).then_some(RouteEvidence { model, reasoning })
}

pub fn parse_model(value: &str) -> Option<ModelFamily> {
    match value.trim().to_ascii_lowercase().as_str() {
        "luna" | "gpt-5.6-luna" => Some(ModelFamily::Luna),
        "terra" | "gpt-5.6-terra" => Some(ModelFamily::Terra),
        "sol" | "gpt-5.6-sol" => Some(ModelFamily::Sol),
        _ => None,
    }
}

pub fn parse_reasoning(value: &str) -> Option<ReasoningLevel> {
    match value.trim().to_ascii_lowercase().as_str() {
        "max" => Some(ReasoningLevel::Max),
        "high" => Some(ReasoningLevel::High),
        "medium" | "med" => Some(ReasoningLevel::Medium),
        _ => None,
    }
}

pub fn route_result(result: &DecisionResult, evidence: Option<&RouteEvidence>) -> RouteDecision {
    RouteDecision::from_score(result).with_application(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::{DecisionEvidence, DecisionResult};

    fn result(score: f64, confidence: f64, status: DecisionStatus) -> DecisionResult {
        DecisionResult {
            status,
            answer: None,
            evidence: DecisionEvidence {
                score: Some(score),
                confidence: Some(confidence),
                ..DecisionEvidence::default()
            },
            fallback: None,
            reason: "accepted".to_owned(),
        }
    }

    #[test]
    fn difficulty_maps_to_requested_model_and_escalation_chain() {
        let low = RouteDecision::from_score(&result(0.0, 0.9, DecisionStatus::Accepted));
        assert_eq!(low.model, Some(ModelFamily::Luna));
        assert_eq!(low.reasoning, Some(ReasoningLevel::Max));
        assert_eq!(
            low.fallback_chain,
            vec![ModelFamily::Luna, ModelFamily::Terra, ModelFamily::Sol]
        );

        let medium = RouteDecision::from_score(&result(1.0, 0.9, DecisionStatus::Accepted));
        assert_eq!(medium.model, Some(ModelFamily::Terra));
        assert_eq!(medium.reasoning, Some(ReasoningLevel::High));

        let high = RouteDecision::from_score(&result(2.0, 0.9, DecisionStatus::Accepted));
        assert_eq!(high.model, Some(ModelFamily::Sol));
        assert_eq!(high.reasoning, Some(ReasoningLevel::Medium));
    }

    #[test]
    fn application_is_applied_only_when_model_and_reasoning_match() {
        let decision = RouteDecision::from_score(&result(0.0, 0.9, DecisionStatus::Accepted));
        let applied = decision.clone().with_application(Some(&RouteEvidence {
            model: Some(ModelFamily::Luna),
            reasoning: Some(ReasoningLevel::Max),
        }));
        assert_eq!(applied.status, RouteStatus::Applied);

        let failed = decision.with_application(Some(&RouteEvidence {
            model: Some(ModelFamily::Sol),
            reasoning: Some(ReasoningLevel::Medium),
        }));
        assert_eq!(failed.status, RouteStatus::Failed);
    }

    #[test]
    fn malformed_or_unapplied_routes_are_explicit() {
        assert_eq!(score_to_difficulty(f64::NAN), TaskDifficulty::Unknown);
        assert_eq!(score_to_difficulty(3.0), TaskDifficulty::Unknown);
        assert_eq!(
            route_for(TaskDifficulty::Unknown).2,
            Vec::<ModelFamily>::new()
        );
        assert_eq!(parse_model("gpt-5.6-terra"), Some(ModelFamily::Terra));
        assert_eq!(parse_model("LUNA"), Some(ModelFamily::Luna));
        assert_eq!(parse_model("sol"), Some(ModelFamily::Sol));
        assert_eq!(parse_model("unknown"), None);
        assert_eq!(parse_reasoning("med"), Some(ReasoningLevel::Medium));
        assert_eq!(parse_reasoning("MAX"), Some(ReasoningLevel::Max));
        assert_eq!(parse_reasoning("high"), Some(ReasoningLevel::High));
        assert_eq!(parse_reasoning("unknown"), None);
        assert!(route_evidence_from_payload(None, None).is_none());
        assert!(route_evidence_from_payload(Some("unknown"), Some("unknown")).is_none());
        let failed = RouteDecision::failed("timeout");
        assert_eq!(failed.status, RouteStatus::Failed);
        assert!(failed.model.is_none());

        let without_score = DecisionResult {
            status: DecisionStatus::Accepted,
            answer: None,
            evidence: DecisionEvidence::default(),
            fallback: None,
            reason: "missing score".to_owned(),
        };
        assert_eq!(
            RouteDecision::from_score(&without_score).status,
            RouteStatus::Failed
        );
        assert_eq!(
            RouteDecision::from_score(&result(1.0, 0.4, DecisionStatus::Degraded)).status,
            RouteStatus::Failed
        );
    }
}
