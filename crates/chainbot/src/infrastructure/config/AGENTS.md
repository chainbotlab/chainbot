# Local Rules

## Scope
- Position: Infrastructure configuration adapter subtree.
- Logic: Owns root layout resolution, root config decoding, and package loading adapters.
- Constraints: Keep backend-agnostic configuration contracts in `../../domain/`.

## Members
- `mod.rs`: Config adapter module boundary.
- `root_layout.rs`: Workspace root layout discovery adapter.
- `loader.rs`: Root config decoding and assembly adapter.
- `package_loader.rs`: Package-level loading and integration adapter.
