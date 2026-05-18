# AGENTS.md

## Scope
- Position: Builtin-trigger subsystem.
- Owns: Trigger contracts, dispatch, registries, and emitter fan-out for builtin trigger kinds.
- Excludes: Trigger-plane orchestration outside the shared builtin boundary.

## Constraints
- Keep subtype mapping and emitter registration in this folder.
- Trigger-plane code should depend on the shared trigger contracts and dispatch helpers exported here.

## Members
- `mod.rs`: Builtin trigger namespace root.
- `catalog.rs`: Builtin trigger catalog surface.
- `context.rs`: Trigger execution context.
- `contract.rs`: Stable builtin-trigger contracts.
- `dispatch.rs`: Emitter dispatch wiring.
- `registry.rs`: Builtin-trigger registry.
- `emitters/`: Concrete per-kind builtin trigger behaviors.
