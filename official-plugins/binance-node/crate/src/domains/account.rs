use std::collections::BTreeMap;

use reqwest::Client;
use serde_json::Value;

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{
    optional_input_string, push_optional_query_param, request_context, require_api_key, require_api_secret,
    required_input_string, success,
};
use crate::errors::PluginError;
use crate::provider::{signed_get, ProductLine};

pub async fn get_account(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let mut query = Vec::new();
    push_optional_query_param(request, "recv_window", "recvWindow", &mut query);
    let payload = signed_get(client, &context, context.product_line.account_path(), query, api_key, api_secret).await?;
    Ok(success(BTreeMap::from([(String::from("account"), payload)]), None))
}

pub async fn get_balances(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let mut query = Vec::new();
    push_optional_query_param(request, "recv_window", "recvWindow", &mut query);

    let balances = match context.product_line {
        ProductLine::Spot => {
            let account = signed_get(client, &context, context.product_line.account_path(), query, api_key, api_secret).await?;
            account.get("balances").cloned().unwrap_or(Value::Null)
        }
        _ => {
            let path = context
                .product_line
                .balances_path()
                .ok_or_else(|| PluginError::Unsupported(String::from("balances are unavailable for this product_line")))?;
            signed_get(client, &context, path, query, api_key, api_secret).await?
        }
    };

    Ok(success(
        BTreeMap::from([(String::from("balances"), balances)]),
        None,
    ))
}

pub async fn get_positions(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let symbol_filter = optional_input_string(request, "symbol");
    let mut query = Vec::new();
    push_optional_query_param(request, "symbol", "symbol", &mut query);
    push_optional_query_param(request, "recv_window", "recvWindow", &mut query);

    let positions = match context.product_line {
        ProductLine::Spot => {
            return Err(PluginError::Unsupported(String::from(
                "positions are unavailable for spot product_line",
            )));
        }
        ProductLine::Usdm => {
            let path = context
                .product_line
                .positions_path()
                .ok_or_else(|| PluginError::Unsupported(String::from("positions path is unavailable")))?;
            signed_get(client, &context, path, query, api_key, api_secret).await?
        }
        ProductLine::Coinm => {
            let account = signed_get(client, &context, context.product_line.account_path(), query, api_key, api_secret).await?;
            let positions = account.get("positions").cloned().unwrap_or(Value::Null);
            filter_positions(positions, symbol_filter.as_deref())
        }
    };

    Ok(success(
        BTreeMap::from([(String::from("positions"), positions)]),
        None,
    ))
}

fn filter_positions(value: Value, symbol_filter: Option<&str>) -> Value {
    match (value, symbol_filter) {
        (Value::Array(items), Some(symbol)) => Value::Array(
            items
                .into_iter()
                .filter(|item| item.get("symbol").and_then(Value::as_str) == Some(symbol))
                .collect(),
        ),
        (value, _) => value,
    }
}

pub async fn get_open_orders(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let mut query = Vec::new();
    push_optional_query_param(request, "symbol", "symbol", &mut query);
    push_optional_query_param(request, "recv_window", "recvWindow", &mut query);
    let payload = signed_get(client, &context, context.product_line.open_orders_path(), query, api_key, api_secret).await?;
    Ok(success(BTreeMap::from([(String::from("orders"), payload)]), None))
}

pub async fn get_order(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let mut query = vec![(String::from("symbol"), required_input_string(request, "symbol")?)];
    push_optional_query_param(request, "order_id", "orderId", &mut query);
    push_optional_query_param(request, "orig_client_order_id", "origClientOrderId", &mut query);
    push_optional_query_param(request, "recv_window", "recvWindow", &mut query);

    if !query.iter().any(|(key, _)| key == "orderId" || key == "origClientOrderId") {
        return Err(PluginError::InvalidInput(String::from(
            "order_id or orig_client_order_id is required",
        )));
    }

    let payload = signed_get(client, &context, context.product_line.order_path(), query, api_key, api_secret).await?;
    Ok(success(BTreeMap::from([(String::from("order"), payload)]), None))
}
