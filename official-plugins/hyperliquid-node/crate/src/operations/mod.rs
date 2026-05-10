use reqwest::Client;
use serde_json::json;

use crate::contract::{PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::provider::{output_map, post_info, RequestContext};

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    validate_request(&request)?;
    let client = Client::new();
    match request.operation.as_str() {
        "hyperliquid_get_all_mids" => get_all_mids(&client, &request).await,
        "hyperliquid_get_l2_book" => get_l2_book(&client, &request).await,
        "hyperliquid_get_candle_snapshot" => get_candle_snapshot(&client, &request).await,
        other => Err(PluginError::Unsupported(format!("operation {other} is not supported"))),
    }
}

fn validate_request(request: &PluginRequest) -> Result<(), PluginError> {
    if request.contract_version.is_empty() {
        return Err(PluginError::InvalidInput(String::from("contract_version is required")));
    }
    if request.contract_version != "1.0.0" {
        return Err(PluginError::InvalidInput(format!(
            "unsupported contract_version {}",
            request.contract_version
        )));
    }
    if request.plugin_id.is_empty() {
        return Err(PluginError::InvalidInput(String::from("plugin_id is required")));
    }
    if request.plugin_id != "hyperliquid-node" {
        return Err(PluginError::InvalidInput(format!("plugin_id must be hyperliquid-node, got {}", request.plugin_id)));
    }
    if request.node_id.is_empty() {
        return Err(PluginError::InvalidInput(String::from("node_id is required")));
    }
    Ok(())
}

fn request_context(request: &PluginRequest) -> Result<RequestContext, PluginError> {
    RequestContext::new(request.input_string("base_url"), request.allowed_origins())
}

async fn get_all_mids(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let mut body = json!({ "type": "allMids" });
    if let Some(dex) = request.input_string("dex") {
        body["dex"] = json!(dex);
    }
    let response = post_info(client, &context, body).await?;
    Ok(PluginResponse::success(output_map("all_mids", response), None))
}

async fn get_l2_book(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let coin = request
        .input_string("coin")
        .ok_or_else(|| PluginError::InvalidInput(String::from("input coin is required")))?;
    let mut body = json!({
        "type": "l2Book",
        "coin": coin,
    });
    if let Some(n_sig_figs) = request.input_i64("nSigFigs") {
        body["nSigFigs"] = json!(n_sig_figs);
    }
    if let Some(mantissa) = request.input_i64("mantissa") {
        body["mantissa"] = json!(mantissa);
    }
    let response = post_info(client, &context, body).await?;
    Ok(PluginResponse::success(output_map("l2_book", response), None))
}

async fn get_candle_snapshot(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let coin = request
        .input_string("coin")
        .ok_or_else(|| PluginError::InvalidInput(String::from("input coin is required")))?;
    let interval = request
        .input_string("interval")
        .ok_or_else(|| PluginError::InvalidInput(String::from("input interval is required")))?;
    let start_time = request
        .input_i64("startTime")
        .ok_or_else(|| PluginError::InvalidInput(String::from("input startTime is required")))?;
    let end_time = request
        .input_i64("endTime")
        .ok_or_else(|| PluginError::InvalidInput(String::from("input endTime is required")))?;
    let body = json!({
        "type": "candleSnapshot",
        "req": {
            "coin": coin,
            "interval": interval,
            "startTime": start_time,
            "endTime": end_time,
        }
    });
    let response = post_info(client, &context, body).await?;
    Ok(PluginResponse::success(output_map("candles", response), None))
}
