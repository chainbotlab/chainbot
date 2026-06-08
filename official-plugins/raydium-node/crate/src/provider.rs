use reqwest::{Client, Url};
use serde_json::Value;

use crate::contract::PluginRequest;
use crate::errors::PluginError;

pub const DEFAULT_SWAP_BASE_URL: &str = "https://transaction-v1.raydium.io";

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
