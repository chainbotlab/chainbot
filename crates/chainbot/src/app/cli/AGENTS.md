# AGENTS.md

## Scope
- Position: CLI command surface for the `chainbot` process.
- Owns: Intent mapping, argument parsing, help rendering, and CLI read-model wiring.
- Excludes: Transport-independent runtime execution logic.

## Constraints
- Keep runtime execution logic in `../runtime/`.
- Keep backend-agnostic contracts in `../../domain/`.

## Members
- `mod.rs`: CLI boundary root.
- `commands.rs`: Top-level command intent definitions.
- `parse.rs`: CLI argument parsing and normalization.
- `help.rs`: User-facing help rendering.
- `view/`: Output shaping for CLI-visible models.
