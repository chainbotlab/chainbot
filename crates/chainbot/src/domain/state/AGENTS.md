# Local Rules

## Scope
- Position: Domain runtime-state schema subtree.
- Logic: Owns persisted state record schemas, lease snapshots, and schema validation helpers.
- Constraints: Keep storage backend behavior in `../../infrastructure/state/`.

## Members
- `mod.rs`: State domain module boundary.
- `lease.rs`: Daemon lease result and snapshot domain types.
- `model.rs`: Runtime-state schema model and validation helpers.
- `records.rs`: Persisted run, trigger, inbox, and snapshot record types.
