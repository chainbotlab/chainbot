use std::collections::BTreeMap;

use serde_json::Value;

use crate::contract::{PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::provider::RequestContext;

pub mod account;
pub mod market;

pub fn success(output: BTreeMap<String, Value>, result_state: Option<&'static str>) -> PluginResponse {
    PluginResponse::success(output, result_state)
}

pub fn optional_input_string(request: &PluginRequest, key: &str) -> Option<String> {
    request.input.get(key).and_then(value_to_string)
}

pub fn required_input_string(request: &PluginRequest, key: &str) -> Result<String, PluginError> {
    optional_input_string(request, key)
        .ok_or_else(|| PluginError::InvalidInput(format!("{key} is required")))
}

pub fn push_optional_query_param(
    request: &PluginRequest,
    input_key: &str,
    target_key: &str,
    query: &mut Vec<(String, String)>,
) {
    if let Some(value) = optional_input_string(request, input_key) {
        query.push((String::from(target_key), value));
    }
}

pub fn request_context(request: &PluginRequest) -> Result<RequestContext, PluginError> {
    let inst_type = required_input_string(request, "inst_type")?;
    let environment = optional_input_string(request, "environment");
    let base_url = optional_input_string(request, "base_url");
    RequestContext::new(
        &inst_type,
        environment.as_deref(),
        base_url.as_deref(),
        request.allowed_origins().to_vec(),
    )
}

pub fn require_api_key(request: &PluginRequest) -> Result<&str, PluginError> {
    request
        .activation_secret("api_key")
        .ok_or_else(|| PluginError::InvalidInput(String::from("activation api_key is required")))
}

pub fn require_api_secret(request: &PluginRequest) -> Result<&str, PluginError> {
    request
        .activation_secret("api_secret")
        .ok_or_else(|| PluginError::InvalidInput(String::from("activation api_secret is required")))
}

pub fn require_passphrase(request: &PluginRequest) -> Result<&str, PluginError> {
    request
        .activation_secret("passphrase")
        .ok_or_else(|| PluginError::InvalidInput(String::from("activation passphrase is required")))
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}
