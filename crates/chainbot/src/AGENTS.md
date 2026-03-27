# Local Rules

## Architecture
- Position: Rust source tree for the `chainbot` binary crate.
- Logic: Source files define the executable entrypoint and the V2-line public module boundary (app/domain/infrastructure + facade subtrees).
- Constraints: Keep Rust file headers aligned with actual inputs, outputs, and architectural position.

## Public Module Map
The frozen public surface is defined in `lib.rs`:
- `app/` — CLI dispatch, CLI view read-models, root-definition assembly, and runtime execution orchestration
- `domain/` — backend-agnostic workflow, trigger, runtime, and state contracts
- `infrastructure/` — root-layout resolution, package loading, and state persistence backends
- `builtins/` — unified builtin namespace (facade subtree, preserved)
- `ingress/` — listener-backed trigger ingress runtime (facade subtree, preserved)
- `plugin/` — encapsulated plugin subsystem (facade subtree, preserved)
- `errors/`, `script_protocol/`, `secrets/` — shared utility contracts

## Legacy Root Shims
- Retired root shim files are removed from `src/`.
- Runtime execution helpers now live under `app::cli` and `app::runtime`.
- Trigger-plane compatibility constructors are implemented under `app::runtime` while domain acceptance remains in `domain::trigger`.

## Members
- `main.rs`: Binary entrypoint that routes CLI stdout/stderr and explicit process exit codes.
- `lib.rs`: Public module map that freezes the V2 boundary for MVP contracts.
