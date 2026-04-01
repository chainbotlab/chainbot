use crate::client::{encode_body, execute, parse_headers, parse_method, HttpRequestSpec};
use crate::contract::{PluginRequest, PluginResponse};
use crate::errors::PluginError;

pub async fn handle(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    let raw_url = request
        .input_string("url")
        .ok_or_else(|| PluginError::InvalidRequest("input `url` is required".to_owned()))?;
    if raw_url.trim_start().starts_with("secret://") {
        return Err(PluginError::InvalidRequest(
            "input `url` must not contain secret references; use activation secrets"
                .to_owned(),
        ));
    }
    let url = reqwest::Url::parse(raw_url)
        .map_err(|source| PluginError::InvalidRequest(format!("invalid request url: {source}")))?;
    let headers = parse_headers(request.input.get("headers"))?;
    let activation_authorization = request.activation_secret("authorization").map(str::to_owned);
    if activation_authorization.is_some() && headers.contains_key("authorization") {
        return Err(PluginError::InvalidRequest(
            "workflow headers must not set `authorization` when activation secret slot `authorization` is configured"
                .to_owned(),
        ));
    }
    let spec = HttpRequestSpec {
        url,
        method: parse_method(request.input_string("method"))?,
        headers,
        body: encode_body(request.input.get("body"))?,
        activation_authorization,
        allowed_origins: request.allowed_origins().to_vec(),
    };
    let output = execute(spec).await?;
    Ok(PluginResponse::success(output))
}
