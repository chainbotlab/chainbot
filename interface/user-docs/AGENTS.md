# AGENTS.md

## Scope
- Position: Mintlify user-facing documentation site.
- Owns: Setup guides, concepts, CLI reference, and operator guidance.
- Excludes: Implementation internals and duplicated design docs.

## Constraints
- Pages must derive from repository evidence only.
- Do not invent APIs, commands, or features that are not implemented.
- Keep navigation aligned with the current introduction, quickstart, workspace layout, CLI overview, and storage guide structure.

## Members
- `docs.json`: Navigation and site configuration.
- `package.json`: App-local tooling manifest.
- `pages/`: User-facing documentation pages.
