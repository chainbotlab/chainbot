use std::collections::BTreeMap;

use reqwest::Client;
use serde_json::{json, Map, Value};

use crate::contract::{metadata, PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::{provider, rpc};

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    validate_request(&request)?;
    let client = Client::new();
    match request.operation.as_str() {
        "sanctum_get_lsts" => get_lsts(&client, &request).await,
        "sanctum_get_lst" => get_lst(&client, &request).await,
        "sanctum_create_swap_order" => create_swap_order(&client, &request).await,
        "sanctum_execute_swap_order" => execute_swap_order(&client, &request).await,
        "sanctum_send_swap_transaction" => send_swap_transaction(&client, &request).await,
        "sanctum_get_signature_status" => get_signature_status(&client, &request).await,
        other => Err(PluginError::Unsupported(format!(
            "operation {other} is not supported"
        ))),
    }
}

fn validate_request(request: &PluginRequest) -> Result<(), PluginError> {
    if request.contract_version.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from(
            "contract_version must not be empty",
        )));
    }
    if request.contract_version != "1.0.0" {
        return Err(PluginError::InvalidInput(format!(
            "unsupported contract_version {}",
            request.contract_version
        )));
    }
    if request.plugin_id != "sanctum-node" {
        return Err(PluginError::InvalidInput(format!(
            "plugin_id must be sanctum-node, got {}",
            request.plugin_id
        )));
    }
    if request.node_id.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from(
            "node_id must not be empty",
        )));
    }
    Ok(())
}

async fn get_lsts(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let url = provider::swap_base_url(request)?.join("lsts").map_err(|error| {
        PluginError::InvalidInput(format!("invalid Sanctum lsts endpoint: {error}"))
    })?;
    let payload = provider::get_json(client, url, request.activation_secret("api_key")).await?;
    let output = BTreeMap::from([
        (String::from("lsts"), payload),
        (String::from("metadata"), metadata("sanctum")),
    ]);
    Ok(PluginResponse::success(output, None))
}

async fn get_lst(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let mint_or_symbol = required_string(request, "mint_or_symbol")?;
    let path = format!("lsts/{mint_or_symbol}");
    let url = provider::swap_base_url(request)?.join(&path).map_err(|error| {
        PluginError::InvalidInput(format!("invalid Sanctum lst endpoint: {error}"))
    })?;
    let payload = provider::get_json(client, url, request.activation_secret("api_key")).await?;
    let output = BTreeMap::from([
        (String::from("lst"), payload),
        (String::from("metadata"), metadata("sanctum")),
    ]);
    Ok(PluginResponse::success(output, None))
}

async fn create_swap_order(
    client: &Client,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let input_mint = required_string(request, "input_mint")?;
    let output_mint = required_string(request, "output_mint")?;
    let amount = required_string(request, "amount")?;
    let mode = request.input_string("mode").unwrap_or("ExactIn");
    if mode != "ExactIn" && mode != "ExactOut" {
        return Err(PluginError::InvalidInput(format!(
            "mode must be ExactIn or ExactOut, got {mode}"
        )));
    }
    let signer = required_string(request, "signer")?;
    let mut url = provider::swap_base_url(request)?
        .join("swap/token/order")
        .map_err(|error| {
            PluginError::InvalidInput(format!("invalid Sanctum order endpoint: {error}"))
        })?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("inp", input_mint);
        query.append_pair("out", output_mint);
        query.append_pair("amt", amount);
        query.append_pair("mode", mode);
        query.append_pair("signer", signer);
        if let Some(slippage_bps) = request.input.get("slippage_bps").and_then(value_as_query_string) {
            query.append_pair("slippageBps", &slippage_bps);
        }
    }

    let order = provider::get_json(client, url, request.activation_secret("api_key")).await?;
    let output = order_output(order)?;
    Ok(PluginResponse::success(output, Some("prepared")))
}

async fn execute_swap_order(
    client: &Client,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let signed_transaction = required_string(request, "signed_transaction")?;
    let order_response = request
        .input
        .get("order_response")
        .cloned()
        .ok_or_else(|| PluginError::InvalidInput(String::from("order_response is required")))?;
    let mut body = Map::new();
    body.insert(
        String::from("signedTx"),
        Value::String(signed_transaction.to_owned()),
    );
    body.insert(String::from("orderResponse"), order_response);
    let url = provider::swap_base_url(request)?
        .join("swap/token/execute")
        .map_err(|error| {
            PluginError::InvalidInput(format!("invalid Sanctum execute endpoint: {error}"))
        })?;
    let response = provider::post_json(
        client,
        url,
        Value::Object(body),
        request.activation_secret("api_key"),
    )
    .await?;
    let tx_signature = response
        .get("txSignature")
        .or_else(|| response.get("signature"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            PluginError::Api(String::from(
                "execute response did not include txSignature",
            ))
        })?;
    let output = BTreeMap::from([
        (String::from("status"), Value::String(String::from("submitted"))),
        (
            String::from("transaction_id"),
            Value::String(tx_signature.to_owned()),
        ),
        (String::from("execute_response"), response),
        (String::from("metadata"), metadata("sanctum")),
    ]);
    Ok(PluginResponse::success(output, Some("submitted")))
}

async fn send_swap_transaction(
    client: &Client,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let endpoint = required_string(request, "endpoint")?;
    let signed_transaction = required_string(request, "signed_transaction")?;
    let encoding = request.input_string("encoding").unwrap_or("base64");
    if encoding != "base64" && encoding != "base58" {
        return Err(PluginError::InvalidInput(format!(
            "encoding must be base64 or base58, got {encoding}"
        )));
    }
    let confirmation_mode = request.input_string("confirmation_mode").unwrap_or("confirmed");
    let preflight = request.input_bool("preflight").unwrap_or(true);
    let signature = rpc::send_transaction(
        client,
        endpoint,
        signed_transaction,
        encoding,
        confirmation_mode,
        preflight,
    )
    .await?;
    if confirmation_mode == "submit_only" {
        return Ok(PluginResponse::success(
            write_output("submitted", signature, confirmation_mode),
            Some("submitted"),
        ));
    }

    let _status = rpc::wait_for_signature_status(client, endpoint, &signature).await?;
    Ok(PluginResponse::success(
        write_output("settled", signature, confirmation_mode),
        Some("settled"),
    ))
}

async fn get_signature_status(
    client: &Client,
    request: &PluginRequest,
) -> Result<PluginResponse, PluginError> {
    let endpoint = required_string(request, "endpoint")?;
    let signature = required_string(request, "signature")?;
    let status = rpc::signature_status(client, endpoint, signature).await?;
    let output = BTreeMap::from([
        (String::from("signature"), Value::String(signature.to_owned())),
        (
            String::from("status"),
            serde_json::to_value(status).map_err(PluginError::from)?,
        ),
        (String::from("metadata"), metadata("sanctum")),
    ]);
    Ok(PluginResponse::success(output, None))
}

fn order_output(order: Value) -> Result<BTreeMap<String, Value>, PluginError> {
    let tx = order
        .get("tx")
        .and_then(Value::as_str)
        .ok_or_else(|| PluginError::Api(String::from("order response did not include tx")))?;
    let mut output = BTreeMap::from([
        (String::from("status"), Value::String(String::from("prepared"))),
        (String::from("order_response"), order.clone()),
        (
            String::from("swap_transaction"),
            Value::String(tx.to_owned()),
        ),
        (String::from("metadata"), metadata("sanctum")),
    ]);
    copy_field(&mut output, &order, "inpAmt", "input_amount");
    copy_field(&mut output, &order, "outAmt", "output_amount");
    copy_field(&mut output, &order, "source", "source");
    copy_field(&mut output, &order, "feeAmt", "fee_amount");
    copy_field(&mut output, &order, "feeMint", "fee_mint");
    Ok(output)
}

fn required_string<'a>(request: &'a PluginRequest, key: &str) -> Result<&'a str, PluginError> {
    request
        .input_string(key)
        .ok_or_else(|| PluginError::InvalidInput(format!("{key} is required")))
}

fn value_as_query_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn write_output(status: &str, transaction_id: String, confirmation_mode: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (String::from("status"), Value::String(status.to_owned())),
        (
            String::from("transaction_id"),
            Value::String(transaction_id),
        ),
        (
            String::from("metadata"),
            json!({"provider": "sanctum", "confirmation_mode": confirmation_mode}),
        ),
    ])
}

fn copy_field(output: &mut BTreeMap<String, Value>, source: &Value, source_key: &str, output_key: &str) {
    if let Some(value) = source.get(source_key) {
        output.insert(output_key.to_owned(), value.clone());
    }
}
