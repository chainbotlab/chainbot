# Local Rules

## Architecture
- Position: Mintlify documentation site for ChainBot user-facing content.
- Logic: Source of truth for setup guides, concepts, CLI reference, and operator guidance.
- Constraints: No implementation internals, no duplication of design docs.

## Members
- `docs.json`: Mintlify project configuration.
- `package.json`: App-local Node tooling with `mintlify` as the local package and `mint` as the CLI entrypoint.
- `pages/`: MDX pages mapped to navigation.

## Conventions
- Pages derive from repository evidence (README, design docs, examples).
- No fabricated APIs, commands, or features absent from repo docs.
- Navigation order: Introduction, Quickstart, Workspace Layout, CLI Overview, Storage Guide.

## Review Triggers
- Add or remove a user-facing page.
- Change navigation structure or top-level grouping.
