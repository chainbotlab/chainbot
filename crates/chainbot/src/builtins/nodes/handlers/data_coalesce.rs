//! [INPUT]
//! Builtin data.coalesce requests with candidate values and coalesce operation metadata.
//!
//! [OUTPUT]
//! Produces the first non-null candidate as builtin node output or a usage error for unsupported operations.
//!
//! [ROLE]
//! Implements the builtin data.coalesce node behavior.

use std::collections::BTreeMap;

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct DataCoalesceHandler;

impl BuiltinNodeHandler for DataCoalesceHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_DATA_COALESCE_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let operation = request.operation.trim();
        if !operation.is_empty() && operation != "run" && operation != "first" {
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has unsupported data.coalesce operation {}",
                    request.workflow_id, request.node_id, operation
                ),
            });
        }

        let values = request
            .inputs
            .get("values")
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} requires data.coalesce values input",
                    request.workflow_id, request.node_id
                ),
            })?
            .as_array()
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} expects data.coalesce values to be an array",
                    request.workflow_id, request.node_id
                ),
            })?;

        let resolved = values
            .iter()
            .find(|value| !value.is_null())
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let outputs = BTreeMap::from([(String::from("result"), resolved)]);
        Ok(BuiltinNodeResult {
            outputs: outputs.clone(),
            run_scoped: outputs,
            ..BuiltinNodeResult::default()
        })
    }
}
