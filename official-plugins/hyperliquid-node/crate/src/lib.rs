use std::fmt::Display;

use contract::{JsonRpcId, JsonRpcResponse, PluginResponse, RequestEnvelope};

pub mod contract;
pub mod errors;
pub mod operations;
pub mod provider;

pub async fn handle_request_json(input: &str) -> Result<String, errors::PluginError> {
    match handle_request_json_inner(input).await {
        Ok(response) => Ok(response),
        Err(error) => {
            if let Some(id) = extract_jsonrpc_id(input) {
                Ok(failure_jsonrpc_response(id, &error.to_string()))
            } else {
                Ok(failure_response(&error.to_string()))
            }
        }
    }
}

async fn handle_request_json_inner(input: &str) -> Result<String, errors::PluginError> {
    let envelope: RequestEnvelope = serde_json::from_str(input)?;
    let (request, id) = envelope.into_request().map_err(errors::PluginError::InvalidInput)?;
    let response = operations::dispatch(request).await?;
    if let Some(id) = id {
        Ok(serde_json::to_string(&JsonRpcResponse::success(id, response))?)
    } else {
        Ok(success_response(&response))
    }
}

pub fn failure_response(message: impl AsRef<str>) -> String {
    serde_json::to_string(&PluginResponse::failure(message.as_ref(), None))
        .unwrap_or_else(|_| String::from("{\"contract_version\":\"1.0.0\",\"success\":false,\"error\":\"internal serialization error\"}"))
}

fn success_response(response: &PluginResponse) -> String {
    serde_json::to_string(response)
        .unwrap_or_else(|_| String::from("{\"contract_version\":\"1.0.0\",\"success\":false,\"error\":\"internal serialization error\"}"))
}

fn failure_jsonrpc_response(id: JsonRpcId, message: impl Display) -> String {
    serde_json::to_string(&JsonRpcResponse::failure(id, message.to_string()))
        .unwrap_or_else(|_| String::from("{\"jsonrpc\":\"2.0\",\"id\":0,\"error\":{\"code\":-32000,\"message\":\"internal serialization error\"}}"))
}

fn extract_jsonrpc_id(input: &str) -> Option<JsonRpcId> {
    serde_json::from_str::<RequestEnvelope>(input).ok().and_then(|envelope| match envelope {
        RequestEnvelope::Legacy(_) => None,
        RequestEnvelope::JsonRpc(request) => Some(request.id),
    })
}
