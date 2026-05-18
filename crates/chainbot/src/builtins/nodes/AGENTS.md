# AGENTS.md

## Scope
- Position: Builtin workflow-node subsystem.
- Owns: Node contracts, execution context, dispatch, registry, input resolution, script worker helpers, and per-kind handlers.
- Excludes: External execution-plane orchestration.

## Constraints
- Keep node kinds, registries, and handlers inside this folder.
- Execution-plane code should consume the contracts and dispatch surface exported here.

## Members
- `mod.rs`: Builtin node namespace root.
- `catalog.rs`: Builtin node catalog surface.
- `context.rs`: Execution context for builtin nodes.
- `contract.rs`: Stable builtin-node contracts.
- `dispatch.rs`: Handler dispatch wiring.
- `input_resolver.rs`: Input resolution and normalization.
- `registry.rs`: Builtin-node registry.
- `script_worker.rs`: Script-execution helper logic.
- `handlers/`: Concrete per-kind builtin node behaviors.
