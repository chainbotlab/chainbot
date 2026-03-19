use std::collections::BTreeMap;
use std::sync::Arc;

use crate::builtins::triggers::context::BuiltinTriggerContext;
use crate::errors::ContractError;
use crate::trigger::{TriggerDefinition, TriggerEmission};

pub type BuiltinTriggerEmitter =
    fn(&BuiltinTriggerContext, &TriggerDefinition) -> Result<Vec<TriggerEmission>, ContractError>;

pub trait BuiltinTriggerHandler: Send + Sync {
    fn kind(&self) -> &str;

    fn emit(
        &self,
        context: &BuiltinTriggerContext,
        definition: &TriggerDefinition,
    ) -> Result<Vec<TriggerEmission>, ContractError>;
}

#[derive(Default, Clone)]
pub struct BuiltinTriggerRegistry {
    emitters: BTreeMap<String, Arc<dyn BuiltinTriggerHandler>>,
}

impl std::fmt::Debug for BuiltinTriggerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltinTriggerRegistry")
            .field(
                "registered_kinds",
                &self.emitters.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl BuiltinTriggerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_handler<H>(&mut self, handler: H)
    where
        H: BuiltinTriggerHandler + 'static,
    {
        self.emitters
            .insert(handler.kind().to_owned(), Arc::new(handler));
    }

    pub fn register(&mut self, kind: impl Into<String>, emitter: BuiltinTriggerEmitter) {
        self.register_handler(FunctionBuiltinTriggerHandler::new(kind.into(), emitter));
    }

    pub fn emit(
        &self,
        context: &BuiltinTriggerContext,
        definition: &TriggerDefinition,
    ) -> Result<Vec<TriggerEmission>, ContractError> {
        let builtin_kind = crate::builtins::triggers::dispatch::builtin_trigger_kind(definition)?;
        let emitter = self.emitters.get(builtin_kind).ok_or_else(|| {
            ContractError::InvalidTriggerDefinitionField {
                trigger_id: definition.trigger_id.clone(),
                field: "trigger.source",
                detail: format!("unknown builtin trigger source {builtin_kind}"),
            }
        })?;
        emitter.emit(context, definition)
    }
}

struct FunctionBuiltinTriggerHandler {
    kind: String,
    emitter: BuiltinTriggerEmitter,
}

impl FunctionBuiltinTriggerHandler {
    fn new(kind: String, emitter: BuiltinTriggerEmitter) -> Self {
        Self { kind, emitter }
    }
}

impl BuiltinTriggerHandler for FunctionBuiltinTriggerHandler {
    fn kind(&self) -> &str {
        &self.kind
    }

    fn emit(
        &self,
        context: &BuiltinTriggerContext,
        definition: &TriggerDefinition,
    ) -> Result<Vec<TriggerEmission>, ContractError> {
        (self.emitter)(context, definition)
    }
}
