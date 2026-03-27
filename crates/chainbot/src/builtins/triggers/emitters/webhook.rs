//! [INPUT]
//! Webhook trigger definitions, ingress webhook param decoders, and builtin trigger handler contracts.
//!
//! [OUTPUT]
//! Validates webhook trigger definitions and delegates emission to the ingress-backed webhook runtime path.
//!
//! [ROLE]
//! Implements the builtin webhook trigger adapter over ingress contracts.

use crate::builtins::triggers::contract::BuiltinTriggerHandler;
use crate::domain::trigger::{TriggerDefinition, TriggerEmission};
use crate::errors::ContractError;
use crate::ingress::contract::{decode_webhook_params, BUILTIN_TRIGGER_WEBHOOK_KIND};

#[derive(Debug, Clone, Copy)]
pub struct WebhookTriggerHandler;

impl BuiltinTriggerHandler for WebhookTriggerHandler {
    fn kind(&self) -> &str {
        BUILTIN_TRIGGER_WEBHOOK_KIND
    }

    fn validate(&self, definition: &TriggerDefinition) -> Result<(), ContractError> {
        let _ = decode_webhook_params(definition)?;
        Ok(())
    }

    fn emit(
        &self,
        _context: &crate::builtins::triggers::context::BuiltinTriggerContext,
        definition: &TriggerDefinition,
    ) -> Result<Vec<TriggerEmission>, ContractError> {
        let _ = decode_webhook_params(definition)?;
        Ok(Vec::new())
    }
}
