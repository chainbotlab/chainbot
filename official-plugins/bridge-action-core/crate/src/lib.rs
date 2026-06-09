use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub const CONTRACT_VERSION: &str = "1.0.0";
pub const JSONRPC_VERSION: &str = "2.0";
pub const EXECUTE_METHOD: &str = "node.exec.v2";

#[derive(Debug)]
pub enum PluginError {
    Json(serde_json::Error),
    InvalidInput(String),
    Unsupported(String),
}

impl Display for PluginError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => write!(f, "json error: {error}"),
            Self::InvalidInput(message) => write!(f, "invalid input: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
        }
    }
}

impl std::error::Error for PluginError {}

impl From<serde_json::Error> for PluginError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginRequest {
    pub contract_version: String,
    pub plugin_id: String,
    pub node_id: String,
    pub operation: String,
    #[serde(default)]
    pub input: BTreeMap<String, Value>,
    #[serde(default)]
    pub activation: Option<PluginActivationEnvelope>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PluginActivationEnvelope {
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginResponse {
    pub contract_version: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_state: Option<&'static str>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RequestEnvelope {
    Legacy(PluginRequest),
    JsonRpc(JsonRpcRequest),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    String(String),
    Number(i64),
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    pub method: String,
    pub params: PluginRequest,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcSuccessResult {
    pub contract_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_state: Option<&'static str>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: JsonRpcId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<JsonRpcSuccessResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Copy)]
pub struct OperationSpec {
    pub name: &'static str,
    pub action_kind: &'static str,
    pub target_input: &'static str,
    pub chain_input: &'static str,
    pub required_inputs: &'static [&'static str],
    pub optional_inputs: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub struct PluginSpec {
    pub plugin_id: &'static str,
    pub provider: &'static str,
    pub operations: &'static [OperationSpec],
}

impl PluginResponse {
    pub fn success(output: BTreeMap<String, Value>, result_state: Option<&'static str>) -> Self {
        Self {
            contract_version: String::from(CONTRACT_VERSION),
            success: true,
            result_state,
            output,
            error: None,
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            contract_version: String::from(CONTRACT_VERSION),
            success: false,
            result_state: None,
            output: BTreeMap::new(),
            error: Some(message.into()),
        }
    }
}

impl RequestEnvelope {
    pub fn into_request(self) -> Result<(PluginRequest, Option<JsonRpcId>), String> {
        match self {
            Self::Legacy(request) => Ok((request, None)),
            Self::JsonRpc(envelope) => {
                if envelope.jsonrpc != JSONRPC_VERSION {
                    return Err(format!("jsonrpc must be {JSONRPC_VERSION}"));
                }
                if envelope.method != EXECUTE_METHOD {
                    return Err(format!("method must be {EXECUTE_METHOD}"));
                }
                Ok((envelope.params, Some(envelope.id)))
            }
        }
    }
}

impl JsonRpcResponse {
    pub fn success(id: JsonRpcId, response: PluginResponse) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: Some(JsonRpcSuccessResult {
                contract_version: CONTRACT_VERSION,
                result_state: response.result_state,
                output: response.output,
            }),
            error: None,
        }
    }

    pub fn failure(id: JsonRpcId, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32000,
                message: message.into(),
            }),
        }
    }
}

pub async fn handle_request_json(input: &str, spec: &'static PluginSpec) -> Result<String, PluginError> {
    Ok(match handle_request_json_inner(input, spec).await {
        Ok(response) => response,
        Err(error) => match extract_jsonrpc_id(input) {
            Some(id) => failure_jsonrpc_response(id, &error.to_string()),
            None => failure_response(&error.to_string()),
        },
    })
}

pub fn failure_response(message: &str) -> String {
    serde_json::to_string(&PluginResponse::failure(message))
        .unwrap_or_else(|_| String::from("{\"contract_version\":\"1.0.0\",\"success\":false,\"error\":\"internal serialization error\"}"))
}

async fn handle_request_json_inner(input: &str, spec: &'static PluginSpec) -> Result<String, PluginError> {
    let envelope: RequestEnvelope = serde_json::from_str(input)?;
    let (request, jsonrpc_id) = envelope.into_request().map_err(PluginError::InvalidInput)?;
    let response = dispatch(request, spec)?;
    match jsonrpc_id {
        Some(id) => serde_json::to_string(&JsonRpcResponse::success(id, response))
            .map_err(PluginError::from),
        None => serde_json::to_string(&response).map_err(PluginError::from),
    }
}

fn failure_jsonrpc_response(id: JsonRpcId, message: &str) -> String {
    serde_json::to_string(&JsonRpcResponse::failure(id, message))
        .unwrap_or_else(|_| failure_response(message))
}

fn extract_jsonrpc_id(input: &str) -> Option<JsonRpcId> {
    let value: Value = serde_json::from_str(input).ok()?;
    let object = value.as_object()?;
    if object.get("jsonrpc")?.as_str()? != JSONRPC_VERSION {
        return None;
    }
    serde_json::from_value(object.get("id")?.clone()).ok()
}

fn dispatch(request: PluginRequest, spec: &'static PluginSpec) -> Result<PluginResponse, PluginError> {
    validate_request(&request, spec)?;
    let operation = spec
        .operations
        .iter()
        .find(|operation| operation.name == request.operation)
        .ok_or_else(|| PluginError::Unsupported(format!("operation {} is not supported", request.operation)))?;
    for key in operation.required_inputs {
        if !request.input.contains_key(*key) {
            return Err(PluginError::InvalidInput(format!("{key} is required")));
        }
    }
    let action = unsigned_action(&request, spec, operation)?;
    Ok(PluginResponse::success(
        BTreeMap::from([
            (String::from("unsigned_action"), action),
            (String::from("metadata"), json!({"provider": spec.provider})),
        ]),
        Some("prepared"),
    ))
}

fn validate_request(request: &PluginRequest, spec: &PluginSpec) -> Result<(), PluginError> {
    if request.contract_version.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from("contract_version must not be empty")));
    }
    if request.contract_version != CONTRACT_VERSION {
        return Err(PluginError::InvalidInput(format!(
            "unsupported contract_version {}",
            request.contract_version
        )));
    }
    if request.plugin_id != spec.plugin_id {
        return Err(PluginError::InvalidInput(format!(
            "plugin_id must be {}, got {}",
            spec.plugin_id, request.plugin_id
        )));
    }
    if request.node_id.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from("node_id must not be empty")));
    }
    Ok(())
}

fn unsigned_action(
    request: &PluginRequest,
    spec: &PluginSpec,
    operation: &OperationSpec,
) -> Result<Value, PluginError> {
    let target = required_string(request, operation.target_input)?;
    let chain_id = request
        .input
        .get(operation.chain_input)
        .cloned()
        .ok_or_else(|| PluginError::InvalidInput(format!("{} is required", operation.chain_input)))?;
    let encoded = encoded_call_data(request, operation)?;
    let value = request
        .input
        .get("value")
        .cloned()
        .unwrap_or_else(|| Value::String(String::from("0")));
    let mut parameters = Map::new();
    for key in operation.required_inputs.iter().chain(operation.optional_inputs.iter()) {
        if let Some(value) = request.input.get(*key) {
            parameters.insert((*key).to_string(), value.clone());
        }
    }
    Ok(json!({
        "provider": spec.provider,
        "operation": operation.name,
        "action_kind": operation.action_kind,
        "chain_id": chain_id,
        "to": target,
        "data": encoded.data,
        "value": value,
        "calldata_format": encoded.format,
        "parameters": parameters,
    }))
}

fn required_string<'a>(request: &'a PluginRequest, key: &str) -> Result<&'a str, PluginError> {
    request
        .input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| PluginError::InvalidInput(format!("{key} is required")))
}

struct EncodedCallData {
    data: String,
    format: &'static str,
}

fn encoded_call_data(
    request: &PluginRequest,
    operation: &OperationSpec,
) -> Result<EncodedCallData, PluginError> {
    match operation.action_kind {
        "op_stack_l1_standard_bridge_deposit" => Ok(EncodedCallData {
            data: abi_encode_deposit_erc20_to(request)?,
            format: "depositERC20To(address,address,address,uint256,uint32,bytes)",
        }),
        "op_stack_l2_standard_bridge_withdraw" => Ok(EncodedCallData {
            data: abi_encode_withdraw_to(request)?,
            format: "withdrawTo(address,address,uint256,uint32,bytes)",
        }),
        "polygon_pos_deposit" => Ok(EncodedCallData {
            data: abi_encode_deposit_for(request)?,
            format: "depositFor(address,address,bytes)",
        }),
        "polygon_pos_exit" => Ok(EncodedCallData {
            data: abi_encode_exit(request)?,
            format: "exit(bytes)",
        }),
        "polygon_state_sync" => Ok(EncodedCallData {
            data: abi_encode_sync_state(request)?,
            format: "syncState(address,bytes)",
        }),
        "arbitrum_l1_gateway_deposit" => Ok(EncodedCallData {
            data: abi_encode_arbitrum_outbound_transfer_custom_refund(request)?,
            format: "outboundTransferCustomRefund(address,address,address,uint256,uint256,uint256,bytes)",
        }),
        "arbitrum_l2_gateway_withdraw" => Ok(EncodedCallData {
            data: abi_encode_arbitrum_outbound_transfer(request)?,
            format: "outboundTransfer(address,address,uint256,bytes)",
        }),
        "oft_send" => Ok(EncodedCallData {
            data: abi_encode_oft_send(request)?,
            format: "send((uint32,bytes32,uint256,uint256,bytes,bytes,bytes),(uint256,uint256),address)",
        }),
        "stargate_send_token" => Ok(EncodedCallData {
            data: abi_encode_stargate_send_token(request)?,
            format: "sendToken((uint32,bytes32,uint256,uint256,bytes,bytes,bytes),(uint256,uint256),address)",
        }),
        "oft_quote_send" => Ok(EncodedCallData {
            data: abi_encode_oft_quote_send(request)?,
            format: "quoteSend((uint32,bytes32,uint256,uint256,bytes,bytes,bytes),bool)",
        }),
        "oapp_set_peer" => Ok(EncodedCallData {
            data: abi_encode_set_peer(request)?,
            format: "setPeer(uint32,bytes32)",
        }),
        "warp_route_transfer" => Ok(EncodedCallData {
            data: abi_encode_hyperlane_transfer_remote(request)?,
            format: "transferRemote(uint32,bytes32,uint256)",
        }),
        "mailbox_dispatch" => Ok(EncodedCallData {
            data: abi_encode_hyperlane_dispatch(request)?,
            format: "dispatch(uint32,bytes32,bytes)",
        }),
        "its_interchain_transfer" => Ok(EncodedCallData {
            data: abi_encode_axelar_interchain_transfer(request)?,
            format: "interchainTransfer(bytes32,string,bytes,uint256)",
        }),
        "gmp_call_contract" => Ok(EncodedCallData {
            data: abi_encode_axelar_call_contract(request)?,
            format: "callContract(string,string,bytes)",
        }),
        "ccip_token_transfer" => Ok(EncodedCallData {
            data: abi_encode_ccip_send(request)?,
            format: "ccipSend(uint64,(bytes,bytes,(address,uint256)[],address,bytes))",
        }),
        "ntt_transfer" => Ok(EncodedCallData {
            data: abi_encode_wormhole_ntt_transfer(request)?,
            format: wormhole_ntt_transfer_format(request),
        }),
        "wormhole_message" => Ok(EncodedCallData {
            data: abi_encode_wormhole_publish_message(request)?,
            format: "publishMessage(uint32,bytes,uint8)",
        }),
        _ => Ok(EncodedCallData {
            data: request
                .input
                .get("call_data")
                .cloned()
                .unwrap_or_else(|| Value::String(String::from("0x")))
                .as_str()
                .ok_or_else(|| PluginError::InvalidInput(String::from("call_data must be a string")))?
                .to_owned(),
            format: "caller_supplied",
        }),
    }
}

fn abi_encode_deposit_erc20_to(request: &PluginRequest) -> Result<String, PluginError> {
    let extra_data = optional_hex_bytes(request, "extra_data")?;
    Ok(format!(
        "0x838b2520{}{}{}{}{}{}{}",
        encode_address_word(required_string(request, "l1_token")?)?,
        encode_address_word(required_string(request, "l2_token")?)?,
        encode_address_word(required_string(request, "to")?)?,
        encode_uint_word_from_value(required_value(request, "amount")?)?,
        encode_u32_word(required_value(request, "min_gas_limit")?)?,
        encode_uint_word("192")?,
        encode_bytes_tail(&extra_data)?,
    ))
}

fn abi_encode_withdraw_to(request: &PluginRequest) -> Result<String, PluginError> {
    let extra_data = optional_hex_bytes(request, "extra_data")?;
    Ok(format!(
        "0xa3a79548{}{}{}{}{}{}",
        encode_address_word(required_string(request, "l2_token")?)?,
        encode_address_word(required_string(request, "to")?)?,
        encode_uint_word_from_value(required_value(request, "amount")?)?,
        encode_u32_word(required_value(request, "min_gas_limit")?)?,
        encode_uint_word("160")?,
        encode_bytes_tail(&extra_data)?,
    ))
}

fn abi_encode_deposit_for(request: &PluginRequest) -> Result<String, PluginError> {
    let deposit_data = required_hex_bytes(request, "deposit_data")?;
    Ok(format!(
        "0xe3dec8fb{}{}{}{}",
        encode_address_word(required_string(request, "user")?)?,
        encode_address_word(required_string(request, "root_token")?)?,
        encode_uint_word("96")?,
        encode_bytes_tail(&deposit_data)?,
    ))
}

fn abi_encode_exit(request: &PluginRequest) -> Result<String, PluginError> {
    let exit_payload = required_hex_bytes(request, "exit_payload")?;
    Ok(format!(
        "0x3805550f{}{}",
        encode_uint_word("32")?,
        encode_bytes_tail(&exit_payload)?,
    ))
}

fn abi_encode_sync_state(request: &PluginRequest) -> Result<String, PluginError> {
    let data = required_hex_bytes(request, "data")?;
    Ok(format!(
        "0x16f19831{}{}{}",
        encode_address_word(required_string(request, "receiver")?)?,
        encode_uint_word("64")?,
        encode_bytes_tail(&data)?,
    ))
}

fn abi_encode_arbitrum_outbound_transfer_custom_refund(
    request: &PluginRequest,
) -> Result<String, PluginError> {
    let gateway_data = optional_hex_bytes(request, "gateway_data")?;
    let refund_to = request
        .input
        .get("refund_to")
        .and_then(Value::as_str)
        .unwrap_or(required_string(request, "to")?);
    Ok(format!(
        "0x4fb1a07b{}{}{}{}{}{}{}{}",
        encode_address_word(required_string(request, "l1_token")?)?,
        encode_address_word(refund_to)?,
        encode_address_word(required_string(request, "to")?)?,
        encode_uint_word_from_value(required_value(request, "amount")?)?,
        encode_uint_word_from_value(required_value(request, "max_gas")?)?,
        encode_uint_word_from_value(required_value(request, "gas_price_bid")?)?,
        encode_uint_word("224")?,
        encode_bytes_tail(&gateway_data)?,
    ))
}

fn abi_encode_arbitrum_outbound_transfer(request: &PluginRequest) -> Result<String, PluginError> {
    let gateway_data = optional_hex_bytes(request, "gateway_data")?;
    Ok(format!(
        "0x7b3a3c8b{}{}{}{}{}",
        encode_address_word(required_string(request, "l2_token")?)?,
        encode_address_word(required_string(request, "to")?)?,
        encode_uint_word_from_value(required_value(request, "amount")?)?,
        encode_uint_word("128")?,
        encode_bytes_tail(&gateway_data)?,
    ))
}

fn abi_encode_oft_send(request: &PluginRequest) -> Result<String, PluginError> {
    Ok(format!(
        "0xc7c7f5b3{}{}{}{}",
        encode_uint_word("128")?,
        encode_messaging_fee(request)?,
        encode_address_word(required_string(request, "refundAddress")?)?,
        encode_send_param_tail(request)?,
    ))
}

fn abi_encode_stargate_send_token(request: &PluginRequest) -> Result<String, PluginError> {
    Ok(format!(
        "0xcbef2aa9{}{}{}{}",
        encode_uint_word("128")?,
        encode_messaging_fee(request)?,
        encode_address_word(required_string(request, "refundAddress")?)?,
        encode_send_param_tail(request)?,
    ))
}

fn abi_encode_oft_quote_send(request: &PluginRequest) -> Result<String, PluginError> {
    Ok(format!(
        "0x3b6f743b{}{}{}",
        encode_uint_word("64")?,
        encode_bool_word(input_bool(request, "payInLzToken")?.unwrap_or(false)),
        encode_send_param_tail(request)?,
    ))
}

fn abi_encode_set_peer(request: &PluginRequest) -> Result<String, PluginError> {
    Ok(format!(
        "0x3400288b{}{}",
        encode_u32_word(required_value(request, "eid")?)?,
        encode_bytes32_word(required_string(request, "peer")?, "peer")?,
    ))
}

fn abi_encode_hyperlane_transfer_remote(request: &PluginRequest) -> Result<String, PluginError> {
    Ok(format!(
        "0x81b4e8b4{}{}{}",
        encode_u32_word(required_value(request, "destination_domain")?)?,
        encode_recipient_bytes32(required_string(request, "recipient")?)?,
        encode_uint_word_from_value(required_value(request, "amount")?)?,
    ))
}

fn abi_encode_hyperlane_dispatch(request: &PluginRequest) -> Result<String, PluginError> {
    let message_body = required_hex_bytes(request, "messageBody")?;
    Ok(format!(
        "0xfa31de01{}{}{}{}",
        encode_u32_word(required_value(request, "destination_domain")?)?,
        encode_recipient_bytes32(required_string(request, "recipient")?)?,
        encode_uint_word("96")?,
        encode_bytes_tail(&message_body)?,
    ))
}

fn abi_encode_axelar_interchain_transfer(request: &PluginRequest) -> Result<String, PluginError> {
    let destination_chain = required_string(request, "destinationChain")?;
    let destination_address = required_hex_bytes(request, "destinationAddress")?;
    let destination_chain_offset = 128usize;
    let destination_address_offset = destination_chain_offset + abi_string_tail_len(destination_chain);
    Ok(format!(
        "0xe24a240b{}{}{}{}{}{}",
        encode_bytes32_word(required_string(request, "tokenId")?, "tokenId")?,
        encode_uint_word(&destination_chain_offset.to_string())?,
        encode_uint_word(&destination_address_offset.to_string())?,
        encode_uint_word_from_value(required_value(request, "amount")?)?,
        encode_string_tail(destination_chain)?,
        encode_bytes_tail(&destination_address)?,
    ))
}

fn abi_encode_axelar_call_contract(request: &PluginRequest) -> Result<String, PluginError> {
    let destination_chain = required_string(request, "destinationChain")?;
    let destination_contract_address = required_string(request, "destinationContractAddress")?;
    let payload = required_hex_bytes(request, "payload")?;
    let destination_chain_offset = 96usize;
    let destination_contract_offset = destination_chain_offset + abi_string_tail_len(destination_chain);
    let payload_offset = destination_contract_offset + abi_string_tail_len(destination_contract_address);
    Ok(format!(
        "0x1c92115f{}{}{}{}{}{}",
        encode_uint_word(&destination_chain_offset.to_string())?,
        encode_uint_word(&destination_contract_offset.to_string())?,
        encode_uint_word(&payload_offset.to_string())?,
        encode_string_tail(destination_chain)?,
        encode_string_tail(destination_contract_address)?,
        encode_bytes_tail(&payload)?,
    ))
}

fn abi_encode_ccip_send(request: &PluginRequest) -> Result<String, PluginError> {
    let receiver = ccip_receiver_bytes(request)?;
    let message_data = optional_hex_bytes(request, "message_data")?;
    let extra_args = optional_hex_bytes(request, "extra_args")?;
    let receiver_offset = 160usize;
    let data_offset = receiver_offset + abi_bytes_tail_len(&receiver);
    let token_amounts_offset = data_offset + abi_bytes_tail_len(&message_data);
    let extra_args_offset = token_amounts_offset + ccip_token_amounts_tail_len();
    let mut data = format!(
        "0x96f4e9f9{}{}{}{}{}{}{}{}{}{}",
        encode_u64_word(required_value(request, "destination_chain_selector")?)?,
        encode_uint_word("64")?,
        encode_uint_word(&receiver_offset.to_string())?,
        encode_uint_word(&data_offset.to_string())?,
        encode_uint_word(&token_amounts_offset.to_string())?,
        encode_fee_token_word(request)?,
        encode_uint_word(&extra_args_offset.to_string())?,
        encode_bytes_tail(&receiver)?,
        encode_bytes_tail(&message_data)?,
        encode_ccip_token_amounts_tail(request)?,
    );
    data.push_str(&encode_bytes_tail(&extra_args)?);
    Ok(data)
}

fn abi_encode_wormhole_ntt_transfer(request: &PluginRequest) -> Result<String, PluginError> {
    if uses_wormhole_ntt_advanced_transfer(request) {
        let transceiver_instructions = optional_hex_bytes_or(request, "transceiver_instructions", "00")?;
        let refund_address = request
            .input
            .get("refund_address")
            .and_then(Value::as_str)
            .unwrap_or(required_string(request, "recipient")?);
        Ok(format!(
            "0xb293f97f{}{}{}{}{}{}{}",
            encode_uint_word_from_value(required_value(request, "amount")?)?,
            encode_u16_word(required_value(request, "recipient_chain")?)?,
            encode_bytes32_word(required_string(request, "recipient")?, "recipient")?,
            encode_bytes32_word(refund_address, "refund_address")?,
            encode_bool_word(input_bool(request, "queue")?.unwrap_or(false)),
            encode_uint_word("192")?,
            encode_bytes_tail(&transceiver_instructions)?,
        ))
    } else {
        Ok(format!(
            "0x961b94d0{}{}{}",
            encode_uint_word_from_value(required_value(request, "amount")?)?,
            encode_u16_word(required_value(request, "recipient_chain")?)?,
            encode_bytes32_word(required_string(request, "recipient")?, "recipient")?,
        ))
    }
}

fn abi_encode_wormhole_publish_message(request: &PluginRequest) -> Result<String, PluginError> {
    let payload = required_hex_bytes(request, "payload")?;
    Ok(format!(
        "0xb19a437e{}{}{}{}",
        encode_optional_u32_word(request, "nonce", "0")?,
        encode_uint_word("96")?,
        encode_optional_u8_word(request, "consistency_level", "1")?,
        encode_bytes_tail(&payload)?,
    ))
}

fn ccip_receiver_bytes(request: &PluginRequest) -> Result<String, PluginError> {
    let receiver = required_string(request, "receiver")?;
    let normalized = normalize_hex_bytes(receiver, "receiver")?;
    if normalized.len() == 40 {
        encode_address_word(receiver)
    } else {
        Ok(normalized)
    }
}

fn encode_fee_token_word(request: &PluginRequest) -> Result<String, PluginError> {
    match request.input.get("fee_token") {
        Some(value) => encode_address_word(value_as_string(value, "fee_token")?),
        None => encode_address_word("0x0000000000000000000000000000000000000000"),
    }
}

fn encode_ccip_token_amounts_tail(request: &PluginRequest) -> Result<String, PluginError> {
    Ok(format!(
        "{}{}{}",
        encode_uint_word("1")?,
        encode_address_word(required_string(request, "token")?)?,
        encode_uint_word_from_value(required_value(request, "amount")?)?,
    ))
}

fn ccip_token_amounts_tail_len() -> usize {
    32 + 64
}

fn uses_wormhole_ntt_advanced_transfer(request: &PluginRequest) -> bool {
    request.input.contains_key("refund_address")
        || request.input.contains_key("queue")
        || request.input.contains_key("transceiver_instructions")
}

fn wormhole_ntt_transfer_format(request: &PluginRequest) -> &'static str {
    if uses_wormhole_ntt_advanced_transfer(request) {
        "transfer(uint256,uint16,bytes32,bytes32,bool,bytes)"
    } else {
        "transfer(uint256,uint16,bytes32)"
    }
}

fn encode_send_param_tail(request: &PluginRequest) -> Result<String, PluginError> {
    let extra_options = optional_hex_bytes(request, "extraOptions")?;
    let compose_msg = optional_hex_bytes(request, "composeMsg")?;
    let oft_cmd = optional_hex_bytes(request, "oftCmd")?;
    let extra_offset = 224usize;
    let compose_offset = extra_offset + abi_bytes_tail_len(&extra_options);
    let oft_offset = compose_offset + abi_bytes_tail_len(&compose_msg);
    Ok(format!(
        "{}{}{}{}{}{}{}{}{}{}",
        encode_u32_word(required_value(request, "dstEid")?)?,
        encode_recipient_bytes32(required_string(request, "to")?)?,
        encode_uint_word_from_value(required_value(request, "amountLD")?)?,
        encode_uint_word_from_value(required_value(request, "minAmountLD")?)?,
        encode_uint_word(&extra_offset.to_string())?,
        encode_uint_word(&compose_offset.to_string())?,
        encode_uint_word(&oft_offset.to_string())?,
        encode_bytes_tail(&extra_options)?,
        encode_bytes_tail(&compose_msg)?,
        encode_bytes_tail(&oft_cmd)?,
    ))
}

fn encode_messaging_fee(request: &PluginRequest) -> Result<String, PluginError> {
    Ok(format!(
        "{}{}",
        encode_uint_word_from_value(required_value(request, "nativeFee")?)?,
        encode_uint_word_from_value(required_value(request, "lzTokenFee")?)?,
    ))
}

fn required_value<'a>(request: &'a PluginRequest, key: &str) -> Result<&'a Value, PluginError> {
    request
        .input
        .get(key)
        .ok_or_else(|| PluginError::InvalidInput(format!("{key} is required")))
}

fn input_bool(request: &PluginRequest, key: &str) -> Result<Option<bool>, PluginError> {
    match request.input.get(key) {
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(Value::String(value)) if value == "true" => Ok(Some(true)),
        Some(Value::String(value)) if value == "false" => Ok(Some(false)),
        Some(_) => Err(PluginError::InvalidInput(format!("{key} must be a boolean"))),
        None => Ok(None),
    }
}

fn optional_hex_bytes(request: &PluginRequest, key: &str) -> Result<String, PluginError> {
    match request.input.get(key) {
        Some(value) => normalize_hex_bytes(value_as_string(value, key)?, key),
        None => Ok(String::new()),
    }
}

fn optional_hex_bytes_or(request: &PluginRequest, key: &str, default: &str) -> Result<String, PluginError> {
    match request.input.get(key) {
        Some(value) => normalize_hex_bytes(value_as_string(value, key)?, key),
        None => normalize_hex_bytes(default, key),
    }
}

fn required_hex_bytes(request: &PluginRequest, key: &str) -> Result<String, PluginError> {
    normalize_hex_bytes(required_string(request, key)?, key)
}

fn value_as_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, PluginError> {
    value
        .as_str()
        .ok_or_else(|| PluginError::InvalidInput(format!("{key} must be a string")))
}

fn normalize_hex_bytes(value: &str, key: &str) -> Result<String, PluginError> {
    let raw = value.strip_prefix("0x").unwrap_or(value);
    if raw.len() % 2 != 0 || !raw.chars().all(|char| char.is_ascii_hexdigit()) {
        return Err(PluginError::InvalidInput(format!(
            "{key} must be 0x-prefixed even-length hex bytes"
        )));
    }
    Ok(raw.to_ascii_lowercase())
}

fn encode_bytes_tail(bytes: &str) -> Result<String, PluginError> {
    let byte_len = bytes.len() / 2;
    let padded_len = if byte_len == 0 {
        0
    } else {
        ((byte_len + 31) / 32) * 64
    };
    Ok(format!(
        "{}{bytes:0<padded_len$}",
        encode_uint_word(&byte_len.to_string())?,
    ))
}

fn encode_string_tail(value: &str) -> Result<String, PluginError> {
    let bytes = value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    encode_bytes_tail(&bytes)
}

fn abi_bytes_tail_len(bytes: &str) -> usize {
    let byte_len = bytes.len() / 2;
    32 + if byte_len == 0 {
        0
    } else {
        ((byte_len + 31) / 32) * 32
    }
}

fn abi_string_tail_len(value: &str) -> usize {
    let byte_len = value.len();
    32 + if byte_len == 0 {
        0
    } else {
        ((byte_len + 31) / 32) * 32
    }
}

fn encode_address_word(address: &str) -> Result<String, PluginError> {
    let raw = address.strip_prefix("0x").unwrap_or(address);
    if raw.len() != 40 || !raw.chars().all(|char| char.is_ascii_hexdigit()) {
        return Err(PluginError::InvalidInput(String::from(
            "address inputs must be 20-byte hex addresses",
        )));
    }
    Ok(format!("{raw:0>64}").to_ascii_lowercase())
}

fn encode_recipient_bytes32(value: &str) -> Result<String, PluginError> {
    let raw = value.strip_prefix("0x").unwrap_or(value);
    if raw.len() == 40 && raw.chars().all(|char| char.is_ascii_hexdigit()) {
        return Ok(format!("{raw:0>64}").to_ascii_lowercase());
    }
    encode_bytes32_word(value, "to")
}

fn encode_bytes32_word(value: &str, key: &str) -> Result<String, PluginError> {
    let raw = value.strip_prefix("0x").unwrap_or(value);
    if raw.len() != 64 || !raw.chars().all(|char| char.is_ascii_hexdigit()) {
        return Err(PluginError::InvalidInput(format!("{key} must be 32-byte hex")));
    }
    Ok(raw.to_ascii_lowercase())
}

fn encode_bool_word(value: bool) -> String {
    format!("{:0>64}", if value { "1" } else { "0" })
}

fn encode_u32_word(value: &Value) -> Result<String, PluginError> {
    encode_bounded_uint_word(value, u32::MAX as u128, "uint32")
}

fn encode_u64_word(value: &Value) -> Result<String, PluginError> {
    encode_bounded_uint_word(value, u64::MAX as u128, "uint64")
}

fn encode_u16_word(value: &Value) -> Result<String, PluginError> {
    encode_bounded_uint_word(value, u16::MAX as u128, "uint16")
}

fn encode_u8_word(value: &Value) -> Result<String, PluginError> {
    encode_bounded_uint_word(value, u8::MAX as u128, "uint8")
}

fn encode_optional_u32_word(
    request: &PluginRequest,
    key: &str,
    default: &str,
) -> Result<String, PluginError> {
    match request.input.get(key) {
        Some(value) => encode_u32_word(value),
        None => encode_uint_word(default),
    }
}

fn encode_optional_u8_word(
    request: &PluginRequest,
    key: &str,
    default: &str,
) -> Result<String, PluginError> {
    match request.input.get(key) {
        Some(value) => encode_u8_word(value),
        None => encode_uint_word(default),
    }
}

fn encode_bounded_uint_word(value: &Value, max: u128, label: &str) -> Result<String, PluginError> {
    let value = uint_string_from_value(value)?;
    let parsed = value
        .parse::<u128>()
        .map_err(|_| PluginError::InvalidInput(format!("{label} input is too large")))?;
    if parsed > max {
        return Err(PluginError::InvalidInput(format!("{label} input exceeds maximum value")));
    }
    encode_uint_word(&value)
}

fn encode_uint_word_from_value(value: &Value) -> Result<String, PluginError> {
    encode_uint_word(&uint_string_from_value(value)?)
}

fn uint_string_from_value(value: &Value) -> Result<String, PluginError> {
    match value {
        Value::String(value) => normalize_uint_string(value),
        Value::Number(value) => normalize_uint_string(&value.to_string()),
        _ => Err(PluginError::InvalidInput(String::from(
            "uint inputs must be integer strings or JSON numbers",
        ))),
    }
}

fn normalize_uint_string(value: &str) -> Result<String, PluginError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|char| char.is_ascii_digit()) {
        return Err(PluginError::InvalidInput(String::from(
            "uint inputs must be non-negative integers",
        )));
    }
    Ok(trimmed.trim_start_matches('0').to_owned().if_empty_then_zero())
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
            "uint input does not fit uint256",
        )));
    }
    Ok(format!("{hex:0>64}"))
}

fn div_mod_decimal_string(value: &str, divisor: u32) -> Result<(String, u32), PluginError> {
    let mut quotient = String::new();
    let mut remainder = 0u32;
    for char in value.chars() {
        let digit = char
            .to_digit(10)
            .ok_or_else(|| PluginError::InvalidInput(String::from("uint input must be decimal")))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    const OPERATIONS: &[OperationSpec] = &[
        OperationSpec {
            name: "test_prepare_transfer",
            action_kind: "test_transfer",
            target_input: "router",
            chain_input: "chain_id",
            required_inputs: &["chain_id", "router", "amount"],
            optional_inputs: &["call_data", "value"],
        },
        OperationSpec {
            name: "base_prepare_l1_standard_bridge_deposit",
            action_kind: "op_stack_l1_standard_bridge_deposit",
            target_input: "l1_standard_bridge",
            chain_input: "l1_chain_id",
            required_inputs: &[
                "l1_chain_id",
                "l1_standard_bridge",
                "l1_token",
                "l2_token",
                "to",
                "amount",
                "min_gas_limit",
            ],
            optional_inputs: &["extra_data"],
        },
        OperationSpec {
            name: "base_prepare_l2_standard_bridge_withdraw",
            action_kind: "op_stack_l2_standard_bridge_withdraw",
            target_input: "l2_standard_bridge",
            chain_input: "l2_chain_id",
            required_inputs: &[
                "l2_chain_id",
                "l2_standard_bridge",
                "l2_token",
                "to",
                "amount",
                "min_gas_limit",
            ],
            optional_inputs: &["extra_data"],
        },
        OperationSpec {
            name: "polygon_prepare_pos_deposit",
            action_kind: "polygon_pos_deposit",
            target_input: "root_chain_manager",
            chain_input: "l1_chain_id",
            required_inputs: &["l1_chain_id", "root_chain_manager", "user", "root_token", "deposit_data"],
            optional_inputs: &[],
        },
        OperationSpec {
            name: "polygon_prepare_pos_exit",
            action_kind: "polygon_pos_exit",
            target_input: "root_chain_manager",
            chain_input: "l1_chain_id",
            required_inputs: &["l1_chain_id", "root_chain_manager", "exit_payload"],
            optional_inputs: &[],
        },
        OperationSpec {
            name: "polygon_prepare_state_sync",
            action_kind: "polygon_state_sync",
            target_input: "state_sender",
            chain_input: "l1_chain_id",
            required_inputs: &["l1_chain_id", "state_sender", "receiver", "data"],
            optional_inputs: &[],
        },
        OperationSpec {
            name: "arbitrum_prepare_l1_erc20_deposit",
            action_kind: "arbitrum_l1_gateway_deposit",
            target_input: "l1_gateway_router",
            chain_input: "l1_chain_id",
            required_inputs: &[
                "l1_chain_id",
                "l1_gateway_router",
                "l1_token",
                "to",
                "amount",
                "max_gas",
                "gas_price_bid",
            ],
            optional_inputs: &["refund_to", "gateway_data"],
        },
        OperationSpec {
            name: "arbitrum_prepare_l2_erc20_withdraw",
            action_kind: "arbitrum_l2_gateway_withdraw",
            target_input: "l2_gateway_router",
            chain_input: "l2_chain_id",
            required_inputs: &["l2_chain_id", "l2_gateway_router", "l2_token", "to", "amount"],
            optional_inputs: &["gateway_data"],
        },
        OperationSpec {
            name: "layerzero_prepare_oft_send",
            action_kind: "oft_send",
            target_input: "oft",
            chain_input: "source_chain_id",
            required_inputs: &[
                "source_chain_id",
                "oft",
                "dstEid",
                "to",
                "amountLD",
                "minAmountLD",
                "nativeFee",
                "lzTokenFee",
                "refundAddress",
            ],
            optional_inputs: &["extraOptions", "composeMsg", "oftCmd"],
        },
        OperationSpec {
            name: "stargate_prepare_send_token",
            action_kind: "stargate_send_token",
            target_input: "stargate",
            chain_input: "source_chain_id",
            required_inputs: &[
                "source_chain_id",
                "stargate",
                "dstEid",
                "to",
                "amountLD",
                "minAmountLD",
                "nativeFee",
                "lzTokenFee",
                "refundAddress",
            ],
            optional_inputs: &["extraOptions", "composeMsg", "oftCmd"],
        },
        OperationSpec {
            name: "usdt0_prepare_quote_send",
            action_kind: "oft_quote_send",
            target_input: "oft",
            chain_input: "source_chain_id",
            required_inputs: &["source_chain_id", "oft", "dstEid", "to", "amountLD", "minAmountLD"],
            optional_inputs: &["extraOptions", "composeMsg", "oftCmd", "payInLzToken"],
        },
        OperationSpec {
            name: "layerzero_prepare_set_peer",
            action_kind: "oapp_set_peer",
            target_input: "oapp",
            chain_input: "chain_id",
            required_inputs: &["chain_id", "oapp", "eid", "peer"],
            optional_inputs: &[],
        },
        OperationSpec {
            name: "hyperlane_prepare_warp_transfer",
            action_kind: "warp_route_transfer",
            target_input: "warp_router",
            chain_input: "source_chain_id",
            required_inputs: &["source_chain_id", "warp_router", "destination_domain", "recipient", "amount"],
            optional_inputs: &[],
        },
        OperationSpec {
            name: "hyperlane_prepare_message",
            action_kind: "mailbox_dispatch",
            target_input: "mailbox",
            chain_input: "source_chain_id",
            required_inputs: &["source_chain_id", "mailbox", "destination_domain", "recipient", "messageBody"],
            optional_inputs: &[],
        },
        OperationSpec {
            name: "axelar_prepare_interchain_transfer",
            action_kind: "its_interchain_transfer",
            target_input: "interchain_token_service",
            chain_input: "source_chain_id",
            required_inputs: &[
                "source_chain_id",
                "interchain_token_service",
                "tokenId",
                "destinationChain",
                "destinationAddress",
                "amount",
            ],
            optional_inputs: &[],
        },
        OperationSpec {
            name: "axelar_prepare_call_contract",
            action_kind: "gmp_call_contract",
            target_input: "gateway",
            chain_input: "source_chain_id",
            required_inputs: &[
                "source_chain_id",
                "gateway",
                "destinationChain",
                "destinationContractAddress",
                "payload",
            ],
            optional_inputs: &[],
        },
        OperationSpec {
            name: "ccip_prepare_token_transfer",
            action_kind: "ccip_token_transfer",
            target_input: "router",
            chain_input: "source_chain_id",
            required_inputs: &["source_chain_id", "router", "destination_chain_selector", "receiver", "token", "amount"],
            optional_inputs: &["fee_token", "message_data", "extra_args"],
        },
        OperationSpec {
            name: "wormhole_prepare_ntt_transfer",
            action_kind: "ntt_transfer",
            target_input: "ntt_manager",
            chain_input: "source_chain_id",
            required_inputs: &["source_chain_id", "ntt_manager", "amount", "recipient", "recipient_chain"],
            optional_inputs: &["refund_address", "queue", "transceiver_instructions"],
        },
        OperationSpec {
            name: "wormhole_prepare_message",
            action_kind: "wormhole_message",
            target_input: "core_bridge",
            chain_input: "source_chain_id",
            required_inputs: &["source_chain_id", "core_bridge", "payload"],
            optional_inputs: &["nonce", "consistency_level"],
        },
    ];

    const SPEC: PluginSpec = PluginSpec {
        plugin_id: "test-node",
        provider: "test-provider",
        operations: OPERATIONS,
    };

    #[tokio::test]
    async fn malformed_jsonrpc_params_preserve_error_envelope() {
        let response = handle_request_json(
            &json!({
                "jsonrpc": "2.0",
                "id": 42,
                "method": "node.exec.v2",
                "params": {
                    "contract_version": 1
                }
            })
            .to_string(),
            &SPEC,
        )
        .await
        .expect("error response should serialize");

        let payload: Value = serde_json::from_str(&response).expect("json response");
        assert_eq!(payload["jsonrpc"], json!("2.0"));
        assert_eq!(payload["id"], json!(42));
        assert!(payload.get("error").is_some(), "response should be a JSON-RPC error");
        assert!(payload.get("success").is_none(), "legacy response shape should not be used");
    }

    #[tokio::test]
    async fn jsonrpc_request_returns_unsigned_action() {
        let response = handle_request_json(
            &json!({
                "jsonrpc": "2.0",
                "id": 9,
                "method": "node.exec.v2",
                "params": {
                    "contract_version": "1.0.0",
                    "plugin_id": "test-node",
                    "node_id": "node-1",
                    "operation": "test_prepare_transfer",
                    "input": {
                        "chain_id": 1,
                        "router": "0x1111111111111111111111111111111111111111",
                        "amount": "100",
                        "call_data": "0x1234"
                    }
                }
            })
            .to_string(),
            &SPEC,
        )
        .await
        .expect("request should succeed");

        let payload: Value = serde_json::from_str(&response).expect("response should decode");
        assert_eq!(payload["result"]["result_state"], json!("prepared"));
        assert_eq!(
            payload["result"]["output"]["unsigned_action"]["to"],
            json!("0x1111111111111111111111111111111111111111")
        );
        assert_eq!(
            payload["result"]["output"]["unsigned_action"]["data"],
            json!("0x1234")
        );
        assert_eq!(
            payload["result"]["output"]["metadata"]["provider"],
            json!("test-provider")
        );
    }

    #[tokio::test]
    async fn base_l1_deposit_matches_cast_calldata() {
        let payload = execute_legacy(
            "base_prepare_l1_standard_bridge_deposit",
            json!({
                "l1_chain_id": 1,
                "l1_standard_bridge": "0x4444444444444444444444444444444444444444",
                "l1_token": "0x1111111111111111111111111111111111111111",
                "l2_token": "0x2222222222222222222222222222222222222222",
                "to": "0x3333333333333333333333333333333333333333",
                "amount": "1000",
                "min_gas_limit": 200000,
                "extra_data": "0x1234"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["calldata_format"],
            json!("depositERC20To(address,address,address,uint256,uint32,bytes)")
        );
        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0x838b252000000000000000000000000011111111111111111111111111111111111111110000000000000000000000002222222222222222222222222222222222222222000000000000000000000000333333333333333333333333333333333333333300000000000000000000000000000000000000000000000000000000000003e80000000000000000000000000000000000000000000000000000000000030d4000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn base_l2_withdraw_matches_cast_calldata() {
        let payload = execute_legacy(
            "base_prepare_l2_standard_bridge_withdraw",
            json!({
                "l2_chain_id": 8453,
                "l2_standard_bridge": "0x4444444444444444444444444444444444444444",
                "l2_token": "0x2222222222222222222222222222222222222222",
                "to": "0x3333333333333333333333333333333333333333",
                "amount": "1000",
                "min_gas_limit": 200000,
                "extra_data": "0x1234"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0xa3a795480000000000000000000000002222222222222222222222222222222222222222000000000000000000000000333333333333333333333333333333333333333300000000000000000000000000000000000000000000000000000000000003e80000000000000000000000000000000000000000000000000000000000030d4000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn polygon_deposit_matches_cast_calldata() {
        let payload = execute_legacy(
            "polygon_prepare_pos_deposit",
            json!({
                "l1_chain_id": 1,
                "root_chain_manager": "0x4444444444444444444444444444444444444444",
                "user": "0x3333333333333333333333333333333333333333",
                "root_token": "0x1111111111111111111111111111111111111111",
                "deposit_data": "0x1234"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0xe3dec8fb00000000000000000000000033333333333333333333333333333333333333330000000000000000000000001111111111111111111111111111111111111111000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn polygon_exit_matches_cast_calldata() {
        let payload = execute_legacy(
            "polygon_prepare_pos_exit",
            json!({
                "l1_chain_id": 1,
                "root_chain_manager": "0x4444444444444444444444444444444444444444",
                "exit_payload": "0x1234"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0x3805550f000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn polygon_state_sync_matches_cast_calldata() {
        let payload = execute_legacy(
            "polygon_prepare_state_sync",
            json!({
                "l1_chain_id": 1,
                "state_sender": "0x4444444444444444444444444444444444444444",
                "receiver": "0x3333333333333333333333333333333333333333",
                "data": "0x1234"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0x16f198310000000000000000000000003333333333333333333333333333333333333333000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn arbitrum_l1_deposit_matches_cast_calldata() {
        let payload = execute_legacy(
            "arbitrum_prepare_l1_erc20_deposit",
            json!({
                "l1_chain_id": 1,
                "l1_gateway_router": "0x4444444444444444444444444444444444444444",
                "l1_token": "0x1111111111111111111111111111111111111111",
                "refund_to": "0x5555555555555555555555555555555555555555",
                "to": "0x3333333333333333333333333333333333333333",
                "amount": "1000",
                "max_gas": "200000",
                "gas_price_bid": "100000000",
                "gateway_data": "0x1234"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["calldata_format"],
            json!("outboundTransferCustomRefund(address,address,address,uint256,uint256,uint256,bytes)")
        );
        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0x4fb1a07b00000000000000000000000011111111111111111111111111111111111111110000000000000000000000005555555555555555555555555555555555555555000000000000000000000000333333333333333333333333333333333333333300000000000000000000000000000000000000000000000000000000000003e80000000000000000000000000000000000000000000000000000000000030d400000000000000000000000000000000000000000000000000000000005f5e10000000000000000000000000000000000000000000000000000000000000000e000000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn arbitrum_l2_withdraw_matches_cast_calldata() {
        let payload = execute_legacy(
            "arbitrum_prepare_l2_erc20_withdraw",
            json!({
                "l2_chain_id": 42161,
                "l2_gateway_router": "0x4444444444444444444444444444444444444444",
                "l2_token": "0x2222222222222222222222222222222222222222",
                "to": "0x3333333333333333333333333333333333333333",
                "amount": "1000",
                "gateway_data": "0x1234"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0x7b3a3c8b0000000000000000000000002222222222222222222222222222222222222222000000000000000000000000333333333333333333333333333333333333333300000000000000000000000000000000000000000000000000000000000003e8000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn oft_send_matches_cast_calldata() {
        let payload = execute_legacy(
            "layerzero_prepare_oft_send",
            layerzero_send_input("oft"),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["calldata_format"],
            json!("send((uint32,bytes32,uint256,uint256,bytes,bytes,bytes),(uint256,uint256),address)")
        );
        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0xc7c7f5b30000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000004d00000000000000000000000000000000000000000000000000000000000000000000000000000000000000005555555555555555555555555555555555555555000000000000000000000000000000000000000000000000000000000000759e000000000000000000000000333333333333333333333333333333333333333300000000000000000000000000000000000000000000000000000000000003e800000000000000000000000000000000000000000000000000000000000003de00000000000000000000000000000000000000000000000000000000000000e000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000160000000000000000000000000000000000000000000000000000000000000000212340000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002abcd0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn stargate_send_token_matches_cast_calldata() {
        let payload = execute_legacy(
            "stargate_prepare_send_token",
            layerzero_send_input("stargate"),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["calldata_format"],
            json!("sendToken((uint32,bytes32,uint256,uint256,bytes,bytes,bytes),(uint256,uint256),address)")
        );
        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0xcbef2aa90000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000004d00000000000000000000000000000000000000000000000000000000000000000000000000000000000000005555555555555555555555555555555555555555000000000000000000000000000000000000000000000000000000000000759e000000000000000000000000333333333333333333333333333333333333333300000000000000000000000000000000000000000000000000000000000003e800000000000000000000000000000000000000000000000000000000000003de00000000000000000000000000000000000000000000000000000000000000e000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000160000000000000000000000000000000000000000000000000000000000000000212340000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002abcd0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn oft_quote_send_matches_cast_calldata() {
        let payload = execute_legacy(
            "usdt0_prepare_quote_send",
            json!({
                "source_chain_id": 1,
                "oft": "0x4444444444444444444444444444444444444444",
                "dstEid": 30110,
                "to": "0x3333333333333333333333333333333333333333",
                "amountLD": "1000",
                "minAmountLD": "990",
                "extraOptions": "0x1234",
                "composeMsg": "0xabcd",
                "oftCmd": "0x",
                "payInLzToken": false
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0x3b6f743b00000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000759e000000000000000000000000333333333333333333333333333333333333333300000000000000000000000000000000000000000000000000000000000003e800000000000000000000000000000000000000000000000000000000000003de00000000000000000000000000000000000000000000000000000000000000e000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000160000000000000000000000000000000000000000000000000000000000000000212340000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002abcd0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn set_peer_matches_cast_calldata() {
        let payload = execute_legacy(
            "layerzero_prepare_set_peer",
            json!({
                "chain_id": 1,
                "oapp": "0x4444444444444444444444444444444444444444",
                "eid": 30110,
                "peer": "0x0000000000000000000000003333333333333333333333333333333333333333"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0x3400288b000000000000000000000000000000000000000000000000000000000000759e0000000000000000000000003333333333333333333333333333333333333333")
        );
    }

    #[tokio::test]
    async fn hyperlane_warp_transfer_matches_cast_calldata() {
        let payload = execute_legacy(
            "hyperlane_prepare_warp_transfer",
            json!({
                "source_chain_id": 1,
                "warp_router": "0x4444444444444444444444444444444444444444",
                "destination_domain": 30110,
                "recipient": "0x3333333333333333333333333333333333333333",
                "amount": "1000"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["calldata_format"],
            json!("transferRemote(uint32,bytes32,uint256)")
        );
        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0x81b4e8b4000000000000000000000000000000000000000000000000000000000000759e000000000000000000000000333333333333333333333333333333333333333300000000000000000000000000000000000000000000000000000000000003e8")
        );
    }

    #[tokio::test]
    async fn hyperlane_mailbox_dispatch_matches_cast_calldata() {
        let payload = execute_legacy(
            "hyperlane_prepare_message",
            json!({
                "source_chain_id": 1,
                "mailbox": "0x4444444444444444444444444444444444444444",
                "destination_domain": 30110,
                "recipient": "0x3333333333333333333333333333333333333333",
                "messageBody": "0x1234"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["calldata_format"],
            json!("dispatch(uint32,bytes32,bytes)")
        );
        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0xfa31de01000000000000000000000000000000000000000000000000000000000000759e0000000000000000000000003333333333333333333333333333333333333333000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn axelar_interchain_transfer_matches_cast_calldata() {
        let payload = execute_legacy(
            "axelar_prepare_interchain_transfer",
            json!({
                "source_chain_id": 1,
                "interchain_token_service": "0x4444444444444444444444444444444444444444",
                "tokenId": "0x1111111111111111111111111111111111111111111111111111111111111111",
                "destinationChain": "Ethereum",
                "destinationAddress": "0x3333333333333333333333333333333333333333",
                "amount": "1000"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["calldata_format"],
            json!("interchainTransfer(bytes32,string,bytes,uint256)")
        );
        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0xe24a240b1111111111111111111111111111111111111111111111111111111111111111000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000003e80000000000000000000000000000000000000000000000000000000000000008457468657265756d00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000143333333333333333333333333333333333333333000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn axelar_call_contract_matches_cast_calldata() {
        let payload = execute_legacy(
            "axelar_prepare_call_contract",
            json!({
                "source_chain_id": 1,
                "gateway": "0x4444444444444444444444444444444444444444",
                "destinationChain": "Ethereum",
                "destinationContractAddress": "0x3333333333333333333333333333333333333333",
                "payload": "0x1234"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["calldata_format"],
            json!("callContract(string,string,bytes)")
        );
        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0x1c92115f000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000008457468657265756d000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002a3078333333333333333333333333333333333333333333333333333333333333333333333333333333330000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn ccip_token_transfer_matches_cast_calldata() {
        let payload = execute_legacy(
            "ccip_prepare_token_transfer",
            json!({
                "source_chain_id": 1,
                "router": "0x4444444444444444444444444444444444444444",
                "destination_chain_selector": "16015286601757825753",
                "receiver": "0x3333333333333333333333333333333333333333",
                "token": "0x1111111111111111111111111111111111111111",
                "amount": "1000",
                "message_data": "0x1234",
                "extra_args": "0x181dcf10"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["calldata_format"],
            json!("ccipSend(uint64,(bytes,bytes,(address,uint256)[],address,bytes))")
        );
        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0x96f4e9f9000000000000000000000000000000000000000000000000de41ba4fc9d91ad9000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000e000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000018000000000000000000000000000000000000000000000000000000000000000200000000000000000000000003333333333333333333333333333333333333333000000000000000000000000000000000000000000000000000000000000000212340000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000111111111111111111111111111111111111111100000000000000000000000000000000000000000000000000000000000003e80000000000000000000000000000000000000000000000000000000000000004181dcf1000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn wormhole_ntt_basic_transfer_matches_cast_calldata() {
        let payload = execute_legacy(
            "wormhole_prepare_ntt_transfer",
            json!({
                "source_chain_id": 1,
                "ntt_manager": "0x4444444444444444444444444444444444444444",
                "amount": "1000",
                "recipient_chain": 5,
                "recipient": "0x0000000000000000000000003333333333333333333333333333333333333333"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["calldata_format"],
            json!("transfer(uint256,uint16,bytes32)")
        );
        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0x961b94d000000000000000000000000000000000000000000000000000000000000003e800000000000000000000000000000000000000000000000000000000000000050000000000000000000000003333333333333333333333333333333333333333")
        );
    }

    #[tokio::test]
    async fn wormhole_ntt_advanced_transfer_matches_cast_calldata() {
        let payload = execute_legacy(
            "wormhole_prepare_ntt_transfer",
            json!({
                "source_chain_id": 1,
                "ntt_manager": "0x4444444444444444444444444444444444444444",
                "amount": "1000",
                "recipient_chain": 5,
                "recipient": "0x0000000000000000000000003333333333333333333333333333333333333333",
                "refund_address": "0x0000000000000000000000005555555555555555555555555555555555555555",
                "queue": true,
                "transceiver_instructions": "0x1234"
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["calldata_format"],
            json!("transfer(uint256,uint16,bytes32,bytes32,bool,bytes)")
        );
        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0xb293f97f00000000000000000000000000000000000000000000000000000000000003e8000000000000000000000000000000000000000000000000000000000000000500000000000000000000000033333333333333333333333333333333333333330000000000000000000000005555555555555555555555555555555555555555000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[tokio::test]
    async fn wormhole_message_matches_cast_calldata() {
        let payload = execute_legacy(
            "wormhole_prepare_message",
            json!({
                "source_chain_id": 1,
                "core_bridge": "0x4444444444444444444444444444444444444444",
                "payload": "0x1234",
                "nonce": 7,
                "consistency_level": 1
            }),
        )
        .await;

        assert_eq!(
            payload["output"]["unsigned_action"]["calldata_format"],
            json!("publishMessage(uint32,bytes,uint8)")
        );
        assert_eq!(
            payload["output"]["unsigned_action"]["data"],
            json!("0xb19a437e00000000000000000000000000000000000000000000000000000000000000070000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000")
        );
    }

    fn layerzero_send_input(target_key: &str) -> Value {
        let mut input = json!({
            "source_chain_id": 1,
            "dstEid": 30110,
            "to": "0x3333333333333333333333333333333333333333",
            "amountLD": "1000",
            "minAmountLD": "990",
            "extraOptions": "0x1234",
            "composeMsg": "0xabcd",
            "oftCmd": "0x",
            "nativeFee": "77",
            "lzTokenFee": "0",
            "refundAddress": "0x5555555555555555555555555555555555555555"
        });
        input[target_key] = json!("0x4444444444444444444444444444444444444444");
        input
    }

    async fn execute_legacy(operation: &str, input: Value) -> Value {
        let response = handle_request_json(
            &json!({
                "contract_version": "1.0.0",
                "plugin_id": "test-node",
                "node_id": "node-1",
                "operation": operation,
                "input": input
            })
            .to_string(),
            &SPEC,
        )
        .await
        .expect("request should succeed");

        serde_json::from_str(&response).expect("response should decode")
    }
}
