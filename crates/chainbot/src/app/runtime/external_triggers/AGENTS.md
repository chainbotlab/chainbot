# Local Rules

## Scope
- Position: External-trigger runtime hosting subtree.
- Logic: Owns process or Wasmtime trigger host supervision, listener loops, and emission bridging to trigger-plane contracts.
- Constraints: Keep domain trigger acceptance in `../../../domain/trigger/`.

## Members
- `mod.rs`: External-trigger runtime module boundary.
- `process_listener.rs`: Process-backed external trigger listener loop.
- `supervisor.rs`: External trigger host supervision and lifecycle control.
- `wasmtime.rs`: Wasmtime-backed runtime host integration.
