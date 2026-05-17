# AGENTS.md

## Scope
- Position: Configuration adapter subtree.
- Owns: Root layout resolution, root config decoding, package loading, and plugin activation parsing/validation.
- Excludes: Backend-agnostic configuration contracts.

## Constraints
- Keep pure contracts in `../../domain/`.

## Members
- `mod.rs`: Config-adapter exports.
- `root_layout.rs`: Root layout resolution.
- `loader.rs`: Root config loading.
- `package_loader.rs`: Package discovery and loading.
