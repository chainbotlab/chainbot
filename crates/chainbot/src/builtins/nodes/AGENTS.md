# Local Rules

## Scope
- Position: Workflow builtin-node subsystem for the `chainbot` crate.
- Logic: Owns trait-backed builtin node contracts, runtime context, dispatch helpers, registry assembly, secret-aware input resolution, and per-handler execution modules.
- Constraints: Keep builtin node kinds, registry wiring, and handler implementations inside this folder; `executor.rs` should only consume the contract and dispatch surfaces.

## Members
- `mod.rs`: Builtin-node subsystem root that exposes contract, context, dispatch, registry, and handler modules.
- `context.rs`: Shared runtime context and secret-decrypt mode for builtin node handlers.
- `contract.rs`: Builtin node request/result types, handler trait, registry type, and the test-seeded handler constructor plus closure-based extension adapter.
- `dispatch.rs`: Node-definition to builtin-kind resolution helper used by the execution plane.
- `input_resolver.rs`: Secret-aware input materialization reused by builtin node handlers.
- `registry.rs`: Canonical builtin node registry assembly plus builtin kind constants and registry tests.
- `script_worker.rs`: Script-node subprocess host, limits, and runtime failure mapping owned by the builtin node subsystem.
- `handlers/`: Per-builtin node handlers such as identity, subflow-output, HTTP, script, and external-node execution.
