pub mod read;
pub mod write;

use std::collections::BTreeMap;

use reqwest::Client;
use serde_json::Value;

use crate::contract::{metadata_with_confirmation_mode, PluginRequest, PluginResponse};
use crate::errors::PluginError;

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    validate_request(&request)?;
    let client = Client::new();
    let output = match request.operation.as_str() {
        "eth_get_balance" => read::get_balance(&client, &request).await?,
        "eth_get_token_balance" => read::get_token_balance(&client, &request).await?,
        "eth_raw_read" => read::raw_read(&client, &request).await?,
        "eth_transfer_native" => write::transfer_native(&request).await?,
        "eth_raw_write" => write::raw_write(&request).await?,
        other => {
            return Err(PluginError::Unsupported(format!(
                "operation {other} is not supported"
            )))
        }
    };
    Ok(output)
}

fn validate_request(request: &PluginRequest) -> Result<(), PluginError> {
    if request.contract_version.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from(
            "contract_version must not be empty",
        )));
    }
    if request.plugin_id.trim().is_empty() || request.node_id.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from(
            "plugin_id and node_id must not be empty",
        )));
    }
    Ok(())
}

pub fn success(output: BTreeMap<String, Value>, result_state: Option<&'static str>) -> PluginResponse {
    PluginResponse::success(output, result_state)
}

pub fn write_output(status: &str, transaction_id: String, confirmation_mode: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (String::from("status"), Value::String(status.to_owned())),
        (
            String::from("transaction_id"),
            Value::String(transaction_id),
        ),
        (
            String::from("metadata"),
            metadata_with_confirmation_mode(confirmation_mode),
        ),
    ])
}
