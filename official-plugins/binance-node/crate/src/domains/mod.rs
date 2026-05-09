use std::collections::BTreeMap;

use serde_json::Value;

use crate::contract::{metadata_with_confirmation_mode, PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::provider::RequestContext;

pub mod account;
pub mod market;
pub mod order;
pub mod user_stream;

pub fn success(output: BTreeMap<String, Value>, result_state: Option<&'static str>) -> PluginResponse {
    PluginResponse::success(output, result_state)
}

pub fn write_output(status: &str, transaction_id: String, confirmation_mode: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (String::from("status"), Value::String(status.to_owned())),
        (String::from("transaction_id"), Value::String(transaction_id)),
        (
            String::from("metadata"),
            metadata_with_confirmation_mode(confirmation_mode),
        ),
    ])
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
    let product_line = required_input_string(request, "product_line")?;
    let environment = optional_input_string(request, "environment");
    let base_url = optional_input_string(request, "base_url");
    crate::provider::RequestContext::new(
        &product_line,
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

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}
