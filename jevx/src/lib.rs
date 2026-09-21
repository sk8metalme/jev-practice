mod config;
mod discovery;
mod error;
pub mod evaluation;
mod gateway;
mod ranking;
mod redaction;
mod telemetry;
mod types;

pub use config::Config;
pub use discovery::{SkillRoot, discover_skill_roots, discover_skills, parse_skill_file};
pub use error::JevxError;
pub use gateway::GatewayJudge;
pub use ranking::{local_score, suggest_with_judge, suggest_with_optional_judge};
pub use redaction::redact;
pub use telemetry::{Stats, TelemetryEvent, append_telemetry, read_stats};
pub use types::{
    CandidateDecision, CandidateResult, Judge, JudgeCandidate, JudgeRequest, JudgeResponse,
    Metrics, SkillRecord, SuggestInput, SuggestionResult, Usage,
};

pub const MODEL_ID: &str = "typesafe-ai/jev";
pub const DEFAULT_ENDPOINT: &str = "https://ai-gateway.vercel.sh/v1/evaluate";
