# Local Rules

## Scope
- Position: Plugin subsystem folder for the `chainbot` crate.
- Logic: Owns the stable plugin facade plus its internal contract and host implementation modules.
- Constraints: Keep external consumers on `crate::plugin` / `chainbot::plugin`; contract and host implementation details stay internal to this folder.

## Members
- `mod.rs`: Stable public facade that re-exports the supported plugin contract and host surfaces.
- `contract.rs`: Internal plugin manifest plus richer optional discovery metadata and external-node protocol contract types with schema validation helpers.
- `host.rs`: Internal external node-plugin host runtime with executable-path policy, per-operation schema enforcement, process execution, and environment guards.
- `source/`: Remote plugin source discovery and install subtree for github/git transports, source manifests, prepare/build flows, and safe replacement transactions.
