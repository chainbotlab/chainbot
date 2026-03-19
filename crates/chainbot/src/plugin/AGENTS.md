# Local Rules

## Scope
- Position: Plugin subsystem folder for the `chainbot` crate.
- Logic: Owns the stable plugin facade plus its internal contract and host implementation modules.
- Constraints: Keep external consumers on `crate::plugin` / `chainbot::plugin`; contract and host implementation details stay internal to this folder.

## Members
- `mod.rs`: Stable public facade that re-exports the supported plugin contract and host surfaces.
- `contract.rs`: Internal plugin manifest and external-node protocol contract types plus schema validation helpers.
- `host.rs`: Internal external node-plugin host runtime with executable-path policy, process execution, and environment guards.
