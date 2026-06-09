use std::collections::BTreeMap;
use std::time::Duration;

use alloy::network::TransactionBuilder;
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use reqwest::Client;
use serde_json::Value;

use crate::contract::{metadata, PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::provider::{
    decode_u256_array, encode_get_amounts_out, encode_swap_exact_tokens_for_tokens, eth_call,
    parse_address, parse_path, parse_u256_dec, provider_with_wallet, signer_address,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RECEIPT_TIMEOUT: Duration = Duration::from_secs(120);

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    validate_request(&request)?;
    let client = http_client()?;
    match request.operation.as_str() {
        "uniswap_get_amounts_out" => get_amounts_out(&client, &request).await,
        "uniswap_watch_price" => watch_price(&client, &request).await,
        "uniswap_swap_exact_tokens_for_tokens" => swap_exact_tokens_for_tokens(&request).await,
        other => Err(PluginError::Unsupported(format!(
            "operation {other} is not supported"
        ))),
    }
}

fn http_client() -> Result<Client, PluginError> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(PluginError::from)
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
    if request.plugin_id != "uniswap-node" {
        return Err(PluginError::InvalidInput(format!(
            "plugin_id must be uniswap-node, got {}",
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

async fn get_amounts_out(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let amounts = quote_amounts_out(client, request).await?;
    Ok(PluginResponse::success(amounts_output(amounts), None))
}

async fn watch_price(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let amounts = quote_amounts_out(client, request).await?;
    let amount_out = amounts
        .last()
        .copied()
        .ok_or_else(|| PluginError::Rpc(String::from("router returned no output amount")))?;
    let threshold = parse_u256_dec(required_string(request, "threshold_out")?, "threshold_out")?;
    let comparison = request.input_string("comparison").unwrap_or("gte");
    let triggered = match comparison {
        "gte" => amount_out >= threshold,
        "lte" => amount_out <= threshold,
        other => {
            return Err(PluginError::InvalidInput(format!(
                "comparison must be gte or lte, got {other}"
            )))
        }
    };
    let output = BTreeMap::from([
        (String::from("triggered"), Value::Bool(triggered)),
        (String::from("amount_out"), Value::String(amount_out.to_string())),
        (
            String::from("threshold_out"),
            Value::String(threshold.to_string()),
        ),
        (
            String::from("comparison"),
            Value::String(comparison.to_owned()),
        ),
        (String::from("metadata"), metadata("uniswap-v2-router", None)),
    ]);
    Ok(PluginResponse::success(output, None))
}

async fn swap_exact_tokens_for_tokens(request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let endpoint = required_string(request, "endpoint")?;
    let router = parse_address(required_string(request, "router")?, "router")?;
    let amount_in = parse_u256_dec(required_string(request, "amount_in")?, "amount_in")?;
    let amount_out_min = parse_u256_dec(required_string(request, "amount_out_min")?, "amount_out_min")?;
    let path = request
        .input
        .get("path")
        .ok_or_else(|| PluginError::InvalidInput(String::from("path is required")))
        .and_then(parse_path)?;
    let signer_secret = request
        .activation_secret("signer")
        .ok_or_else(|| PluginError::InvalidInput(String::from("activation signer is required")))?;
    let recipient = match request.input_string("recipient") {
        Some(recipient) => parse_address(recipient, "recipient")?,
        None => signer_address(signer_secret)?,
    };
    let deadline = parse_u256_dec(required_string(request, "deadline")?, "deadline")?;
    let confirmation_mode = confirmation_mode(request)?;
    let data = encode_swap_exact_tokens_for_tokens(
        amount_in,
        amount_out_min,
        &path,
        recipient,
        deadline,
    );
    let from = signer_address(signer_secret)?;
    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(router)
        .with_input(data);
    send_transaction(endpoint, signer_secret, tx, confirmation_mode).await
}

async fn quote_amounts_out(client: &Client, request: &PluginRequest) -> Result<Vec<alloy::primitives::U256>, PluginError> {
    let endpoint = required_string(request, "endpoint")?;
    let router = parse_address(required_string(request, "router")?, "router")?;
    let amount_in = parse_u256_dec(required_string(request, "amount_in")?, "amount_in")?;
    let path = request
        .input
        .get("path")
        .ok_or_else(|| PluginError::InvalidInput(String::from("path is required")))
        .and_then(parse_path)?;
    let block_tag = request.input_string("block_tag").unwrap_or("latest");
    let data = encode_get_amounts_out(amount_in, &path);
    let encoded = eth_call(client, endpoint, router, data, block_tag).await?;
    decode_u256_array(&encoded)
}

async fn send_transaction(
    endpoint: &str,
    signer_secret: &str,
    tx: TransactionRequest,
    confirmation_mode: &str,
) -> Result<PluginResponse, PluginError> {
    let provider = provider_with_wallet(endpoint, signer_secret)?;
    let pending = provider
        .send_transaction(tx)
        .await
        .map_err(|error| PluginError::Rpc(error.to_string()))?;

    let tx_hash = format!("{:#x}", pending.tx_hash());
    if confirmation_mode == "submit_only" {
        return Ok(PluginResponse::success(
            write_output("submitted", tx_hash, confirmation_mode),
            Some("submitted"),
        ));
    }

    let receipt = pending
        .with_timeout(Some(RECEIPT_TIMEOUT))
        .get_receipt()
        .await
        .map_err(|error| PluginError::Rpc(error.to_string()))?;
    if !receipt.status() {
        return Ok(PluginResponse::success(
            write_output("failed", tx_hash, confirmation_mode),
            Some("failed"),
        ));
    }

    Ok(PluginResponse::success(
        write_output("confirmed", tx_hash, confirmation_mode),
        Some("settled"),
    ))
}

fn required_string<'a>(request: &'a PluginRequest, key: &str) -> Result<&'a str, PluginError> {
    request
        .input_string(key)
        .ok_or_else(|| PluginError::InvalidInput(format!("{key} is required")))
}

fn confirmation_mode(request: &PluginRequest) -> Result<&str, PluginError> {
    let confirmation_mode = request.input_string("confirmation_mode").unwrap_or("safe");
    match confirmation_mode {
        "submit_only" | "safe" => Ok(confirmation_mode),
        other => Err(PluginError::InvalidInput(format!(
            "confirmation_mode must be submit_only or safe, got {other}"
        ))),
    }
}

fn amounts_output(amounts: Vec<alloy::primitives::U256>) -> BTreeMap<String, Value> {
    let amount_out = amounts.last().map(ToString::to_string).unwrap_or_default();
    BTreeMap::from([
        (
            String::from("amounts"),
            Value::Array(
                amounts
                    .iter()
                    .map(|amount| Value::String(amount.to_string()))
                    .collect(),
            ),
        ),
        (String::from("amount_out"), Value::String(amount_out)),
        (String::from("metadata"), metadata("uniswap-v2-router", None)),
    ])
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
            metadata("uniswap-v2-router", Some(confirmation_mode)),
        ),
    ])
}
