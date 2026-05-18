# AGENTS.md

## Scope
- Position: Per-kind builtin node handlers.
- Owns: Concrete data, assertion, identity, script, fail, and subflow-output behaviors.
- Excludes: Shared node contracts and dispatch wiring.

## Constraints
- Keep shared contracts and dispatch in `../`.

## Members
- `mod.rs`: Handler exports.
- `assert.rs`: Assertion node behavior.
- `data_*.rs`: Data transformation and movement nodes.
- `identity.rs`: Pass-through identity behavior.
- `script.rs`: Script node behavior.
- `emit_subflow_output.rs`: Subflow-output emission behavior.
- `fail.rs`: Explicit failure node behavior.
