# Local Rules

## Scope
- Position: CLI read-model rendering subtree.
- Logic: Owns user-facing output models for status, observe, and catalog style command responses.
- Constraints: Keep data acquisition outside this folder; this folder formats already-computed view payloads.

## Members
- `mod.rs`: View rendering module boundary.
- `status.rs`: Status-oriented CLI output shaping.
- `observe.rs`: Observation stream and run-progress output shaping.
- `catalog.rs`: Builtin capability catalog output shaping.
