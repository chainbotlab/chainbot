//! [INPUT]
//! Builtin trigger context, manual trigger definitions, and decoded manual trigger params.
//!
//! [OUTPUT]
//! Validates manual trigger definitions and emits manual trigger events on demand.
//!
//! [ROLE]
//! Implements the builtin manual trigger emitter.

use serde::Deserialize;

use crate::builtins::triggers::context::BuiltinTriggerContext;
use crate::builtins::triggers::contract::{decode_builtin_trigger_params, BuiltinTriggerHandler};
use crate::domain::trigger::{TriggerDefinition, TriggerEmission};
use crate::errors::ContractError;

pub(crate) const BUILTIN_TRIGGER_MANUAL_KIND: &str = "manual";

#[derive(Debug, Clone, Copy)]
pub struct ManualTriggerHandler;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ManualTriggerParams {}

impl BuiltinTriggerHandler for ManualTriggerHandler {
    fn kind(&self) -> &str {
        BUILTIN_TRIGGER_MANUAL_KIND
    }

    fn validate(&self, definition: &TriggerDefinition) -> Result<(), ContractError> {
        let _: ManualTriggerParams = decode_builtin_trigger_params(definition)?;
        Ok(())
    }

    fn emit(
        &self,
        context: &BuiltinTriggerContext,
        definition: &TriggerDefinition,
    ) -> Result<Vec<TriggerEmission>, ContractError> {
        let _: ManualTriggerParams = decode_builtin_trigger_params(definition)?;
        Ok(vec![TriggerEmission {
            event_id: format!("builtin-event-{}", definition.trigger_id),
            occurred_at_ms: context.now_ms,
            checkpoint: None,
            source: Some(definition.source.clone()),
            payload: serde_json::json!({
                "kind": BUILTIN_TRIGGER_MANUAL_KIND,
                "source": definition.source,
            }),
            dedup_key: None,
            dedup_window_ms: None,
            cooldown_key: None,
            cooldown_ms: None,
        }])
    }
}
