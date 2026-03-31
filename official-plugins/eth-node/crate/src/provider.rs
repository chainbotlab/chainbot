use alloy::network::EthereumWallet;
use alloy::primitives::{Address, Bytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::errors::PluginError;

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

pub async fn get_balance(client: &Client, endpoint: &str, address: &str, block_tag: &str) -> Result<U256, PluginError> {
    let balance_hex: String = rpc_call(
        client,
        endpoint,
        "eth_getBalance",
        json!([address, block_tag]),
    )
    .await?;
    parse_u256_hex(&balance_hex)
}

pub async fn raw_read(client: &Client, endpoint: &str, method: &str, params: Value) -> Result<Value, PluginError> {
    rpc_call(client, endpoint, method, params).await
}

pub async fn token_balance(
    client: &Client,
    endpoint: &str,
    token_address: Address,
    wallet_address: Address,
    block_tag: &str,
) -> Result<U256, PluginError> {
    let mut call_data = String::from("0x70a08231");
    call_data.push_str(&format!("{:0>64}", hex::encode(wallet_address.as_slice())));
    let result_hex: String = rpc_call(
        client,
        endpoint,
        "eth_call",
        json!([
            {"to": format!("{token_address:#x}"), "data": call_data},
            block_tag
        ]),
    )
    .await?;
    parse_u256_hex(&result_hex)
}

pub fn signer_from_secret(secret: &str) -> Result<PrivateKeySigner, PluginError> {
    secret
        .parse::<PrivateKeySigner>()
        .map_err(|error| PluginError::Signing(error.to_string()))
}

pub fn wallet_from_secret(secret: &str) -> Result<EthereumWallet, PluginError> {
    Ok(EthereumWallet::from(signer_from_secret(secret)?))
}

pub fn signer_address(secret: &str) -> Result<Address, PluginError> {
    Ok(signer_from_secret(secret)?.address())
}

pub fn provider_with_wallet(endpoint: &str, secret: &str) -> Result<impl Provider, PluginError> {
    let wallet = wallet_from_secret(secret)?;
    let url = endpoint
        .parse()
        .map_err(|error| PluginError::InvalidInput(format!("invalid endpoint: {error}")))?;
    Ok(ProviderBuilder::new().wallet(wallet).connect_http(url))
}

pub fn parse_address(value: &str, field: &str) -> Result<Address, PluginError> {
    value
        .parse::<Address>()
        .map_err(|error| PluginError::InvalidInput(format!("invalid {field}: {error}")))
}

pub fn parse_u256_dec(value: &str, field: &str) -> Result<U256, PluginError> {
    U256::from_str_radix(value, 10)
        .map_err(|error| PluginError::InvalidInput(format!("invalid {field}: {error}")))
}

pub fn parse_u256_hex(value: &str) -> Result<U256, PluginError> {
    let raw = value.strip_prefix("0x").unwrap_or(value);
    U256::from_str_radix(raw, 16)
        .map_err(|error| PluginError::Rpc(format!("invalid hex quantity {value}: {error}")))
}

pub fn transaction_request_from_value(value: &Value) -> Result<TransactionRequest, PluginError> {
    serde_json::from_value(value.clone()).map_err(PluginError::from)
}

pub fn bytes_from_hex(value: &str, field: &str) -> Result<Bytes, PluginError> {
    let raw = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(raw)
        .map_err(|error| PluginError::InvalidInput(format!("invalid {field}: {error}")))?;
    Ok(Bytes::from(bytes))
}
