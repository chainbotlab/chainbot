use std::collections::BTreeMap;

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct DataParseJsonHandler;

impl BuiltinNodeHandler for DataParseJsonHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_DATA_PARSE_JSON_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let operation = request.operation.trim();
        if !operation.is_empty() && operation != "run" && operation != "parse" {
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has unsupported parse_json operation {}",
                    request.workflow_id, request.node_id, operation
                ),
            });
        }

        let text = request
            .inputs
            .get("text")
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} requires data.parse_json text input",
                    request.workflow_id, request.node_id
                ),
            })?
            .as_str()
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} expects data.parse_json text to be a string",
                    request.workflow_id, request.node_id
                ),
            })?;
        let parsed = serde_json::from_str::<serde_json::Value>(text).map_err(|source| {
            ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} failed to parse JSON text: {source}",
                    request.workflow_id, request.node_id
                ),
            }
        })?;

        let outputs = BTreeMap::from([(String::from("result"), parsed)]);
        Ok(BuiltinNodeResult {
            outputs: outputs.clone(),
            run_scoped: outputs,
            ..BuiltinNodeResult::default()
        })
    }
}
