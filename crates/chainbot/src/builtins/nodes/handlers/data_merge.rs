//! [INPUT]
//! Builtin data.merge requests containing merge candidates and merge operation metadata.
//!
//! [OUTPUT]
//! Produces merged structured data outputs or usage errors for unsupported merge operations.
//!
//! [ROLE]
//! Implements the builtin data.merge node behavior.

use std::collections::BTreeMap;

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct DataMergeHandler;

impl BuiltinNodeHandler for DataMergeHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_DATA_MERGE_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let operation = request.operation.trim();
        if !operation.is_empty() && operation != "run" && operation != "objects" {
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has unsupported merge operation {}",
                    request.workflow_id, request.node_id, operation
                ),
            });
        }

        let mut merged = serde_json::Map::new();
        if let Some(objects) = request.inputs.get("objects") {
            let objects = objects.as_array().ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} expects data.merge objects to be an array of objects",
                    request.workflow_id, request.node_id
                ),
            })?;
            for value in objects {
                merge_object_into(&mut merged, value, request)?;
            }
        } else {
            for key in ["left", "right", "extra"] {
                if let Some(value) = request.inputs.get(key) {
                    merge_object_into(&mut merged, value, request)?;
                }
            }
        }

        let outputs = BTreeMap::from([(String::from("result"), serde_json::Value::Object(merged))]);
        Ok(BuiltinNodeResult {
            outputs: outputs.clone(),
            run_scoped: outputs,
            ..BuiltinNodeResult::default()
        })
    }
}

fn merge_object_into(
    target: &mut serde_json::Map<String, serde_json::Value>,
    value: &serde_json::Value,
    request: &BuiltinNodeRequest,
) -> Result<(), ContractError> {
    let object = value.as_object().ok_or_else(|| ContractError::CliUsage {
        message: format!(
            "workflow {} node {} expects every data.merge value to be an object",
            request.workflow_id, request.node_id
        ),
    })?;
    for (key, merged_value) in object {
        target.insert(key.clone(), merged_value.clone());
    }
    Ok(())
}
