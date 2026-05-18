# AGENTS.md

## Scope
- Position: Application crate for the ChainBot workspace.
- Owns: The package manifest, executable source tree, and integration-test surface for the `chainbot` binary crate.
- Excludes: Root workspace policy, sibling top-level areas, and standalone official plugin package sources.

## Constraints
- Keep crate responsibilities local to this workspace member.
- Update nearby manifests and local maps when files move across crate or test boundaries.

## Members
- `Cargo.toml`: Package manifest for the `chainbot` workspace member.
- `src/`: Binary entrypoint, public module surface, and internal runtime implementation.
- `tests/`: Integration tests covering contract compatibility, root loading, runtime-state persistence, scheduler semantics, and host boundaries.
