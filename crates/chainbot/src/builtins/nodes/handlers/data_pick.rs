use std::collections::BTreeMap;

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct DataPickHandler;

impl BuiltinNodeHandler for DataPickHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_DATA_PICK_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let operation = request.operation.trim();
        if !operation.is_empty() && operation != "run" && operation != "fields" {
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has unsupported pick operation {}",
                    request.workflow_id, request.node_id, operation
                ),
            });
        }

        let input = request
            .inputs
            .get("input")
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} requires object input at data.pick input",
                    request.workflow_id, request.node_id
                ),
            })?;
        let input = input.as_object().ok_or_else(|| ContractError::CliUsage {
            message: format!(
                "workflow {} node {} expects data.pick input to be an object",
                request.workflow_id, request.node_id
            ),
        })?;
        let fields = request
            .inputs
            .get("fields")
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} requires data.pick fields input",
                    request.workflow_id, request.node_id
                ),
            })?;
        let fields = fields.as_array().ok_or_else(|| ContractError::CliUsage {
            message: format!(
                "workflow {} node {} expects data.pick fields to be an array of strings",
                request.workflow_id, request.node_id
            ),
        })?;

        let mut selected = serde_json::Map::new();
        for field in fields {
            let field_name = field.as_str().ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} expects each data.pick field to be a string",
                    request.workflow_id, request.node_id
                ),
            })?;
            if let Some(value) = input.get(field_name) {
                selected.insert(field_name.to_owned(), value.clone());
            }
        }

        let outputs =
            BTreeMap::from([(String::from("result"), serde_json::Value::Object(selected))]);
        Ok(BuiltinNodeResult {
            outputs: outputs.clone(),
            run_scoped: outputs,
            ..BuiltinNodeResult::default()
        })
    }
}
