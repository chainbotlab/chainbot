use alloy::primitives::{Address, Bytes, U256};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

pub const GET_AMOUNTS_OUT_SELECTOR: [u8; 4] = [0xd0, 0x6d, 0xe0, 0x4f];

pub async fn rpc_call<T: DeserializeOwned>(
    client: &Client,
    endpoint: &str,
    method: &str,
    params: Value,
) -> Result<T, String> {
    let response = client
        .post(endpoint)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let payload: Value = response.json().await.map_err(|error| error.to_string())?;
    if let Some(error) = payload.get("error") {
        return Err(format!("rpc error: {error}"));
    }
    serde_json::from_value(payload.get("result").cloned().unwrap_or(Value::Null))
        .map_err(|error| error.to_string())
}

pub async fn quote_amounts_out(
    client: &Client,
    endpoint: &str,
    router: Address,
    amount_in: U256,
    path: &[Address],
    block_tag: &str,
) -> Result<Vec<U256>, String> {
    let data = encode_get_amounts_out(amount_in, path);
    let result: String = rpc_call(
        client,
        endpoint,
        "eth_call",
        json!([
            {"to": format!("{router:#x}"), "data": format!("0x{}", hex::encode(data.as_ref()))},
            block_tag
        ]),
    )
    .await?;
    let bytes = bytes_from_hex(&result, "eth_call result")?;
    decode_u256_array(&bytes)
}

pub fn parse_address(value: &str, field: &str) -> Result<Address, String> {
    value
        .parse::<Address>()
        .map_err(|error| format!("invalid {field}: {error}"))
}

pub fn parse_u256_dec(value: &str, field: &str) -> Result<U256, String> {
    U256::from_str_radix(value, 10).map_err(|error| format!("invalid {field}: {error}"))
}

pub fn parse_path(value: &Value) -> Result<Vec<Address>, String> {
    let path = value
        .as_array()
        .ok_or_else(|| String::from("params.path must be an array"))?;
    if path.len() < 2 {
        return Err(String::from("params.path must contain at least two token addresses"));
    }
    path.iter()
        .enumerate()
        .map(|(index, value)| {
            let raw = value
                .as_str()
                .ok_or_else(|| format!("params.path[{index}] must be an address string"))?;
            parse_address(raw, &format!("params.path[{index}]"))
        })
        .collect()
}

pub fn encode_get_amounts_out(amount_in: U256, path: &[Address]) -> Bytes {
    let mut data = Vec::new();
    data.extend_from_slice(&GET_AMOUNTS_OUT_SELECTOR);
    encode_u256(&mut data, amount_in);
    encode_u256(&mut data, U256::from(64));
    encode_u256(&mut data, U256::from(path.len()));
    for address in path {
        encode_address(&mut data, *address);
    }
    Bytes::from(data)
}

pub fn decode_u256_array(data: &Bytes) -> Result<Vec<U256>, String> {
    let bytes = data.as_ref();
    if bytes.len() < 64 {
        return Err(String::from("encoded uint256[] result is too short"));
    }
    if U256::from_be_slice(&bytes[0..32]) != U256::from(32) {
        return Err(String::from("unexpected uint256[] offset"));
    }
    let len: usize = U256::from_be_slice(&bytes[32..64])
        .try_into()
        .map_err(|_| String::from("uint256[] length does not fit usize"))?;
    let expected_len = 64 + len * 32;
    if bytes.len() < expected_len {
        return Err(String::from("encoded uint256[] result is truncated"));
    }
    let mut values = Vec::with_capacity(len);
    for index in 0..len {
        let start = 64 + index * 32;
        values.push(U256::from_be_slice(&bytes[start..start + 32]));
    }
    Ok(values)
}

fn bytes_from_hex(value: &str, field: &str) -> Result<Bytes, String> {
    let raw = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(raw).map_err(|error| format!("invalid {field}: {error}"))?;
    Ok(Bytes::from(bytes))
}

fn encode_address(data: &mut Vec<u8>, address: Address) {
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(address.as_slice());
}

fn encode_u256(data: &mut Vec<u8>, value: U256) {
    data.extend_from_slice(&value.to_be_bytes::<32>());
}
