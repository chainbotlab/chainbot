use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum PluginError {
    Json(serde_json::Error),
    Reqwest(reqwest::Error),
    InvalidInput(String),
    Rpc(String),
    Signing(String),
    Unsupported(String),
}

impl Display for PluginError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => write!(f, "json error: {error}"),
            Self::Reqwest(error) => write!(f, "request error: {error}"),
            Self::InvalidInput(message) => write!(f, "invalid input: {message}"),
            Self::Rpc(message) => write!(f, "rpc error: {message}"),
            Self::Signing(message) => write!(f, "signing error: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
        }
    }
}

impl std::error::Error for PluginError {}

impl From<serde_json::Error> for PluginError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<reqwest::Error> for PluginError {
    fn from(value: reqwest::Error) -> Self {
        Self::Reqwest(value)
    }
}
