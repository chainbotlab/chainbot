# Local Rules

## Architecture
- Position: Workspace member container.
- Logic: Root workspace manifest -> member crate manifests -> crate-local source trees.
- Constraints: Only Cargo workspace members belong here; keep each crate self-described with a local `AGENTS.md`.

## Members
- `chainbot/`: Primary application crate for the repository.
