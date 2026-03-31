use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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

#[derive(Debug, Clone, Deserialize)]
pub struct PluginActivationEnvelope {
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginResponse {
    pub contract_version: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_state: Option<&'static str>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl PluginRequest {
    pub fn input_string(&self, key: &str) -> Option<&str> {
        self.input.get(key).and_then(Value::as_str)
    }

    pub fn input_bool(&self, key: &str) -> Option<bool> {
        self.input.get(key).and_then(Value::as_bool)
    }

    pub fn activation_secret(&self, key: &str) -> Option<&str> {
        self.activation
            .as_ref()
            .and_then(|activation| activation.secrets.get(key))
            .map(String::as_str)
    }
}

impl PluginResponse {
    pub fn success(output: BTreeMap<String, Value>, result_state: Option<&'static str>) -> Self {
        Self {
            contract_version: String::from("1.0.0"),
            success: true,
            result_state,
            output,
            error: None,
        }
    }

    pub fn failure(message: impl Into<String>, result_state: Option<&'static str>) -> Self {
        Self {
            contract_version: String::from("1.0.0"),
            success: false,
            result_state,
            output: BTreeMap::new(),
            error: Some(message.into()),
        }
    }
}

pub fn metadata_with_confirmation_mode(confirmation_mode: &str) -> Value {
    json!({"confirmation_mode": confirmation_mode})
}
