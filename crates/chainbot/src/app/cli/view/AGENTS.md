# AGENTS.md

## Scope
- Position: CLI read-model rendering subtree.
- Owns: User-facing output shaping for status, observe, catalog, and plugin-source views.
- Excludes: Data acquisition, orchestration, and runtime computation.

## Constraints
- Only format already-computed payloads.

## Members
- `mod.rs`: Shared view exports.
- `status.rs`: Status-oriented CLI output.
- `observe.rs`: Observation/readout formatting.
- `catalog.rs`: Catalog and discovery formatting.
- `plugin_source.rs`: Plugin-source-specific presentation.
