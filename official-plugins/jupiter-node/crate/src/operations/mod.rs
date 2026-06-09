use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Map, Value};

use crate::contract::{metadata, PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::{provider, rpc};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    validate_request(&request)?;
    let client = http_client()?;
    match request.operation.as_str() {
        "jupiter_get_quote" => get_quote(&client, &request).await,
        "jupiter_build_swap" => build_swap(&client, &request).await,
        "jupiter_send_swap_transaction" => send_swap_transaction(&client, &request).await,
        "jupiter_get_signature_status" => get_signature_status(&client, &request).await,
        other => Err(PluginError::Unsupported(format!(
            "operation {other} is not supported"
        ))),
    }
}

fn http_client() -> Result<Client, PluginError> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(PluginError::from)
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
    if request.plugin_id != "jupiter-node" {
        return Err(PluginError::InvalidInput(format!(
            "plugin_id must be jupiter-node, got {}",
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

async fn get_quote(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let input_mint = required_string(request, "input_mint")?;
    let output_mint = required_string(request, "output_mint")?;
    let amount = required_string(request, "amount")?;
    let mut url = provider::swap_base_url(request)?.join("quote").map_err(|error| {
        PluginError::InvalidInput(format!("invalid Jupiter quote endpoint: {error}"))
    })?;
    let mut query_pairs = vec![
        (String::from("inputMint"), input_mint.to_owned()),
        (String::from("outputMint"), output_mint.to_owned()),
        (String::from("amount"), amount.to_owned()),
    ];
    append_optional_query(&mut query_pairs, request, "slippage_bps", "slippageBps");
    append_optional_query(&mut query_pairs, request, "swap_mode", "swapMode");
    append_optional_query(&mut query_pairs, request, "dexes", "dexes");
    append_optional_query(&mut query_pairs, request, "exclude_dexes", "excludeDexes");
    append_optional_query(
        &mut query_pairs,
        request,
        "restrict_intermediate_tokens",
        "restrictIntermediateTokens",
    );
    append_optional_query(&mut query_pairs, request, "only_direct_routes", "onlyDirectRoutes");
    append_optional_query(
        &mut query_pairs,
        request,
        "as_legacy_transaction",
        "asLegacyTransaction",
    );
    append_optional_query(&mut query_pairs, request, "platform_fee_bps", "platformFeeBps");
    append_optional_query(&mut query_pairs, request, "max_accounts", "maxAccounts");
    append_optional_query(&mut query_pairs, request, "instruction_version", "instructionVersion");
    append_optional_query(&mut query_pairs, request, "dynamic_slippage", "dynamicSlippage");
    append_optional_query(&mut query_pairs, request, "for_jito_bundle", "forJitoBundle");
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in query_pairs {
            query.append_pair(&key, &value);
        }
    }

    let api_key = provider::api_key_for_url(request, &url);
    let quote = provider::get_json(client, url, api_key).await?;
    let output = quote_output(quote);
    Ok(PluginResponse::success(output, None))
}

async fn build_swap(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let quote_response = request
        .input
        .get("quote_response")
        .cloned()
        .ok_or_else(|| PluginError::InvalidInput(String::from("quote_response is required")))?;
    let user_public_key = required_string(request, "user_public_key")?;
    let mut body = Map::new();
    body.insert(String::from("quoteResponse"), quote_response);
    body.insert(
        String::from("userPublicKey"),
        Value::String(user_public_key.to_owned()),
    );
    append_optional_body(&mut body, request, "dynamic_compute_unit_limit", "dynamicComputeUnitLimit");
    append_optional_body(&mut body, request, "dynamic_slippage", "dynamicSlippage");
    append_optional_body(
        &mut body,
        request,
        "prioritization_fee_lamports",
        "prioritizationFeeLamports",
    );
    append_optional_body(&mut body, request, "wrap_and_unwrap_sol", "wrapAndUnwrapSol");
    append_optional_body(&mut body, request, "use_shared_accounts", "useSharedAccounts");
    append_optional_body(&mut body, request, "fee_account", "feeAccount");
    append_optional_body(&mut body, request, "tracking_account", "trackingAccount");
    append_optional_body(&mut body, request, "as_legacy_transaction", "asLegacyTransaction");
    append_optional_body(
        &mut body,
        request,
        "destination_token_account",
        "destinationTokenAccount",
    );

    let url = provider::swap_base_url(request)?.join("swap").map_err(|error| {
        PluginError::InvalidInput(format!("invalid Jupiter swap endpoint: {error}"))
    })?;
    let api_key = provider::api_key_for_url(request, &url);
    let swap = provider::post_json(
        client,
        url,
        Value::Object(body),
        api_key,
    )
    .await?;
    let output = swap_output(swap)?;
    Ok(PluginResponse::success(output, Some("prepared")))
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
    let confirmation_mode = confirmation_mode(request)?;
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

    let _status = rpc::wait_for_signature_status(client, endpoint, &signature, confirmation_mode).await?;
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
        (String::from("metadata"), metadata("jupiter")),
    ]);
    Ok(PluginResponse::success(output, None))
}

fn required_string<'a>(request: &'a PluginRequest, key: &str) -> Result<&'a str, PluginError> {
    request
        .input_string(key)
        .ok_or_else(|| PluginError::InvalidInput(format!("{key} is required")))
}

fn confirmation_mode(request: &PluginRequest) -> Result<&str, PluginError> {
    let confirmation_mode = request.input_string("confirmation_mode").unwrap_or("confirmed");
    match confirmation_mode {
        "submit_only" | "processed" | "confirmed" | "finalized" => Ok(confirmation_mode),
        other => Err(PluginError::InvalidInput(format!(
            "confirmation_mode must be submit_only, processed, confirmed, or finalized, got {other}"
        ))),
    }
}

fn append_optional_query(
    query: &mut Vec<(String, String)>,
    request: &PluginRequest,
    input_key: &str,
    query_key: &str,
) {
    if let Some(value) = request.input.get(input_key).and_then(value_as_query_string) {
        query.push((query_key.to_owned(), value));
    }
}

fn append_optional_body(
    body: &mut Map<String, Value>,
    request: &PluginRequest,
    input_key: &str,
    body_key: &str,
) {
    if let Some(value) = request.input.get(input_key) {
        body.insert(body_key.to_owned(), value.clone());
    }
}

fn value_as_query_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn quote_output(quote: Value) -> BTreeMap<String, Value> {
    let mut output = BTreeMap::new();
    output.insert(String::from("quote_response"), quote.clone());
    copy_field(&mut output, &quote, "inputMint", "input_mint");
    copy_field(&mut output, &quote, "outputMint", "output_mint");
    copy_field(&mut output, &quote, "inAmount", "in_amount");
    copy_field(&mut output, &quote, "outAmount", "out_amount");
    copy_field(
        &mut output,
        &quote,
        "otherAmountThreshold",
        "other_amount_threshold",
    );
    copy_field(&mut output, &quote, "priceImpactPct", "price_impact_pct");
    copy_field(&mut output, &quote, "routePlan", "route_plan");
    output.insert(String::from("metadata"), metadata("jupiter"));
    output
}

fn swap_output(swap: Value) -> Result<BTreeMap<String, Value>, PluginError> {
    let swap_transaction = swap
        .get("swapTransaction")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            PluginError::Api(String::from(
                "swap response did not include swapTransaction",
            ))
        })?;
    let mut output = BTreeMap::from([
        (String::from("status"), Value::String(String::from("prepared"))),
        (
            String::from("swap_transaction"),
            Value::String(swap_transaction.to_owned()),
        ),
        (String::from("swap_response"), swap.clone()),
        (String::from("metadata"), metadata("jupiter")),
    ]);
    copy_field(
        &mut output,
        &swap,
        "lastValidBlockHeight",
        "last_valid_block_height",
    );
    Ok(output)
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
            json!({"provider": "jupiter", "confirmation_mode": confirmation_mode}),
        ),
    ])
}

fn copy_field(output: &mut BTreeMap<String, Value>, source: &Value, source_key: &str, output_key: &str) {
    if let Some(value) = source.get(source_key) {
        output.insert(output_key.to_owned(), value.clone());
    }
}
