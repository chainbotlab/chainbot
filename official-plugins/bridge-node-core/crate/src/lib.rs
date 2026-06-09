use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub const CONTRACT_VERSION: &str = "1.0.0";
pub const JSONRPC_VERSION: &str = "2.0";
pub const EXECUTE_METHOD: &str = "node.exec.v2";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BODY_BYTES: usize = 512 * 1024;
const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.azure.internal"];

#[derive(Debug)]
pub enum PluginError {
    Json(serde_json::Error),
    Reqwest(reqwest::Error),
    InvalidInput(String),
    Api(String),
    Unsupported(String),
}

impl Display for PluginError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => write!(f, "json error: {error}"),
            Self::Reqwest(error) => write!(f, "request error: {error}"),
            Self::InvalidInput(message) => write!(f, "invalid input: {message}"),
            Self::Api(message) => write!(f, "bridge api error: {message}"),
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

impl From<reqwest::Error> for PluginError {
    fn from(value: reqwest::Error) -> Self {
        Self::Reqwest(value)
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

impl PluginRequest {
    pub fn input_string(&self, key: &str) -> Option<&str> {
        self.input.get(key).and_then(Value::as_str)
    }

    pub fn activation_secret(&self, key: &str) -> Option<&str> {
        self.activation
            .as_ref()
            .and_then(|activation| activation.secrets.get(key))
            .map(String::as_str)
    }

    pub fn allowed_origins(&self) -> &[String] {
        self.activation
            .as_ref()
            .map(|activation| activation.allowed_origins.as_slice())
            .unwrap_or(&[])
    }
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

#[derive(Debug, Clone, Copy)]
pub enum InputPlacement {
    Query,
    Body,
}

#[derive(Debug, Clone, Copy)]
pub struct OperationSpec {
    pub name: &'static str,
    pub method: MethodSpec,
    pub default_path: &'static str,
    pub input_placement: InputPlacement,
    pub output_key: &'static str,
    pub result_state: Option<&'static str>,
    pub required_inputs: &'static [&'static str],
    pub optional_inputs: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub enum MethodSpec {
    Get,
    Post,
}

#[derive(Debug, Clone, Copy)]
pub struct PluginSpec {
    pub plugin_id: &'static str,
    pub provider: &'static str,
    pub default_base_url: &'static str,
    pub operations: &'static [OperationSpec],
    pub api_key_header: Option<&'static str>,
    pub api_key_secret: Option<&'static str>,
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
    let response = dispatch(request, spec).await?;
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
    serde_json::from_str::<RequestEnvelope>(input).ok().and_then(|envelope| match envelope {
        RequestEnvelope::Legacy(_) => None,
        RequestEnvelope::JsonRpc(request) => Some(request.id),
    })
}

async fn dispatch(request: PluginRequest, spec: &'static PluginSpec) -> Result<PluginResponse, PluginError> {
    validate_request(&request, spec)?;
    let operation = spec
        .operations
        .iter()
        .find(|operation| operation.name == request.operation)
        .ok_or_else(|| PluginError::Unsupported(format!("operation {} is not supported", request.operation)))?;
    execute_api_operation(&request, spec, operation).await
}

fn validate_request(request: &PluginRequest, spec: &PluginSpec) -> Result<(), PluginError> {
    if request.contract_version.trim().is_empty() {
        return Err(PluginError::InvalidInput(String::from(
            "contract_version must not be empty",
        )));
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
        return Err(PluginError::InvalidInput(String::from(
            "node_id must not be empty",
        )));
    }
    Ok(())
}

async fn execute_api_operation(
    request: &PluginRequest,
    spec: &PluginSpec,
    operation: &OperationSpec,
) -> Result<PluginResponse, PluginError> {
    for key in operation.required_inputs {
        if !request.input.contains_key(*key) {
            return Err(PluginError::InvalidInput(format!("{key} is required")));
        }
    }

    let base_url = request
        .input_string("base_url")
        .unwrap_or(spec.default_base_url);
    let mut url = base_url_with_slash(base_url)?;
    let path = request
        .input_string("path")
        .unwrap_or(operation.default_path)
        .trim_start_matches('/');
    url = url
        .join(path)
        .map_err(|error| PluginError::InvalidInput(format!("invalid operation path: {error}")))?;
    validate_destination_policy(&url, request.allowed_origins(), request.input_string("base_url").is_some())?;

    let mut body = Map::new();
    let mut query_pairs = Vec::new();
    for key in operation.required_inputs.iter().chain(operation.optional_inputs.iter()) {
        if let Some(value) = request.input.get(*key) {
            match operation.input_placement {
                InputPlacement::Query => query_pairs.push((key.to_string(), value_as_query_string(value)?)),
                InputPlacement::Body => {
                    body.insert((*key).to_string(), value.clone());
                }
            }
        }
    }
    if matches!(operation.input_placement, InputPlacement::Query) {
        let mut query = url.query_pairs_mut();
        for (key, value) in query_pairs {
            query.append_pair(&key, &value);
        }
    }

    let host = url.host_str().ok_or_else(|| {
        PluginError::InvalidInput(String::from("operation url must include a host"))
    })?;
    let addresses = resolve_destination(&url)?;
    let client = Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .build()?;

    let mut http_request = client.request(method(operation.method), url);
    let headers = request_headers(request, spec)?;
    if !headers.is_empty() {
        http_request = http_request.headers(headers);
    }
    if matches!(operation.input_placement, InputPlacement::Body) {
        http_request = http_request.json(&Value::Object(body));
    }

    let response = http_request.send().await?;
    let status = response.status();
    if status.is_redirection() {
        return Err(PluginError::Api(String::from("redirect responses are not supported")));
    }
    if response.content_length().unwrap_or(0) > MAX_RESPONSE_BODY_BYTES as u64 {
        return Err(PluginError::Api(format!(
            "response body exceeds {} bytes",
            MAX_RESPONSE_BODY_BYTES
        )));
    }
    let body_bytes = response.bytes().await?;
    if body_bytes.len() > MAX_RESPONSE_BODY_BYTES {
        return Err(PluginError::Api(format!(
            "response body exceeds {} bytes",
            MAX_RESPONSE_BODY_BYTES
        )));
    }
    let payload: Value = serde_json::from_slice(&body_bytes)?;
    if !status.is_success() {
        return Err(PluginError::Api(format!("http status {status}: {payload}")));
    }
    let output = BTreeMap::from([
        (operation.output_key.to_string(), payload.clone()),
        (String::from("metadata"), json!({"provider": spec.provider})),
    ]);
    Ok(PluginResponse::success(output, operation.result_state))
}

fn request_headers(request: &PluginRequest, spec: &PluginSpec) -> Result<HeaderMap, PluginError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        HeaderValue::from_static("application/json"),
    );
    if let Some(header) = spec.api_key_header {
        let api_key = spec
            .api_key_secret
            .and_then(|secret| request.activation_secret(secret))
            .or_else(|| request.input_string("api_key"));
        if let Some(api_key) = api_key {
            let header_name = HeaderName::from_bytes(header.as_bytes()).map_err(|error| {
                PluginError::InvalidInput(format!("invalid api key header {header}: {error}"))
            })?;
            let header_value = HeaderValue::from_str(api_key).map_err(|error| {
                PluginError::InvalidInput(format!("invalid api key value: {error}"))
            })?;
            headers.insert(header_name, header_value);
        }
    }
    if let Some(integrator_id) = request.input_string("integrator_id") {
        headers.insert(
            HeaderName::from_static("x-integrator-id"),
            HeaderValue::from_str(integrator_id).map_err(|error| {
                PluginError::InvalidInput(format!("invalid integrator_id header value: {error}"))
            })?,
        );
    }
    Ok(headers)
}

fn method(method: MethodSpec) -> Method {
    match method {
        MethodSpec::Get => Method::GET,
        MethodSpec::Post => Method::POST,
    }
}

fn base_url_with_slash(raw: &str) -> Result<Url, PluginError> {
    let mut url = Url::parse(raw)
        .map_err(|error| PluginError::InvalidInput(format!("invalid base_url: {error}")))?;
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path().trim_end_matches('/'));
        url.set_path(&path);
    }
    Ok(url)
}

fn value_as_query_string(value: &Value) -> Result<String, PluginError> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Array(values) => Ok(values
            .iter()
            .map(value_as_query_string)
            .collect::<Result<Vec<_>, _>>()?
            .join(",")),
        _ => Err(PluginError::InvalidInput(String::from(
            "query inputs must be strings, numbers, booleans, or arrays of those values",
        ))),
    }
}

fn validate_destination_policy(
    url: &Url,
    allowed_origins: &[String],
    requires_binding: bool,
) -> Result<(), PluginError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(PluginError::InvalidInput(String::from(
            "destination url must use http or https",
        )));
    }
    validate_allowed_origins(url, allowed_origins, requires_binding)?;
    let host = url.host_str().ok_or_else(|| {
        PluginError::InvalidInput(String::from("destination url must include a host"))
    })?;
    if BLOCKED_HOSTS.iter().any(|blocked| host.eq_ignore_ascii_case(blocked)) {
        return Err(PluginError::InvalidInput(format!("destination host `{host}` is blocked")));
    }
    let _ = resolve_destination(url)?;
    Ok(())
}

fn validate_allowed_origins(
    url: &Url,
    allowed_origins: &[String],
    requires_binding: bool,
) -> Result<(), PluginError> {
    if !requires_binding {
        return Ok(());
    }
    if allowed_origins.is_empty() {
        return Err(PluginError::InvalidInput(String::from(
            "custom base_url requires at least one allowed origin",
        )));
    }
    let request_origin = normalize_origin(url)?;
    let permitted = allowed_origins.iter().any(|origin| {
        Url::parse(origin)
            .ok()
            .and_then(|parsed| normalize_origin(&parsed).ok())
            .as_deref()
            == Some(request_origin.as_str())
    });
    if permitted {
        Ok(())
    } else {
        Err(PluginError::InvalidInput(format!(
            "request origin {request_origin} is not allowlisted"
        )))
    }
}

fn normalize_origin(url: &Url) -> Result<String, PluginError> {
    let host = url.host_str().ok_or_else(|| {
        PluginError::InvalidInput(String::from("destination url must include a host"))
    })?;
    let port = url.port_or_known_default().ok_or_else(|| {
        PluginError::InvalidInput(String::from("destination url must use a known port for its scheme"))
    })?;
    Ok(format!("{}://{}:{}", url.scheme(), host, port))
}

fn resolve_destination(url: &Url) -> Result<Vec<SocketAddr>, PluginError> {
    let host = url.host_str().ok_or_else(|| {
        PluginError::InvalidInput(String::from("destination url must include a host"))
    })?;
    let port = url.port_or_known_default().ok_or_else(|| {
        PluginError::InvalidInput(String::from("destination url must use a known port for its scheme"))
    })?;
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|source| PluginError::Api(format!("failed to resolve destination host: {source}")))?
        .collect::<Vec<_>>();
    for address in &addresses {
        validate_ip(address.ip())?;
    }
    Ok(addresses)
}

fn validate_ip(ip: IpAddr) -> Result<(), PluginError> {
    match ip {
        IpAddr::V4(ip) => {
            if ip.is_loopback() && allow_loopback_for_tests() {
                return Ok(());
            }
            let octets = ip.octets();
            let is_shared_range = octets[0] == 100 && (64..=127).contains(&octets[1]);
            if ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || is_shared_range
                || ip.octets() == [169, 254, 169, 254]
            {
                return Err(PluginError::InvalidInput(format!("destination address `{ip}` is blocked")));
            }
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return validate_ip(IpAddr::V4(mapped));
            }
            if ip.is_loopback() && allow_loopback_for_tests() {
                return Ok(());
            }
            if ip.is_loopback()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
            {
                return Err(PluginError::InvalidInput(format!("destination address `{ip}` is blocked")));
            }
        }
    }
    Ok(())
}

fn allow_loopback_for_tests() -> bool {
    cfg!(debug_assertions)
        && matches!(std::env::var("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS").as_deref(), Ok("1"))
        && matches!(std::env::var("CHAINBOT_INTERNAL_ALLOW_TEST_DESTINATIONS").as_deref(), Ok("1"))
}
