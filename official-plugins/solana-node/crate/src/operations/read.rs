use std::collections::BTreeMap;

use reqwest::Client;
use serde_json::{json, Value};

use crate::contract::{PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::operations::success;
use crate::provider::{
    get_balance as rpc_get_balance, get_token_balance as rpc_get_token_balance, raw_read as rpc_raw_read,
};

pub async fn get_balance(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let endpoint = request
        .input_string("endpoint")
        .ok_or_else(|| PluginError::InvalidInput(String::from("endpoint is required")))?;
    let address = request
        .input_string("address")
        .ok_or_else(|| PluginError::InvalidInput(String::from("address is required")))?;
    let commitment = request.input_string("commitment").unwrap_or("confirmed");
    let balance = rpc_get_balance(client, endpoint, address, commitment).await?;
    Ok(success(
        BTreeMap::from([(String::from("balance"), Value::String(balance.to_string()))]),
        None,
    ))
}

pub async fn get_token_balance(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let endpoint = request
        .input_string("endpoint")
        .ok_or_else(|| PluginError::InvalidInput(String::from("endpoint is required")))?;
    let token_account = request
        .input_string("token_account")
        .ok_or_else(|| PluginError::InvalidInput(String::from("token_account is required")))?;
    let commitment = request.input_string("commitment").unwrap_or("confirmed");
    let balance = rpc_get_token_balance(client, endpoint, token_account, commitment).await?;
    Ok(success(
        BTreeMap::from([(String::from("balance"), Value::String(balance))]),
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
