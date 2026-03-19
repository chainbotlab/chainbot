use std::collections::BTreeMap;

use crate::builtins::triggers::context::BuiltinTriggerContext;
use crate::builtins::triggers::contract::BuiltinTriggerRegistry;
use crate::builtins::triggers::emitters::{
    manual::ManualTriggerHandler, market_tick::MarketTickTriggerHandler,
};
use crate::errors::ContractError;
use crate::trigger::{TriggerDefinition, TriggerEmission, TriggerKind};

pub fn build_builtin_trigger_emissions(
    definitions: &[TriggerDefinition],
    now_ms: i64,
) -> Result<BTreeMap<String, Vec<TriggerEmission>>, ContractError> {
    let registry = default_builtin_trigger_registry();
    let context = BuiltinTriggerContext { now_ms };
    let mut emissions = BTreeMap::new();

    for definition in definitions {
        if !matches!(definition.kind(), Ok(TriggerKind::Builtin)) {
            continue;
        }

        emissions.insert(
            definition.trigger_id.clone(),
            registry.emit(&context, definition)?,
        );
    }

    Ok(emissions)
}

pub(crate) fn default_builtin_trigger_registry() -> BuiltinTriggerRegistry {
    let mut registry = BuiltinTriggerRegistry::new();
    registry.register_handler(ManualTriggerHandler);
    registry.register_handler(MarketTickTriggerHandler);
    registry
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::builtins::triggers::emitters::{
        manual::BUILTIN_TRIGGER_MANUAL_KIND, market_tick::BUILTIN_TRIGGER_MARKET_TICK_KIND,
    };

    use super::*;

    fn fanout_emission(
        context: &BuiltinTriggerContext,
        definition: &TriggerDefinition,
    ) -> Result<Vec<TriggerEmission>, ContractError> {
        Ok(vec![
            TriggerEmission {
                event_id: format!("fanout-a-{}", definition.trigger_id),
                occurred_at_ms: context.now_ms,
                source: Some(definition.source.clone()),
                payload: serde_json::json!({"kind": "fanout", "index": 1}),
                dedup_key: None,
                dedup_window_ms: None,
                cooldown_key: None,
                cooldown_ms: None,
            },
            TriggerEmission {
                event_id: format!("fanout-b-{}", definition.trigger_id),
                occurred_at_ms: context.now_ms,
                source: Some(definition.source.clone()),
                payload: serde_json::json!({"kind": "fanout", "index": 2}),
                dedup_key: None,
                dedup_window_ms: None,
                cooldown_key: None,
                cooldown_ms: None,
            },
        ])
    }

    fn trigger_definition(trigger_id: &str, kind: &str, source: &str) -> TriggerDefinition {
        TriggerDefinition {
            api_version: "2.0.0".to_owned(),
            trigger_id: trigger_id.to_owned(),
            kind: kind.to_owned(),
            source: source.to_owned(),
            plugin: None,
            workflow_id: format!("wf-{trigger_id}"),
            enabled: true,
            input_mapping: BTreeMap::new(),
            package_root: PathBuf::new(),
        }
    }

    #[test]
    fn build_builtin_trigger_emissions_supports_alias_and_source_resolution() {
        let emissions = build_builtin_trigger_emissions(
            &[
                trigger_definition("manual-trigger", "manual", "manual-source"),
                trigger_definition("market-trigger", "builtin", "market_tick"),
            ],
            1_710_300_100_000,
        )
        .expect("builtin trigger definitions should emit events");

        assert_eq!(
            emissions["manual-trigger"][0].payload,
            serde_json::json!({"kind": "manual", "source": "manual-source"})
        );
        assert_eq!(
            emissions["market-trigger"][0].payload,
            serde_json::json!({"kind": "market_tick", "source": "market_tick", "symbol": "BTCUSDT"})
        );
    }

    #[test]
    fn build_builtin_trigger_emissions_rejects_unknown_builtin_source() {
        let error = build_builtin_trigger_emissions(
            &[trigger_definition("custom-trigger", "builtin", "custom")],
            1_710_300_100_000,
        )
        .expect_err("unknown builtin trigger source should fail deterministically");

        assert!(error
            .to_string()
            .contains("unknown builtin trigger source custom"));
    }

    #[test]
    fn builtin_trigger_registry_supports_multi_event_emitters() {
        let mut registry = BuiltinTriggerRegistry::new();
        registry.register("fanout", fanout_emission);

        let emissions = registry
            .emit(
                &BuiltinTriggerContext {
                    now_ms: 1_710_300_100_000,
                },
                &trigger_definition("fanout-trigger", "builtin", "fanout"),
            )
            .expect("fanout emitter should return multiple events");

        assert_eq!(emissions.len(), 2);
        assert_eq!(emissions[0].event_id, "fanout-a-fanout-trigger");
        assert_eq!(emissions[1].event_id, "fanout-b-fanout-trigger");
    }

    #[test]
    fn default_builtin_trigger_registry_registers_trait_handlers() {
        let registry = default_builtin_trigger_registry();
        let manual = registry
            .emit(
                &BuiltinTriggerContext {
                    now_ms: 1_710_300_100_000,
                },
                &trigger_definition("manual-trigger", "manual", BUILTIN_TRIGGER_MANUAL_KIND),
            )
            .expect("manual trigger handler should be registered");
        assert_eq!(manual.len(), 1);

        let market = registry
            .emit(
                &BuiltinTriggerContext {
                    now_ms: 1_710_300_100_000,
                },
                &trigger_definition(
                    "market-trigger",
                    "market_tick",
                    BUILTIN_TRIGGER_MARKET_TICK_KIND,
                ),
            )
            .expect("market tick trigger handler should be registered");
        assert_eq!(market.len(), 1);
    }
}
