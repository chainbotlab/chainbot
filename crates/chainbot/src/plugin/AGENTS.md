# AGENTS.md

## Scope
- Position: Plugin subsystem folder.
- Owns: Stable plugin facade plus internal contract, host, and source-install modules.
- Excludes: External consumers reaching into internal modules directly.

## Constraints
- External users stay on `crate::plugin` and `chainbot::plugin`.
- Contract and host internals remain behind this facade boundary.

## Members
- `mod.rs`: Stable plugin facade.
- `contract.rs`: Plugin runtime contracts.
- `host.rs`: Plugin host execution surface.
- `source/`: Remote source discovery and installation support.
