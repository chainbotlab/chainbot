# Local Rules

## Architecture
- Position: Top-level interface surfaces that sit alongside the Cargo workspace without changing the root into a JavaScript monorepo.
- Logic: Shared repository policy -> `interface/AGENTS.md` -> app-local files inside `land-page/` and `user-docs/`.
- Constraints: Keep Node tooling app-local, avoid root `package.json`, and keep cross-app coupling minimal.

## Members
- `land-page/`: Astro marketing site for the ChainBot product narrative and primary calls to action.
- `user-docs/`: Mintlify documentation site for user-facing setup, concepts, and operator guidance.

## Dependencies
- Root `AGENTS.md`: Repository-wide topology, critical paths, and workspace constraints.
- `docs/`: Source material for durable user-facing docs and implementation records.

## Review Triggers
- Add or remove an interface app.
- Change the shared branding, navigation contract, or deployment boundary for interface surfaces.
- Introduce root-level JavaScript tooling or any coupling that affects the pure Cargo workspace rule.
