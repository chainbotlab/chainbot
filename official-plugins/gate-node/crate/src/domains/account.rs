use std::collections::BTreeMap;

use reqwest::Client;

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{
    push_optional_query_param, request_context, require_api_key, require_api_secret, required_input_string, success,
};
use crate::errors::PluginError;
use crate::provider::signed_get;

pub async fn get_accounts(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let mut query = Vec::new();
    push_optional_query_param(request, "currency", "currency", &mut query);
    let payload = signed_get(client, &context, "/api/v4/spot/accounts", query, api_key, api_secret).await?;
    Ok(success(BTreeMap::from([(String::from("accounts"), payload)]), None))
}

pub async fn get_open_orders(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let mut query = Vec::new();
    push_optional_query_param(request, "currency_pair", "currency_pair", &mut query);
    push_optional_query_param(request, "status", "status", &mut query);
    push_optional_query_param(request, "page", "page", &mut query);
    push_optional_query_param(request, "limit", "limit", &mut query);
    let payload = signed_get(client, &context, "/api/v4/spot/open_orders", query, api_key, api_secret).await?;
    Ok(success(BTreeMap::from([(String::from("orders"), payload)]), None))
}

pub async fn get_order(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let order_id = required_input_string(request, "order_id")?;
    let path = format!("/api/v4/spot/orders/{order_id}");
    let mut query = Vec::new();
    push_optional_query_param(request, "currency_pair", "currency_pair", &mut query);
    push_optional_query_param(request, "account", "account", &mut query);
    let payload = signed_get(client, &context, &path, query, api_key, api_secret).await?;
    Ok(success(BTreeMap::from([(String::from("order"), payload)]), None))
}
