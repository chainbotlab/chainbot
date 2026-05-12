use reqwest::Client;

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{account, market, order};
use crate::errors::PluginError;

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    validate_request(&request)?;
    let client = Client::new();
    match request.operation.as_str() {
        "gate_get_server_time" => market::get_server_time(&client, &request).await,
        "gate_get_currency_pairs" => market::get_currency_pairs(&client, &request).await,
        "gate_get_ticker" => market::get_ticker(&client, &request).await,
        "gate_get_depth" => market::get_depth(&client, &request).await,
        "gate_get_klines" => market::get_klines(&client, &request).await,
        "gate_get_accounts" => account::get_accounts(&client, &request).await,
        "gate_get_open_orders" => account::get_open_orders(&client, &request).await,
        "gate_get_order" => account::get_order(&client, &request).await,
        "gate_place_order" => order::place_order(&client, &request).await,
        "gate_cancel_order" => order::cancel_order(&client, &request).await,
        "gate_cancel_all_orders" => order::cancel_all_orders(&client, &request).await,
        other => Err(PluginError::Unsupported(format!(
            "operation {other} is not supported"
        ))),
    }
}

fn validate_request(request: &PluginRequest) -> Result<(), PluginError> {
    if request.contract_version.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from(
            "contract_version must not be empty",
        )));
    }
    if request.contract_version != "1.0.0" {
        return Err(PluginError::InvalidInput(format!(
            "unsupported contract_version {}",
            request.contract_version
        )));
    }
    if request.plugin_id.trim().is_empty() || request.node_id.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from(
            "plugin_id and node_id must not be empty",
        )));
    }
    if request.plugin_id != "gate-node" {
        return Err(PluginError::InvalidInput(format!(
            "plugin_id must be gate-node, got {}",
            request.plugin_id
        )));
    }
    Ok(())
}
