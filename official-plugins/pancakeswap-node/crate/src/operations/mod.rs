use std::collections::BTreeMap;

use alloy::network::TransactionBuilder;
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use serde_json::Value;

use crate::contract::{metadata, PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::provider::{
    bytes_from_hex, encode_exact_input, encode_exact_input_single, parse_address, parse_u24_dec,
    parse_u256_dec, provider_with_wallet, signer_address,
};

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    validate_request(&request)?;
    match request.operation.as_str() {
        "pancakeswap_v3_swap_exact_input_single" => swap_exact_input_single(&request).await,
        "pancakeswap_v3_swap_exact_input" => swap_exact_input(&request).await,
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
    if request.plugin_id != "pancakeswap-node" {
        return Err(PluginError::InvalidInput(format!(
            "plugin_id must be pancakeswap-node, got {}",
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

async fn swap_exact_input_single(request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let router = parse_address(required_string(request, "router")?, "router")?;
    let token_in = parse_address(required_string(request, "token_in")?, "token_in")?;
    let token_out = parse_address(required_string(request, "token_out")?, "token_out")?;
    let fee = parse_u24_dec(required_string(request, "fee")?, "fee")?;
    let recipient = recipient(request)?;
    let deadline = parse_u256_dec(required_string(request, "deadline")?, "deadline")?;
    let amount_in = parse_u256_dec(required_string(request, "amount_in")?, "amount_in")?;
    let amount_out_minimum = parse_u256_dec(
        required_string(request, "amount_out_minimum")?,
        "amount_out_minimum",
    )?;
    let sqrt_price_limit_x96 = request
        .input_string("sqrt_price_limit_x96")
        .map(|value| parse_u256_dec(value, "sqrt_price_limit_x96"))
        .transpose()?
        .unwrap_or_default();
    let data = encode_exact_input_single(
        token_in,
        token_out,
        fee,
        recipient,
        deadline,
        amount_in,
        amount_out_minimum,
        sqrt_price_limit_x96,
    );
    send_router_transaction(request, router, data).await
}

async fn swap_exact_input(request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let router = parse_address(required_string(request, "router")?, "router")?;
    let encoded_path = bytes_from_hex(required_string(request, "encoded_path")?, "encoded_path")?;
    if encoded_path.len() < 43 {
        return Err(PluginError::InvalidInput(String::from(
            "encoded_path must contain at least tokenIn, fee, tokenOut",
        )));
    }
    let recipient = recipient(request)?;
    let deadline = parse_u256_dec(required_string(request, "deadline")?, "deadline")?;
    let amount_in = parse_u256_dec(required_string(request, "amount_in")?, "amount_in")?;
    let amount_out_minimum = parse_u256_dec(
        required_string(request, "amount_out_minimum")?,
        "amount_out_minimum",
    )?;
    let data = encode_exact_input(
        &encoded_path,
        recipient,
        deadline,
        amount_in,
        amount_out_minimum,
    );
    send_router_transaction(request, router, data).await
}

async fn send_router_transaction(
    request: &PluginRequest,
    router: alloy::primitives::Address,
    data: alloy::primitives::Bytes,
) -> Result<PluginResponse, PluginError> {
    let endpoint = required_string(request, "endpoint")?;
    let signer_secret = request
        .activation_secret("signer")
        .ok_or_else(|| PluginError::InvalidInput(String::from("activation signer is required")))?;
    let from = signer_address(signer_secret)?;
    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(router)
        .with_input(data);
    let confirmation_mode = request.input_string("confirmation_mode").unwrap_or("safe");
    send_transaction(endpoint, signer_secret, tx, confirmation_mode).await
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

    let _receipt = pending
        .get_receipt()
        .await
        .map_err(|error| PluginError::Rpc(error.to_string()))?;

    Ok(PluginResponse::success(
        write_output("confirmed", tx_hash, confirmation_mode),
        Some("settled"),
    ))
}

fn recipient(request: &PluginRequest) -> Result<alloy::primitives::Address, PluginError> {
    if let Some(recipient) = request.input_string("recipient") {
        return parse_address(recipient, "recipient");
    }
    let signer_secret = request
        .activation_secret("signer")
        .ok_or_else(|| PluginError::InvalidInput(String::from("activation signer is required")))?;
    signer_address(signer_secret)
}

fn required_string<'a>(request: &'a PluginRequest, key: &str) -> Result<&'a str, PluginError> {
    request
        .input_string(key)
        .ok_or_else(|| PluginError::InvalidInput(format!("{key} is required")))
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
            metadata(confirmation_mode),
        ),
    ])
}
