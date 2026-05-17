# AGENTS.md

## Scope
- Position: External-trigger runtime hosting subtree.
- Owns: Process and Wasmtime host supervision, listener loops, and emission bridging for external triggers.
- Excludes: Domain trigger acceptance semantics.

## Constraints
- Keep acceptance logic in `../../../domain/trigger/`.

## Members
- `mod.rs`: External-trigger runtime exports.
- `process_listener.rs`: Process-backed listener runtime.
- `supervisor.rs`: Supervision orchestration.
- `wasmtime.rs`: Wasmtime-backed trigger hosting.
