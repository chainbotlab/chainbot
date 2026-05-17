# AGENTS.md

## Scope
- Position: Backend-agnostic contract layer.
- Owns: Durable types and validation rules for workflow, trigger, runtime, and state domains.
- Excludes: Filesystem, database, network, and subprocess side effects.

## Constraints
- Side effects belong in `../infrastructure/`, `../ingress/`, and `../app/`.

## Members
- `mod.rs`: Domain namespace root.
- `workflow/`: Workflow definitions, variable namespaces, and subflow contracts.
- `trigger/`: Trigger contracts, emission payloads, and acceptance logic.
- `runtime/`: Runtime scheduling and reporting contracts.
- `state/`: Persisted runtime-state schemas and validation helpers.
