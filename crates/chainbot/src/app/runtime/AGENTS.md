# AGENTS.md

## Scope
- Position: Application runtime orchestration subtree.
- Owns: Daemon lifecycle, workflow execution entrypoints, and supervision composition.
- Excludes: Pure domain contracts and concrete infrastructure adapters.

## Constraints
- Keep contracts in `../../domain/`.
- Keep concrete adapters in `../../infrastructure/`.

## Members
- `mod.rs`: Runtime orchestration exports.
- `daemon.rs`: Process-lifecycle and daemon orchestration.
- `execution.rs`: Workflow execution entrypoints.
- `external_triggers/`: External-trigger host supervision and listener orchestration.
