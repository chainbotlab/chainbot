use alloy::network::EthereumWallet;
use alloy::primitives::{Address, Bytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;

use crate::errors::PluginError;

pub const EXACT_INPUT_SINGLE_SELECTOR: [u8; 4] = [0x41, 0x4b, 0xf3, 0x89];
pub const EXACT_INPUT_SELECTOR: [u8; 4] = [0xc0, 0x4b, 0x8d, 0x59];

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

pub fn parse_u24_dec(value: &str, field: &str) -> Result<u32, PluginError> {
    let parsed = value
        .parse::<u32>()
        .map_err(|error| PluginError::InvalidInput(format!("invalid {field}: {error}")))?;
    if parsed > 0x00ff_ffff {
        return Err(PluginError::InvalidInput(format!("{field} exceeds uint24")));
    }
    Ok(parsed)
}

pub fn parse_u256_dec(value: &str, field: &str) -> Result<U256, PluginError> {
    U256::from_str_radix(value, 10)
        .map_err(|error| PluginError::InvalidInput(format!("invalid {field}: {error}")))
}

pub fn bytes_from_hex(value: &str, field: &str) -> Result<Bytes, PluginError> {
    let raw = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(raw)
        .map_err(|error| PluginError::InvalidInput(format!("invalid {field}: {error}")))?;
    Ok(Bytes::from(bytes))
}

pub fn encode_exact_input_single(
    token_in: Address,
    token_out: Address,
    fee: u32,
    recipient: Address,
    deadline: U256,
    amount_in: U256,
    amount_out_minimum: U256,
    sqrt_price_limit_x96: U256,
) -> Bytes {
    let mut data = Vec::new();
    data.extend_from_slice(&EXACT_INPUT_SINGLE_SELECTOR);
    encode_address(&mut data, token_in);
    encode_address(&mut data, token_out);
    encode_u256(&mut data, U256::from(fee));
    encode_address(&mut data, recipient);
    encode_u256(&mut data, deadline);
    encode_u256(&mut data, amount_in);
    encode_u256(&mut data, amount_out_minimum);
    encode_u256(&mut data, sqrt_price_limit_x96);
    Bytes::from(data)
}

pub fn encode_exact_input(
    encoded_path: &Bytes,
    recipient: Address,
    deadline: U256,
    amount_in: U256,
    amount_out_minimum: U256,
) -> Bytes {
    let mut data = Vec::new();
    data.extend_from_slice(&EXACT_INPUT_SELECTOR);
    encode_u256(&mut data, U256::from(160));
    encode_address(&mut data, recipient);
    encode_u256(&mut data, deadline);
    encode_u256(&mut data, amount_in);
    encode_u256(&mut data, amount_out_minimum);
    encode_bytes_tail(&mut data, encoded_path);
    Bytes::from(data)
}

fn encode_bytes_tail(data: &mut Vec<u8>, bytes: &Bytes) {
    encode_u256(data, U256::from(bytes.len()));
    data.extend_from_slice(bytes.as_ref());
    let padding = (32 - (bytes.len() % 32)) % 32;
    data.extend(std::iter::repeat(0u8).take(padding));
}

fn encode_address(data: &mut Vec<u8>, address: Address) {
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(address.as_slice());
}

fn encode_u256(data: &mut Vec<u8>, value: U256) {
    data.extend_from_slice(&value.to_be_bytes::<32>());
}
