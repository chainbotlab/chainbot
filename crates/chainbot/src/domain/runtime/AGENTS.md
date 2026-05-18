# AGENTS.md

## Scope
- Position: Runtime domain contracts.
- Owns: Scheduling contracts and run-report models.
- Excludes: Runtime execution side effects.

## Constraints
- Keep runtime side effects in `../../app/`.

## Members
- `mod.rs`: Runtime-domain exports.
- `contract.rs`: Scheduling and runtime contracts.
- `report.rs`: Run-report models.
