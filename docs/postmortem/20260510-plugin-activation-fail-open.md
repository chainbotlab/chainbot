# Symptom

Official chain plugin manifests declared activation requirements that the runtime did not consistently enforce. Two gaps existed:

- external node requests could omit the activation envelope even when the manifest required activation secrets
- external trigger startup could proceed without `allowed_origins` even when the manifest declared `requires_allowed_origins = true`

# Root Cause

The contract checks were split across three layers and each one had a fail-open path:

- `crates/chainbot/src/app/definitions/validate.rs` only rejected missing activation when `required_secret_slots` was non-empty, and only required `allowed_origins` when `secret_bindings` were present
- `crates/chainbot/src/plugin/contract.rs` returned `Ok(())` when `ExternalNodePluginRequest.activation` was `None`
- `crates/chainbot/src/app/runtime/external_triggers/process_listener.rs` converted missing trigger activation bindings into `ResolvedTriggerActivation::default()` without consulting the manifest contract

That left the manifest as documentation instead of an enforced boundary.

# Fix Applied

- tightened root validation so missing activation now fails when a plugin requires either secret slots or `allowed_origins`
- tightened node request validation so `activation = None` fails closed when the manifest requires activation
- tightened trigger runtime activation resolution so startup fails before spawn when required activation is absent
- added regression tests for node host, trigger root validation, and trigger runtime fail-closed behavior

# Validation

Targeted tests to run after the fix:

- `cargo test -p chainbot --test node_plugin_host external_node_plugin_rejects_missing_required_activation_envelope`
- `cargo test -p chainbot --test config_loading trigger_activation_rejects_missing_plugin_activation_when_allowed_origins_are_required`
- `cargo test -p chainbot --test chain_trigger_runtime chain_trigger_runtime_rejects_missing_required_allowed_origins_activation`

Recommended combined verification:

- `cargo test -p chainbot --test node_plugin_host --test config_loading --test chain_trigger_runtime`

# Preventive Safeguards

- keep activation contracts fail-closed at every boundary that can construct or forward activation data
- when adding a new manifest requirement, add both positive and negative tests at the root validation layer and the runtime protocol layer
- prefer manifest-aware helpers over default/empty activation fallbacks in runtime orchestration code
