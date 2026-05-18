# AGENTS.md

## Scope
- Position: Application definition-loading and validation boundary.
- Owns: Root bundle assembly and cross-package validation for workspace definitions.
- Excludes: Filesystem and storage adapter concerns.

## Constraints
- Keep storage and path adapters in `../../infrastructure/`.

## Members
- `mod.rs`: Definition-loading boundary root.
- `root_bundle.rs`: Root workspace bundle assembly.
- `validate.rs`: Cross-package validation flows.
