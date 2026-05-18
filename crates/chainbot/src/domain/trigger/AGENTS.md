# AGENTS.md

## Scope
- Position: Trigger contracts and acceptance subtree.
- Owns: Trigger definitions, emission payloads, and pure acceptance logic.
- Excludes: Listener transport and host supervision.

## Constraints
- Keep transport and supervision in `../../ingress/` and `../../app/runtime/`.

## Members
- `mod.rs`: Trigger-domain exports.
- `contract.rs`: Trigger definitions and contracts.
- `emission.rs`: Emission payload models.
- `acceptance.rs`: Pure acceptance and normalization logic.
