use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, Mac};
use reqwest::{header::HeaderValue, Client, Method};
use serde_json::Value;
use sha2::Sha256;
use url::Url;

use crate::errors::PluginError;

type HmacSha256 = Hmac<Sha256>;
const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.azure.internal"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductLine {
    Spot,
    Futures,
}

pub struct RequestContext {
    pub product_line: ProductLine,
    pub base_url: String,
    pub allowed_origins: Vec<String>,
}

impl ProductLine {
    pub fn parse(value: &str) -> Result<Self, PluginError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "spot" => Ok(Self::Spot),
            "futures" | "usdt-futures" | "coin-futures" | "usdc-futures" | "mix" => Ok(Self::Futures),
            other => Err(PluginError::InvalidInput(format!("product_line must be spot or futures, got {other}"))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Futures => "futures",
        }
    }
}

impl RequestContext {
    pub fn new(product_line: &str, base_url: Option<&str>, allowed_origins: Vec<String>) -> Result<Self, PluginError> {
        let product_line = ProductLine::parse(product_line)?;
        let base_url = match base_url {
            Some(value) if !value.trim().is_empty() => value.trim_end_matches('/').to_owned(),
            _ => String::from("https://api.bitget.com"),
        };
        Ok(Self { product_line, base_url, allowed_origins })
    }
}

pub async fn public_get(client: &Client, context: &RequestContext, path: &str, query: &[(String, String)]) -> Result<Value, PluginError> {
    send_request(client, Method::GET, context, path, query, None, None, None, None).await
}

pub async fn signed_request(
    client: &Client,
    method: Method,
    context: &RequestContext,
    path: &str,
    query: &[(String, String)],
    body: Option<&str>,
    api_key: &str,
    api_secret: &str,
    passphrase: &str,
) -> Result<Value, PluginError> {
    send_request(
        client,
        method,
        context,
        path,
        query,
        body,
        Some(api_key),
        Some(api_secret),
        Some(passphrase),
    )
    .await
}

async fn send_request(
    client: &Client,
    method: Method,
    context: &RequestContext,
    path: &str,
    query: &[(String, String)],
    body: Option<&str>,
    api_key: Option<&str>,
    api_secret: Option<&str>,
    passphrase: Option<&str>,
) -> Result<Value, PluginError> {
    let mut url = Url::parse(&context.base_url).map_err(|e| PluginError::InvalidInput(format!("invalid base_url: {e}")))?;
    url.set_path(path);
    if !query.is_empty() {
        let encoded = query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
        url.set_query(Some(&encoded));
    }

    validate_destination_policy(&url)?;
    validate_allowed_origins(&url, &context.allowed_origins, api_key.is_some())?;

    let mut request = client.request(method.clone(), url.clone());
    let body_value = body.unwrap_or("");
    if let Some(key) = api_key {
        let ts = current_time_ms()?.to_string();
        let sign = build_signature(&ts, method.as_str(), path, url.query(), body_value, api_secret.ok_or_else(|| PluginError::InvalidInput(String::from("api_secret is required")))?)?;
        request = request
            .header("ACCESS-KEY", HeaderValue::from_str(key).map_err(|e| PluginError::InvalidInput(format!("invalid ACCESS-KEY header: {e}")))?)
            .header("ACCESS-SIGN", sign)
            .header("ACCESS-TIMESTAMP", ts)
            .header("ACCESS-PASSPHRASE", passphrase.unwrap_or_default());
    }
    if !body_value.is_empty() {
        request = request.body(body_value.to_owned()).header("Content-Type", "application/json");
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    let payload: Value = serde_json::from_str(&text).unwrap_or(Value::String(text.clone()));
    if status.is_success() {
        Ok(payload)
    } else {
        Err(PluginError::Rpc(format!("bitget http {}: {}", status.as_u16(), text)))
    }
}

fn build_signature(timestamp: &str, method: &str, path: &str, query: Option<&str>, body: &str, secret: &str) -> Result<String, PluginError> {
    let mut prehash = format!("{timestamp}{method}{path}");
    if let Some(q) = query {
        prehash.push('?');
        prehash.push_str(q);
    }
    prehash.push_str(body);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|e| PluginError::Signing(e.to_string()))?;
    mac.update(prehash.as_bytes());
    Ok(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}

fn current_time_ms() -> Result<i64, PluginError> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| PluginError::Rpc(e.to_string()))?;
    i64::try_from(duration.as_millis()).map_err(|e| PluginError::Rpc(e.to_string()))
}

fn validate_allowed_origins(url: &Url, allowed_origins: &[String], requires_binding: bool) -> Result<(), PluginError> {
    if !requires_binding {
        return Ok(());
    }
    if allowed_origins.is_empty() {
        return Err(PluginError::InvalidInput(String::from("activation secrets require at least one allowed origin")));
    }
    let origin = normalize_origin(url)?;
    let permitted = allowed_origins.iter().any(|item| Url::parse(item).ok().and_then(|v| normalize_origin(&v).ok()).as_deref() == Some(origin.as_str()));
    if permitted {
        Ok(())
    } else {
        Err(PluginError::InvalidInput(format!("request origin {} is not allowlisted for activation secrets", origin)))
    }
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
            if ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_multicast() || ip.is_unspecified() {
                return Err(PluginError::InvalidInput(format!("destination address `{ip}` is blocked")));
            }
        }
        IpAddr::V6(ip) => {
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
