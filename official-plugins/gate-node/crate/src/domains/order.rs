use std::collections::BTreeMap;

use reqwest::Client;
use serde_json::{json, Value};

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{
    optional_input_string, push_optional_query_param, request_context, require_api_key, require_api_secret,
    required_input_string, success, write_output,
};
use crate::errors::PluginError;
use crate::provider::{signed_delete, signed_get, signed_post};

pub async fn place_order(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let currency_pair = required_input_string(request, "currency_pair")?;
    let confirmation_mode = optional_input_string(request, "confirmation_mode")
        .unwrap_or_else(|| String::from("safe"));

    let mut body = serde_json::Map::new();
    body.insert(String::from("currency_pair"), Value::String(currency_pair.clone()));
    body.insert(String::from("side"), Value::String(required_input_string(request, "side")?));
    body.insert(String::from("amount"), Value::String(required_input_string(request, "amount")?));
    body.insert(String::from("price"), Value::String(required_input_string(request, "price")?));

    push_optional_body_param(request, "account", "account", &mut body);
    push_optional_body_param(request, "text", "text", &mut body);
    push_optional_body_param(request, "time_in_force", "time_in_force", &mut body);
    push_optional_body_param(request, "iceberg", "iceberg", &mut body);
    push_optional_body_param(request, "auto_borrow", "auto_borrow", &mut body);
    push_optional_body_param(request, "auto_repay", "auto_repay", &mut body);
    push_optional_body_param(request, "stp_act", "stp_act", &mut body);
    push_optional_body_param(request, "action_mode", "action_mode", &mut body);

    let submitted = signed_post(
        client,
        &context,
        "/api/v4/spot/orders",
        vec![],
        Value::Object(body),
        api_key,
        api_secret,
    )
    .await?;

    let transaction_id = order_transaction_id(&submitted, "place");
    if confirmation_mode == "submit_only" {
        return Ok(success(
            enrich_write_output(
                write_output("submitted", transaction_id, &confirmation_mode),
                &confirmation_mode,
                submitted,
                false,
            ),
            Some("submitted"),
        ));
    }

    let confirmed = maybe_query_order(client, request, &context, api_key, api_secret, &submitted).await;
    let query_succeeded = confirmed.is_ok();
    let resolved = confirmed.unwrap_or_else(|_| submitted.clone());
    let transaction_id = order_transaction_id(&resolved, "place");
    let resolution = classify_order_resolution(&resolved, query_succeeded);
    Ok(success(
        enrich_write_output(
            write_output(resolution.status, transaction_id, &confirmation_mode),
            &confirmation_mode,
            resolved,
            resolution.terminal,
        ),
        Some(resolution.result_state),
    ))
}

pub async fn cancel_order(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let order_id = required_input_string(request, "order_id")?;
    let currency_pair = required_input_string(request, "currency_pair")?;
    let confirmation_mode = optional_input_string(request, "confirmation_mode")
        .unwrap_or_else(|| String::from("safe"));

    let path = format!("/api/v4/spot/orders/{order_id}");
    let mut query = vec![(String::from("currency_pair"), currency_pair.clone())];
    push_optional_query_param(request, "account", "account", &mut query);
    push_optional_query_param(request, "action_mode", "action_mode", &mut query);

    let response = signed_delete(client, &context, &path, query, api_key, api_secret).await?;
    let transaction_id = order_transaction_id(&response, "cancel");
    let resolution = if confirmation_mode == "submit_only" {
        OrderResolution {
            status: "submitted",
            result_state: "submitted",
            terminal: false,
        }
    } else {
        classify_terminal_status(&response)
    };
    Ok(success(
        enrich_write_output(
            write_output(resolution.status, transaction_id, &confirmation_mode),
            &confirmation_mode,
            response,
            resolution.terminal,
        ),
        Some(resolution.result_state),
    ))
}

pub async fn cancel_all_orders(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let currency_pair = required_input_string(request, "currency_pair")?;
    let confirmation_mode = optional_input_string(request, "confirmation_mode")
        .unwrap_or_else(|| String::from("safe"));

    let mut query = vec![(String::from("currency_pair"), currency_pair.clone())];
    push_optional_query_param(request, "side", "side", &mut query);
    push_optional_query_param(request, "account", "account", &mut query);
    push_optional_query_param(request, "action_mode", "action_mode", &mut query);

    let response = signed_delete(client, &context, "/api/v4/spot/orders", query, api_key, api_secret).await?;
    let transaction_id = format!("cancel-all:{currency_pair}");
    let resolution = if confirmation_mode == "submit_only" {
        OrderResolution {
            status: "submitted",
            result_state: "submitted",
            terminal: false,
        }
    } else {
        classify_terminal_status(&response)
    };
    Ok(success(
        enrich_write_output(
            write_output(resolution.status, transaction_id, &confirmation_mode),
            &confirmation_mode,
            response,
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
    submitted: &Value,
) -> Result<Value, PluginError> {
    let order_id = submitted
        .get("id")
        .and_then(value_as_query_string)
        .or_else(|| optional_input_string(request, "order_id"));
    let order_id = match order_id {
        Some(value) => value,
        None => return Ok(submitted.clone()),
    };

    let currency_pair = submitted
        .get("currency_pair")
        .and_then(value_as_query_string)
        .or_else(|| optional_input_string(request, "currency_pair"));

    let path = format!("/api/v4/spot/orders/{order_id}");
    let mut query = Vec::new();
    if let Some(currency_pair) = currency_pair {
        query.push((String::from("currency_pair"), currency_pair));
    }
    signed_get(client, context, &path, query, api_key, api_secret).await
}

fn order_transaction_id(response: &Value, prefix: &str) -> String {
    response
        .get("id")
        .and_then(value_as_query_string)
        .or_else(|| response.get("text").and_then(value_as_query_string))
        .unwrap_or_else(|| String::from(prefix))
}

fn enrich_write_output(
    mut output: BTreeMap<String, Value>,
    confirmation_mode: &str,
    response: Value,
    terminal: bool,
) -> BTreeMap<String, Value> {
    output.insert(
        String::from("metadata"),
        json!({
            "confirmation_mode": confirmation_mode,
            "terminal": terminal,
            "response": response,
        }),
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
        return OrderResolution {
            status: "pending_confirmation",
            result_state: "submitted",
            terminal: false,
        };
    }

    let gate_status = response.get("status").and_then(Value::as_str);
    let terminal = matches!(gate_status, Some("closed") | Some("cancelled") | Some("finished"));

    if terminal {
        return OrderResolution {
            status: "confirmed",
            result_state: "settled",
            terminal: true,
        };
    }

    OrderResolution {
        status: "submitted",
        result_state: "submitted",
        terminal: false,
    }
}

fn classify_terminal_status(response: &Value) -> OrderResolution {
    let terminal = match response {
        Value::Array(items) => items.iter().all(is_terminal_status_value),
        _ => is_terminal_status_value(response),
    };

    if terminal {
        OrderResolution {
            status: "confirmed",
            result_state: "settled",
            terminal: true,
        }
    } else {
        OrderResolution {
            status: "submitted",
            result_state: "submitted",
            terminal: false,
        }
    }
}

fn is_terminal_status_value(value: &Value) -> bool {
    matches!(
        value.get("status").and_then(Value::as_str),
        Some("closed") | Some("cancelled") | Some("finished")
    )
}

fn value_as_query_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn push_optional_body_param(
    request: &PluginRequest,
    input_key: &str,
    target_key: &str,
    body: &mut serde_json::Map<String, Value>,
) {
    if let Some(value) = optional_input_string(request, input_key) {
        body.insert(String::from(target_key), Value::String(value));
    }
}
