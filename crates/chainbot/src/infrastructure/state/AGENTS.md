# AGENTS.md

## Scope
- Position: Runtime-state persistence adapter subtree.
- Owns: File and database stores plus sqlite coordination helpers.
- Excludes: Persisted schema contracts.

## Constraints
- Keep schema contracts in `../../domain/state/`.

## Members
- `mod.rs`: State-adapter exports.
- `file_store.rs`: File-backed runtime-state store.
- `db_store.rs`: Database-backed runtime-state store.
- `sqlite_coordination.rs`: SQLite coordination helpers.
