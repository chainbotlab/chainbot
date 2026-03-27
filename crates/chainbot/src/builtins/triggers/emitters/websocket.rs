//! [INPUT]
//! WebSocket trigger definitions, ingress websocket param decoders, and builtin trigger handler contracts.
//!
//! [OUTPUT]
//! Validates websocket trigger definitions and delegates emission to the ingress-backed websocket runtime path.
//!
//! [ROLE]
//! Implements the builtin websocket trigger adapter over ingress contracts.

use crate::builtins::triggers::contract::BuiltinTriggerHandler;
use crate::domain::trigger::{TriggerDefinition, TriggerEmission};
use crate::errors::ContractError;
use crate::ingress::contract::{decode_websocket_params, BUILTIN_TRIGGER_WEBSOCKET_KIND};

#[derive(Debug, Clone, Copy)]
pub struct WebSocketTriggerHandler;

impl BuiltinTriggerHandler for WebSocketTriggerHandler {
    fn kind(&self) -> &str {
        BUILTIN_TRIGGER_WEBSOCKET_KIND
    }

    fn validate(&self, definition: &TriggerDefinition) -> Result<(), ContractError> {
        let _ = decode_websocket_params(definition)?;
        Ok(())
    }

    fn emit(
        &self,
        _context: &crate::builtins::triggers::context::BuiltinTriggerContext,
        definition: &TriggerDefinition,
    ) -> Result<Vec<TriggerEmission>, ContractError> {
        let _ = decode_websocket_params(definition)?;
        Ok(Vec::new())
    }
}
