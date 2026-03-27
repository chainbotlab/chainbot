//! [INPUT]
//! Builtin data.get requests containing source payloads, lookup paths, and operation metadata.
//!
//! [OUTPUT]
//! Extracts a value by path from structured input data or returns a usage error for invalid operations.
//!
//! [ROLE]
//! Implements the builtin data.get node behavior.

use std::collections::BTreeMap;

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct DataGetHandler;

impl BuiltinNodeHandler for DataGetHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_DATA_GET_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let operation = request.operation.trim();
        if !operation.is_empty() && operation != "run" && operation != "path" {
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has unsupported data.get operation {}",
                    request.workflow_id, request.node_id, operation
                ),
            });
        }

        let input = request
            .inputs
            .get("input")
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} requires data.get input",
                    request.workflow_id, request.node_id
                ),
            })?;
        let path = request
            .inputs
            .get("path")
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} requires data.get path input",
                    request.workflow_id, request.node_id
                ),
            })?
            .as_str()
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} expects data.get path to be a string",
                    request.workflow_id, request.node_id
                ),
            })?;

        let resolved =
            resolve_path(input, path)
                .cloned()
                .ok_or_else(|| ContractError::CliUsage {
                    message: format!(
                        "workflow {} node {} missing data.get path {}",
                        request.workflow_id, request.node_id, path
                    ),
                })?;

        let outputs = BTreeMap::from([(String::from("result"), resolved)]);
        Ok(BuiltinNodeResult {
            outputs: outputs.clone(),
            run_scoped: outputs,
            ..BuiltinNodeResult::default()
        })
    }
}

fn resolve_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    if path.is_empty() {
        return Some(value);
    }

    let mut current = value;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        current = match current {
            serde_json::Value::Object(object) => object.get(segment)?,
            serde_json::Value::Array(entries) => entries.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}
