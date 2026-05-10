use reqwest::Client;

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{account, market, order, user_stream};
use crate::errors::PluginError;

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    validate_request(&request)?;
    let client = Client::new();
    match request.operation.as_str() {
        "binance_get_server_time" => market::get_server_time(&client, &request).await,
        "binance_get_exchange_info" => market::get_exchange_info(&client, &request).await,
        "binance_get_ticker" => market::get_ticker(&client, &request).await,
        "binance_get_depth" => market::get_depth(&client, &request).await,
        "binance_get_klines" => market::get_klines(&client, &request).await,
        "binance_get_account" => account::get_account(&client, &request).await,
        "binance_get_balances" => account::get_balances(&client, &request).await,
        "binance_get_open_orders" => account::get_open_orders(&client, &request).await,
        "binance_get_order" => account::get_order(&client, &request).await,
        "binance_get_positions" => account::get_positions(&client, &request).await,
        "binance_place_order" => order::place_order(&client, &request).await,
        "binance_cancel_order" => order::cancel_order(&client, &request).await,
        "binance_cancel_all_orders" => order::cancel_all_orders(&client, &request).await,
        "binance_create_user_stream" => user_stream::create_user_stream(&client, &request).await,
        "binance_keepalive_user_stream" => user_stream::keepalive_user_stream(&client, &request).await,
        "binance_close_user_stream" => user_stream::close_user_stream(&client, &request).await,
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
    if request.plugin_id != "binance-node" {
        return Err(PluginError::InvalidInput(format!(
            "plugin_id must be binance-node, got {}",
            request.plugin_id
        )));
    }
    Ok(())
}
