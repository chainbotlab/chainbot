use std::collections::BTreeMap;

use reqwest::Client;

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{push_optional_query_param, request_context, required_input_string, success};
use crate::errors::PluginError;
use crate::provider::public_get;

pub async fn get_server_time(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let payload = public_get(client, &context, "/v5/market/time", vec![]).await?;
    let server_time = payload.get("timeNano").cloned().or_else(|| payload.get("timeSecond").cloned()).unwrap_or(payload);
    Ok(success(BTreeMap::from([(String::from("server_time"), server_time)]), None))
}

pub async fn get_exchange_info(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let mut query = vec![(String::from("category"), context.product_line.as_category().to_owned())];
    push_optional_query_param(request, "symbol", "symbol", &mut query);
    let payload = public_get(client, &context, "/v5/market/instruments-info", query).await?;
    Ok(success(BTreeMap::from([(String::from("exchange_info"), payload)]), None))
}

pub async fn get_ticker(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let mut query = vec![(String::from("category"), context.product_line.as_category().to_owned())];
    push_optional_query_param(request, "symbol", "symbol", &mut query);
    let payload = public_get(client, &context, "/v5/market/tickers", query).await?;
    Ok(success(BTreeMap::from([(String::from("ticker"), payload)]), None))
}

pub async fn get_depth(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let mut query = vec![
        (String::from("category"), context.product_line.as_category().to_owned()),
        (String::from("symbol"), required_input_string(request, "symbol")?),
    ];
    push_optional_query_param(request, "limit", "limit", &mut query);
    let payload = public_get(client, &context, "/v5/market/orderbook", query).await?;
    Ok(success(BTreeMap::from([(String::from("depth"), payload)]), None))
}

pub async fn get_klines(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let mut query = vec![
        (String::from("category"), context.product_line.as_category().to_owned()),
        (String::from("symbol"), required_input_string(request, "symbol")?),
        (String::from("interval"), required_input_string(request, "interval")?),
    ];
    push_optional_query_param(request, "start_time", "start", &mut query);
    push_optional_query_param(request, "end_time", "end", &mut query);
    push_optional_query_param(request, "limit", "limit", &mut query);
    let payload = public_get(client, &context, "/v5/market/kline", query).await?;
    Ok(success(BTreeMap::from([(String::from("klines"), payload)]), None))
}
