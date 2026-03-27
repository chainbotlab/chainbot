# Local Rules

## Scope
- Position: Infrastructure runtime-state persistence adapter subtree.
- Logic: Owns file-backed and database-backed runtime-state stores plus sqlite coordination helpers.
- Constraints: Keep persisted schema contracts in `../../domain/state/`.

## Members
- `mod.rs`: State adapter module boundary.
- `file_store.rs`: File-backed runtime-state store.
- `db_store.rs`: Database-backed runtime-state store.
- `sqlite_coordination.rs`: Sqlite coordination and lease support helpers.
