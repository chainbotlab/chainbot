use std::collections::BTreeMap;

use reqwest::Client;

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{push_optional_query_param, request_context, required_input_string, success};
use crate::errors::PluginError;
use crate::provider::public_get;

pub async fn get_server_time(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let payload = public_get(client, &context, "/api/v4/spot/time", vec![]).await?;
    let server_time = payload.get("server_time").cloned().unwrap_or(payload);
    Ok(success(
        BTreeMap::from([(String::from("server_time"), server_time)]),
        None,
    ))
}

pub async fn get_currency_pairs(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let payload = public_get(client, &context, "/api/v4/spot/currency_pairs", vec![]).await?;
    Ok(success(
        BTreeMap::from([(String::from("currency_pairs"), payload)]),
        None,
    ))
}

pub async fn get_ticker(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let mut query = Vec::new();
    push_optional_query_param(request, "currency_pair", "currency_pair", &mut query);
    let payload = public_get(client, &context, "/api/v4/spot/tickers", query).await?;
    Ok(success(BTreeMap::from([(String::from("ticker"), payload)]), None))
}

pub async fn get_depth(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let mut query = vec![(String::from("currency_pair"), required_input_string(request, "currency_pair")?)];
    push_optional_query_param(request, "limit", "limit", &mut query);
    let payload = public_get(client, &context, "/api/v4/spot/order_book", query).await?;
    Ok(success(BTreeMap::from([(String::from("depth"), payload)]), None))
}

pub async fn get_klines(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let mut query = vec![
        (String::from("currency_pair"), required_input_string(request, "currency_pair")?),
        (String::from("interval"), required_input_string(request, "interval")?),
    ];
    push_optional_query_param(request, "from", "from", &mut query);
    push_optional_query_param(request, "to", "to", &mut query);
    push_optional_query_param(request, "limit", "limit", &mut query);
    let payload = public_get(client, &context, "/api/v4/spot/candlesticks", query).await?;
    Ok(success(BTreeMap::from([(String::from("klines"), payload)]), None))
}
