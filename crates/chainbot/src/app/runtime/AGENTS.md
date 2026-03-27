# Local Rules

## Scope
- Position: Application runtime orchestration subtree.
- Logic: Owns daemon lifecycle, workflow execution entrypoints, and runtime-side supervision composition.
- Constraints: Keep backend-agnostic contracts in `../../domain/` and adapter implementations in `../../infrastructure/`.

## Members
- `mod.rs`: Runtime module boundary and shared runtime exports.
- `daemon.rs`: Daemon lifecycle orchestration and loop management.
- `execution.rs`: Workflow runtime execution composition.
- `external_triggers/`: External-trigger runtime hosting and supervision.
