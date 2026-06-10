# ChainBot Interface Surfaces Implementation

## Summary

This change adds a new top-level `interface/` area with two standalone frontend surfaces:

- `interface/land-page/` — Astro landing page for product introduction and docs entry
- `interface/user-docs/` — Mintlify user documentation site

The repository root remains a pure Cargo workspace. No root-level JavaScript workspace was introduced.

## Structure

```text
interface/
|- AGENTS.md
|- land-page/
|  |- AGENTS.md
|  |- package.json
|  |- astro.config.mjs
|  |- src/
|  `- public/
`- user-docs/
   |- AGENTS.md
   |- docs.json
   |- package.json
   |- pages/
   `- scripts/
```

## Validation

### Landing page

```bash
cd interface/land-page
npm install
npm test
npm run check
npm run build
```

Observed result in this implementation pass:

- `npm test` passed
- `npm run check` passed
- `npm run build` passed

### User docs

```bash
cd interface/user-docs
npm install
npm test
npm run build
```

Observed result in this implementation pass:

- `npm test` passed via `scripts/validate-docs.mjs`
- `npm run build` invoked `mint validate`, but the installed Mintlify CLI crashed with a React hook error before content validation completed

The structural docs validation remains in place so navigation and frontmatter are still checked inside the repository.

## Notes

- `.gitignore` was updated so app-local `package.json` and `package-lock.json` files can be tracked.
- `AGENTS.md`, `README.md`, `CONTRIBUTING.md`, and `docs/archive/decisions/CHAINBOT_WORKSPACE_DESIGN.md` were updated to acknowledge `interface/`.
- `docs/user/AGENTS.md` now points to the active Mintlify docs surface instead of claiming there are no active user-facing docs.
