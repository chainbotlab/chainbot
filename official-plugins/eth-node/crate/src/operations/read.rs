use std::collections::BTreeMap;

use reqwest::Client;
use serde_json::{json, Value};

use crate::contract::{PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::operations::success;
use crate::provider::{
    get_balance as rpc_get_balance, parse_address, raw_read as rpc_raw_read, token_balance,
};

pub async fn get_balance(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let endpoint = request
        .input_string("endpoint")
        .ok_or_else(|| PluginError::InvalidInput(String::from("endpoint is required")))?;
    let address = request
        .input_string("address")
        .ok_or_else(|| PluginError::InvalidInput(String::from("address is required")))?;
    let block_tag = request.input_string("block_tag").unwrap_or("latest");
    let balance = rpc_get_balance(client, endpoint, address, block_tag).await?;
    Ok(success(
        BTreeMap::from([(String::from("balance"), Value::String(balance.to_string()))]),
        None,
    ))
}

pub async fn get_token_balance(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let endpoint = request
        .input_string("endpoint")
        .ok_or_else(|| PluginError::InvalidInput(String::from("endpoint is required")))?;
    let token_address = parse_address(
        request
            .input_string("token_address")
            .ok_or_else(|| PluginError::InvalidInput(String::from("token_address is required")))?,
        "token_address",
    )?;
    let wallet_address = parse_address(
        request
            .input_string("wallet_address")
            .ok_or_else(|| PluginError::InvalidInput(String::from("wallet_address is required")))?,
        "wallet_address",
    )?;
    let block_tag = request.input_string("block_tag").unwrap_or("latest");
    let balance = token_balance(client, endpoint, token_address, wallet_address, block_tag).await?;
    Ok(success(
        BTreeMap::from([(String::from("balance"), Value::String(balance.to_string()))]),
        None,
    ))
}

pub async fn raw_read(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let endpoint = request
        .input_string("endpoint")
        .ok_or_else(|| PluginError::InvalidInput(String::from("endpoint is required")))?;
    let method = request
        .input_string("method")
        .ok_or_else(|| PluginError::InvalidInput(String::from("method is required")))?;
    let params = request.input.get("params").cloned().unwrap_or_else(|| json!([]));
    let result = rpc_raw_read(client, endpoint, method, params).await?;
    Ok(success(
        BTreeMap::from([
            (String::from("result"), result),
            (String::from("metadata"), json!({"method": method})),
        ]),
        None,
    ))
}
