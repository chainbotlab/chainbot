//! [INPUT]
//! Builtin data.stringify_json requests carrying structured JSON values and operation metadata.
//!
//! [OUTPUT]
//! Produces stringified JSON output or usage errors for unsupported operations.
//!
//! [ROLE]
//! Implements the builtin data.stringify_json node behavior.

use std::collections::BTreeMap;

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct DataStringifyJsonHandler;

impl BuiltinNodeHandler for DataStringifyJsonHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_DATA_STRINGIFY_JSON_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let operation = request.operation.trim();
        if !operation.is_empty() && operation != "run" && operation != "stringify" {
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has unsupported stringify_json operation {}",
                    request.workflow_id, request.node_id, operation
                ),
            });
        }

        let value = request
            .inputs
            .get("value")
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} requires data.stringify_json value input",
                    request.workflow_id, request.node_id
                ),
            })?;
        let rendered = serde_json::to_string(value).map_err(|source| ContractError::CliUsage {
            message: format!(
                "workflow {} node {} failed to stringify value as JSON: {source}",
                request.workflow_id, request.node_id
            ),
        })?;

        let outputs =
            BTreeMap::from([(String::from("result"), serde_json::Value::String(rendered))]);
        Ok(BuiltinNodeResult {
            outputs: outputs.clone(),
            run_scoped: outputs,
            ..BuiltinNodeResult::default()
        })
    }
}
