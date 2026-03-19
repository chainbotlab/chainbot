use crate::builtins::triggers::context::BuiltinTriggerContext;
use crate::builtins::triggers::contract::BuiltinTriggerHandler;
use crate::errors::ContractError;
use crate::trigger::{TriggerDefinition, TriggerEmission, TRIGGER_KIND_MARKET_TICK_ALIAS};

pub(crate) const BUILTIN_TRIGGER_MARKET_TICK_KIND: &str = TRIGGER_KIND_MARKET_TICK_ALIAS;

#[derive(Debug, Clone, Copy)]
pub struct MarketTickTriggerHandler;

impl BuiltinTriggerHandler for MarketTickTriggerHandler {
    fn kind(&self) -> &str {
        BUILTIN_TRIGGER_MARKET_TICK_KIND
    }

    fn emit(
        &self,
        context: &BuiltinTriggerContext,
        definition: &TriggerDefinition,
    ) -> Result<Vec<TriggerEmission>, ContractError> {
        Ok(vec![TriggerEmission {
            event_id: format!("builtin-event-{}", definition.trigger_id),
            occurred_at_ms: context.now_ms,
            source: Some(definition.source.clone()),
            payload: serde_json::json!({
                "kind": BUILTIN_TRIGGER_MARKET_TICK_KIND,
                "source": definition.source,
                "symbol": "BTCUSDT",
            }),
            dedup_key: None,
            dedup_window_ms: None,
            cooldown_key: None,
            cooldown_ms: None,
        }])
    }
}
