//! [INPUT]
//! Builtin data.template requests containing template strings, input data, and operation metadata.
//!
//! [OUTPUT]
//! Renders templated outputs from structured inputs or returns usage and contract errors.
//!
//! [ROLE]
//! Implements the builtin data.template node behavior.

use std::collections::BTreeMap;

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct DataTemplateHandler;

impl BuiltinNodeHandler for DataTemplateHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_DATA_TEMPLATE_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let operation = request.operation.trim();
        if !operation.is_empty() && operation != "run" && operation != "render" {
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has unsupported template operation {}",
                    request.workflow_id, request.node_id, operation
                ),
            });
        }

        let template = request
            .inputs
            .get("template")
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} requires data.template template input",
                    request.workflow_id, request.node_id
                ),
            })?;
        let template = template.as_str().ok_or_else(|| ContractError::CliUsage {
            message: format!(
                "workflow {} node {} expects data.template template to be a string",
                request.workflow_id, request.node_id
            ),
        })?;
        let values = request
            .inputs
            .get("values")
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} requires data.template values input",
                    request.workflow_id, request.node_id
                ),
            })?;
        let values = values.as_object().ok_or_else(|| ContractError::CliUsage {
            message: format!(
                "workflow {} node {} expects data.template values to be an object",
                request.workflow_id, request.node_id
            ),
        })?;

        let rendered = render_template(template, values, request)?;
        let outputs =
            BTreeMap::from([(String::from("result"), serde_json::Value::String(rendered))]);
        Ok(BuiltinNodeResult {
            outputs: outputs.clone(),
            run_scoped: outputs,
            ..BuiltinNodeResult::default()
        })
    }
}

fn render_template(
    template: &str,
    values: &serde_json::Map<String, serde_json::Value>,
    request: &BuiltinNodeRequest,
) -> Result<String, ContractError> {
    let mut rendered = String::new();
    let mut remainder = template;

    while let Some(start) = remainder.find("{{") {
        rendered.push_str(&remainder[..start]);
        let after_start = &remainder[start + 2..];
        let end = after_start
            .find("}}")
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has unterminated template placeholder",
                    request.workflow_id, request.node_id
                ),
            })?;
        let key = after_start[..end].trim();
        if key.is_empty() {
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has empty template placeholder",
                    request.workflow_id, request.node_id
                ),
            });
        }
        let value = resolve_template_value(values, key).ok_or_else(|| ContractError::CliUsage {
            message: format!(
                "workflow {} node {} missing template value for {}",
                request.workflow_id, request.node_id, key
            ),
        })?;
        rendered.push_str(&json_value_to_text(value)?);
        remainder = &after_start[end + 2..];
    }

    rendered.push_str(remainder);
    Ok(rendered)
}

fn resolve_template_value<'a>(
    values: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<&'a serde_json::Value> {
    let mut segments = key.split('.');
    let first = segments.next()?;
    let mut current = values.get(first)?;
    for segment in segments {
        current = current.get(segment)?;
    }
    Some(current)
}

fn json_value_to_text(value: &serde_json::Value) -> Result<String, ContractError> {
    match value {
        serde_json::Value::String(text) => Ok(text.clone()),
        serde_json::Value::Null => Ok(String::from("null")),
        serde_json::Value::Bool(flag) => Ok(flag.to_string()),
        serde_json::Value::Number(number) => Ok(number.to_string()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => serde_json::to_string(value)
            .map_err(|source| ContractError::CliUsage {
                message: format!("failed to stringify data.template value as JSON: {source}"),
            }),
    }
}
