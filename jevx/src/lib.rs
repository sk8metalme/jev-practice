pub mod compact_assist;
mod config;
pub mod cost;
pub mod data;
pub mod decision;
mod discovery;
mod error;
pub mod evaluation;
mod gateway;
pub mod hook_config;
pub mod hooks;
mod ranking;
pub mod recorder;
mod redaction;
pub mod review;
pub mod route;
mod storage;
mod telemetry;
mod types;

pub use config::Config;
pub use cost::{
    CodexUsage, CostAccumulator, CostBasis, CostEstimate, CostStatus, CostSummary, TokenPricing,
    total_cost,
};
pub use decision::{
    DECISION_CONTRACT_VERSION, DecisionCache, DecisionContract, DecisionEvidence,
    DecisionExecution, DecisionExecutionOptions, DecisionFailure, DecisionJudge, DecisionMode,
    DecisionPolicy, DecisionRequest, DecisionResponse, DecisionResult, DecisionStatus,
    FallbackReason, InMemoryDecisionCache, LegacyJudgeAdapter, PredicateCriteria, QuestionSpec,
    Severity, StatePlan, TypedAnswer, execute_contract, replay_contract,
};
pub use discovery::{SkillRoot, discover_skill_roots, discover_skills, parse_skill_file};
pub use error::JevxError;
pub use gateway::GatewayJudge;
pub use ranking::{
    LocalRanking, local_score, rank_candidates, suggest_with_judge, suggest_with_optional_judge,
};
pub use recorder::{
    DecisionReceipt, DecisionRecorder, DecisionStats, JsonlDecisionRecorder,
    read_decision_receipts, read_decision_stats, replay_receipt,
};
pub use redaction::redact;
pub use review::{
    CostUnitSummary, FixApplication, FixOperation, FixPlan, FixStatus, REVIEW_CONTRACT_VERSION,
    REVIEW_SCHEMA_VERSION, ReviewCategory, ReviewFinding, ReviewReceipt, ReviewRequest,
    ReviewResponse, ReviewStats, ReviewStatus, ReviewTarget, append_review_receipt, apply_fix_plan,
    fix_operations_from_json, read_review_stats, review_contracts, review_with_optional_judge,
};
pub use route::{
    ModelFamily, ROUTE_SCHEMA_VERSION, ReasoningLevel, RouteDecision, RouteEvidence, RouteStatus,
    TaskDifficulty, route_contract, route_evidence_from_payload, route_for, route_result,
};
pub use telemetry::{
    Stats, TELEMETRY_SCHEMA_VERSION, TelemetryEvent, append_telemetry, read_stats,
};
pub use types::{
    CandidateDecision, CandidateResult, Judge, JudgeCandidate, JudgeEvaluation, JudgeRequest,
    JudgeResponse, Metrics, SUGGESTION_SCHEMA_VERSION, SkillRecord, SuggestInput, SuggestionResult,
    Usage,
};

pub const MODEL_ID: &str = "typesafe-ai/jev";
pub const DEFAULT_ENDPOINT: &str = "https://ai-gateway.vercel.sh/v1/evaluate";
