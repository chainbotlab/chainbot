use reqwest::{Client, Url};
use serde_json::Value;

use crate::contract::PluginRequest;
use crate::errors::PluginError;

pub const DEFAULT_SWAP_BASE_URL: &str = "https://sanctum-api.ironforge.network";

pub fn api_key_for_url<'a>(request: &'a PluginRequest, url: &Url) -> Option<&'a str> {
    let api_key = request.activation_secret("api_key")?;
    if is_default_origin(url) || is_allowed_origin(request, url) {
        Some(api_key)
    } else {
        None
    }
}

pub fn swap_base_url(request: &PluginRequest) -> Result<Url, PluginError> {
    let raw = request.input_string("base_url").unwrap_or(DEFAULT_SWAP_BASE_URL);
    let mut url = Url::parse(raw)
        .map_err(|error| PluginError::InvalidInput(format!("invalid base_url: {error}")))?;
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path().trim_end_matches('/'));
        url.set_path(&path);
    }
    Ok(url)
}

pub async fn get_json(
    client: &Client,
    url: Url,
    api_key: Option<&str>,
) -> Result<Value, PluginError> {
    let mut request = client.get(url);
    if let Some(api_key) = api_key {
        request = request.header("x-api-key", api_key);
    }
    let response = request.send().await?;
    parse_api_response(response).await
}

pub async fn post_json(
    client: &Client,
    url: Url,
    body: Value,
    api_key: Option<&str>,
) -> Result<Value, PluginError> {
    let mut request = client.post(url).json(&body);
    if let Some(api_key) = api_key {
        request = request.header("x-api-key", api_key);
    }
    let response = request.send().await?;
    parse_api_response(response).await
}

async fn parse_api_response(response: reqwest::Response) -> Result<Value, PluginError> {
    let status = response.status();
    let payload: Value = response.json().await?;
    if !status.is_success() {
        return Err(PluginError::Api(format!(
            "http status {status}: {payload}"
        )));
    }
    if let Some(error) = payload.get("error") {
        return Err(PluginError::Api(error.to_string()));
    }
    Ok(payload)
}

fn is_default_origin(url: &Url) -> bool {
    Url::parse(DEFAULT_SWAP_BASE_URL)
        .ok()
        .and_then(|default| normalize_origin(&default).ok())
        .as_deref()
        == normalize_origin(url).ok().as_deref()
}

fn is_allowed_origin(request: &PluginRequest, url: &Url) -> bool {
    let Some(activation) = request.activation.as_ref() else {
        return false;
    };
    let Ok(request_origin) = normalize_origin(url) else {
        return false;
    };
    activation.allowed_origins.iter().any(|origin| {
        Url::parse(origin)
            .ok()
            .and_then(|origin| normalize_origin(&origin).ok())
            .as_deref()
            == Some(request_origin.as_str())
    })
}

fn normalize_origin(url: &Url) -> Result<String, PluginError> {
    let port = url.port_or_known_default().ok_or_else(|| {
        PluginError::InvalidInput(String::from("base_url must use a known port for its scheme"))
    })?;
    let host = url
        .host_str()
        .ok_or_else(|| PluginError::InvalidInput(String::from("base_url must include a host")))?;
    Ok(format!("{}://{}:{}", url.scheme(), host, port))
}
