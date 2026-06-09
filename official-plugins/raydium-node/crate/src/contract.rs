use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const JSONRPC_VERSION: &str = "2.0";
pub const CONTRACT_VERSION: &str = "1.0.0";
pub const EXECUTE_METHOD: &str = "node.exec.v2";

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
    pub contract_version: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_state: Option<&'static str>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RequestEnvelope {
    Legacy(PluginRequest),
    JsonRpc(JsonRpcRequest),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    String(String),
    Number(i64),
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    pub method: String,
    pub params: PluginRequest,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcSuccessResult {
    pub contract_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_state: Option<&'static str>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: JsonRpcId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<JsonRpcSuccessResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl PluginRequest {
    pub fn input_string(&self, key: &str) -> Option<&str> {
        self.input.get(key).and_then(Value::as_str)
    }

    pub fn input_bool(&self, key: &str) -> Option<bool> {
        self.input.get(key).and_then(Value::as_bool)
    }

    pub fn input_u64(&self, key: &str) -> Option<u64> {
        self.input.get(key).and_then(Value::as_u64)
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
            contract_version: String::from(CONTRACT_VERSION),
            success: true,
            result_state,
            output,
            error: None,
        }
    }
}

impl RequestEnvelope {
    pub fn into_request(self) -> Result<(PluginRequest, Option<JsonRpcId>), String> {
        match self {
            Self::Legacy(request) => Ok((request, None)),
            Self::JsonRpc(envelope) => {
                if envelope.jsonrpc != JSONRPC_VERSION {
                    return Err(format!("jsonrpc must be {JSONRPC_VERSION}"));
                }
                if envelope.method != EXECUTE_METHOD {
                    return Err(format!("method must be {EXECUTE_METHOD}"));
                }
                Ok((envelope.params, Some(envelope.id)))
            }
        }
    }
}

impl JsonRpcResponse {
    pub fn success(id: JsonRpcId, response: PluginResponse) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: Some(JsonRpcSuccessResult {
                contract_version: CONTRACT_VERSION,
                result_state: response.result_state,
                output: response.output,
            }),
            error: None,
        }
    }

    pub fn failure(id: JsonRpcId, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32000,
                message: message.into(),
            }),
        }
    }
}

pub fn metadata(provider: &str) -> Value {
    json!({"provider": provider})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> PluginRequest {
        PluginRequest {
            contract_version: String::from(CONTRACT_VERSION),
            plugin_id: String::from("raydium-node"),
            node_id: String::from("node-1"),
            operation: String::from("raydium_compute_swap"),
            input: BTreeMap::new(),
            activation: None,
        }
    }

    #[test]
    fn jsonrpc_envelope_accepts_node_exec_v2() {
        let envelope = RequestEnvelope::JsonRpc(JsonRpcRequest {
            jsonrpc: String::from(JSONRPC_VERSION),
            id: JsonRpcId::Number(1),
            method: String::from(EXECUTE_METHOD),
            params: request(),
        });

        let (_request, id) = envelope.into_request().expect("envelope should be valid");

        assert!(matches!(id, Some(JsonRpcId::Number(1))));
    }

    #[test]
    fn jsonrpc_envelope_rejects_legacy_node_execute_method() {
        let envelope = RequestEnvelope::JsonRpc(JsonRpcRequest {
            jsonrpc: String::from(JSONRPC_VERSION),
            id: JsonRpcId::Number(1),
            method: String::from("node.execute"),
            params: request(),
        });

        let error = envelope.into_request().expect_err("legacy method should be rejected");

        assert!(error.contains(EXECUTE_METHOD));
    }
}
