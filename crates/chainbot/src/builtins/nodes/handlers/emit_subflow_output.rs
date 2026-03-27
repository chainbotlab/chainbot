//! [INPUT]
//! Builtin emit-subflow-output requests carrying resolved node inputs from a child workflow.
//!
//! [OUTPUT]
//! Returns builtin node results whose subflow output mirrors the provided inputs.
//!
//! [ROLE]
//! Implements the builtin node that forwards child outputs to the parent subflow boundary.

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct EmitSubflowOutputHandler;

impl BuiltinNodeHandler for EmitSubflowOutputHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_EMIT_SUBFLOW_OUTPUT_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        Ok(BuiltinNodeResult {
            subflow_output: request.inputs.clone(),
            ..BuiltinNodeResult::default()
        })
    }
}
