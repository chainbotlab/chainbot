use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct PluginRequest {
    pub contract_version: String,
    pub plugin_id: String,
    pub node_id: String,
    pub operation: String,
    #[serde(default)]
    pub input: BTreeMap<String, Value>,
    #[serde(default)]
    pub activation: Option<PluginActivationEnvelope>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PluginActivationEnvelope {
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginResponse {
    pub contract_version: &'static str,
    pub success: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl PluginRequest {
    pub fn input_string(&self, key: &str) -> Option<&str> {
        self.input.get(key).and_then(Value::as_str)
    }

    pub fn activation_secret(&self, key: &str) -> Option<&str> {
        self.activation
            .as_ref()
            .and_then(|activation| activation.secrets.get(key))
            .map(String::as_str)
    }

    pub fn allowed_origins(&self) -> &[String] {
        self.activation
            .as_ref()
            .map(|activation| activation.allowed_origins.as_slice())
            .unwrap_or(&[])
    }
}

impl PluginResponse {
    pub fn success(output: BTreeMap<String, Value>) -> Self {
        Self {
            contract_version: "1.0.0",
            success: true,
            output,
            error: None,
        }
    }
}
