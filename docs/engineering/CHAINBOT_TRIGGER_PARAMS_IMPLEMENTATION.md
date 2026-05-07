# ChainBot Trigger Params Implementation

## Scope

- Add a stable `params` extension slot to `TriggerDefinition` without weakening the fixed trigger core contract.
- Keep builtin trigger validation load-time capable through the builtin trigger registry.
- Implement builtin `cron` as a params-backed builtin trigger subtype.
- Preserve trigger-plane restart-safe duplicate suppression and payload mapping behavior.

## Files Changed

- `crates/chainbot/Cargo.toml`
- `crates/chainbot/src/config.rs`
- `crates/chainbot/src/trigger.rs`
- `crates/chainbot/src/builtins/triggers/mod.rs`
- `crates/chainbot/src/builtins/triggers/contract.rs`
- `crates/chainbot/src/builtins/triggers/registry.rs`
- `crates/chainbot/src/builtins/triggers/emitters/mod.rs`
- `crates/chainbot/src/builtins/triggers/emitters/manual.rs`
- `crates/chainbot/src/builtins/triggers/emitters/market_tick.rs`
- `crates/chainbot/src/builtins/triggers/emitters/cron.rs`
- `crates/chainbot/tests/trigger_plane.rs`
- `docs/decisions/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md`
- `docs/engineering/CHAINBOT_V21_CONFIG_IMPLEMENTATION.md`
- `docs/engineering/CHAINBOT_TRIGGER_PARAMS_IMPLEMENTATION.md`
- `docs/engineering/AGENTS.md`
- `AGENTS.md`

## Contract Changes

- `TriggerDefinition` now owns `params: BTreeMap<String, serde_json::Value>` in addition to the existing fixed core fields.
- Top-level trigger manifests remain `deny_unknown_fields`; extensibility happens inside `[params]`, not via arbitrary new top-level keys.
- Builtin trigger validation now runs through the builtin trigger registry so subtype-specific params can fail during load and trigger-plane open.
- Canonical trigger kinds remain `builtin` and `external_plugin`; legacy alias kinds are still accepted for compatibility with existing roots.

## Builtin Trigger Changes

- `BuiltinTriggerHandler` now supports a `validate()` hook alongside `emit()`.
- `manual` validates that `[params]` is empty.
- `market_tick` validates and decodes optional `params.symbol`, defaulting to `BTCUSDT`.
- New builtin `cron` validates `params.schedule` plus optional `params.timezone = "UTC"` and emits stable minute-slot events.

## External Trigger Host Changes

- External trigger host still passes `--trigger-id <trigger_id>` argv for compatibility.
- External trigger host now also writes `TriggerPluginInput` JSON to plugin stdin.
- `TriggerPluginInput` currently includes `api_version`, `trigger_id`, `source`, and `params`.
- Existing plugins that ignore stdin continue to work.
- Params-aware plugins can decode stdin and derive trigger-specific behavior from `params`.

## Cron Runtime Semantics

- `cron` evaluates the current serve snapshot only.
- A cron slot is rounded down to the current UTC minute.
- Matching cron slots emit `event_id = "cron:<trigger_id>:<slot_start_ms>"`.
- The emitted `dedup_key` reuses that stable slot identity so restarts do not replay the same slot.
- No missed-slot replay is performed.

## Validation

- `lsp_diagnostics` on modified Rust files
- targeted `cargo test` coverage for builtin trigger registry and trigger-plane integration
- targeted `cargo check` for workspace compilation

## Notes

- This keeps the public trigger contract typed at the routing boundary while avoiding per-subtype public-struct churn.
- External trigger plugins now receive `params` through stdin while retaining the legacy argv trigger id for compatibility.
