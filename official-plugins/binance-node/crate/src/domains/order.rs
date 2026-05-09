use std::collections::BTreeMap;

use reqwest::Client;
use serde_json::{json, Value};

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{
    optional_input_string, push_optional_query_param, request_context, require_api_key, require_api_secret,
    required_input_string, success, write_output,
};
use crate::errors::PluginError;
use crate::provider::{signed_delete, signed_get, signed_post, ProductLine};

pub async fn place_order(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let symbol = required_input_string(request, "symbol")?;
    let confirmation_mode = optional_input_string(request, "confirmation_mode")
        .unwrap_or_else(|| String::from("safe"));

    let mut query = vec![
        (String::from("symbol"), symbol.clone()),
        (String::from("side"), required_input_string(request, "side")?),
        (String::from("type"), required_input_string(request, "type")?),
    ];
    push_optional_query_param(request, "quantity", "quantity", &mut query);
    push_optional_query_param(request, "quote_order_qty", "quoteOrderQty", &mut query);
    push_optional_query_param(request, "price", "price", &mut query);
    push_optional_query_param(request, "time_in_force", "timeInForce", &mut query);
    push_optional_query_param(request, "client_order_id", "newClientOrderId", &mut query);
    push_optional_query_param(request, "reduce_only", "reduceOnly", &mut query);
    push_optional_query_param(request, "position_side", "positionSide", &mut query);
    push_optional_query_param(request, "stop_price", "stopPrice", &mut query);
    push_optional_query_param(request, "close_position", "closePosition", &mut query);
    push_optional_query_param(request, "working_type", "workingType", &mut query);
    push_optional_query_param(request, "recv_window", "recvWindow", &mut query);

    let submitted = signed_post(client, &context, context.product_line.order_path(), query, api_key, api_secret).await?;
    let transaction_id = order_transaction_id(&submitted, &symbol, "place");
    if confirmation_mode == "submit_only" {
        return Ok(success(
            enrich_write_output(
                write_output("submitted", transaction_id, &confirmation_mode),
                &confirmation_mode,
                context.product_line,
                submitted,
                false,
            ),
            Some("submitted"),
        ));
    }

    let confirmed = maybe_query_order(client, request, &context, api_key, api_secret, &symbol, &submitted).await;
    let query_succeeded = confirmed.is_ok();
    let resolved = confirmed.unwrap_or_else(|_| submitted.clone());
    let transaction_id = order_transaction_id(&resolved, &symbol, "place");
    let resolution = classify_order_resolution(&resolved, query_succeeded);
    Ok(success(
        enrich_write_output(
            write_output(resolution.status, transaction_id, &confirmation_mode),
            &confirmation_mode,
            context.product_line,
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
    let symbol = required_input_string(request, "symbol")?;
    let confirmation_mode = optional_input_string(request, "confirmation_mode")
        .unwrap_or_else(|| String::from("safe"));

    let mut query = vec![(String::from("symbol"), symbol.clone())];
    push_optional_query_param(request, "order_id", "orderId", &mut query);
    push_optional_query_param(request, "orig_client_order_id", "origClientOrderId", &mut query);
    push_optional_query_param(request, "recv_window", "recvWindow", &mut query);
    if !query.iter().any(|(key, _)| key == "orderId" || key == "origClientOrderId") {
        return Err(PluginError::InvalidInput(String::from(
            "order_id or orig_client_order_id is required",
        )));
    }

    let response = signed_delete(client, &context, context.product_line.order_path(), query, api_key, api_secret).await?;
    let transaction_id = order_transaction_id(&response, &symbol, "cancel");
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
            context.product_line,
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
    let symbol = required_input_string(request, "symbol")?;
    let confirmation_mode = optional_input_string(request, "confirmation_mode")
        .unwrap_or_else(|| String::from("safe"));

    let mut query = vec![(String::from("symbol"), symbol.clone())];
    push_optional_query_param(request, "recv_window", "recvWindow", &mut query);
    let response = signed_delete(
        client,
        &context,
        context.product_line.cancel_all_orders_path(),
        query,
        api_key,
        api_secret,
    )
    .await?;

    let transaction_id = format!("cancel-all:{symbol}");
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
            context.product_line,
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
    symbol: &str,
    submitted: &Value,
) -> Result<Value, PluginError> {
    let mut query = vec![(String::from("symbol"), String::from(symbol))];
    if let Some(order_id) = submitted.get("orderId") {
        if let Some(order_id) = value_as_query_string(order_id) {
            query.push((String::from("orderId"), order_id));
        }
    } else if let Some(client_order_id) = submitted.get("clientOrderId") {
        if let Some(client_order_id) = value_as_query_string(client_order_id) {
            query.push((String::from("origClientOrderId"), client_order_id));
        }
    } else if let Some(client_order_id) = optional_input_string(request, "client_order_id") {
        query.push((String::from("origClientOrderId"), client_order_id));
    } else {
        return Ok(submitted.clone());
    }

    signed_get(client, context, context.product_line.order_path(), query, api_key, api_secret).await
}

fn order_transaction_id(response: &Value, symbol: &str, prefix: &str) -> String {
    response
        .get("orderId")
        .and_then(value_as_query_string)
        .or_else(|| response.get("clientOrderId").and_then(value_as_query_string))
        .unwrap_or_else(|| format!("{prefix}:{symbol}"))
}

fn enrich_write_output(
    mut output: BTreeMap<String, Value>,
    confirmation_mode: &str,
    product_line: ProductLine,
    response: Value,
    terminal: bool,
) -> BTreeMap<String, Value> {
    output.insert(
        String::from("metadata"),
        json!({
            "confirmation_mode": confirmation_mode,
            "product_line": product_line.as_str(),
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

    let binance_status = response.get("status").and_then(Value::as_str);
    let terminal = matches!(
        binance_status,
        Some("FILLED") | Some("CANCELED") | Some("REJECTED") | Some("EXPIRED")
    );

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
        Some("FILLED") | Some("CANCELED") | Some("REJECTED") | Some("EXPIRED")
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
