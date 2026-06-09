use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::{sleep, Duration, Instant};

use crate::errors::PluginError;

const SIGNATURE_STATUS_TIMEOUT: Duration = Duration::from_secs(60);
const SIGNATURE_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(500);

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
    confirmation_mode: &str,
) -> Result<RpcSignatureStatus, PluginError> {
    let deadline = Instant::now() + SIGNATURE_STATUS_TIMEOUT;
    loop {
        if let Some(status) = signature_status(client, endpoint, signature).await? {
            if let Some(error) = &status.err {
                return Err(PluginError::Rpc(format!(
                    "transaction settled with error: {error}"
                )));
            }
            if status_reached(&status, confirmation_mode) {
                return Ok(status);
            }
        }
        if Instant::now() >= deadline {
            return Err(PluginError::Rpc(format!(
                "transaction status did not reach {confirmation_mode} before timeout"
            )));
        }
        sleep(SIGNATURE_STATUS_POLL_INTERVAL).await;
    }
}

fn preflight_commitment(confirmation_mode: &str) -> &str {
    match confirmation_mode {
        "processed" => "processed",
        "finalized" => "finalized",
        _ => "confirmed",
    }
}

fn status_reached(status: &RpcSignatureStatus, confirmation_mode: &str) -> bool {
    let Some(current) = status.confirmation_status.as_deref() else {
        return confirmation_mode == "processed";
    };
    commitment_rank(current) >= commitment_rank(confirmation_mode)
}

fn commitment_rank(commitment: &str) -> u8 {
    match commitment {
        "processed" => 0,
        "confirmed" => 1,
        "finalized" => 2,
        _ => 1,
    }
}
