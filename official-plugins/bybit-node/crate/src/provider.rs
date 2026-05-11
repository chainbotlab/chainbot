use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use reqwest::{header::HeaderValue, Client, Method, Response};
use serde_json::{json, Value};
use sha2::Sha256;
use url::{form_urlencoded, Url};

use crate::errors::PluginError;

type HmacSha256 = Hmac<Sha256>;
const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.azure.internal"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductLine {
    Spot,
    Linear,
    Inverse,
    Option,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Mainnet,
    Testnet,
}

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub product_line: ProductLine,
    pub environment: Environment,
    pub base_url: String,
    pub allowed_origins: Vec<String>,
}

impl ProductLine {
    pub fn parse(value: &str) -> Result<Self, PluginError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "spot" => Ok(Self::Spot),
            "linear" => Ok(Self::Linear),
            "inverse" => Ok(Self::Inverse),
            "option" => Ok(Self::Option),
            other => Err(PluginError::InvalidInput(format!(
                "product_line must be one of spot, linear, inverse, option, got {other}"
            ))),
        }
    }

    pub fn as_category(&self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Linear => "linear",
            Self::Inverse => "inverse",
            Self::Option => "option",
        }
    }
}

impl Environment {
    pub fn parse(value: Option<&str>) -> Result<Self, PluginError> {
        match value.unwrap_or("mainnet").trim().to_ascii_lowercase().as_str() {
            "mainnet" | "prod" | "production" => Ok(Self::Mainnet),
            "testnet" | "test" => Ok(Self::Testnet),
            other => Err(PluginError::InvalidInput(format!(
                "environment must be mainnet or testnet, got {other}"
            ))),
        }
    }
}

impl RequestContext {
    pub fn new(
        product_line: &str,
        environment: Option<&str>,
        base_url: Option<&str>,
        allowed_origins: Vec<String>,
    ) -> Result<Self, PluginError> {
        let product_line = ProductLine::parse(product_line)?;
        let environment = Environment::parse(environment)?;
        let base_url = match base_url {
            Some(value) if !value.trim().is_empty() => {
                Url::parse(value).map_err(|error| {
                    PluginError::InvalidInput(format!("invalid base_url: {error}"))
                })?;
                value.trim_end_matches('/').to_owned()
            }
            _ => match environment {
                Environment::Mainnet => String::from("https://api.bybit.com"),
                Environment::Testnet => String::from("https://api-testnet.bybit.com"),
            },
        };
        Ok(Self { product_line, environment, base_url, allowed_origins })
    }
}

pub async fn public_get(client: &Client, context: &RequestContext, path: &str, query: Vec<(String, String)>) -> Result<Value, PluginError> {
    send_json_request(client, Method::GET, context, path, query, None, None).await
}

pub async fn signed_get(client: &Client, context: &RequestContext, path: &str, query: Vec<(String, String)>, api_key: &str, api_secret: &str, recv_window: &str) -> Result<Value, PluginError> {
    send_json_request(client, Method::GET, context, path, query, Some((api_key, api_secret, recv_window)), None).await
}

pub async fn signed_post(client: &Client, context: &RequestContext, path: &str, body: Value, api_key: &str, api_secret: &str, recv_window: &str) -> Result<Value, PluginError> {
    send_json_request(client, Method::POST, context, path, vec![], Some((api_key, api_secret, recv_window)), Some(body)).await
}

fn signed_headers(
    method: &Method,
    query_string: &str,
    body_string: &str,
    api_key: &str,
    api_secret: &str,
    recv_window: &str,
) -> Result<Vec<(&'static str, String)>, PluginError> {
    let timestamp = current_time_ms()?.to_string();
    let payload = if *method == Method::GET {
        format!("{timestamp}{api_key}{recv_window}{query_string}")
    } else {
        format!("{timestamp}{api_key}{recv_window}{body_string}")
    };
    let signature = sign_payload(&payload, api_secret)?;
    Ok(vec![
        ("X-BAPI-API-KEY", api_key.to_owned()),
        ("X-BAPI-TIMESTAMP", timestamp),
        ("X-BAPI-SIGN", signature),
        ("X-BAPI-RECV-WINDOW", recv_window.to_owned()),
        ("X-BAPI-SIGN-TYPE", String::from("2")),
    ])
}

async fn send_json_request(
    client: &Client,
    method: Method,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
    signing: Option<(&str, &str, &str)>,
    body: Option<Value>,
) -> Result<Value, PluginError> {
    let mut url = Url::parse(&context.base_url).map_err(|error| PluginError::InvalidInput(format!("invalid base_url: {error}")))?;
    url.set_path(path);
    let query_string = encode_query(&query);
    if !query_string.is_empty() {
        url.set_query(Some(&query_string));
    }

    validate_destination_policy(&url)?;
    validate_allowed_origins(&url, &context.allowed_origins, signing.is_some())?;

    let body_string = body.as_ref().map(|v| v.to_string()).unwrap_or_default();
    let mut request = client.request(method.clone(), url);
    if let Some((api_key, api_secret, recv_window)) = signing {
        for (key, value) in signed_headers(&method, &query_string, &body_string, api_key, api_secret, recv_window)? {
            request = request.header(
                key,
                HeaderValue::from_str(&value)
                    .map_err(|error| PluginError::InvalidInput(format!("invalid signing header: {error}")))?,
            );
        }
    }
    if let Some(value) = body {
        request = request.header("content-type", "application/json").body(value.to_string());
    }
    parse_json_response(request.send().await?).await
}

async fn parse_json_response(response: Response) -> Result<Value, PluginError> {
    let status = response.status();
    let body = response.text().await?;
    if body.trim().is_empty() {
        return Err(PluginError::Rpc(format!("bybit http {} returned empty response", status.as_u16())));
    }
    let payload: Value = serde_json::from_str(&body).map_err(|error| PluginError::Rpc(format!("bybit returned invalid json: {error}")))?;
    if !status.is_success() {
        return Err(PluginError::Rpc(format!("bybit http {}: {}", status.as_u16(), payload)));
    }
    let ret_code = payload.get("retCode").and_then(Value::as_i64).unwrap_or(0);
    if ret_code != 0 {
        let ret_msg = payload.get("retMsg").and_then(Value::as_str).unwrap_or("request failed");
        return Err(PluginError::Rpc(format!("bybit retCode {ret_code}: {ret_msg}")));
    }
    Ok(payload.get("result").cloned().unwrap_or(json!({})))
}

fn encode_query(query: &[(String, String)]) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (k, v) in query {
        serializer.append_pair(k, v);
    }
    serializer.finish()
}

fn sign_payload(payload: &str, secret: &str) -> Result<String, PluginError> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|error| PluginError::Signing(error.to_string()))?;
    mac.update(payload.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn current_time_ms() -> Result<i64, PluginError> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| PluginError::Rpc(error.to_string()))?;
    i64::try_from(duration.as_millis()).map_err(|error| PluginError::Rpc(error.to_string()))
}

fn validate_allowed_origins(url: &Url, allowed_origins: &[String], requires_binding: bool) -> Result<(), PluginError> {
    if !requires_binding {
        return Ok(());
    }
    if allowed_origins.is_empty() {
        return Err(PluginError::InvalidInput(String::from("activation secrets require at least one allowed origin")));
    }
    let request_origin = normalize_origin(url)?;
    let permitted = allowed_origins.iter().any(|origin| {
        Url::parse(origin)
            .ok()
            .and_then(|parsed| normalize_origin(&parsed).ok())
            .as_deref()
            == Some(request_origin.as_str())
    });
    if !permitted {
        return Err(PluginError::InvalidInput(format!("request origin {} is not allowlisted for activation secrets", request_origin)));
    }
    Ok(())
}

fn normalize_origin(url: &Url) -> Result<String, PluginError> {
    let scheme = url.scheme();
    let host = url.host_str().ok_or_else(|| PluginError::InvalidInput(String::from("base_url must include a host")))?;
    let port = url.port_or_known_default().ok_or_else(|| PluginError::InvalidInput(String::from("base_url must use a known port for its scheme")))?;
    Ok(format!("{scheme}://{host}:{port}"))
}

fn validate_destination_policy(url: &Url) -> Result<Vec<SocketAddr>, PluginError> {
    if !matches!(url.scheme(), "https")
        && !(matches!(url.scheme(), "http") && allow_loopback_for_tests() && is_loopback_host(url.host_str()))
    {
        return Err(PluginError::InvalidInput(String::from("base_url must use https")));
    }
    let host = url.host_str().ok_or_else(|| PluginError::InvalidInput(String::from("base_url must include a host")))?;
    if BLOCKED_HOSTS.iter().any(|blocked| host.eq_ignore_ascii_case(blocked)) {
        return Err(PluginError::InvalidInput(format!("destination host `{host}` is blocked")));
    }
    let port = url.port_or_known_default().ok_or_else(|| PluginError::InvalidInput(String::from("base_url must use a known port for its scheme")))?;
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| PluginError::Rpc(format!("failed to resolve destination host: {error}")))?
        .collect::<Vec<_>>();
    for address in &addresses {
        validate_ip(address.ip())?;
    }
    Ok(addresses)
}

fn is_loopback_host(host: Option<&str>) -> bool {
    matches!(host, Some("localhost") | Some("127.0.0.1") | Some("::1"))
}

fn validate_ip(ip: IpAddr) -> Result<(), PluginError> {
    match ip {
        IpAddr::V4(ip) => {
            if ip.is_loopback() && allow_loopback_for_tests() {
                return Ok(());
            }
            let octets = ip.octets();
            let is_shared_range = octets[0] == 100 && (64..=127).contains(&octets[1]);
            if ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_multicast() || ip.is_unspecified() || is_shared_range || ip.octets() == [169, 254, 169, 254] {
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
            if ip.is_loopback() || ip.is_multicast() || ip.is_unspecified() || ip.is_unique_local() || ip.is_unicast_link_local() {
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
