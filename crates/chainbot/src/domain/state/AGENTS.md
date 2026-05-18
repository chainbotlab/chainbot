# AGENTS.md

## Scope
- Position: Runtime-state schema domain subtree.
- Owns: Persisted record schemas, lease snapshots, and schema validation helpers.
- Excludes: Storage backend behavior.

## Constraints
- Keep backend persistence behavior in `../../infrastructure/state/`.

## Members
- `mod.rs`: State-domain exports.
- `lease.rs`: Lease-related domain models.
- `model.rs`: Persisted state models.
- `records.rs`: Runtime-state record schemas.
