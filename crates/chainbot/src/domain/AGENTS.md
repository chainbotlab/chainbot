# AGENTS.md

## Scope
- Position: Backend-agnostic contract layer for workflows, triggers, runtime scheduling, and persisted state records.
- Logic: Owns durable Rust types and validation rules that define what ChainBot workflows, triggers, run reports, and runtime state mean independent of any concrete adapter.
- Constraints: Keep filesystem, database, network, and subprocess behavior out of this folder; those side effects belong to `../infrastructure/`, `../ingress/`, or `../app/`.

## Members
- `mod.rs`: Domain boundary root exporting workflow, trigger, runtime, and state modules.
- `workflow/`: Workflow manifest contracts, variable namespaces, subflow wiring, and conditional execution semantics.
- `trigger/`: Trigger manifest contracts, emissions, and domain acceptance logic.
- `runtime/`: Runtime scheduling contracts and workflow run reporting types.
- `state/`: Persisted runtime-state schemas, record summaries, and lease snapshots.
