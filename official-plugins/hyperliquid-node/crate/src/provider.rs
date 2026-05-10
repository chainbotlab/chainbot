use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use reqwest::{Client, Url};
use serde_json::Value;

use crate::errors::PluginError;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.azure.internal"];

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub base_url: Url,
    pub allowed_origins: Vec<String>,
    pub requires_origin_binding: bool,
}

impl RequestContext {
    pub fn new(base_url: Option<&str>, allowed_origins: &[String]) -> Result<Self, PluginError> {
        let requires_origin_binding = base_url.is_some();
        let raw = base_url.unwrap_or("https://api.hyperliquid.xyz");
        let base_url = Url::parse(raw)
            .map_err(|error| PluginError::InvalidInput(format!("invalid base_url: {error}")))?;
        validate_destination_policy(&base_url, allowed_origins, requires_origin_binding)?;
        Ok(Self {
            base_url,
            allowed_origins: allowed_origins.to_vec(),
            requires_origin_binding,
        })
    }
}

pub async fn post_info(
    _client: &Client,
    context: &RequestContext,
    body: Value,
) -> Result<Value, PluginError> {
    let url = context
        .base_url
        .join("/info")
        .map_err(|error| PluginError::InvalidInput(format!("invalid info endpoint: {error}")))?;
    validate_destination_policy(&url, &context.allowed_origins, context.requires_origin_binding)?;

    let host = url.host_str().ok_or_else(|| PluginError::InvalidInput(String::from("info url must include host")))?;
    let resolved_addresses = resolve_destination(&url)?;
    let client = Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &resolved_addresses)
        .build()
        .map_err(|source| PluginError::Rpc(format!("failed to build HTTP client: {source}")))?;

    let response = client
        .post(url)
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(PluginError::Rpc(format!("hyperliquid info request failed with status {status}: {text}")));
    }
    serde_json::from_str(&text)
        .map_err(|error| PluginError::Rpc(format!("hyperliquid info response was not valid json: {error}")))
}

fn resolve_destination(url: &Url) -> Result<Vec<SocketAddr>, PluginError> {
    let host = url.host_str().ok_or_else(|| PluginError::InvalidInput(String::from("destination url must include a host")))?;
    let port = url.port_or_known_default().ok_or_else(|| {
        PluginError::InvalidInput(String::from("destination url must use a known port for its scheme"))
    })?;
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|source| PluginError::Rpc(format!("failed to resolve destination host: {source}")))?
        .collect::<Vec<_>>();
    for address in &addresses {
        validate_ip(address.ip())?;
    }
    Ok(addresses)
}

fn validate_destination_policy(url: &Url, allowed_origins: &[String], requires_binding: bool) -> Result<(), PluginError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(PluginError::InvalidInput(String::from(
            "destination url must use http or https",
        )));
    }
    validate_allowed_origins(url, allowed_origins, requires_binding)?;
    let host = url.host_str().ok_or_else(|| PluginError::InvalidInput(String::from("destination url must include a host")))?;
    if BLOCKED_HOSTS.iter().any(|blocked| host.eq_ignore_ascii_case(blocked)) {
        return Err(PluginError::InvalidInput(format!("destination host `{host}` is blocked")));
    }
    let _ = resolve_destination(url)?;
    Ok(())
}

fn validate_allowed_origins(url: &Url, allowed_origins: &[String], requires_binding: bool) -> Result<(), PluginError> {
    if !requires_binding {
        return Ok(());
    }
    if allowed_origins.is_empty() {
        return Err(PluginError::InvalidInput(String::from(
            "activation origin binding requires at least one allowed origin",
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
            "request origin {request_origin} is not allowlisted for activation origin binding"
        )))
    }
}

fn normalize_origin(url: &Url) -> Result<String, PluginError> {
    let scheme = match url.scheme() {
        "http" => "http",
        "https" => "https",
        other => {
            return Err(PluginError::InvalidInput(format!(
                "unsupported origin scheme {other}"
            )))
        }
    };
    let host = url.host_str().ok_or_else(|| PluginError::InvalidInput(String::from("destination url must include a host")))?;
    let port = url.port_or_known_default().ok_or_else(|| {
        PluginError::InvalidInput(String::from("destination url must use a known port for its scheme"))
    })?;
    Ok(format!("{scheme}://{host}:{port}"))
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

pub fn output_map(key: &str, value: Value) -> BTreeMap<String, Value> {
    BTreeMap::from([(key.to_owned(), value)])
}
