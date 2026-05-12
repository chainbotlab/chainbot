use std::collections::BTreeMap;

use reqwest::{Client, Method};
use serde_json::{json, Map, Value};

use crate::contract::{metadata_with_confirmation_mode, PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::provider::{public_get, signed_request, ProductLine, RequestContext};

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    validate_request(&request)?;
    let product_line = request.input_string("product_line").unwrap_or("spot");
    let context = RequestContext::new(
        product_line,
        request.input_string("base_url"),
        request.allowed_origins().to_vec(),
    )?;
    let client = Client::new();
    match request.operation.as_str() {
        "bitget_get_server_time" => read_server_time(&client, &context).await,
        "bitget_get_ticker" => read_ticker(&client, &context, &request).await,
        "bitget_get_depth" => read_depth(&client, &context, &request).await,
        "bitget_get_klines" => read_klines(&client, &context, &request).await,
        "bitget_get_account" => read_account(&client, &context, &request).await,
        "bitget_get_balances" => read_balances(&client, &context, &request).await,
        "bitget_get_open_orders" => read_open_orders(&client, &context, &request).await,
        "bitget_get_order" => read_order(&client, &context, &request).await,
        "bitget_get_positions" => read_positions(&client, &context, &request).await,
        "bitget_place_order" => write_order(&client, &context, &request, WriteMode::Place).await,
        "bitget_cancel_order" => write_order(&client, &context, &request, WriteMode::Cancel).await,
        "bitget_cancel_all_orders" => {
            write_order(&client, &context, &request, WriteMode::CancelAll).await
        }
        _ => Err(PluginError::Unsupported(format!(
            "operation {} is not supported",
            request.operation
        ))),
    }
}

#[derive(Clone, Copy)]
enum WriteMode {
    Place,
    Cancel,
    CancelAll,
}

fn validate_request(request: &PluginRequest) -> Result<(), PluginError> {
    if request.contract_version != "1.0.0" {
        return Err(PluginError::InvalidInput(format!(
            "unsupported contract_version {}",
            request.contract_version
        )));
    }
    if request.plugin_id != "bitget-node" {
        return Err(PluginError::InvalidInput(format!(
            "plugin_id must be bitget-node, got {}",
            request.plugin_id
        )));
    }
    if request.node_id.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from(
            "node_id must not be empty",
        )));
    }
    Ok(())
}

async fn read_server_time(
    client: &Client,
    context: &RequestContext,
) -> Result<PluginResponse, PluginError> {
    let payload = public_get(client, context, "/api/v2/public/time", &[]).await?;
    let server_time = payload
        .get("data")
        .and_then(|value| value.get("serverTime"))
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or_default();
    Ok(success_output(
        String::from("server_time"),
        json!(server_time),
        None,
    ))
}

async fn read_ticker(
    client: &Client,
    context: &RequestContext,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let payload = match context.product_line {
        ProductLine::Spot => {
            let mut query = Vec::new();
            push_optional_query_param(request, "symbol", "symbol", &mut query);
            public_get(client, context, "/api/v2/spot/market/tickers", &query).await?
        }
        ProductLine::Futures => {
            let mut query = vec![(String::from("productType"), product_type(request))];
            if let Some(symbol) = request.input_string("symbol") {
                query.push((String::from("symbol"), symbol.to_string()));
                public_get(client, context, "/api/v2/mix/market/ticker", &query).await?
            } else {
                public_get(client, context, "/api/v2/mix/market/tickers", &query).await?
            }
        }
    };
    Ok(success_output(
        String::from("ticker"),
        extract_data(&payload),
        None,
    ))
}

async fn read_depth(
    client: &Client,
    context: &RequestContext,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let symbol = required_input_string(request, "symbol")?;
    let payload = match context.product_line {
        ProductLine::Spot => {
            let mut query = vec![(String::from("symbol"), symbol.to_string())];
            push_optional_query_param(request, "limit", "limit", &mut query);
            push_optional_query_param(request, "type", "type", &mut query);
            public_get(client, context, "/api/v2/spot/market/orderbook", &query).await?
        }
        ProductLine::Futures => {
            let mut query = vec![
                (String::from("symbol"), symbol.to_string()),
                (String::from("productType"), product_type(request)),
            ];
            push_optional_query_param(request, "limit", "limit", &mut query);
            push_optional_query_param(request, "precision", "precision", &mut query);
            public_get(client, context, "/api/v2/mix/market/merge-depth", &query).await?
        }
    };
    Ok(success_output(
        String::from("depth"),
        extract_data(&payload),
        None,
    ))
}

async fn read_klines(
    client: &Client,
    context: &RequestContext,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let symbol = required_input_string(request, "symbol")?;
    let interval = required_input_string(request, "interval")?;
    let payload = match context.product_line {
        ProductLine::Spot => {
            let mut query = vec![
                (String::from("symbol"), symbol.to_string()),
                (String::from("granularity"), interval.to_string()),
            ];
            push_optional_query_param(request, "start_time", "startTime", &mut query);
            push_optional_query_param(request, "end_time", "endTime", &mut query);
            push_optional_query_param(request, "limit", "limit", &mut query);
            public_get(client, context, "/api/v2/spot/market/candles", &query).await?
        }
        ProductLine::Futures => {
            let mut query = vec![
                (String::from("symbol"), symbol.to_string()),
                (String::from("productType"), product_type(request)),
                (String::from("granularity"), interval.to_string()),
            ];
            push_optional_query_param(request, "start_time", "startTime", &mut query);
            push_optional_query_param(request, "end_time", "endTime", &mut query);
            push_optional_query_param(request, "limit", "limit", &mut query);
            public_get(client, context, "/api/v2/mix/market/candles", &query).await?
        }
    };
    Ok(success_output(
        String::from("klines"),
        extract_data(&payload),
        None,
    ))
}

async fn read_account(
    client: &Client,
    context: &RequestContext,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let payload = match context.product_line {
        ProductLine::Spot => {
            signed_request(
                client,
                Method::GET,
                context,
                "/api/v2/spot/account/info",
                &[],
                None,
                require_secret(request, "api_key")?,
                require_secret(request, "api_secret")?,
                require_secret(request, "passphrase")?,
            )
            .await?
        }
        ProductLine::Futures => {
            let mut query = vec![(String::from("productType"), product_type(request))];
            if let (Some(symbol), Some(margin_coin)) = (
                request.input_string("symbol"),
                request.input_string("margin_coin"),
            ) {
                query.push((String::from("symbol"), symbol.to_string()));
                query.push((String::from("marginCoin"), margin_coin.to_string()));
                signed_request(
                    client,
                    Method::GET,
                    context,
                    "/api/v2/mix/account/account",
                    &query,
                    None,
                    require_secret(request, "api_key")?,
                    require_secret(request, "api_secret")?,
                    require_secret(request, "passphrase")?,
                )
                .await?
            } else {
                signed_request(
                    client,
                    Method::GET,
                    context,
                    "/api/v2/mix/account/accounts",
                    &query,
                    None,
                    require_secret(request, "api_key")?,
                    require_secret(request, "api_secret")?,
                    require_secret(request, "passphrase")?,
                )
                .await?
            }
        }
    };
    Ok(success_output(
        String::from("account"),
        extract_data(&payload),
        None,
    ))
}

async fn read_balances(
    client: &Client,
    context: &RequestContext,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let payload = match context.product_line {
        ProductLine::Spot => {
            let mut query = Vec::new();
            push_optional_query_param(request, "coin", "coin", &mut query);
            push_optional_query_param(request, "asset_type", "assetType", &mut query);
            signed_request(
                client,
                Method::GET,
                context,
                "/api/v2/spot/account/assets",
                &query,
                None,
                require_secret(request, "api_key")?,
                require_secret(request, "api_secret")?,
                require_secret(request, "passphrase")?,
            )
            .await?
        }
        ProductLine::Futures => {
            let query = vec![(String::from("productType"), product_type(request))];
            signed_request(
                client,
                Method::GET,
                context,
                "/api/v2/mix/account/accounts",
                &query,
                None,
                require_secret(request, "api_key")?,
                require_secret(request, "api_secret")?,
                require_secret(request, "passphrase")?,
            )
            .await?
        }
    };
    Ok(success_output(
        String::from("balances"),
        extract_data(&payload),
        None,
    ))
}

async fn read_open_orders(
    client: &Client,
    context: &RequestContext,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let payload = match context.product_line {
        ProductLine::Spot => {
            let mut query = Vec::new();
            push_optional_query_param(request, "symbol", "symbol", &mut query);
            signed_request(
                client,
                Method::GET,
                context,
                "/api/v2/spot/trade/unfilled-orders",
                &query,
                None,
                require_secret(request, "api_key")?,
                require_secret(request, "api_secret")?,
                require_secret(request, "passphrase")?,
            )
            .await?
        }
        ProductLine::Futures => {
            let mut query = vec![(String::from("productType"), product_type(request))];
            push_optional_query_param(request, "symbol", "symbol", &mut query);
            signed_request(
                client,
                Method::GET,
                context,
                "/api/v2/mix/order/orders-pending",
                &query,
                None,
                require_secret(request, "api_key")?,
                require_secret(request, "api_secret")?,
                require_secret(request, "passphrase")?,
            )
            .await?
        }
    };
    Ok(success_output(
        String::from("orders"),
        extract_data(&payload),
        None,
    ))
}

async fn read_order(
    client: &Client,
    context: &RequestContext,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let payload = match context.product_line {
        ProductLine::Spot => {
            let mut query = Vec::new();
            push_order_identifier(request, &mut query)?;
            signed_request(
                client,
                Method::GET,
                context,
                "/api/v2/spot/trade/orderInfo",
                &query,
                None,
                require_secret(request, "api_key")?,
                require_secret(request, "api_secret")?,
                require_secret(request, "passphrase")?,
            )
            .await?
        }
        ProductLine::Futures => {
            let symbol = required_input_string(request, "symbol")?;
            let mut query = vec![
                (String::from("symbol"), symbol.to_string()),
                (String::from("productType"), product_type(request)),
            ];
            push_order_identifier(request, &mut query)?;
            signed_request(
                client,
                Method::GET,
                context,
                "/api/v2/mix/order/detail",
                &query,
                None,
                require_secret(request, "api_key")?,
                require_secret(request, "api_secret")?,
                require_secret(request, "passphrase")?,
            )
            .await?
        }
    };
    Ok(success_output(
        String::from("order"),
        extract_data(&payload),
        None,
    ))
}

async fn read_positions(
    client: &Client,
    context: &RequestContext,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    if context.product_line == ProductLine::Spot {
        return Err(PluginError::InvalidInput(String::from(
            "positions are unavailable for spot",
        )));
    }
    let mut query = vec![(String::from("productType"), product_type(request))];
    push_optional_query_param(request, "symbol", "symbol", &mut query);
    let payload = signed_request(
        client,
        Method::GET,
        context,
        "/api/v2/mix/position/all-position",
        &query,
        None,
        require_secret(request, "api_key")?,
        require_secret(request, "api_secret")?,
        require_secret(request, "passphrase")?,
    )
    .await?;
    Ok(success_output(
        String::from("positions"),
        extract_data(&payload),
        None,
    ))
}

async fn write_order(
    client: &Client,
    context: &RequestContext,
    request: &PluginRequest,
    mode: WriteMode,
) -> Result<PluginResponse, PluginError> {
    let confirmation_mode = request.input_string("confirmation_mode").unwrap_or("safe");
    let body = match (context.product_line, mode) {
        (ProductLine::Spot, WriteMode::Place) => spot_place_body(request)?,
        (ProductLine::Spot, WriteMode::Cancel) => spot_cancel_body(request)?,
        (ProductLine::Spot, WriteMode::CancelAll) => spot_cancel_all_body(request)?,
        (ProductLine::Futures, WriteMode::Place) => futures_place_body(request)?,
        (ProductLine::Futures, WriteMode::Cancel) => futures_cancel_body(request)?,
        (ProductLine::Futures, WriteMode::CancelAll) => futures_cancel_all_body(request)?,
    };
    let path = match (context.product_line, mode) {
        (ProductLine::Spot, WriteMode::Place) => "/api/v2/spot/trade/place-order",
        (ProductLine::Spot, WriteMode::Cancel) => "/api/v2/spot/trade/cancel-order",
        (ProductLine::Spot, WriteMode::CancelAll) => "/api/v2/spot/trade/cancel-symbol-order",
        (ProductLine::Futures, WriteMode::Place) => "/api/v2/mix/order/place-order",
        (ProductLine::Futures, WriteMode::Cancel) => "/api/v2/mix/order/cancel-order",
        (ProductLine::Futures, WriteMode::CancelAll) => "/api/v2/mix/order/cancel-all-orders",
    };
    let payload = signed_request(
        client,
        Method::POST,
        context,
        path,
        &[],
        Some(&body),
        require_secret(request, "api_key")?,
        require_secret(request, "api_secret")?,
        require_secret(request, "passphrase")?,
    )
    .await?;

    let transaction_id = transaction_id_for(mode, context.product_line, request, &payload);
    let confirmation = if confirmation_mode == "submit_only" {
        ConfirmationOutcome::submitted()
    } else {
        confirm_write(client, context, request, mode, &payload).await?
    };
    let mut output = BTreeMap::new();
    output.insert(String::from("status"), json!(confirmation.status));
    output.insert(String::from("transaction_id"), transaction_id);
    output.insert(
        String::from("metadata"),
        json!({
            "confirmation_mode": confirmation_mode,
            "product_line": context.product_line.as_str(),
            "response": extract_data(&payload),
            "terminal": confirmation.terminal,
            "confirmation_response": confirmation.payload,
        }),
    );
    Ok(PluginResponse::success(output, Some(confirmation.result_state)))
}

struct ConfirmationOutcome {
    status: &'static str,
    result_state: &'static str,
    terminal: bool,
    payload: Value,
}

impl ConfirmationOutcome {
    fn submitted() -> Self {
        Self {
            status: "submitted",
            result_state: "submitted",
            terminal: false,
            payload: Value::Null,
        }
    }

    fn pending(payload: Value) -> Self {
        Self {
            status: "pending_confirmation",
            result_state: "submitted",
            terminal: false,
            payload,
        }
    }

    fn settled(payload: Value) -> Self {
        Self {
            status: "confirmed",
            result_state: "settled",
            terminal: true,
            payload,
        }
    }
}

async fn confirm_write(
    client: &Client,
    context: &RequestContext,
    request: &PluginRequest,
    mode: WriteMode,
    submitted_payload: &Value,
) -> Result<ConfirmationOutcome, PluginError> {
    match mode {
        WriteMode::Place | WriteMode::Cancel => confirm_single_order(client, context, request, submitted_payload).await,
        WriteMode::CancelAll => confirm_cancel_all(client, context, request).await,
    }
}

async fn confirm_single_order(
    client: &Client,
    context: &RequestContext,
    request: &PluginRequest,
    submitted_payload: &Value,
) -> Result<ConfirmationOutcome, PluginError> {
    let identifier = confirmation_identifier(request, submitted_payload);
    let Some((key, value)) = identifier else {
        return Ok(ConfirmationOutcome::pending(Value::Null));
    };

    let payload = match context.product_line {
        ProductLine::Spot => {
            let mut query = vec![(key, value)];
            signed_request(
                client,
                Method::GET,
                context,
                "/api/v2/spot/trade/orderInfo",
                &query,
                None,
                require_secret(request, "api_key")?,
                require_secret(request, "api_secret")?,
                require_secret(request, "passphrase")?,
            )
            .await
        }
        ProductLine::Futures => {
            let symbol = required_input_string(request, "symbol")?;
            let mut query = vec![
                (String::from("symbol"), symbol.to_string()),
                (String::from("productType"), product_type(request)),
                (key, value),
            ];
            signed_request(
                client,
                Method::GET,
                context,
                "/api/v2/mix/order/detail",
                &query,
                None,
                require_secret(request, "api_key")?,
                require_secret(request, "api_secret")?,
                require_secret(request, "passphrase")?,
            )
            .await
        }
    };

    match payload {
        Ok(payload) => {
            let status = extract_order_status(&extract_data(&payload));
            if status.as_deref().is_some_and(is_terminal_order_status) {
                Ok(ConfirmationOutcome::settled(extract_data(&payload)))
            } else {
                Ok(ConfirmationOutcome::pending(extract_data(&payload)))
            }
        }
        Err(_) => Ok(ConfirmationOutcome::pending(Value::Null)),
    }
}

async fn confirm_cancel_all(
    client: &Client,
    context: &RequestContext,
    request: &PluginRequest,
) -> Result<ConfirmationOutcome, PluginError> {
    let payload = match context.product_line {
        ProductLine::Spot => {
            let mut query = Vec::new();
            push_optional_query_param(request, "symbol", "symbol", &mut query);
            signed_request(
                client,
                Method::GET,
                context,
                "/api/v2/spot/trade/unfilled-orders",
                &query,
                None,
                require_secret(request, "api_key")?,
                require_secret(request, "api_secret")?,
                require_secret(request, "passphrase")?,
            )
            .await
        }
        ProductLine::Futures => {
            let mut query = vec![(String::from("productType"), product_type(request))];
            push_optional_query_param(request, "symbol", "symbol", &mut query);
            signed_request(
                client,
                Method::GET,
                context,
                "/api/v2/mix/order/orders-pending",
                &query,
                None,
                require_secret(request, "api_key")?,
                require_secret(request, "api_secret")?,
                require_secret(request, "passphrase")?,
            )
            .await
        }
    };

    match payload {
        Ok(payload) => {
            let data = extract_data(&payload);
            let empty = data.as_array().is_some_and(|items| items.is_empty());
            if empty {
                Ok(ConfirmationOutcome::settled(data))
            } else {
                Ok(ConfirmationOutcome::pending(data))
            }
        }
        Err(_) => Ok(ConfirmationOutcome::pending(Value::Null)),
    }
}

fn confirmation_identifier(request: &PluginRequest, submitted_payload: &Value) -> Option<(String, String)> {
    let data = extract_data(submitted_payload);
    if let Some(value) = data.get("orderId").and_then(value_to_string) {
        return Some((String::from("orderId"), value));
    }
    if let Some(value) = data.get("clientOid").and_then(value_to_string) {
        return Some((String::from("clientOid"), value));
    }
    if let Some(value) = request.input.get("order_id").and_then(value_to_string) {
        return Some((String::from("orderId"), value));
    }
    request
        .input
        .get("client_order_id")
        .and_then(value_to_string)
        .map(|value| (String::from("clientOid"), value))
}

fn extract_order_status(data: &Value) -> Option<String> {
    if let Some(status) = data.get("status").and_then(value_to_string) {
        return Some(status.to_ascii_lowercase());
    }
    data.as_array()
        .and_then(|items| items.first())
        .and_then(|item| item.get("status"))
        .and_then(value_to_string)
        .map(|value| value.to_ascii_lowercase())
}

fn is_terminal_order_status(status: &str) -> bool {
    matches!(
        status,
        "filled" | "cancelled" | "canceled" | "rejected" | "full_fill" | "partially_canceled"
    )
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn success_output(key: String, value: Value, result_state: Option<&'static str>) -> PluginResponse {
    PluginResponse::success(BTreeMap::from([(key, value)]), result_state)
}

fn extract_data(payload: &Value) -> Value {
    payload.get("data").cloned().unwrap_or(Value::Null)
}

fn required_input_string<'a>(request: &'a PluginRequest, key: &str) -> Result<&'a str, PluginError> {
    request
        .input_string(key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| PluginError::InvalidInput(format!("input {key} is required")))
}

fn push_optional_query_param(
    request: &PluginRequest,
    input_key: &str,
    target_key: &str,
    query: &mut Vec<(String, String)>,
) {
    if let Some(value) = request.input_string(input_key) {
        query.push((String::from(target_key), value.to_string()));
    }
}

fn push_order_identifier(
    request: &PluginRequest,
    target: &mut Vec<(String, String)>,
) -> Result<(), PluginError> {
    if let Some(order_id) = request.input_string("order_id") {
        target.push((String::from("orderId"), order_id.to_string()));
        return Ok(());
    }
    if let Some(client_order_id) = request.input_string("client_order_id") {
        target.push((String::from("clientOid"), client_order_id.to_string()));
        return Ok(());
    }
    Err(PluginError::InvalidInput(String::from(
        "one of order_id or client_order_id is required",
    )))
}

fn product_type(request: &PluginRequest) -> String {
    request
        .input_string("product_type")
        .map(|value| value.to_ascii_uppercase())
        .unwrap_or_else(|| String::from("USDT-FUTURES"))
}

fn insert_if_string(body: &mut Map<String, Value>, request: &PluginRequest, input_key: &str, target_key: &str) {
    if let Some(value) = request.input_string(input_key) {
        body.insert(String::from(target_key), json!(value));
    }
}

fn spot_place_body(request: &PluginRequest) -> Result<String, PluginError> {
    let mut body = Map::new();
    body.insert(String::from("symbol"), json!(required_input_string(request, "symbol")?));
    body.insert(String::from("side"), json!(required_input_string(request, "side")?));
    body.insert(String::from("orderType"), json!(required_input_string(request, "type")?));
    if let Some(force) = request.input_string("force").or_else(|| request.input_string("time_in_force")) {
        body.insert(String::from("force"), json!(force));
    }
    if let Some(quantity) = request.input_string("quantity") {
        body.insert(String::from("size"), json!(quantity));
    }
    insert_if_string(&mut body, request, "price", "price");
    insert_if_string(&mut body, request, "client_order_id", "clientOid");
    serde_json::to_string(&Value::Object(body)).map_err(PluginError::from)
}

fn spot_cancel_body(request: &PluginRequest) -> Result<String, PluginError> {
    let mut body = Map::new();
    body.insert(String::from("symbol"), json!(required_input_string(request, "symbol")?));
    if let Some(order_id) = request.input_string("order_id") {
        body.insert(String::from("orderId"), json!(order_id));
    } else if let Some(client_order_id) = request.input_string("client_order_id") {
        body.insert(String::from("clientOid"), json!(client_order_id));
    } else {
        return Err(PluginError::InvalidInput(String::from(
            "one of order_id or client_order_id is required",
        )));
    }
    serde_json::to_string(&Value::Object(body)).map_err(PluginError::from)
}

fn spot_cancel_all_body(request: &PluginRequest) -> Result<String, PluginError> {
    let mut body = Map::new();
    body.insert(String::from("symbol"), json!(required_input_string(request, "symbol")?));
    serde_json::to_string(&Value::Object(body)).map_err(PluginError::from)
}

fn futures_place_body(request: &PluginRequest) -> Result<String, PluginError> {
    let mut body = Map::new();
    body.insert(String::from("symbol"), json!(required_input_string(request, "symbol")?));
    body.insert(String::from("productType"), json!(product_type(request)));
    body.insert(String::from("side"), json!(required_input_string(request, "side")?));
    body.insert(String::from("orderType"), json!(required_input_string(request, "type")?));
    if let Some(quantity) = request.input_string("quantity") {
        body.insert(String::from("size"), json!(quantity));
    }
    insert_if_string(&mut body, request, "price", "price");
    insert_if_string(&mut body, request, "margin_mode", "marginMode");
    insert_if_string(&mut body, request, "margin_coin", "marginCoin");
    insert_if_string(&mut body, request, "trade_side", "tradeSide");
    insert_if_string(&mut body, request, "force", "force");
    insert_if_string(&mut body, request, "client_order_id", "clientOid");
    serde_json::to_string(&Value::Object(body)).map_err(PluginError::from)
}

fn futures_cancel_body(request: &PluginRequest) -> Result<String, PluginError> {
    let mut body = Map::new();
    body.insert(String::from("symbol"), json!(required_input_string(request, "symbol")?));
    body.insert(String::from("productType"), json!(product_type(request)));
    insert_if_string(&mut body, request, "margin_coin", "marginCoin");
    if let Some(order_id) = request.input_string("order_id") {
        body.insert(String::from("orderId"), json!(order_id));
    } else if let Some(client_order_id) = request.input_string("client_order_id") {
        body.insert(String::from("clientOid"), json!(client_order_id));
    } else {
        return Err(PluginError::InvalidInput(String::from(
            "one of order_id or client_order_id is required",
        )));
    }
    serde_json::to_string(&Value::Object(body)).map_err(PluginError::from)
}

fn futures_cancel_all_body(request: &PluginRequest) -> Result<String, PluginError> {
    let mut body = Map::new();
    body.insert(String::from("productType"), json!(product_type(request)));
    insert_if_string(&mut body, request, "symbol", "symbol");
    serde_json::to_string(&Value::Object(body)).map_err(PluginError::from)
}

fn transaction_id_for(
    mode: WriteMode,
    product_line: ProductLine,
    request: &PluginRequest,
    payload: &Value,
) -> Value {
    let data = extract_data(payload);
    if let Some(order_id) = data.get("orderId") {
        return order_id.clone();
    }
    if let Some(client_oid) = data.get("clientOid") {
        return client_oid.clone();
    }
    if let Some(symbol) = data.get("symbol") {
        return symbol.clone();
    }
    if let Some(order_id) = request.input.get("order_id") {
        return order_id.clone();
    }
    if let Some(client_order_id) = request.input.get("client_order_id") {
        return client_order_id.clone();
    }
    let prefix = match mode {
        WriteMode::Place => "place",
        WriteMode::Cancel => "cancel",
        WriteMode::CancelAll => "cancel-all",
    };
    json!(format!("{prefix}:{}:{}", product_line.as_str(), request.input_string("symbol").unwrap_or("unknown")))
}

fn require_secret<'a>(request: &'a PluginRequest, key: &str) -> Result<&'a str, PluginError> {
    request
        .activation_secret(key)
        .ok_or_else(|| PluginError::InvalidInput(format!("activation secret {key} is required")))
}
