use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

use base64::Engine;
use hmac::{Hmac, Mac};
use reqwest::{header::HeaderValue, Client, Method, Response};
use serde_json::Value;
use sha2::Sha256;
use time::{format_description::FormatItem, macros::format_description, OffsetDateTime};
use url::{form_urlencoded, Url};

use crate::errors::PluginError;

type HmacSha256 = Hmac<Sha256>;
const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.azure.internal"];
const OKX_TIMESTAMP_FORMAT: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstType {
    Spot,
    Swap,
    Futures,
    Option,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Mainnet,
    Testnet,
}

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub inst_type: InstType,
    pub environment: Environment,
    pub base_url: String,
    pub allowed_origins: Vec<String>,
}

impl InstType {
    pub fn parse(value: &str) -> Result<Self, PluginError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "spot" => Ok(Self::Spot),
            "swap" => Ok(Self::Swap),
            "futures" | "future" => Ok(Self::Futures),
            "option" | "options" => Ok(Self::Option),
            other => Err(PluginError::InvalidInput(format!(
                "inst_type must be one of spot, swap, futures, option, got {other}"
            ))),
        }
    }

    pub fn as_api_value(&self) -> &'static str {
        match self {
            Self::Spot => "SPOT",
            Self::Swap => "SWAP",
            Self::Futures => "FUTURES",
            Self::Option => "OPTION",
        }
    }

    pub fn server_time_path(&self) -> &'static str {
        "/api/v5/public/time"
    }

    pub fn instruments_path(&self) -> &'static str {
        "/api/v5/public/instruments"
    }

    pub fn ticker_path(&self) -> &'static str {
        "/api/v5/market/ticker"
    }

    pub fn account_balance_path(&self) -> &'static str {
        "/api/v5/account/balance"
    }

    fn default_base_url(&self, environment: Environment) -> &'static str {
        match environment {
            Environment::Mainnet => "https://www.okx.com",
            Environment::Testnet => "https://www.okx.com",
        }
    }
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
        inst_type: &str,
        environment: Option<&str>,
        base_url: Option<&str>,
        allowed_origins: Vec<String>,
    ) -> Result<Self, PluginError> {
        let inst_type = InstType::parse(inst_type)?;
        let environment = Environment::parse(environment)?;
        let base_url = match base_url {
            Some(value) if !value.trim().is_empty() => {
                Url::parse(value)
                    .map_err(|error| PluginError::InvalidInput(format!("invalid base_url: {error}")))?;
                value.trim_end_matches('/').to_owned()
            }
            _ => inst_type.default_base_url(environment).to_owned(),
        };
        Ok(Self {
            inst_type,
            environment,
            base_url,
            allowed_origins,
        })
    }
}

pub async fn public_get(
    client: &Client,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
) -> Result<Value, PluginError> {
    send_json_request(client, Method::GET, context, path, query, None, None, None).await
}

pub async fn signed_get(
    client: &Client,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
    api_key: &str,
    api_secret: &str,
    passphrase: &str,
) -> Result<Value, PluginError> {
    send_json_request(
        client,
        Method::GET,
        context,
        path,
        query,
        Some(api_key),
        Some(api_secret),
        Some(passphrase),
    )
    .await
}

async fn send_json_request(
    client: &Client,
    method: Method,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
    api_key: Option<&str>,
    api_secret: Option<&str>,
    passphrase: Option<&str>,
) -> Result<Value, PluginError> {
    let url = build_url(context, path, query)?;
    let requires_binding = api_key.is_some() || api_secret.is_some() || passphrase.is_some();
    validate_destination_policy(url.as_str(), &context.allowed_origins, requires_binding)?;

    let mut request = client.request(method.clone(), url.clone());
    if let Some(api_key) = api_key {
        request = request.header("OK-ACCESS-KEY", HeaderValue::from_str(api_key).map_err(|error| {
            PluginError::InvalidInput(format!("invalid api_key header value: {error}"))
        })?);
    }
    if let (Some(api_secret), Some(passphrase)) = (api_secret, passphrase) {
        let timestamp = current_timestamp_iso8601()?;
        let path_and_query = path_and_query(&url);
        let signature = sign_request(&timestamp, method.as_str(), &path_and_query, "", api_secret)?;
        request = request
            .header(
                "OK-ACCESS-SIGN",
                HeaderValue::from_str(&signature)
                    .map_err(|error| PluginError::InvalidInput(format!("invalid sign header value: {error}")))?,
            )
            .header(
                "OK-ACCESS-TIMESTAMP",
                HeaderValue::from_str(&timestamp).map_err(|error| {
                    PluginError::InvalidInput(format!("invalid timestamp header value: {error}"))
                })?,
            )
            .header(
                "OK-ACCESS-PASSPHRASE",
                HeaderValue::from_str(passphrase).map_err(|error| {
                    PluginError::InvalidInput(format!("invalid passphrase header value: {error}"))
                })?,
            );
    }

    let response = request.send().await?;
    parse_json_response(response).await
}

fn build_url(
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
) -> Result<Url, PluginError> {
    let mut url = Url::parse(&context.base_url)
        .map_err(|error| PluginError::InvalidInput(format!("invalid base_url: {error}")))?;
    url.set_path(path);
    if !query.is_empty() {
        let query = encode_query(&query);
        url.set_query(Some(&query));
    }
    Ok(url)
}

fn encode_query(query: &[(String, String)]) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (key, value) in query {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

fn path_and_query(url: &Url) -> String {
    match url.query() {
        Some(query) if !query.is_empty() => format!("{}?{query}", url.path()),
        _ => url.path().to_owned(),
    }
}

fn sign_request(
    timestamp: &str,
    method: &str,
    request_path: &str,
    body: &str,
    api_secret: &str,
) -> Result<String, PluginError> {
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes())
        .map_err(|error| PluginError::Signing(format!("failed to initialize signer: {error}")))?;
    mac.update(timestamp.as_bytes());
    mac.update(method.as_bytes());
    mac.update(request_path.as_bytes());
    mac.update(body.as_bytes());
    Ok(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}

fn current_timestamp_iso8601() -> Result<String, PluginError> {
    OffsetDateTime::now_utc()
        .format(OKX_TIMESTAMP_FORMAT)
        .map_err(|error| PluginError::Signing(format!("failed to format okx timestamp: {error}")))
}

async fn parse_json_response(response: Response) -> Result<Value, PluginError> {
    let status = response.status();
    let body = response.text().await?;
    if status.is_success() {
        if body.trim().is_empty() {
            return Ok(serde_json::json!({}));
        }
        return serde_json::from_str(&body).map_err(PluginError::from);
    }

    let message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|payload| {
            payload
                .get("msg")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| payload.get("message").and_then(Value::as_str).map(str::to_owned))
        })
        .unwrap_or_else(|| body.clone());
    Err(PluginError::Rpc(format!("okx http {}: {}", status.as_u16(), message)))
}

fn validate_destination_policy(url: &str, allowed_origins: &[String], requires_binding: bool) -> Result<(), PluginError> {
    let parsed = Url::parse(url).map_err(|error| PluginError::InvalidInput(format!("invalid destination url: {error}")))?;
    validate_scheme(&parsed)?;
    validate_allowed_origins(&parsed, allowed_origins, requires_binding)?;
    validate_ip_resolution(&parsed)?;
    Ok(())
}

fn validate_scheme(url: &Url) -> Result<(), PluginError> {
    if matches!(url.scheme(), "https") {
        return Ok(());
    }
    if matches!(url.scheme(), "http") && allow_loopback_for_tests() && is_loopback_host(url.host_str()) {
        return Ok(());
    }
    Err(PluginError::InvalidInput(String::from("destination url must use https")))
}

fn is_loopback_host(host: Option<&str>) -> bool {
    matches!(host, Some("localhost") | Some("127.0.0.1") | Some("::1"))
}

fn validate_allowed_origins(url: &Url, allowed_origins: &[String], requires_binding: bool) -> Result<(), PluginError> {
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
    if permitted {
        Ok(())
    } else {
        Err(PluginError::InvalidInput(format!(
            "request origin {} is not allowlisted for activation secrets",
            request_origin
        )))
    }
}

fn normalize_origin(url: &Url) -> Result<String, PluginError> {
    let scheme = match url.scheme() {
        "http" => "http",
        "https" => "https",
        other => return Err(PluginError::InvalidInput(format!("unsupported origin scheme {other}"))),
    };
    let host = url
        .host_str()
        .ok_or_else(|| PluginError::InvalidInput(String::from("destination url must include a host")))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| PluginError::InvalidInput(String::from("destination url must use a known port")))?;
    Ok(format!("{scheme}://{host}:{port}"))
}

fn validate_ip_resolution(url: &Url) -> Result<(), PluginError> {
    let host = url
        .host_str()
        .ok_or_else(|| PluginError::InvalidInput(String::from("destination url must include a host")))?;
    if BLOCKED_HOSTS.iter().any(|blocked| host.eq_ignore_ascii_case(blocked)) {
        return Err(PluginError::InvalidInput(format!("destination host `{host}` is blocked")));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| PluginError::InvalidInput(String::from("destination url must use a known port")))?;
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| PluginError::InvalidInput(format!("failed to resolve destination host: {error}")))?
        .collect::<Vec<SocketAddr>>();
    for address in &addresses {
        validate_ip(address.ip())?;
    }
    Ok(())
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
