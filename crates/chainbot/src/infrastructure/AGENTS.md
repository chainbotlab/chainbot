# AGENTS.md

## Scope
- Position: Concrete adapter layer for workspace layout resolution, package loading, and runtime-state persistence.
- Logic: Owns filesystem- and storage-backed implementations that satisfy the crate's configuration and runtime-state contracts.
- Constraints: Keep adapter side effects here; do not move backend-agnostic validation or scheduling contracts out of `../domain/`, and do not duplicate CLI orchestration from `../app/`.

## Members
- `mod.rs`: Infrastructure boundary root exporting config and runtime-state adapters.
- `config/`: Root-layout resolution, root config decoding, and package loading.
- `state/`: File-backed and database-backed runtime-state stores plus coordination helpers.
