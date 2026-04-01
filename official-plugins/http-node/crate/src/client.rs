use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Method, Url};
use serde_json::Value;

use crate::errors::PluginError;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BODY_BYTES: usize = 256 * 1024;
const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.azure.internal"];
const BLOCKED_RESPONSE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "set-cookie",
    "set-cookie2",
];

fn allow_loopback_for_tests() -> bool {
    std::env::var_os("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS").is_some()
}

#[derive(Debug)]
pub struct HttpRequestSpec {
    pub url: Url,
    pub method: Method,
    pub headers: HeaderMap,
    pub body: Option<Vec<u8>>,
    pub activation_authorization: Option<String>,
    pub allowed_origins: Vec<String>,
}

pub async fn execute(spec: HttpRequestSpec) -> Result<BTreeMap<String, Value>, PluginError> {
    let resolved_addresses = validate_destination_policy(&spec.url)?;
    validate_allowed_origins(&spec.url, &spec.allowed_origins, spec.activation_authorization.is_some())?;

    let host = spec.url.host_str().ok_or_else(|| {
        PluginError::InvalidRequest("input `url` must include a host".to_owned())
    })?;

    let client = Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &resolved_addresses)
        .build()
        .map_err(|source| PluginError::RequestFailed(format!("failed to build HTTP client: {source}")))?;

    let mut request = client.request(spec.method.clone(), spec.url.clone());
    if !spec.headers.is_empty() {
        request = request.headers(spec.headers.clone());
    }
    if let Some(authorization) = spec.activation_authorization.as_deref() {
        request = request.header("authorization", authorization);
    }
    if let Some(body) = spec.body {
        request = request.body(body);
    }

    let response = request
        .send()
        .await
        .map_err(|source| PluginError::RequestFailed(format!("request failed: {source}")))?;

    let status = response.status();
    if status.is_redirection() && response.headers().contains_key(reqwest::header::LOCATION) {
        return Err(PluginError::RequestFailed(
            "redirect responses are not supported".to_owned(),
        ));
    }
    let final_url = response.url().to_string();
    let content_length = response.content_length().unwrap_or(0);
    if content_length > MAX_RESPONSE_BODY_BYTES as u64 {
        return Err(PluginError::RequestFailed(format!(
            "response body exceeds {} bytes",
            MAX_RESPONSE_BODY_BYTES
        )));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !content_type.is_empty() && !is_text_content_type(&content_type) {
        return Err(PluginError::RequestFailed(format!(
            "unsupported response content-type {content_type}"
        )));
    }

    let response_headers = response.headers().clone();
    let body_bytes = response
        .bytes()
        .await
        .map_err(|source| PluginError::RequestFailed(format!("failed to read response body: {source}")))?;
    if body_bytes.len() > MAX_RESPONSE_BODY_BYTES {
        return Err(PluginError::RequestFailed(format!(
            "response body exceeds {} bytes",
            MAX_RESPONSE_BODY_BYTES
        )));
    }
    let body = String::from_utf8(body_bytes.to_vec())
        .map_err(|_| PluginError::RequestFailed("response body is not valid UTF-8 text".to_owned()))?;

    let headers = response_headers
        .iter()
        .filter_map(|(key, value)| {
            let lower = key.as_str().to_ascii_lowercase();
            if BLOCKED_RESPONSE_HEADERS.iter().any(|blocked| *blocked == lower) {
                return None;
            }
            Some((
                key.as_str().to_owned(),
                Value::String(value.to_str().unwrap_or_default().to_owned()),
            ))
        })
        .collect::<serde_json::Map<String, Value>>();

    Ok(BTreeMap::from([
        (String::from("status"), Value::from(status.as_u16())),
        (String::from("ok"), Value::from(status.is_success())),
        (String::from("url"), Value::String(final_url)),
        (String::from("body"), Value::String(body)),
        (String::from("headers"), Value::Object(headers)),
    ]))
}

pub fn parse_method(raw: Option<&str>) -> Result<Method, PluginError> {
    let raw = raw.unwrap_or("GET");
    Method::from_bytes(raw.as_bytes())
        .map_err(|source| PluginError::InvalidRequest(format!("invalid HTTP method: {source}")))
}

pub fn parse_headers(value: Option<&serde_json::Value>) -> Result<HeaderMap, PluginError> {
    let Some(value) = value else {
        return Ok(HeaderMap::new());
    };
    let object = value.as_object().ok_or_else(|| {
        PluginError::InvalidRequest("input `headers` must be a JSON object".to_owned())
    })?;

    let mut headers = HeaderMap::new();
    for (name, raw) in object {
        if name.eq_ignore_ascii_case("host") {
            return Err(PluginError::InvalidRequest(
                "workflow headers must not set `host`".to_owned(),
            ));
        }
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            PluginError::InvalidRequest(format!("invalid header name `{name}`: {source}"))
        })?;
        let header_value = raw.as_str().ok_or_else(|| {
            PluginError::InvalidRequest(format!("header `{name}` must be a string"))
        })?;
        if header_value.trim_start().starts_with("secret://") {
            return Err(PluginError::InvalidRequest(format!(
                "header `{name}` must not contain secret references; use activation secrets"
            )));
        }
        let header_value = HeaderValue::from_str(header_value).map_err(|source| {
            PluginError::InvalidRequest(format!("invalid header value for `{name}`: {source}"))
        })?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

pub fn encode_body(value: Option<&serde_json::Value>) -> Result<Option<Vec<u8>>, PluginError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let bytes = match value {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::String(text) => {
            if text.trim_start().starts_with("secret://") {
                return Err(PluginError::InvalidRequest(
                    "input `body` must not contain secret references; use activation secrets"
                        .to_owned(),
                ));
            }
            text.as_bytes().to_vec()
        }
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => serde_json::to_vec(value)
            .map_err(PluginError::from)?,
        other => other.to_string().into_bytes(),
    };
    Ok(Some(bytes))
}

fn is_text_content_type(content_type: &str) -> bool {
    content_type.starts_with("text/")
        || content_type.starts_with("application/json")
        || content_type.contains("+json")
        || content_type.starts_with("application/xml")
        || content_type.contains("+xml")
        || content_type.starts_with("application/x-www-form-urlencoded")
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
        return Err(PluginError::InvalidPolicy(
            "activation secrets require at least one allowed origin".to_owned(),
        ));
    }

    let request_origin = normalize_origin(url)?;
    let permitted = allowed_origins.iter().any(|origin| {
        reqwest::Url::parse(origin)
            .ok()
            .and_then(|parsed| normalize_origin(&parsed).ok())
            .as_deref()
            == Some(request_origin.as_str())
    });
    if !permitted {
        return Err(PluginError::InvalidPolicy(format!(
            "request origin {} is not allowlisted for activation secrets",
            request_origin
        )));
    }
    Ok(())
}

fn normalize_origin(url: &Url) -> Result<String, PluginError> {
    let scheme = url.scheme();
    let host = url.host_str().ok_or_else(|| {
        PluginError::InvalidRequest("input `url` must include a host".to_owned())
    })?;
    let port = url.port_or_known_default().ok_or_else(|| {
        PluginError::InvalidRequest("input `url` must use a known port for its scheme".to_owned())
    })?;
    Ok(format!("{scheme}://{host}:{port}"))
}

fn validate_destination_policy(url: &Url) -> Result<Vec<SocketAddr>, PluginError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(PluginError::InvalidPolicy(
            "input `url` must use http or https".to_owned(),
        ));
    }
    let host = url.host_str().ok_or_else(|| {
        PluginError::InvalidRequest("input `url` must include a host".to_owned())
    })?;
    if BLOCKED_HOSTS.iter().any(|blocked| host.eq_ignore_ascii_case(blocked)) {
        return Err(PluginError::InvalidPolicy(format!(
            "destination host `{host}` is blocked"
        )));
    }
    let port = url.port_or_known_default().ok_or_else(|| {
        PluginError::InvalidRequest("input `url` must use a known port for its scheme".to_owned())
    })?;

    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|source| PluginError::RequestFailed(format!("failed to resolve destination host: {source}")))?;
    let addresses = addresses.collect::<Vec<_>>();
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
