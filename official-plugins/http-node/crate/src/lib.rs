pub mod client;
pub mod contract;
pub mod errors;
pub mod operations;

use contract::PluginRequest;
use errors::PluginError;

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    Ok(match handle_request_json_inner(input).await {
        Ok(response) => response,
        Err(error) => failure_response(&error.to_string()),
    })
}

async fn handle_request_json_inner(input: &str) -> Result<String, PluginError> {
    let request: PluginRequest = serde_json::from_str(input)?;
    let response = operations::dispatch(request).await?;
    serde_json::to_string(&response).map_err(PluginError::from)
}

pub fn failure_response(message: &str) -> String {
    serde_json::json!({
        "contract_version": "1.0.0",
        "success": false,
        "error": message,
    })
    .to_string()
}
