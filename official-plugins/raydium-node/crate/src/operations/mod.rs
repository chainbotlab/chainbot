use std::collections::BTreeMap;

use reqwest::Client;
use serde_json::{json, Map, Value};

use crate::contract::{metadata, PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::{provider, rpc};

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    validate_request(&request)?;
    let client = Client::new();
    match request.operation.as_str() {
        "raydium_compute_swap" => compute_swap(&client, &request).await,
        "raydium_build_swap" => build_swap(&client, &request).await,
        "raydium_send_swap_transaction" => send_swap_transaction(&client, &request).await,
        "raydium_get_signature_status" => get_signature_status(&client, &request).await,
        other => Err(PluginError::Unsupported(format!(
            "operation {other} is not supported"
        ))),
    }
}

fn validate_request(request: &PluginRequest) -> Result<(), PluginError> {
    if request.contract_version.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from(
            "contract_version must not be empty",
        )));
    }
    if request.contract_version != "1.0.0" {
        return Err(PluginError::InvalidInput(format!(
            "unsupported contract_version {}",
            request.contract_version
        )));
    }
    if request.plugin_id != "raydium-node" {
        return Err(PluginError::InvalidInput(format!(
            "plugin_id must be raydium-node, got {}",
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

async fn compute_swap(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let input_mint = required_string(request, "input_mint")?;
    let output_mint = required_string(request, "output_mint")?;
    let amount = required_string(request, "amount")?;
    let swap_type = swap_type(request)?;
    let endpoint = match swap_type {
        "base_in" => "compute/swap-base-in",
        "base_out" => "compute/swap-base-out",
        _ => unreachable!("swap_type validates supported values"),
    };
    let mut url = provider::swap_base_url(request)?.join(endpoint).map_err(|error| {
        PluginError::InvalidInput(format!("invalid Raydium compute endpoint: {error}"))
    })?;
    let mut query_pairs = vec![
        (String::from("inputMint"), input_mint.to_owned()),
        (String::from("outputMint"), output_mint.to_owned()),
        (String::from("amount"), amount.to_owned()),
    ];
    append_optional_query(&mut query_pairs, request, "slippage_bps", "slippageBps");
    append_optional_query(&mut query_pairs, request, "tx_version", "txVersion");
    append_optional_query(&mut query_pairs, request, "referrer", "referrer");
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in query_pairs {
            query.append_pair(&key, &value);
        }
    }

    let quote = provider::get_json(client, url, request.activation_secret("api_key")).await?;
    let output = compute_output(quote);
    Ok(PluginResponse::success(output, None))
}

async fn build_swap(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let swap_response = request
        .input
        .get("swap_response")
        .cloned()
        .ok_or_else(|| PluginError::InvalidInput(String::from("swap_response is required")))?;
    let wallet = required_string(request, "wallet")?;
    let tx_version = request.input_string("tx_version").unwrap_or("V0");
    let compute_unit_price = request
        .input_string("compute_unit_price_micro_lamports")
        .unwrap_or("0");
    let mut body = Map::new();
    body.insert(String::from("swapResponse"), swap_response);
    body.insert(
        String::from("wallet"),
        Value::String(wallet.to_owned()),
    );
    body.insert(String::from("txVersion"), Value::String(tx_version.to_owned()));
    body.insert(
        String::from("computeUnitPriceMicroLamports"),
        Value::String(compute_unit_price.to_owned()),
    );
    append_optional_body(&mut body, request, "wrap_sol", "wrapSol");
    append_optional_body(&mut body, request, "unwrap_sol", "unwrapSol");
    append_optional_body(&mut body, request, "input_account", "inputAccount");
    append_optional_body(&mut body, request, "output_account", "outputAccount");
    append_optional_body(&mut body, request, "jito_info", "jitoInfo");
    append_optional_body(&mut body, request, "referrer_wallet", "referrerWallet");

    let swap_type = swap_type(request)?;
    let endpoint = match swap_type {
        "base_in" => "transaction/swap-base-in",
        "base_out" => "transaction/swap-base-out",
        _ => unreachable!("swap_type validates supported values"),
    };
    let url = provider::swap_base_url(request)?.join(endpoint).map_err(|error| {
        PluginError::InvalidInput(format!("invalid Raydium transaction endpoint: {error}"))
    })?;
    let swap = provider::post_json(
        client,
        url,
        Value::Object(body),
        request.activation_secret("api_key"),
    )
    .await?;
    let output = swap_output(swap)?;
    Ok(PluginResponse::success(output, Some("prepared")))
}

async fn send_swap_transaction(
    client: &Client,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let endpoint = required_string(request, "endpoint")?;
    let signed_transaction = required_string(request, "signed_transaction")?;
    let encoding = request.input_string("encoding").unwrap_or("base64");
    if encoding != "base64" && encoding != "base58" {
        return Err(PluginError::InvalidInput(format!(
            "encoding must be base64 or base58, got {encoding}"
        )));
    }
    let confirmation_mode = request.input_string("confirmation_mode").unwrap_or("confirmed");
    let preflight = request.input_bool("preflight").unwrap_or(true);
    let signature = rpc::send_transaction(
        client,
        endpoint,
        signed_transaction,
        encoding,
        confirmation_mode,
        preflight,
    )
    .await?;
    if confirmation_mode == "submit_only" {
        return Ok(PluginResponse::success(
            write_output("submitted", signature, confirmation_mode),
            Some("submitted"),
        ));
    }

    let _status = rpc::wait_for_signature_status(client, endpoint, &signature).await?;
    Ok(PluginResponse::success(
        write_output("settled", signature, confirmation_mode),
        Some("settled"),
    ))
}

async fn get_signature_status(
    client: &Client,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let endpoint = required_string(request, "endpoint")?;
    let signature = required_string(request, "signature")?;
    let status = rpc::signature_status(client, endpoint, signature).await?;
    let output = BTreeMap::from([
        (String::from("signature"), Value::String(signature.to_owned())),
        (
            String::from("status"),
            serde_json::to_value(status).map_err(PluginError::from)?,
        ),
        (String::from("metadata"), metadata("raydium")),
    ]);
    Ok(PluginResponse::success(output, None))
}

fn required_string<'a>(request: &'a PluginRequest, key: &str) -> Result<&'a str, PluginError> {
    request
        .input_string(key)
        .ok_or_else(|| PluginError::InvalidInput(format!("{key} is required")))
}

fn append_optional_query(
    query: &mut Vec<(String, String)>,
    request: &PluginRequest,
    input_key: &str,
    query_key: &str,
) {
    if let Some(value) = request.input.get(input_key).and_then(value_as_query_string) {
        query.push((query_key.to_owned(), value));
    }
}

fn append_optional_body(
    body: &mut Map<String, Value>,
    request: &PluginRequest,
    input_key: &str,
    body_key: &str,
) {
    if let Some(value) = request.input.get(input_key) {
        body.insert(body_key.to_owned(), value.clone());
    }
}

fn value_as_query_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn compute_output(response: Value) -> BTreeMap<String, Value> {
    let data = response.get("data").cloned().unwrap_or(Value::Null);
    let mut output = BTreeMap::new();
    output.insert(String::from("swap_response"), response.clone());
    copy_field(&mut output, &data, "inputMint", "input_mint");
    copy_field(&mut output, &data, "outputMint", "output_mint");
    copy_field(&mut output, &data, "inputAmount", "input_amount");
    copy_field(&mut output, &data, "outputAmount", "output_amount");
    copy_field(
        &mut output,
        &data,
        "otherAmountThreshold",
        "other_amount_threshold",
    );
    copy_field(&mut output, &data, "priceImpactPct", "price_impact_pct");
    copy_field(&mut output, &data, "routePlan", "route_plan");
    output.insert(String::from("metadata"), metadata("raydium"));
    output
}

fn swap_output(swap: Value) -> Result<BTreeMap<String, Value>, PluginError> {
    let transactions = swap
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            PluginError::Api(String::from(
                "swap response did not include transaction data",
            ))
        })?;
    let transaction_values: Vec<Value> = transactions
        .iter()
        .filter_map(|item| item.get("transaction").and_then(Value::as_str))
        .map(|transaction| Value::String(transaction.to_owned()))
        .collect();
    if transaction_values.is_empty() {
        return Err(PluginError::Api(String::from(
            "swap response did not include transactions",
        )));
    }
    Ok(BTreeMap::from([
        (String::from("status"), Value::String(String::from("prepared"))),
        (
            String::from("transactions"),
            Value::Array(transaction_values),
        ),
        (String::from("build_response"), swap.clone()),
        (String::from("metadata"), metadata("raydium")),
    ]))
}

fn write_output(status: &str, transaction_id: String, confirmation_mode: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (String::from("status"), Value::String(status.to_owned())),
        (
            String::from("transaction_id"),
            Value::String(transaction_id),
        ),
        (
            String::from("metadata"),
            json!({"provider": "raydium", "confirmation_mode": confirmation_mode}),
        ),
    ])
}

fn copy_field(output: &mut BTreeMap<String, Value>, source: &Value, source_key: &str, output_key: &str) {
    if let Some(value) = source.get(source_key) {
        output.insert(output_key.to_owned(), value.clone());
    }
}

fn swap_type(request: &PluginRequest) -> Result<&str, PluginError> {
    let swap_type = request.input_string("swap_type").unwrap_or("base_in");
    match swap_type {
        "base_in" | "base_out" => Ok(swap_type),
        other => Err(PluginError::InvalidInput(format!(
            "swap_type must be base_in or base_out, got {other}"
        ))),
    }
}
