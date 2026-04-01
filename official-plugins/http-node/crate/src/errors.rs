use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum PluginError {
    Json(serde_json::Error),
    InvalidRequest(String),
    InvalidPolicy(String),
    RequestFailed(String),
}

impl From<serde_json::Error> for PluginError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json(source)
    }
}

impl Display for PluginError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(source) => write!(f, "invalid request json: {source}"),
            Self::InvalidRequest(message) => write!(f, "{message}"),
            Self::InvalidPolicy(message) => write!(f, "{message}"),
            Self::RequestFailed(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for PluginError {}
