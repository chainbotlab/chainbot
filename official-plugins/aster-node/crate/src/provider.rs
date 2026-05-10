use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use reqwest::{Client, Url};
use serde_json::Value;

use crate::errors::PluginError;

const DEFAULT_BASE_URL: &str = "https://fapi.asterdex.com";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BODY_BYTES: usize = 64 * 1024;
const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.azure.internal"];

pub async fn get_server_time(base_url_override: Option<&str>) -> Result<BTreeMap<String, Value>, PluginError> {
    let base_url = base_url_override
        .map(normalize_base_url)
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
    let url = Url::parse(&format!("{base_url}/fapi/v3/time")).map_err(|source| {
        PluginError::InvalidRequest(format!("invalid input `base_url`: {source}"))
    })?;
    let resolved_addresses = validate_destination_policy(&url)?;
    let host = url.host_str().ok_or_else(|| {
        PluginError::InvalidRequest("request url must include a host".to_owned())
    })?;

    let client = Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &resolved_addresses)
        .build()
        .map_err(|source| PluginError::RequestFailed(format!("failed to build HTTP client: {source}")))?;

    let response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|source| PluginError::RequestFailed(format!("request failed: {source}")))?;

    if response.status().is_redirection() && response.headers().contains_key(reqwest::header::LOCATION) {
        return Err(PluginError::RequestFailed(
            "redirect responses are not supported".to_owned(),
        ));
    }

    let content_length = response.content_length().unwrap_or(0);
    if content_length > MAX_RESPONSE_BODY_BYTES as u64 {
        return Err(PluginError::RequestFailed(format!(
            "response body exceeds {MAX_RESPONSE_BODY_BYTES} bytes"
        )));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !content_type.is_empty() && !is_json_content_type(&content_type) {
        return Err(PluginError::RequestFailed(format!(
            "unsupported response content-type {content_type}"
        )));
    }

    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|source| PluginError::RequestFailed(format!("failed to read response body: {source}")))?;
    if body.len() > MAX_RESPONSE_BODY_BYTES {
        return Err(PluginError::RequestFailed(format!(
            "response body exceeds {MAX_RESPONSE_BODY_BYTES} bytes"
        )));
    }
    if !status.is_success() {
        return Err(PluginError::RequestFailed(format!(
            "Aster server time request failed with status {}",
            status.as_u16()
        )));
    }

    let payload: Value = serde_json::from_slice(&body).map_err(|source| {
        PluginError::RequestFailed(format!("response body is not valid JSON: {source}"))
    })?;
    let server_time = payload
        .get("serverTime")
        .and_then(Value::as_i64)
        .or_else(|| payload.get("serverTime").and_then(Value::as_u64).and_then(|value| i64::try_from(value).ok()))
        .ok_or_else(|| {
            PluginError::RequestFailed("response body missing integer field `serverTime`".to_owned())
        })?;

    Ok(BTreeMap::from([(
        String::from("server_time"),
        Value::from(server_time),
    )]))
}

fn normalize_base_url(input: &str) -> String {
    input.trim_end_matches('/').to_owned()
}

fn is_json_content_type(content_type: &str) -> bool {
    content_type.starts_with("application/json") || content_type.contains("+json") || content_type.starts_with("text/json")
}

fn validate_destination_policy(url: &Url) -> Result<Vec<SocketAddr>, PluginError> {
    if !matches!(url.scheme(), "https")
        && !(matches!(url.scheme(), "http") && allow_loopback_for_tests() && is_loopback_host(url.host_str()))
    {
        return Err(PluginError::InvalidPolicy(
            "request url must use https".to_owned(),
        ));
    }
    let host = url.host_str().ok_or_else(|| {
        PluginError::InvalidRequest("request url must include a host".to_owned())
    })?;
    if BLOCKED_HOSTS.iter().any(|blocked| host.eq_ignore_ascii_case(blocked)) {
        return Err(PluginError::InvalidPolicy(format!(
            "destination host `{host}` is blocked"
        )));
    }
    let port = url.port_or_known_default().ok_or_else(|| {
        PluginError::InvalidRequest("request url must use a known port for its scheme".to_owned())
    })?;

    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|source| PluginError::RequestFailed(format!("failed to resolve destination host: {source}")))?
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
                return Err(PluginError::InvalidPolicy(format!(
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
                return Err(PluginError::InvalidPolicy(format!(
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
