use std::collections::BTreeMap;

use reqwest::Client;

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{push_optional_query_param, request_context, required_input_string, success};
use crate::errors::PluginError;
use crate::provider::public_get;

pub async fn get_server_time(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let payload = public_get(client, &context, context.inst_type.server_time_path(), vec![]).await?;
    let server_time = payload
        .get("data")
        .and_then(|data| data.as_array())
        .and_then(|items| items.first())
        .and_then(|item| item.get("ts"))
        .cloned()
        .unwrap_or(payload);
    Ok(success(BTreeMap::from([(String::from("server_time"), server_time)]), None))
}

pub async fn get_instruments(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let mut query = vec![(String::from("instType"), context.inst_type.as_api_value().to_owned())];
    push_optional_query_param(request, "inst_family", "instFamily", &mut query);
    push_optional_query_param(request, "inst_id", "instId", &mut query);
    let payload = public_get(client, &context, context.inst_type.instruments_path(), query).await?;
    let instruments = payload.get("data").cloned().unwrap_or(payload);
    Ok(success(BTreeMap::from([(String::from("instruments"), instruments)]), None))
}

pub async fn get_ticker(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let query = vec![(String::from("instId"), required_input_string(request, "inst_id")?)];
    let payload = public_get(client, &context, context.inst_type.ticker_path(), query).await?;
    let ticker = payload
        .get("data")
        .and_then(|data| data.as_array())
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or(payload);
    Ok(success(BTreeMap::from([(String::from("ticker"), ticker)]), None))
}
