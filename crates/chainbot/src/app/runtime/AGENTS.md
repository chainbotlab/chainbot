# AGENTS.md

## Scope
- Position: Application runtime orchestration subtree.
- Owns: Daemon lifecycle, workflow execution entrypoints, and supervision composition.
- Excludes: Pure domain contracts and concrete infrastructure adapters.

## Constraints
- Keep contracts in `../../domain/`.
- Keep concrete adapters in `../../infrastructure/`.
- Accept runtime inputs and return execution/lifecycle outcomes without depending on CLI request or output types.

## Members
- `mod.rs`: Crate-local runtime entrypoints and existing trigger compatibility constructors.
- `context.rs`: Effective root resolution, execution-context loading, recovery, and history retention.
- `run.rs`: Manual workflow selection, shared run execution, and lease-fenced run persistence.
- `daemon.rs`: Daemon lifecycle, trigger-request assembly/replay, and lease-bound serve turns.
- `execution.rs`: Workflow scheduling and node/subflow execution.
- `plugin_activation.rs`: Shared execution-time plugin activation resolution and error redaction.
- `external_triggers/`: External-trigger host supervision and listener orchestration.
