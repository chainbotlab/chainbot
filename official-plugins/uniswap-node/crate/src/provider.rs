use alloy::network::EthereumWallet;
use alloy::primitives::{Address, Bytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::errors::PluginError;

pub const GET_AMOUNTS_OUT_SELECTOR: [u8; 4] = [0xd0, 0x6d, 0xe0, 0x4f];
pub const SWAP_EXACT_TOKENS_FOR_TOKENS_SELECTOR: [u8; 4] = [0x38, 0xed, 0x17, 0x39];

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

pub async fn eth_call(
    client: &Client,
    endpoint: &str,
    to: Address,
    data: Bytes,
    block_tag: &str,
) -> Result<Bytes, PluginError> {
    let result: String = rpc_call(
        client,
        endpoint,
        "eth_call",
        json!([
            {"to": format!("{to:#x}"), "data": format!("0x{}", hex::encode(data.as_ref()))},
            block_tag
        ]),
    )
    .await?;
    bytes_from_hex(&result, "eth_call result")
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

pub fn bytes_from_hex(value: &str, field: &str) -> Result<Bytes, PluginError> {
    let raw = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(raw)
        .map_err(|error| PluginError::InvalidInput(format!("invalid {field}: {error}")))?;
    Ok(Bytes::from(bytes))
}

pub fn parse_path(value: &Value) -> Result<Vec<Address>, PluginError> {
    let path = value
        .as_array()
        .ok_or_else(|| PluginError::InvalidInput(String::from("path must be an array")))?;
    if path.len() < 2 {
        return Err(PluginError::InvalidInput(String::from(
            "path must contain at least two token addresses",
        )));
    }
    path.iter()
        .enumerate()
        .map(|(index, value)| {
            let raw = value.as_str().ok_or_else(|| {
                PluginError::InvalidInput(format!("path[{index}] must be an address string"))
            })?;
            parse_address(raw, &format!("path[{index}]"))
        })
        .collect()
}

pub fn encode_get_amounts_out(amount_in: U256, path: &[Address]) -> Bytes {
    let mut data = Vec::new();
    data.extend_from_slice(&GET_AMOUNTS_OUT_SELECTOR);
    encode_u256(&mut data, amount_in);
    encode_u256(&mut data, U256::from(64));
    encode_address_array_tail(&mut data, path);
    Bytes::from(data)
}

pub fn encode_swap_exact_tokens_for_tokens(
    amount_in: U256,
    amount_out_min: U256,
    path: &[Address],
    recipient: Address,
    deadline: U256,
) -> Bytes {
    let mut data = Vec::new();
    data.extend_from_slice(&SWAP_EXACT_TOKENS_FOR_TOKENS_SELECTOR);
    encode_u256(&mut data, amount_in);
    encode_u256(&mut data, amount_out_min);
    encode_u256(&mut data, U256::from(160));
    encode_address(&mut data, recipient);
    encode_u256(&mut data, deadline);
    encode_address_array_tail(&mut data, path);
    Bytes::from(data)
}

pub fn decode_u256_array(data: &Bytes) -> Result<Vec<U256>, PluginError> {
    let bytes = data.as_ref();
    if bytes.len() < 64 {
        return Err(PluginError::Rpc(String::from(
            "encoded uint256[] result is too short",
        )));
    }
    let offset = decode_word(&bytes[0..32])?;
    if offset != U256::from(32) {
        return Err(PluginError::Rpc(format!(
            "unexpected uint256[] offset {offset}"
        )));
    }
    let len = decode_word(&bytes[32..64])?
        .try_into()
        .map_err(|_| PluginError::Rpc(String::from("uint256[] length does not fit usize")))?;
    let expected_len = 64 + len * 32;
    if bytes.len() < expected_len {
        return Err(PluginError::Rpc(String::from(
            "encoded uint256[] result is truncated",
        )));
    }
    let mut values = Vec::with_capacity(len);
    for index in 0..len {
        let start = 64 + index * 32;
        values.push(decode_word(&bytes[start..start + 32])?);
    }
    Ok(values)
}

fn encode_address_array_tail(data: &mut Vec<u8>, path: &[Address]) {
    encode_u256(data, U256::from(path.len()));
    for address in path {
        encode_address(data, *address);
    }
}

fn encode_address(data: &mut Vec<u8>, address: Address) {
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(address.as_slice());
}

fn encode_u256(data: &mut Vec<u8>, value: U256) {
    data.extend_from_slice(&value.to_be_bytes::<32>());
}

fn decode_word(word: &[u8]) -> Result<U256, PluginError> {
    if word.len() != 32 {
        return Err(PluginError::Rpc(String::from("abi word must be 32 bytes")));
    }
    Ok(U256::from_be_slice(word))
}
