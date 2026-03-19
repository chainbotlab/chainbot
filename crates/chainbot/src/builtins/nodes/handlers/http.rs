use std::collections::BTreeMap;
use std::sync::Arc;

use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Method;

use crate::builtins::nodes::context::BuiltinRuntimeContext;
use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::builtins::nodes::input_resolver::resolve_node_inputs;
use crate::errors::ContractError;
use crate::secrets::redact_text;

#[derive(Debug, Clone)]
pub struct HttpHandler {
    context: Arc<BuiltinRuntimeContext>,
}

#[derive(Debug)]
struct HttpNodeSpec {
    url: String,
    method: Method,
    headers: HeaderMap,
    body: Option<reqwest::blocking::Body>,
}

impl HttpHandler {
    pub fn new(context: Arc<BuiltinRuntimeContext>) -> Self {
        Self { context }
    }
}

impl BuiltinNodeHandler for HttpHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_HTTP_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let resolved = resolve_node_inputs(
            &self.context.root_layout.secrets_dir,
            self.context.secret_mode,
            &request.inputs,
        )?;
        let spec = parse_http_node_request(request, &resolved.values)?;
        let client = Client::builder()
            .build()
            .map_err(|source| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} failed to initialize HTTP client: {source}",
                    request.workflow_id, request.node_id
                ),
            })?;

        let mut http_request = client.request(spec.method.clone(), &spec.url);
        if !spec.headers.is_empty() {
            http_request = http_request.headers(spec.headers);
        }
        if let Some(body) = spec.body {
            http_request = http_request.body(body);
        }

        let response = http_request
            .send()
            .map_err(|source| ContractError::CliUsage {
                message: redact_text(
                    &format!(
                        "workflow {} node {} HTTP request to {} failed: {source}",
                        request.workflow_id, request.node_id, spec.url
                    ),
                    &resolved.resolved_secrets,
                ),
            })?;

        let status = response.status();
        let final_url = response.url().to_string();
        let headers = response
            .headers()
            .iter()
            .map(|(key, value)| {
                (
                    key.as_str().to_owned(),
                    serde_json::Value::String(value.to_str().unwrap_or_default().to_owned()),
                )
            })
            .collect::<serde_json::Map<String, serde_json::Value>>();
        let body = response.text().map_err(|source| ContractError::CliUsage {
            message: redact_text(
                &format!(
                    "workflow {} node {} failed to read HTTP response body from {}: {source}",
                    request.workflow_id, request.node_id, final_url
                ),
                &resolved.resolved_secrets,
            ),
        })?;

        let outputs = BTreeMap::from([
            (
                String::from("status"),
                serde_json::Value::from(status.as_u16()),
            ),
            (
                String::from("ok"),
                serde_json::Value::from(status.is_success()),
            ),
            (String::from("url"), serde_json::Value::String(final_url)),
            (String::from("body"), serde_json::Value::String(body)),
            (String::from("headers"), serde_json::Value::Object(headers)),
        ]);

        Ok(BuiltinNodeResult {
            outputs: outputs.clone(),
            run_scoped: outputs,
            ..BuiltinNodeResult::default()
        })
    }
}

fn parse_http_node_request(
    request: &BuiltinNodeRequest,
    resolved_inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<HttpNodeSpec, ContractError> {
    let url = request.operation.trim();
    if url.is_empty() {
        return Err(ContractError::CliUsage {
            message: format!(
                "workflow {} node {} HTTP node requires a URL in operation",
                request.workflow_id, request.node_id
            ),
        });
    }

    let method = resolved_inputs
        .get("method")
        .map(|value| json_string_field(value, "method", request))
        .transpose()?
        .unwrap_or_else(|| String::from("GET"));
    let method =
        Method::from_bytes(method.as_bytes()).map_err(|source| ContractError::CliUsage {
            message: format!(
                "workflow {} node {} has invalid HTTP method: {source}",
                request.workflow_id, request.node_id
            ),
        })?;

    let headers = resolved_inputs
        .get("headers")
        .map(|value| json_headers_field(value, request))
        .transpose()?
        .unwrap_or_default();
    let body = resolved_inputs
        .get("body")
        .map(|value| json_body_field(value, request))
        .transpose()?;

    Ok(HttpNodeSpec {
        url: url.to_owned(),
        method,
        headers,
        body,
    })
}

fn json_string_field(
    value: &serde_json::Value,
    field: &str,
    request: &BuiltinNodeRequest,
) -> Result<String, ContractError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| ContractError::CliUsage {
            message: format!(
                "workflow {} node {} expects HTTP input {} to be a string",
                request.workflow_id, request.node_id, field
            ),
        })
}

fn json_headers_field(
    value: &serde_json::Value,
    request: &BuiltinNodeRequest,
) -> Result<HeaderMap, ContractError> {
    let object = value.as_object().ok_or_else(|| ContractError::CliUsage {
        message: format!(
            "workflow {} node {} expects HTTP input headers to be an object",
            request.workflow_id, request.node_id
        ),
    })?;

    let mut headers = HeaderMap::new();
    for (key, raw_value) in object {
        let header_name =
            HeaderName::from_bytes(key.as_bytes()).map_err(|source| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has invalid HTTP header name {}: {source}",
                    request.workflow_id, request.node_id, key
                ),
            })?;
        let header_value = json_string_field(raw_value, &format!("headers.{key}"), request)?;
        let header_value =
            HeaderValue::from_str(&header_value).map_err(|source| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has invalid HTTP header value for {}: {source}",
                    request.workflow_id, request.node_id, key
                ),
            })?;
        headers.insert(header_name, header_value);
    }

    Ok(headers)
}

fn json_body_field(
    value: &serde_json::Value,
    request: &BuiltinNodeRequest,
) -> Result<reqwest::blocking::Body, ContractError> {
    match value {
        serde_json::Value::String(text) => Ok(reqwest::blocking::Body::from(text.clone())),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => serde_json::to_string(value)
            .map(reqwest::blocking::Body::from)
            .map_err(|source| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} failed to encode HTTP body as JSON: {source}",
                    request.workflow_id, request.node_id
                ),
            }),
        serde_json::Value::Null => Ok(reqwest::blocking::Body::from(String::new())),
        _ => Ok(reqwest::blocking::Body::from(value.to_string())),
    }
}
