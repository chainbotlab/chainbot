use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const JSONRPC_VERSION: &str = "2.0";
pub const EXECUTE_METHOD: &str = "node.execute";

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
    pub fn success(id: JsonRpcId, output: BTreeMap<String, Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: Some(JsonRpcSuccessResult {
                contract_version: "1.0.0",
                output,
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
