# AGENTS.md

## Scope
- Position: Workflow contracts subtree.
- Owns: Workflow definitions, variable namespaces, subflow linkage, and conditional semantics.
- Excludes: Runtime orchestration.

## Constraints
- Keep orchestration in `../../app/runtime/`.

## Members
- `mod.rs`: Workflow-domain exports.
- `contract.rs`: Workflow definitions and contracts.
- `variables.rs`: Variable namespace semantics.
- `subflow.rs`: Subflow linkage contracts.
- `when.rs`: Conditional execution semantics.
