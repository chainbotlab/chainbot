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
