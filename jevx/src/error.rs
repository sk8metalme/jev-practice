use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum JevxError {
    InvalidInput(String),
    MissingApiKey,
    Provider(String),
    Timeout,
    Io(std::io::Error),
    Json(serde_json::Error),
    Yaml(serde_yaml::Error),
}

impl Display for JevxError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) => write!(formatter, "{message}"),
            Self::MissingApiKey => write!(formatter, "AI_GATEWAY_API_KEY is not configured"),
            Self::Provider(message) => write!(formatter, "{message}"),
            Self::Timeout => write!(formatter, "Jev request timed out"),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Json(error) => write!(formatter, "JSON error: {error}"),
            Self::Yaml(error) => write!(formatter, "YAML error: {error}"),
        }
    }
}

impl std::error::Error for JevxError {}

impl From<std::io::Error> for JevxError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for JevxError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<serde_yaml::Error> for JevxError {
    fn from(error: serde_yaml::Error) -> Self {
        Self::Yaml(error)
    }
}
