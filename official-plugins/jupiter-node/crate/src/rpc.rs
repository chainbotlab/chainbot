use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::errors::PluginError;

#[derive(Debug, Deserialize)]
pub struct RpcStatuses {
    pub value: Vec<Option<RpcSignatureStatus>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RpcSignatureStatus {
    pub slot: u64,
    pub confirmations: Option<u64>,
    pub err: Option<Value>,
    #[serde(rename(deserialize = "confirmationStatus"))]
    pub confirmation_status: Option<String>,
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

pub async fn send_transaction(
    client: &Client,
    endpoint: &str,
    signed_transaction: &str,
    encoding: &str,
    confirmation_mode: &str,
    preflight: bool,
) -> Result<String, PluginError> {
    rpc_call(
        client,
        endpoint,
        "sendTransaction",
        json!([
            signed_transaction,
            {
                "encoding": encoding,
                "preflightCommitment": preflight_commitment(confirmation_mode),
                "skipPreflight": !preflight
            }
        ]),
    )
    .await
}

pub async fn signature_status(
    client: &Client,
    endpoint: &str,
    signature: &str,
) -> Result<Option<RpcSignatureStatus>, PluginError> {
    let result: RpcStatuses = rpc_call(
        client,
        endpoint,
        "getSignatureStatuses",
        json!([[signature], {"searchTransactionHistory": false}]),
    )
    .await?;
    Ok(result.value.into_iter().next().flatten())
}

pub async fn wait_for_signature_status(
    client: &Client,
    endpoint: &str,
    signature: &str,
) -> Result<RpcSignatureStatus, PluginError> {
    let status = signature_status(client, endpoint, signature).await?.ok_or_else(|| {
        PluginError::Rpc(String::from("transaction status is not yet available"))
    })?;
    if let Some(error) = &status.err {
        return Err(PluginError::Rpc(format!(
            "transaction settled with error: {error}"
        )));
    }
    Ok(status)
}

fn preflight_commitment(confirmation_mode: &str) -> &str {
    match confirmation_mode {
        "processed" => "processed",
        "finalized" => "finalized",
        _ => "confirmed",
    }
}
