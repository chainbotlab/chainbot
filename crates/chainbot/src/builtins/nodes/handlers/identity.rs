//! [INPUT]
//! Builtin identity requests carrying already-resolved node inputs.
//!
//! [OUTPUT]
//! Returns the request inputs unchanged as builtin node outputs.
//!
//! [ROLE]
//! Implements the builtin identity node behavior.

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct IdentityHandler;

impl BuiltinNodeHandler for IdentityHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_IDENTITY_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        Ok(BuiltinNodeResult {
            outputs: request.inputs.clone(),
            ..BuiltinNodeResult::default()
        })
    }
}
