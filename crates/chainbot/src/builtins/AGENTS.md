# Local Rules

## Scope
- Position: Unified builtin namespace for the `chainbot` crate.
- Logic: Owns builtin node and builtin trigger contracts, registry assembly, and per-kind handler or emitter modules.
- Constraints: Keep builtin dispatch contracts, kinds, handlers, and emitters inside this folder; `executor.rs` and `trigger.rs` should depend on canonical builtin boundaries instead of builtin-specific logic.

## Members
- `mod.rs`: Unified builtin root that exposes canonical `nodes` and `triggers` module trees.
- `nodes/`: Workflow builtin-node subsystem containing trait-backed handler contracts, registry assembly, dispatch helpers, and per-handler files.
- `triggers/`: Builtin-trigger subsystem containing trait-backed emitter contracts, registry assembly, dispatch helpers, and per-emitter files.
- `AGENTS.md`: Folder manifest for the unified builtin namespace.
