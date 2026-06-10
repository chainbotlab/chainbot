use std::collections::BTreeMap;

use reqwest::Client;
use serde_json::{json, Value};

use crate::contract::{PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::provider::{output_map, post_info, RequestContext};

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    validate_request(&request)?;
    let client = Client::new();
    match request.operation.as_str() {
        "hyperliquid_get_all_mids" => get_all_mids(&client, &request).await,
        "hyperliquid_get_l2_book" => get_l2_book(&client, &request).await,
        "hyperliquid_get_candle_snapshot" => get_candle_snapshot(&client, &request).await,
        "hyperliquid_bridge2_prepare_deposit" => bridge2_prepare_deposit(&request),
        "hyperliquid_bridge2_prepare_withdraw3" => bridge2_prepare_withdraw3(&request),
        "hyperliquid_bridge2_prepare_deposit_with_permit" => bridge2_prepare_deposit_with_permit(&request),
        other => Err(PluginError::Unsupported(format!("operation {other} is not supported"))),
    }
}

fn validate_request(request: &PluginRequest) -> Result<(), PluginError> {
    if request.contract_version.is_empty() {
        return Err(PluginError::InvalidInput(String::from("contract_version is required")));
    }
    if request.contract_version != "1.0.0" {
        return Err(PluginError::InvalidInput(format!(
            "unsupported contract_version {}",
            request.contract_version
        )));
    }
    if request.plugin_id.is_empty() {
        return Err(PluginError::InvalidInput(String::from("plugin_id is required")));
    }
    if request.plugin_id != "hyperliquid-node" {
        return Err(PluginError::InvalidInput(format!("plugin_id must be hyperliquid-node, got {}", request.plugin_id)));
    }
    if request.node_id.is_empty() {
        return Err(PluginError::InvalidInput(String::from("node_id is required")));
    }
    Ok(())
}

fn request_context(request: &PluginRequest) -> Result<RequestContext, PluginError> {
    RequestContext::new(request.input_string("base_url"), request.allowed_origins())
}

async fn get_all_mids(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let mut body = json!({ "type": "allMids" });
    if let Some(dex) = request.input_string("dex") {
        body["dex"] = json!(dex);
    }
    let response = post_info(client, &context, body).await?;
    Ok(PluginResponse::success(output_map("all_mids", response), None))
}

async fn get_l2_book(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let coin = request
        .input_string("coin")
        .ok_or_else(|| PluginError::InvalidInput(String::from("input coin is required")))?;
    let mut body = json!({
        "type": "l2Book",
        "coin": coin,
    });
    if let Some(n_sig_figs) = request.input_i64("nSigFigs") {
        body["nSigFigs"] = json!(n_sig_figs);
    }
    if let Some(mantissa) = request.input_i64("mantissa") {
        body["mantissa"] = json!(mantissa);
    }
    let response = post_info(client, &context, body).await?;
    Ok(PluginResponse::success(output_map("l2_book", response), None))
}

async fn get_candle_snapshot(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let coin = request
        .input_string("coin")
        .ok_or_else(|| PluginError::InvalidInput(String::from("input coin is required")))?;
    let interval = request
        .input_string("interval")
        .ok_or_else(|| PluginError::InvalidInput(String::from("input interval is required")))?;
    let start_time = request
        .input_i64("startTime")
        .ok_or_else(|| PluginError::InvalidInput(String::from("input startTime is required")))?;
    let end_time = request
        .input_i64("endTime")
        .ok_or_else(|| PluginError::InvalidInput(String::from("input endTime is required")))?;
    let body = json!({
        "type": "candleSnapshot",
        "req": {
            "coin": coin,
            "interval": interval,
            "startTime": start_time,
            "endTime": end_time,
        }
    });
    let response = post_info(client, &context, body).await?;
    Ok(PluginResponse::success(output_map("candles", response), None))
}

fn bridge2_prepare_deposit(request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let chain_id = bridge2_chain_id(request)?;
    let bridge = bridge2_address(request)?;
    let amount = required_string(request, "amount")?;
    let amount_units = bridge2_amount_units(request, amount)?;
    let usdc = request
        .input_string("usdc")
        .unwrap_or_else(|| default_usdc_address(chain_id));
    let call_data = erc20_transfer_call_data(bridge, &amount_units)?;
    let output = BTreeMap::from([
        (
            String::from("unsigned_action"),
            json!({
                "provider": "hyperliquid",
                "operation": "hyperliquid_bridge2_prepare_deposit",
                "action_kind": "bridge2_usdc_transfer",
                "chain_id": chain_id,
                "to": usdc,
                "data": call_data,
                "value": "0",
                "calldata_format": "erc20_transfer(address,uint256)",
                "parameters": {
                    "bridge": bridge,
                    "usdc": usdc,
                    "amount": amount,
                    "amount_units": amount_units,
                    "minimum_amount": "5 USDC"
                }
            }),
        ),
        (
            String::from("metadata"),
            json!({"provider": "hyperliquid", "bridge": "bridge2"}),
        ),
    ]);
    Ok(PluginResponse::success(output, Some("prepared")))
}

fn bridge2_prepare_withdraw3(request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let destination = required_string(request, "destination")?;
    let amount = required_string(request, "amount")?;
    let time = request
        .input_i64("time")
        .ok_or_else(|| PluginError::InvalidInput(String::from("input time is required")))?;
    let signature_chain_id = request.input_string("signatureChainId").unwrap_or("0xa4b1");
    let hyperliquid_chain = request.input_string("hyperliquidChain").unwrap_or("Mainnet");
    let action = json!({
        "type": "withdraw3",
        "signatureChainId": signature_chain_id,
        "hyperliquidChain": hyperliquid_chain,
        "destination": destination,
        "amount": amount,
        "time": time
    });
    let output = BTreeMap::from([
        (
            String::from("typed_data"),
            json!({
                "domain": {"chainId": signature_chain_id},
                "primaryType": "Withdraw",
                "message": action,
                "nonce": time
            }),
        ),
        (
            String::from("hyperliquid_action"),
            json!({
                "action": action,
                "nonce": time
            }),
        ),
        (
            String::from("metadata"),
            json!({"provider": "hyperliquid", "bridge": "bridge2"}),
        ),
    ]);
    Ok(PluginResponse::success(output, Some("prepared")))
}

fn bridge2_prepare_deposit_with_permit(request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let chain_id = bridge2_chain_id(request)?;
    let bridge = validate_eth_address(bridge2_address(request)?, "bridge")?;
    let owner = validate_eth_address(required_string(request, "owner")?, "owner")?;
    let value = required_string(request, "value")?;
    let nonce = required_string(request, "nonce")?;
    let deadline = required_string(request, "deadline")?;
    let usdc = validate_eth_address(
        request
            .input_string("usdc")
            .unwrap_or_else(|| default_usdc_address(chain_id)),
        "usdc",
    )?;
    let call_data = optional_hex_data(request, "call_data", "0x")?;
    let output = BTreeMap::from([
        (
            String::from("permit_typed_data"),
            json!({
                "domain": {
                    "name": if chain_id == 42161 { "USD Coin" } else { "USDC2" },
                    "version": if chain_id == 42161 { "2" } else { "1" },
                    "chainId": chain_id,
                    "verifyingContract": usdc
                },
                "types": {
                    "Permit": [
                        {"name": "owner", "type": "address"},
                        {"name": "spender", "type": "address"},
                        {"name": "value", "type": "uint256"},
                        {"name": "nonce", "type": "uint256"},
                        {"name": "deadline", "type": "uint256"}
                    ]
                },
                "primaryType": "Permit",
                "message": {
                    "owner": owner,
                    "spender": bridge,
                    "value": value,
                    "nonce": nonce,
                    "deadline": deadline
                }
            }),
        ),
        (
            String::from("unsigned_action"),
            json!({
                "provider": "hyperliquid",
                "operation": "hyperliquid_bridge2_prepare_deposit_with_permit",
                "action_kind": "bridge2_batched_deposit_with_permit",
                "chain_id": chain_id,
                "to": bridge,
                "data": call_data,
                "value": "0",
                "calldata_format": "caller_supplied",
                "parameters": {
                    "owner": owner,
                    "spender": bridge,
                    "value": value,
                    "nonce": nonce,
                    "deadline": deadline
                }
            }),
        ),
        (
            String::from("metadata"),
            json!({"provider": "hyperliquid", "bridge": "bridge2"}),
        ),
    ]);
    Ok(PluginResponse::success(output, Some("prepared")))
}

fn required_string<'a>(request: &'a PluginRequest, key: &str) -> Result<&'a str, PluginError> {
    request
        .input_string(key)
        .ok_or_else(|| PluginError::InvalidInput(format!("input {key} is required")))
}

fn bridge2_chain_id(request: &PluginRequest) -> Result<i64, PluginError> {
    let chain_id = optional_i64_strict(request, "chain_id")?.unwrap_or(42161);
    if chain_id != 42161 && chain_id != 421614 {
        return Err(PluginError::InvalidInput(format!(
            "chain_id must be 42161 or 421614, got {chain_id}"
        )));
    }
    Ok(chain_id)
}

fn bridge2_address(request: &PluginRequest) -> Result<&str, PluginError> {
    if let Some(bridge) = request.input_string("bridge") {
        return Ok(bridge);
    }
    Ok(match bridge2_chain_id(request)? {
        42161 => "0x2df1c51e09aECF9cacB7bc98cB1742757f163dF7",
        421614 => "0x08cfc1B6b2dCF36A1480b99353A354AA8AC56f89",
        _ => unreachable!("bridge2_chain_id validates supported chains"),
    })
}

fn default_usdc_address(chain_id: i64) -> &'static str {
    match chain_id {
        42161 => "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
        421614 => "0x1baAbB04529D43a73232B713C0FE471f7c7334d5",
        _ => "0x0000000000000000000000000000000000000000",
    }
}

fn bridge2_amount_units(request: &PluginRequest, amount: &str) -> Result<String, PluginError> {
    if let Some(amount_units) = request.input_string("amount_units") {
        return normalize_uint_string(amount_units);
    }
    let decimals = optional_i64_strict(request, "decimals")?.unwrap_or(6);
    decimal_to_units(amount, decimals)
}

fn optional_i64_strict(request: &PluginRequest, key: &str) -> Result<Option<i64>, PluginError> {
    match request.input.get(key) {
        None => Ok(None),
        Some(value) => value.as_i64().map(Some).ok_or_else(|| {
            PluginError::InvalidInput(format!(
                "input {key} must be an integer, got {} {value}",
                value_type_name(value)
            ))
        }),
    }
}

fn optional_hex_data<'a>(
    request: &'a PluginRequest,
    key: &str,
    default: &'a str,
) -> Result<&'a str, PluginError> {
    let value = match request.input.get(key) {
        None => default,
        Some(Value::String(value)) => value.as_str(),
        Some(value) => {
            return Err(PluginError::InvalidInput(format!(
                "input {key} must be a 0x-prefixed hex string, got {} {value}",
                value_type_name(value)
            )));
        }
    };
    validate_hex_data(value, key)?;
    Ok(value)
}

fn validate_hex_data(value: &str, key: &str) -> Result<(), PluginError> {
    let raw = value.strip_prefix("0x").ok_or_else(|| {
        PluginError::InvalidInput(format!("input {key} must be a 0x-prefixed hex string"))
    })?;
    if raw.len() % 2 != 0 || !raw.chars().all(|char| char.is_ascii_hexdigit()) {
        return Err(PluginError::InvalidInput(format!(
            "input {key} must contain valid hex bytes"
        )));
    }
    Ok(())
}

fn validate_eth_address<'a>(address: &'a str, key: &str) -> Result<&'a str, PluginError> {
    let raw = address.strip_prefix("0x").ok_or_else(|| {
        PluginError::InvalidInput(format!("input {key} must be a 0x-prefixed 20-byte hex address"))
    })?;
    if raw.len() != 40 || !raw.chars().all(|char| char.is_ascii_hexdigit()) {
        return Err(PluginError::InvalidInput(format!(
            "input {key} must be a 0x-prefixed 20-byte hex address"
        )));
    }
    Ok(address)
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn normalize_uint_string(value: &str) -> Result<String, PluginError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|char| char.is_ascii_digit()) {
        return Err(PluginError::InvalidInput(String::from(
            "amount_units must be a non-negative integer string",
        )));
    }
    Ok(trimmed.trim_start_matches('0').to_owned().if_empty_then_zero())
}

fn decimal_to_units(value: &str, decimals: i64) -> Result<String, PluginError> {
    if !(0..=38).contains(&decimals) {
        return Err(PluginError::InvalidInput(String::from(
            "decimals must be between 0 and 38",
        )));
    }
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') {
        return Err(PluginError::InvalidInput(String::from(
            "amount must be a non-negative decimal string",
        )));
    }
    let mut parts = trimmed.split('.');
    let whole = parts.next().unwrap_or_default();
    let fractional = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.chars().all(|char| char.is_ascii_digit())
        || !fractional.chars().all(|char| char.is_ascii_digit())
        || fractional.len() > decimals as usize
    {
        return Err(PluginError::InvalidInput(format!(
            "amount must be a decimal string with at most {decimals} fractional digits"
        )));
    }
    let mut units = String::from(whole.trim_start_matches('0'));
    units.push_str(fractional);
    for _ in fractional.len()..decimals as usize {
        units.push('0');
    }
    Ok(units.trim_start_matches('0').to_owned().if_empty_then_zero())
}

fn erc20_transfer_call_data(to: &str, amount_units: &str) -> Result<String, PluginError> {
    let address = encode_address_word(to)?;
    let amount = encode_uint_word(amount_units)?;
    Ok(format!("0xa9059cbb{address}{amount}"))
}

fn encode_address_word(address: &str) -> Result<String, PluginError> {
    let raw = address.strip_prefix("0x").unwrap_or(address);
    if raw.len() != 40 || !raw.chars().all(|char| char.is_ascii_hexdigit()) {
        return Err(PluginError::InvalidInput(String::from(
            "bridge address must be a 20-byte hex address",
        )));
    }
    Ok(format!("{raw:0>64}").to_ascii_lowercase())
}

fn encode_uint_word(value: &str) -> Result<String, PluginError> {
    let mut digits = normalize_uint_string(value)?;
    if digits == "0" {
        return Ok(format!("{:0>64}", "0"));
    }
    let mut hex_digits = String::new();
    while digits != "0" {
        let (quotient, remainder) = div_mod_decimal_string(&digits, 16)?;
        hex_digits.push(std::char::from_digit(remainder, 16).expect("remainder fits hex digit"));
        digits = quotient;
    }
    let hex = hex_digits.chars().rev().collect::<String>();
    if hex.len() > 64 {
        return Err(PluginError::InvalidInput(String::from(
            "amount_units does not fit uint256",
        )));
    }
    Ok(format!("{hex:0>64}"))
}

fn div_mod_decimal_string(value: &str, divisor: u32) -> Result<(String, u32), PluginError> {
    let mut quotient = String::new();
    let mut remainder = 0u32;
    for char in value.chars() {
        let digit = char.to_digit(10).ok_or_else(|| {
            PluginError::InvalidInput(String::from("amount_units must be a decimal integer"))
        })?;
        let current = remainder * 10 + digit;
        let quotient_digit = current / divisor;
        remainder = current % divisor;
        if !quotient.is_empty() || quotient_digit != 0 {
            quotient.push(std::char::from_digit(quotient_digit, 10).expect("quotient digit"));
        }
    }
    if quotient.is_empty() {
        quotient.push('0');
    }
    Ok((quotient, remainder))
}

trait EmptyStringExt {
    fn if_empty_then_zero(self) -> String;
}

impl EmptyStringExt for String {
    fn if_empty_then_zero(self) -> String {
        if self.is_empty() {
            String::from("0")
        } else {
            self
        }
    }
}
