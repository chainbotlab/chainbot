# AGENTS.md

## Scope
- Position: Unified builtin namespace.
- Owns: Builtin node and trigger contracts, registries, and per-kind execution helpers.
- Excludes: Execution-plane orchestration outside the canonical builtin boundary.

## Constraints
- Consumers such as executors and trigger-plane code should depend on the canonical builtin surfaces exported here.

## Members
- `mod.rs`: Builtin namespace root.
- `nodes/`: Builtin workflow-node contracts, registries, and handlers.
- `triggers/`: Builtin trigger contracts, registries, and emitters.
