use std::collections::BTreeMap;

use reqwest::Client;
use serde_json::Value;

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{optional_input_string, push_optional_query_param, request_context, require_api_key, require_api_secret, required_input_string, success};
use crate::errors::PluginError;
use crate::provider::{signed_get, ProductLine};

fn recv_window(request: &PluginRequest) -> String {
    optional_input_string(request, "recv_window").unwrap_or_else(|| String::from("5000"))
}

pub async fn get_account(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let recv_window = recv_window(request);
    let mut query = vec![(String::from("accountType"), String::from("UNIFIED"))];
    let payload = signed_get(client, &context, "/v5/account/wallet-balance", std::mem::take(&mut query), api_key, api_secret, &recv_window).await?;
    Ok(success(BTreeMap::from([(String::from("account"), payload)]), None))
}

pub async fn get_balances(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let recv_window = recv_window(request);
    let payload = signed_get(
        client,
        &context,
        "/v5/account/wallet-balance",
        vec![(String::from("accountType"), String::from("UNIFIED"))],
        api_key,
        api_secret,
        &recv_window,
    )
    .await?;

    let balances = payload
        .get("list")
        .and_then(Value::as_array)
        .and_then(|list| list.first())
        .and_then(|entry| entry.get("coin"))
        .cloned()
        .unwrap_or(Value::Array(vec![]));
    Ok(success(BTreeMap::from([(String::from("balances"), balances)]), None))
}

pub async fn get_open_orders(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let recv_window = recv_window(request);
    let mut query = vec![(String::from("category"), context.product_line.as_category().to_owned())];
    push_optional_query_param(request, "symbol", "symbol", &mut query);
    let payload = signed_get(client, &context, "/v5/order/realtime", query, api_key, api_secret, &recv_window).await?;
    Ok(success(BTreeMap::from([(String::from("orders"), payload)]), None))
}

pub async fn get_order(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let recv_window = recv_window(request);
    let mut query = vec![
        (String::from("category"), context.product_line.as_category().to_owned()),
        (String::from("symbol"), required_input_string(request, "symbol")?),
    ];
    push_optional_query_param(request, "order_id", "orderId", &mut query);
    push_optional_query_param(request, "order_link_id", "orderLinkId", &mut query);
    if !query.iter().any(|(key, _)| key == "orderId" || key == "orderLinkId") {
        return Err(PluginError::InvalidInput(String::from("order_id or order_link_id is required")));
    }
    let payload = signed_get(client, &context, "/v5/order/realtime", query, api_key, api_secret, &recv_window).await?;
    Ok(success(BTreeMap::from([(String::from("order"), payload)]), None))
}

pub async fn get_positions(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    if context.product_line == ProductLine::Spot {
        return Err(PluginError::Unsupported(String::from("positions are unavailable for spot product_line")));
    }
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let recv_window = recv_window(request);
    let mut query = vec![(String::from("category"), context.product_line.as_category().to_owned())];
    push_optional_query_param(request, "symbol", "symbol", &mut query);
    push_optional_query_param(request, "settle_coin", "settleCoin", &mut query);
    let payload = signed_get(client, &context, "/v5/position/list", query, api_key, api_secret, &recv_window).await?;
    Ok(success(BTreeMap::from([(String::from("positions"), payload)]), None))
}
