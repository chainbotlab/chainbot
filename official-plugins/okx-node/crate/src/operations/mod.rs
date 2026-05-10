use reqwest::Client;

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{account, market};
use crate::errors::PluginError;

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    validate_request(&request)?;
    let client = Client::builder().build()?;
    match request.operation.as_str() {
        "okx_get_server_time" => market::get_server_time(&client, &request).await,
        "okx_get_instruments" => market::get_instruments(&client, &request).await,
        "okx_get_ticker" => market::get_ticker(&client, &request).await,
        "okx_get_account_balance" => account::get_account_balance(&client, &request).await,
        other => Err(PluginError::Unsupported(format!("unsupported okx operation {other}"))),
    }
}

fn validate_request(request: &PluginRequest) -> Result<(), PluginError> {
    if request.contract_version.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from("contract_version is required")));
    }
    if request.contract_version != "1.0.0" {
        return Err(PluginError::InvalidInput(format!(
            "unsupported contract_version {}",
            request.contract_version
        )));
    }
    if request.plugin_id.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from("plugin_id is required")));
    }
    if request.plugin_id != "okx-node" {
        return Err(PluginError::InvalidInput(format!(
            "plugin_id must be okx-node, got {}",
            request.plugin_id
        )));
    }
    if request.node_id.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from("node_id is required")));
    }
    Ok(())
}
