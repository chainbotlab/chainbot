# AGENTS.md

## Scope
- Position: Rust source tree for the `chainbot` binary crate.
- Owns: The executable entrypoint, frozen public module boundary, and internal runtime/domain/adapter subtrees.
- Excludes: Crate manifest concerns and integration-test fixtures.

## Constraints
- Keep Rust file headers aligned with actual inputs, outputs, and architectural role.
- Preserve the frozen public boundary exposed by `lib.rs`.
- Retired root shim files stay removed from `src/`; runtime helpers live under `app`, and trigger compatibility constructors stay in `app::runtime` while acceptance remains in `domain::trigger`.

## Members
- `main.rs`: Binary entrypoint that routes CLI stdout/stderr and explicit process exit codes.
- `lib.rs`: Public module map that freezes the supported `chainbot` crate surface.
- `app/`: Process-facing orchestration for CLI, definition loading, and runtime execution.
- `domain/`: Backend-agnostic workflow, trigger, runtime, and state contracts.
- `infrastructure/`: Root-layout resolution, package loading, and state persistence backends.
- `builtins/`: Unified builtin node and trigger namespace.
- `ingress/`: Listener-backed trigger ingress runtime.
- `plugin/`: Encapsulated plugin contract, host, and source-install subsystem.
- `errors/`, `script_protocol/`, `secrets/`: Shared utility contracts used across subtrees.
