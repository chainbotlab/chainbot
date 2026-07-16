# AGENTS.md

## Scope
- Position: External-trigger runtime hosting subtree.
- Owns: Process and Wasmtime host supervision, listener loops, and emission bridging for external triggers.
- Excludes: Domain trigger acceptance semantics.

## Constraints
- Keep acceptance logic in `../../../domain/trigger/`.
- Long-lived process children, control pipes, bounded readers, and Wasm stores are supervisor-owned resources; daemon cycles only drain them.
- `component_v1` uses the crate-local `../../../../wit/trigger-plugin.wit`; `core_v0` is Release N compatibility only.

## Members
- `mod.rs`: External-trigger runtime exports.
- `process_listener.rs`: Bounded short-lived turns and managed process-session resources.
- `supervisor.rs`: Desired-state reconciliation and live session ownership.
- `wasmtime.rs`: Component Model host plus private core-v0 compatibility adapter.
