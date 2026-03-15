# Local Rules

## Architecture
- Position: Application crate for the ChainBot workspace.
- Logic: Cargo manifest defines the crate boundary; `src/` holds executable code.
- Constraints: Keep crate responsibilities local and update nearby manifests when files move.

## Members
- `Cargo.toml`: Package manifest for the `chainbot` workspace member.
- `src/`: Binary entrypoint and crate source files.
- `tests/`: Integration tests for versioned contracts, root loading, scheduler execution semantics, runtime-state persistence, and runtime host boundaries.
