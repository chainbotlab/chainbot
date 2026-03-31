use alloy::network::TransactionBuilder;
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use serde_json::Value;

use crate::contract::{PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::operations::{success, write_output};
use crate::provider::{
    parse_address, parse_u256_dec, provider_with_wallet, signer_address, transaction_request_from_value,
};

pub async fn transfer_native(request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let endpoint = request
        .input_string("endpoint")
        .ok_or_else(|| PluginError::InvalidInput(String::from("endpoint is required")))?;
    let to = parse_address(
        request
            .input_string("to")
            .ok_or_else(|| PluginError::InvalidInput(String::from("to is required")))?,
        "to",
    )?;
    let amount = parse_u256_dec(
        request
            .input_string("amount")
            .ok_or_else(|| PluginError::InvalidInput(String::from("amount is required")))?,
        "amount",
    )?;
    let confirmation_mode = request.input_string("confirmation_mode").unwrap_or("safe");
    let signer_secret = request
        .activation_secret("signer")
        .ok_or_else(|| PluginError::InvalidInput(String::from("activation signer is required")))?;
    let from = signer_address(signer_secret)?;

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(amount);
    send_transaction(endpoint, signer_secret, tx, confirmation_mode).await
}

pub async fn raw_write(request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let endpoint = request
        .input_string("endpoint")
        .ok_or_else(|| PluginError::InvalidInput(String::from("endpoint is required")))?;
    let method = request
        .input_string("method")
        .ok_or_else(|| PluginError::InvalidInput(String::from("method is required")))?;
    if method != "eth_sendTransaction" {
        return Err(PluginError::Unsupported(format!(
            "raw_write only supports eth_sendTransaction in the managed signing path, got {method}"
        )));
    }
    let params = request
        .input
        .get("params")
        .and_then(Value::as_array)
        .ok_or_else(|| PluginError::InvalidInput(String::from("params must be an array")))?;
    let tx_value = params
        .first()
        .ok_or_else(|| PluginError::InvalidInput(String::from("params[0] transaction object is required")))?;
    let signer_secret = request
        .activation_secret("signer")
        .ok_or_else(|| PluginError::InvalidInput(String::from("activation signer is required")))?;
    let mut tx = transaction_request_from_value(tx_value)?;
    if tx.from.is_none() {
        tx = tx.with_from(signer_address(signer_secret)?);
    }
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
        return Ok(success(
            write_output("submitted", tx_hash, confirmation_mode),
            Some("submitted"),
        ));
    }

    let _receipt = pending
        .get_receipt()
        .await
        .map_err(|error| PluginError::Rpc(error.to_string()))?;

    Ok(success(
        write_output("confirmed", tx_hash, confirmation_mode),
        Some("settled"),
    ))
}
