#[derive(Debug)]
pub enum PluginError {
    Json(serde_json::Error),
    Reqwest(reqwest::Error),
    InvalidInput(String),
    Rpc(String),
    Unsupported(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(source) => write!(f, "json error: {source}"),
            Self::Reqwest(source) => write!(f, "request error: {source}"),
            Self::InvalidInput(message) => f.write_str(message),
            Self::Rpc(message) => f.write_str(message),
            Self::Unsupported(message) => f.write_str(message),
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
