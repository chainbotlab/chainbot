# Local Rules

## Scope
- Position: Domain trigger contracts and acceptance subtree.
- Logic: Owns trigger definition contracts, emission payloads, and pure acceptance logic.
- Constraints: Keep listener transport and host supervision in `../../ingress/` and `../../app/runtime/`.

## Members
- `mod.rs`: Trigger domain module boundary.
- `contract.rs`: Trigger definition and validation contracts.
- `emission.rs`: Trigger emission payload models.
- `acceptance.rs`: Emission acceptance and deduplication domain logic.
