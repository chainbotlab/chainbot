pub mod contract;
pub mod domains;
pub mod errors;
pub mod operations;
pub mod provider;

use contract::PluginResponse;
use errors::PluginError;

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    Ok(match handle_request_json_inner(input).await {
        Ok(response) => response,
        Err(error) => match extract_jsonrpc_id(input) {
            Some(id) => failure_jsonrpc_response(id, &error.to_string()),
            None => failure_response(&error.to_string()),
        },
    })
}

async fn handle_request_json_inner(input: &str) -> Result<String, PluginError> {
    let envelope: contract::RequestEnvelope = serde_json::from_str(input)?;
    let (request, jsonrpc_id) = envelope.into_request().map_err(PluginError::InvalidInput)?;
    let response = operations::dispatch(request).await?;
    match jsonrpc_id {
        Some(id) => serde_json::to_string(&contract::JsonRpcResponse::success(id, response))
            .map_err(PluginError::from),
        None => serde_json::to_string(&response).map_err(PluginError::from),
    }
}

pub fn failure_response(message: &str) -> String {
    serde_json::json!({
        "contract_version": "1.0.0",
        "success": false,
        "error": message,
    })
    .to_string()
}

pub fn success_response(response: PluginResponse) -> Result<String, PluginError> {
    Ok(serde_json::to_string(&response)?)
}

pub fn failure_jsonrpc_response(id: contract::JsonRpcId, message: &str) -> String {
    serde_json::to_string(&contract::JsonRpcResponse::failure(id, message))
        .unwrap_or_else(|_| failure_response(message))
}

fn extract_jsonrpc_id(input: &str) -> Option<contract::JsonRpcId> {
    match serde_json::from_str::<contract::RequestEnvelope>(input).ok()? {
        contract::RequestEnvelope::Legacy(_) => None,
        contract::RequestEnvelope::JsonRpc(envelope) => Some(envelope.id),
    }
}
