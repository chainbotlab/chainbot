use std::str::FromStr;

use base64::Engine;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_transaction::Transaction;

use crate::errors::PluginError;

#[derive(Debug, Deserialize)]
struct RpcBalanceValue {
    value: u64,
}

#[derive(Debug, Deserialize)]
struct RpcTokenBalanceValue {
    value: RpcTokenAmount,
}

#[derive(Debug, Deserialize)]
struct RpcTokenAmount {
    amount: String,
}

#[derive(Debug, Deserialize)]
struct RpcBlockhashValue {
    value: RpcBlockhash,
}

#[derive(Debug, Deserialize)]
struct RpcBlockhash {
    blockhash: String,
}

#[derive(Debug, Deserialize)]
struct RpcStatuses {
    value: Vec<Option<RpcSignatureStatus>>,
}

#[derive(Debug, Deserialize)]
struct RpcSignatureStatus {
    err: Option<Value>,
}

pub async fn rpc_call<T: DeserializeOwned>(
    client: &Client,
    endpoint: &str,
    method: &str,
    params: Value,
) -> Result<T, PluginError> {
    let response = client
        .post(endpoint)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await?;
    let payload: Value = response.json().await?;
    if let Some(error) = payload.get("error") {
        return Err(PluginError::Rpc(error.to_string()));
    }
    serde_json::from_value(payload.get("result").cloned().unwrap_or(Value::Null))
        .map_err(PluginError::from)
}

pub async fn get_balance(
    client: &Client,
    endpoint: &str,
    address: &str,
    commitment: &str,
) -> Result<u64, PluginError> {
    let account = parse_pubkey(address, "address")?;
    let result: RpcBalanceValue = rpc_call(
        client,
        endpoint,
        "getBalance",
        json!([account.to_string(), {"commitment": commitment}]),
    )
    .await?;
    Ok(result.value)
}

pub async fn get_token_balance(
    client: &Client,
    endpoint: &str,
    token_account: &str,
    commitment: &str,
) -> Result<String, PluginError> {
    let account = parse_pubkey(token_account, "token_account")?;
    let result: RpcTokenBalanceValue = rpc_call(
        client,
        endpoint,
        "getTokenAccountBalance",
        json!([account.to_string(), {"commitment": commitment}]),
    )
    .await?;
    Ok(result.value.amount)
}

pub async fn raw_read(
    client: &Client,
    endpoint: &str,
    method: &str,
    params: Value,
) -> Result<Value, PluginError> {
    rpc_call(client, endpoint, method, params).await
}

pub async fn fetch_latest_blockhash(client: &Client, endpoint: &str) -> Result<Hash, PluginError> {
    let result: RpcBlockhashValue = rpc_call(
        client,
        endpoint,
        "getLatestBlockhash",
        json!([{"commitment": "confirmed"}]),
    )
    .await?;
    Hash::from_str(&result.value.blockhash)
        .map_err(|error| PluginError::Rpc(format!("invalid blockhash returned by rpc: {error}")))
}

pub async fn send_transaction(
    client: &Client,
    endpoint: &str,
    transaction: &Transaction,
    confirmation_mode: &str,
    preflight: bool,
) -> Result<String, PluginError> {
    let transaction_bytes = bincode::serialize(transaction)?;
    let payload = base64::engine::general_purpose::STANDARD.encode(transaction_bytes);
    rpc_call(
        client,
        endpoint,
        "sendTransaction",
        json!([
            payload,
            {
                "encoding": "base64",
                "preflightCommitment": preflight_commitment(confirmation_mode),
                "skipPreflight": !preflight
            }
        ]),
    )
    .await
}

fn preflight_commitment(confirmation_mode: &str) -> &str {
    match confirmation_mode {
        "processed" => "processed",
        "finalized" => "finalized",
        _ => "confirmed",
    }
}

pub async fn wait_for_signature_status(
    client: &Client,
    endpoint: &str,
    signature: &str,
) -> Result<(), PluginError> {
    let result: RpcStatuses = rpc_call(
        client,
        endpoint,
        "getSignatureStatuses",
        json!([[signature], {"searchTransactionHistory": false}]),
    )
    .await?;
    let status = result
        .value
        .into_iter()
        .next()
        .flatten()
        .ok_or_else(|| PluginError::Rpc(String::from("transaction status is not yet available")))?;
    if let Some(error) = status.err {
        return Err(PluginError::Rpc(format!(
            "transaction settled with error: {error}"
        )));
    }
    Ok(())
}

pub fn parse_pubkey(value: &str, field: &str) -> Result<Pubkey, PluginError> {
    Pubkey::from_str(value)
        .map_err(|error| PluginError::InvalidInput(format!("invalid {field}: {error}")))
}

pub fn parse_lamports(value: &str, field: &str) -> Result<u64, PluginError> {
    value
        .parse::<u64>()
        .map_err(|error| PluginError::InvalidInput(format!("invalid {field}: {error}")))
}

pub fn parse_keypair_from_secret(secret: &str) -> Result<Keypair, PluginError> {
    if secret.trim_start().starts_with('[') {
        let bytes: Vec<u8> = serde_json::from_str(secret)
            .map_err(|error| PluginError::Signing(format!("invalid json keypair bytes: {error}")))?;
        return Keypair::try_from(bytes.as_slice())
            .map_err(|error| PluginError::Signing(format!("invalid keypair bytes: {error}")));
    }

    Keypair::try_from_base58_string(secret)
        .map_err(|error| PluginError::Signing(format!("invalid base58 signer: {error}")))
}
