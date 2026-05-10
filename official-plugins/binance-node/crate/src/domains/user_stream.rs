use std::collections::BTreeMap;

use reqwest::Client;
use serde_json::{json, Value};

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{optional_input_string, request_context, require_api_key, success, required_input_string};
use crate::errors::PluginError;
use crate::provider::{api_key_delete, api_key_post, api_key_put};

pub async fn create_user_stream(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let response = api_key_post(client, &context, context.product_line.user_stream_path(), vec![], api_key).await?;
    let listen_key = response
        .get("listenKey")
        .and_then(Value::as_str)
        .ok_or_else(|| PluginError::Rpc(String::from("listenKey missing from Binance response")))?;
    Ok(success(
        BTreeMap::from([
            (String::from("status"), Value::String(String::from("active"))),
            (String::from("listen_key"), Value::String(listen_key.to_owned())),
            (
                String::from("metadata"),
                json!({
                    "product_line": context.product_line.as_str(),
                }),
            ),
        ]),
        None,
    ))
}

pub async fn keepalive_user_stream(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let query = vec![(String::from("listenKey"), required_input_string(request, "listen_key")?)];
    let response = api_key_put(client, &context, context.product_line.user_stream_path(), query, api_key).await?;
    let listen_key = response
        .get("listenKey")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| optional_input_string(request, "listen_key"));
    Ok(success(
        BTreeMap::from([
            (String::from("status"), Value::String(String::from("active"))),
            (
                String::from("listen_key"),
                listen_key.map(Value::String).unwrap_or(Value::Null),
            ),
            (
                String::from("metadata"),
                json!({
                    "product_line": context.product_line.as_str(),
                    "refreshed": response.get("listenKey").is_some(),
                }),
            ),
        ]),
        None,
    ))
}

pub async fn close_user_stream(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let query = vec![(String::from("listenKey"), required_input_string(request, "listen_key")?)];
    let _response = api_key_delete(client, &context, context.product_line.user_stream_path(), query, api_key).await?;
    Ok(success(
        BTreeMap::from([
            (String::from("status"), Value::String(String::from("closed"))),
            (
                String::from("metadata"),
                json!({
                    "product_line": context.product_line.as_str(),
                }),
            ),
        ]),
        None,
    ))
}
