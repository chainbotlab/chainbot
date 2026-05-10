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
    Usdm,
    Coinm,
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
            "usdm" | "usdsm" | "usd-m" | "usd_m" => Ok(Self::Usdm),
            "coinm" | "coin-m" | "coin_m" => Ok(Self::Coinm),
            other => Err(PluginError::InvalidInput(format!(
                "product_line must be one of spot, usdm, coinm, got {other}"
            ))),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Usdm => "usdm",
            Self::Coinm => "coinm",
        }
    }

    pub fn server_time_path(&self) -> &'static str {
        match self {
            Self::Spot => "/api/v3/time",
            Self::Usdm => "/fapi/v1/time",
            Self::Coinm => "/dapi/v1/time",
        }
    }

    pub fn exchange_info_path(&self) -> &'static str {
        match self {
            Self::Spot => "/api/v3/exchangeInfo",
            Self::Usdm => "/fapi/v1/exchangeInfo",
            Self::Coinm => "/dapi/v1/exchangeInfo",
        }
    }

    pub fn ticker_path(&self) -> &'static str {
        match self {
            Self::Spot => "/api/v3/ticker/24hr",
            Self::Usdm => "/fapi/v1/ticker/24hr",
            Self::Coinm => "/dapi/v1/ticker/24hr",
        }
    }

    pub fn depth_path(&self) -> &'static str {
        match self {
            Self::Spot => "/api/v3/depth",
            Self::Usdm => "/fapi/v1/depth",
            Self::Coinm => "/dapi/v1/depth",
        }
    }

    pub fn klines_path(&self) -> &'static str {
        match self {
            Self::Spot => "/api/v3/klines",
            Self::Usdm => "/fapi/v1/klines",
            Self::Coinm => "/dapi/v1/klines",
        }
    }

    pub fn account_path(&self) -> &'static str {
        match self {
            Self::Spot => "/api/v3/account",
            Self::Usdm => "/fapi/v2/account",
            Self::Coinm => "/dapi/v1/account",
        }
    }

    pub fn balances_path(&self) -> Option<&'static str> {
        match self {
            Self::Spot => None,
            Self::Usdm => Some("/fapi/v2/balance"),
            Self::Coinm => Some("/dapi/v1/balance"),
        }
    }

    pub fn open_orders_path(&self) -> &'static str {
        match self {
            Self::Spot => "/api/v3/openOrders",
            Self::Usdm => "/fapi/v1/openOrders",
            Self::Coinm => "/dapi/v1/openOrders",
        }
    }

    pub fn order_path(&self) -> &'static str {
        match self {
            Self::Spot => "/api/v3/order",
            Self::Usdm => "/fapi/v1/order",
            Self::Coinm => "/dapi/v1/order",
        }
    }

    pub fn positions_path(&self) -> Option<&'static str> {
        match self {
            Self::Spot => None,
            Self::Usdm => Some("/fapi/v2/positionRisk"),
            Self::Coinm => None,
        }
    }

    pub fn cancel_all_orders_path(&self) -> &'static str {
        match self {
            Self::Spot => "/api/v3/openOrders",
            Self::Usdm => "/fapi/v1/allOpenOrders",
            Self::Coinm => "/dapi/v1/allOpenOrders",
        }
    }

    pub fn user_stream_path(&self) -> &'static str {
        match self {
            Self::Spot => "/api/v3/userDataStream",
            Self::Usdm => "/fapi/v1/listenKey",
            Self::Coinm => "/dapi/v1/listenKey",
        }
    }

    fn default_base_url(&self, environment: Environment) -> &'static str {
        match (self, environment) {
            (Self::Spot, Environment::Mainnet) => "https://api.binance.com",
            (Self::Spot, Environment::Testnet) => "https://testnet.binance.vision",
            (Self::Usdm, Environment::Mainnet) => "https://fapi.binance.com",
            (Self::Usdm, Environment::Testnet) => "https://demo-fapi.binance.com",
            (Self::Coinm, Environment::Mainnet) => "https://dapi.binance.com",
            (Self::Coinm, Environment::Testnet) => "https://testnet.binancefuture.com",
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
            _ => product_line.default_base_url(environment).to_owned(),
        };
        Ok(Self {
            product_line,
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
    send_json_request(client, Method::GET, context, path, query, None, None, false).await
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
        true,
    )
    .await
}

pub async fn signed_post(
    client: &Client,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
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
        true,
    )
    .await
}

pub async fn api_key_post(
    client: &Client,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
    api_key: &str,
) -> Result<Value, PluginError> {
    send_json_request(client, Method::POST, context, path, query, Some(api_key), None, false).await
}

pub async fn api_key_put(
    client: &Client,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
    api_key: &str,
) -> Result<Value, PluginError> {
    send_json_request(client, Method::PUT, context, path, query, Some(api_key), None, false).await
}

pub async fn api_key_delete(
    client: &Client,
    context: &RequestContext,
    path: &str,
    query: Vec<(String, String)>,
    api_key: &str,
) -> Result<Value, PluginError> {
    send_json_request(client, Method::DELETE, context, path, query, Some(api_key), None, false).await
}

fn build_url(
    context: &RequestContext,
    path: &str,
    mut query: Vec<(String, String)>,
    api_secret: Option<&str>,
    sign: bool,
) -> Result<Url, PluginError> {
    let mut url = Url::parse(&context.base_url)
        .map_err(|error| PluginError::InvalidInput(format!("invalid base_url: {error}")))?;
    url.set_path(path);

    if sign {
        query.push((String::from("timestamp"), current_time_ms()?.to_string()));
        let query_string = encode_query(&query);
        let signature = sign_query(&query_string, api_secret.ok_or_else(|| {
            PluginError::InvalidInput(String::from("api_secret is required for signed Binance requests"))
        })?)?;
        url.set_query(Some(&format!("{query_string}&signature={signature}")));
        return Ok(url);
    }

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
    sign: bool,
) -> Result<Value, PluginError> {
    let url = build_url(context, path, query, api_secret, sign)?;
    validate_destination_policy(&url)?;
    validate_allowed_origins(&url, &context.allowed_origins, api_key.is_some())?;
    let mut request = client.request(method, url);
    if let Some(key) = api_key {
        request = request.header(
            "X-MBX-APIKEY",
            HeaderValue::from_str(key)
                .map_err(|error| PluginError::InvalidInput(format!("invalid api_key header: {error}")))?,
        );
    }
    parse_json_response(request.send().await?).await
}

async fn parse_json_response(response: Response) -> Result<Value, PluginError> {
    let status = response.status();
    let body = response.text().await?;
    if body.trim().is_empty() {
        if status.is_success() {
            return Ok(json!({}));
        }
        return Err(PluginError::Rpc(format!(
            "binance http {} returned empty response",
            status.as_u16()
        )));
    }

    let payload: Value =
        serde_json::from_str(&body).map_err(|error| PluginError::Rpc(format!("binance returned invalid json: {error}")))?;

    if status.is_success() {
        return Ok(payload);
    }

    let code = payload.get("code").cloned().unwrap_or(Value::Null);
    let message = payload
        .get("msg")
        .and_then(Value::as_str)
        .unwrap_or("request failed");
    Err(PluginError::Rpc(format!(
        "binance http {} code {}: {}",
        status.as_u16(),
        code,
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

fn sign_query(query: &str, secret: &str) -> Result<String, PluginError> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|error| PluginError::Signing(error.to_string()))?;
    mac.update(query.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn current_time_ms() -> Result<i64, PluginError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| PluginError::Rpc(error.to_string()))?;
    i64::try_from(duration.as_millis()).map_err(|error| PluginError::Rpc(error.to_string()))
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
        return Err(PluginError::InvalidInput(String::from(
            "base_url must use https",
        )));
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
