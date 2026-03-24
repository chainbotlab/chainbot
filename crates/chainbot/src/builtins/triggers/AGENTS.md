# Local Rules

## Scope
- Position: Builtin-trigger subsystem for the `chainbot` crate.
- Logic: Owns trait-backed builtin trigger contracts, dispatch helpers, registry assembly, and per-emitter fan-out implementations.
- Constraints: Keep builtin trigger subtype mapping and emitter registration inside this folder; `trigger.rs` should only depend on the shared trigger contract plus builtin dispatch helpers.

## Members
- `mod.rs`: Builtin-trigger subsystem root that exposes context, contract, dispatch, registry, and emitter modules.
- `catalog.rs`: Static builtin-trigger descriptors for CLI capability discovery plus descriptor completeness tests.
- `context.rs`: Shared builtin-trigger evaluation context.
- `contract.rs`: Builtin trigger handler trait, registry type, and function-based extension adapter for tests or custom fanout wiring.
- `dispatch.rs`: Trigger-definition to builtin subtype resolution helper.
- `registry.rs`: Canonical builtin trigger registry assembly and emission fan-out entrypoint.
- `emitters/`: Per-trigger builtin emitters such as manual and market-tick.
