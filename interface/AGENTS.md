# AGENTS.md

## Scope
- Position: Top-level interface surfaces that sit alongside the Cargo workspace.
- Owns: Shared policy for repo-adjacent frontend and documentation apps.
- Excludes: Root-level JavaScript monorepo tooling and Cargo workspace membership.

## Constraints
- Keep Node tooling app-local; do not introduce a root `package.json`.
- Keep cross-app coupling minimal.
- Treat root repository policy and `docs/` as upstream source material rather than duplicating it here.

## Members
- `land-page/`: Astro marketing site for the ChainBot product narrative and primary calls to action.
- `user-docs/`: Mintlify documentation site for user-facing setup, concepts, and operator guidance.

## Docs
- `../AGENTS.md`: Root repository topology and global workspace constraints.
- `../docs/`: Durable source material for user-facing docs and implementation records.
