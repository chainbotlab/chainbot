//! [INPUT]
//! Builtin fail requests containing failure messages and operation metadata.
//!
//! [OUTPUT]
//! Returns a forced builtin node failure or usage errors for unsupported fail operations.
//!
//! [ROLE]
//! Implements the builtin node that deliberately raises workflow execution failures.

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct FailHandler;

impl BuiltinNodeHandler for FailHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_FAIL_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let operation = request.operation.trim();
        if !operation.is_empty() && operation != "run" && operation != "raise" {
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has unsupported fail operation {}",
                    request.workflow_id, request.node_id, operation
                ),
            });
        }

        let message = request
            .inputs
            .get("message")
            .map(|value| json_string_field(value, "message", request))
            .transpose()?
            .unwrap_or_else(|| String::from("explicit failure requested by builtin.flow.fail"));
        let code_suffix = request
            .inputs
            .get("code")
            .map(|value| json_string_field(value, "code", request))
            .transpose()?
            .map(|code| format!(" code={code}"))
            .unwrap_or_default();

        Err(ContractError::CliUsage {
            message: format!(
                "workflow {} node {} {}{}",
                request.workflow_id, request.node_id, message, code_suffix
            ),
        })
    }
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
                "workflow {} node {} expects fail input {} to be a string",
                request.workflow_id, request.node_id, field
            ),
        })
}
