use std::collections::BTreeMap;

use reqwest::Client;
use serde_json::{json, Value};

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{optional_input_string, push_optional_query_param, request_context, require_api_key, require_api_secret, required_input_string, success, write_output};
use crate::errors::PluginError;
use crate::provider::{signed_get, signed_post};

fn recv_window(request: &PluginRequest) -> String {
    optional_input_string(request, "recv_window").unwrap_or_else(|| String::from("5000"))
}

pub async fn place_order(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    execute_write(client, request, "/v5/order/create", "place").await
}

pub async fn cancel_order(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    execute_write(client, request, "/v5/order/cancel", "cancel").await
}

pub async fn cancel_all_orders(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    execute_write(client, request, "/v5/order/cancel-all", "cancel-all").await
}

async fn execute_write(client: &Client, request: &PluginRequest, path: &str, prefix: &str) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let recv_window = recv_window(request);
    let confirmation_mode = optional_input_string(request, "confirmation_mode").unwrap_or_else(|| String::from("safe"));

    let mut body = serde_json::Map::new();
    body.insert(String::from("category"), Value::String(context.product_line.as_category().to_owned()));
    if let Some(symbol) = optional_input_string(request, "symbol") {
        body.insert(String::from("symbol"), Value::String(symbol));
    }
    copy_input(request, &mut body, "side", "side");
    copy_input(request, &mut body, "type", "orderType");
    copy_input(request, &mut body, "qty", "qty");
    copy_input(request, &mut body, "price", "price");
    copy_input(request, &mut body, "time_in_force", "timeInForce");
    copy_input(request, &mut body, "order_link_id", "orderLinkId");
    copy_input(request, &mut body, "order_id", "orderId");
    copy_input(request, &mut body, "reduce_only", "reduceOnly");
    copy_input(request, &mut body, "settle_coin", "settleCoin");
    copy_input(request, &mut body, "base_coin", "baseCoin");

    if path == "/v5/order/create" {
        required_input_string(request, "symbol")?;
        required_input_string(request, "side")?;
        required_input_string(request, "type")?;
        required_input_string(request, "qty")?;
    }

    let submitted = signed_post(client, &context, path, Value::Object(body), api_key, api_secret, &recv_window).await?;
    let first_transaction_id = transaction_id(&submitted, prefix);
    if confirmation_mode == "submit_only" {
        return Ok(success(
            enrich_write_output(write_output("submitted", first_transaction_id, &confirmation_mode), &confirmation_mode, submitted, false),
            Some("submitted"),
        ));
    }

    let maybe_query = maybe_query_order(client, request, &context, api_key, api_secret, &recv_window, &submitted).await;
    let query_succeeded = maybe_query.is_ok();
    let resolved = maybe_query.unwrap_or_else(|_| submitted.clone());
    let resolution = classify_order_resolution(&resolved, query_succeeded);
    Ok(success(
        enrich_write_output(
            write_output(resolution.status, transaction_id(&resolved, prefix), &confirmation_mode),
            &confirmation_mode,
            resolved,
            resolution.terminal,
        ),
        Some(resolution.result_state),
    ))
}

async fn maybe_query_order(
    client: &Client,
    request: &PluginRequest,
    context: &crate::provider::RequestContext,
    api_key: &str,
    api_secret: &str,
    recv_window: &str,
    submitted: &Value,
) -> Result<Value, PluginError> {
    let symbol = optional_input_string(request, "symbol").ok_or_else(|| PluginError::InvalidInput(String::from("symbol is required for safe confirmation")))?;
    let mut query = vec![
        (String::from("category"), context.product_line.as_category().to_owned()),
        (String::from("symbol"), symbol),
    ];
    if let Some(order_id) = submitted.get("orderId").and_then(value_as_query_string) {
        query.push((String::from("orderId"), order_id));
    } else if let Some(link_id) = submitted.get("orderLinkId").and_then(value_as_query_string) {
        query.push((String::from("orderLinkId"), link_id));
    } else {
        push_optional_query_param(request, "order_id", "orderId", &mut query);
        push_optional_query_param(request, "order_link_id", "orderLinkId", &mut query);
    }
    if !query.iter().any(|(k, _)| k == "orderId" || k == "orderLinkId") {
        return Ok(submitted.clone());
    }
    signed_get(client, context, "/v5/order/realtime", query, api_key, api_secret, recv_window).await
}

fn copy_input(request: &PluginRequest, target: &mut serde_json::Map<String, Value>, input_key: &str, target_key: &str) {
    if let Some(value) = request.input.get(input_key).cloned() {
        target.insert(String::from(target_key), value);
    }
}

fn transaction_id(response: &Value, prefix: &str) -> String {
    response
        .get("orderId")
        .and_then(value_as_query_string)
        .or_else(|| response.get("orderLinkId").and_then(value_as_query_string))
        .unwrap_or_else(|| String::from(prefix))
}

fn enrich_write_output(mut output: BTreeMap<String, Value>, confirmation_mode: &str, response: Value, terminal: bool) -> BTreeMap<String, Value> {
    output.insert(
        String::from("metadata"),
        json!({"confirmation_mode": confirmation_mode, "terminal": terminal, "response": response}),
    );
    output
}

struct OrderResolution {
    status: &'static str,
    result_state: &'static str,
    terminal: bool,
}

fn classify_order_resolution(response: &Value, query_succeeded: bool) -> OrderResolution {
    if !query_succeeded {
        return OrderResolution { status: "pending_confirmation", result_state: "submitted", terminal: false };
    }
    let status = extract_status(response);
    let terminal = matches!(status.as_deref(), Some("Filled") | Some("Cancelled") | Some("Rejected") | Some("Deactivated") | Some("PartiallyFilledCanceled"));
    if terminal {
        return OrderResolution { status: "confirmed", result_state: "settled", terminal: true };
    }
    OrderResolution { status: "submitted", result_state: "submitted", terminal: false }
}

fn extract_status(response: &Value) -> Option<String> {
    response
        .get("orderStatus")
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| {
            response
                .get("list")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("orderStatus"))
                .and_then(Value::as_str)
                .map(String::from)
        })
}

fn value_as_query_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}
