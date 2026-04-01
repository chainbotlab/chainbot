# Local Rules

## Scope
- Position: Per-kind builtin node handler implementations.
- Logic: Owns concrete execution behavior for builtin flow, data, identity, subflow-output, and script node kinds.
- Constraints: Keep shared contracts and dispatch wiring in `../`.

## Members
- `mod.rs`: Handler module boundary.
- `assert.rs`: Assertion semantics for flow-control checks.
- `data_*.rs`: Data transformation, selection, merge, parse, and template behaviors.
- `script.rs`: Script worker-backed node execution behavior.
- `identity.rs`: Pass-through node behavior.
- `emit_subflow_output.rs`: Subflow output forwarding behavior.
- `fail.rs`: Explicit failure node behavior.
