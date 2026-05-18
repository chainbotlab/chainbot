# AGENTS.md

## Scope
- Position: Remote plugin source discovery and install subtree.
- Owns: Locator parsing, source manifests, transport materialization, preparation, safe replacement, and rollback primitives.
- Excludes: Runtime plugin contracts and runtime host execution.

## Constraints
- Runtime contracts stay in `../contract.rs`.
- Host execution stays in `../host.rs`.
- This subtree is for discoverability and installation only.

## Members
- `mod.rs`: Source-install namespace root.
- `locator.rs`: Plugin source locator parsing.
- `manifest.rs`: Source manifest models.
- `transport.rs`: Transport materialization helpers.
- `discover.rs`: Source discovery flows.
- `prepare.rs`: Install preparation steps.
- `install.rs`: Safe install and replacement flows.
- `fs.rs`: Filesystem helpers and rollback primitives.
