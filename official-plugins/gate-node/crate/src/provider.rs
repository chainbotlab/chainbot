use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use reqwest::{header::HeaderValue, Client, Method, Response};
use serde_json::{json, Value};
use sha2::{Digest, Sha512};
use url::{form_urlencoded, Url};

use crate::errors::PluginError;

type HmacSha512 = Hmac<Sha512>;
const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.azure.internal"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Mainnet,
    Testnet,
}

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub base_url: String,
    pub allowed_origins: Vec<String>,
}

impl Environment {
    pub fn parse(value: Option<&str>) -> Result<Self, PluginError> {
        match value.unwrap_or("mainnet").trim().to_ascii_lowercase().as_str() {
            "mainnet" | "prod" | "production" => Ok(Self::Mainnet),
            "testnet" | "test" | "demo" => Ok(Self::Testnet),
            other => Err(PluginError::InvalidInput(format!(
                "environment must be mainnet or testnet, got {other}"
            ))),
        }
    }
}

impl RequestContext {
    pub fn new(
        environment: Option<&str>,
        base_url: Option<&str>,
        allowed_origins: Vec<String>,
    ) -> Result<Self, PluginError> {
        let environment = Environment::parse(environment)?;
        let base_url = match base_url {
            Some(value) if !value.trim().is_empty() => {
                Url::parse(value).map_err(|error| {
                    PluginError::InvalidInput(format!("invalid base_url: {error}"))
                })?;
                value.trim_end_matches('/').to_owned()
            }
            _ => default_base_url(environment).to_owned(),
        };
        Ok(Self {
            base_url,
            allowed_origins,
        })
    }
}

fn default_base_url(environment: Environment) -> &'static str {
    match environment {
        Environment::Mainnet => "https://api.gateio.ws",
        Environment::Testnet => "https://api-testnet.gateapi.io",
    }
}

pub async fn public_get(
    client: &Client,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
) -> Result<Value, PluginError> {
    send_json_request(client, Method::GET, context, path, query, None, None, None, false).await
}

pub async fn signed_get(
    client: &Client,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
    api_key: &str,
    api_secret: &str,
) -> Result<Value, PluginError> {
    send_json_request(
        client,
        Method::GET,
        context,
        path,
        query,
        Some(api_key),
        Some(api_secret),
        None,
        true,
    )
    .await
}

pub async fn signed_post(
    client: &Client,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
    body: Value,
    api_key: &str,
    api_secret: &str,
) -> Result<Value, PluginError> {
    send_json_request(
        client,
        Method::POST,
        context,
        path,
        query,
        Some(api_key),
        Some(api_secret),
        Some(body),
        true,
    )
    .await
}

pub async fn signed_delete(
    client: &Client,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
    api_key: &str,
    api_secret: &str,
) -> Result<Value, PluginError> {
    send_json_request(
        client,
        Method::DELETE,
        context,
        path,
        query,
        Some(api_key),
        Some(api_secret),
        None,
        true,
    )
    .await
}

fn build_url(context: &RequestContext, path: &str, query: Vec<(String, String)>) -> Result<Url, PluginError> {
    let mut url = Url::parse(&context.base_url)
        .map_err(|error| PluginError::InvalidInput(format!("invalid base_url: {error}")))?;
    url.set_path(path);
    if !query.is_empty() {
        url.set_query(Some(&encode_query(&query)));
    }
    Ok(url)
}

async fn send_json_request(
    client: &Client,
    method: Method,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
    api_key: Option<&str>,
    api_secret: Option<&str>,
    body: Option<Value>,
    sign: bool,
) -> Result<Value, PluginError> {
    let url = build_url(context, path, query)?;
    validate_destination_policy(&url)?;
    validate_allowed_origins(&url, &context.allowed_origins, api_key.is_some())?;

    let body_text = if let Some(payload) = body {
        Some(serde_json::to_string(&payload).map_err(PluginError::Json)?)
    } else {
        None
    };

    let mut request = client.request(method.clone(), url.clone());

    if sign {
        let key = api_key.ok_or_else(|| PluginError::InvalidInput(String::from("api_key is required")))?;
        let secret = api_secret.ok_or_else(|| PluginError::InvalidInput(String::from("api_secret is required")))?;
        let timestamp = current_time_seconds()?.to_string();
        let signature = sign_gate_request(
            method.as_str(),
            url.path(),
            url.query().unwrap_or(""),
            body_text.as_deref().unwrap_or(""),
            &timestamp,
            secret,
        )?;

        request = request
            .header(
                "KEY",
                HeaderValue::from_str(key)
                    .map_err(|error| PluginError::InvalidInput(format!("invalid api_key header: {error}")))?,
            )
            .header("Timestamp", HeaderValue::from_str(&timestamp).map_err(|error| {
                PluginError::InvalidInput(format!("invalid timestamp header: {error}"))
            })?)
            .header("SIGN", HeaderValue::from_str(&signature).map_err(|error| {
                PluginError::InvalidInput(format!("invalid sign header: {error}"))
            })?);
    }

    if let Some(body) = body_text {
        request = request
            .header("Content-Type", "application/json")
            .body(body);
    }

    parse_json_response(request.send().await?).await
}

fn sign_gate_request(
    method: &str,
    path: &str,
    query: &str,
    body: &str,
    timestamp: &str,
    secret: &str,
) -> Result<String, PluginError> {
    let body_hash = hex::encode(Sha512::digest(body.as_bytes()));
    let payload = format!("{method}\n{path}\n{query}\n{body_hash}\n{timestamp}");
    let mut mac = HmacSha512::new_from_slice(secret.as_bytes())
        .map_err(|error| PluginError::Signing(error.to_string()))?;
    mac.update(payload.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

async fn parse_json_response(response: Response) -> Result<Value, PluginError> {
    let status = response.status();
    let body = response.text().await?;
    if body.trim().is_empty() {
        if status.is_success() {
            return Ok(json!({}));
        }
        return Err(PluginError::Rpc(format!(
            "gate http {} returned empty response",
            status.as_u16()
        )));
    }

    let payload: Value = serde_json::from_str(&body)
        .map_err(|error| PluginError::Rpc(format!("gate returned invalid json: {error}")))?;

    if status.is_success() {
        return Ok(payload);
    }

    let label = payload
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("request_failed");
    let message = payload
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("request failed");
    Err(PluginError::Rpc(format!(
        "gate http {} {}: {}",
        status.as_u16(),
        label,
        message
    )))
}

fn encode_query(query: &[(String, String)]) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (key, value) in query {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

fn current_time_seconds() -> Result<i64, PluginError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| PluginError::Rpc(error.to_string()))?;
    i64::try_from(duration.as_secs()).map_err(|error| PluginError::Rpc(error.to_string()))
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
            "activation secrets require at least one allowed origin",
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
    if !permitted {
        return Err(PluginError::InvalidInput(format!(
            "request origin {} is not allowlisted for activation secrets",
            request_origin
        )));
    }
    Ok(())
}

fn normalize_origin(url: &Url) -> Result<String, PluginError> {
    let scheme = url.scheme();
    let host = url
        .host_str()
        .ok_or_else(|| PluginError::InvalidInput(String::from("base_url must include a host")))?;
    let port = url.port_or_known_default().ok_or_else(|| {
        PluginError::InvalidInput(String::from("base_url must use a known port for its scheme"))
    })?;
    Ok(format!("{scheme}://{host}:{port}"))
}

fn validate_destination_policy(url: &Url) -> Result<Vec<SocketAddr>, PluginError> {
    if !matches!(url.scheme(), "https")
        && !(matches!(url.scheme(), "http") && allow_loopback_for_tests() && is_loopback_host(url.host_str()))
    {
        return Err(PluginError::InvalidInput(String::from("base_url must use https")));
    }
    let host = url
        .host_str()
        .ok_or_else(|| PluginError::InvalidInput(String::from("base_url must include a host")))?;
    if BLOCKED_HOSTS.iter().any(|blocked| host.eq_ignore_ascii_case(blocked)) {
        return Err(PluginError::InvalidInput(format!(
            "destination host `{host}` is blocked"
        )));
    }
    let port = url.port_or_known_default().ok_or_else(|| {
        PluginError::InvalidInput(String::from("base_url must use a known port for its scheme"))
    })?;
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
            if ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || is_shared_range
                || ip.octets() == [169, 254, 169, 254]
            {
                return Err(PluginError::InvalidInput(format!(
                    "destination address `{ip}` is blocked"
                )));
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
                return Err(PluginError::InvalidInput(format!(
                    "destination address `{ip}` is blocked"
                )));
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
