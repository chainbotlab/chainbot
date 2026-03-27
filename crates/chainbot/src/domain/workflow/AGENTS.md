# Local Rules

## Scope
- Position: Domain workflow contracts subtree.
- Logic: Owns workflow definitions, variable namespace contracts, subflow linkage, and conditional semantics.
- Constraints: Keep runtime orchestration in `../../app/runtime/`.

## Members
- `mod.rs`: Workflow domain module boundary.
- `contract.rs`: Core workflow manifest and graph contracts.
- `variables.rs`: Runtime variable namespace and reference contracts.
- `subflow.rs`: Parent-child subflow import and export contracts.
- `when.rs`: Conditional expression contracts for execution gating.
