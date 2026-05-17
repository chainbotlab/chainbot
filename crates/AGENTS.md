# AGENTS.md

## Scope
- Position: Cargo workspace member container.
- Owns: Workspace member directories and crate-local manifests under the repository root.
- Excludes: Repo-adjacent interface apps, docs buckets, and standalone official plugin packages.

## Constraints
- Only Cargo workspace members belong under `crates/`.
- Each workspace member keeps its own local `AGENTS.md` for crate-specific boundaries.

## Members
- `chainbot/`: Primary application crate for the repository.
