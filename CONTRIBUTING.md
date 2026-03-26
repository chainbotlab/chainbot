# Contributing to ChainBot

This guide covers the development setup and architecture for contributors.

## Project Structure

ChainBot uses a Rust workspace layout. The repository root is a pure Cargo workspace; all Rust application code lives under `crates/`, while standalone frontend surfaces live under `interface/` with app-local tooling.

```text
.
|- Cargo.toml          # Workspace manifest
|- Cargo.lock
|- crates/
|   `- chainbot/        # Main application crate
|       |- src/         # Source code
|       `- tests/       # Integration tests
|- interface/          # App-local frontend surfaces
|   |- land-page/      # Astro landing page
|   `- user-docs/      # Mintlify documentation site
|- docs/               # Project documentation
|- postmortem/         # Debugging notes and learnings
`- .sisyphus/          # Planning and evidence
```

## Development Setup

### Prerequisites

- Rust (latest stable)
- Cargo

### Build & Test

```bash
# Validate workspace structure
cargo metadata --no-deps

# Type check all crates
cargo check --workspace

# Run all tests
cargo test --workspace
```

### Interface Apps

Each frontend app keeps its own Node tooling inside `interface/`.

```bash
# Landing page
cd interface/land-page
npm install
npm test
npm run check
npm run build

# User docs
cd interface/user-docs
npm install
npm test
npm run build
```

If `mint validate` fails inside `interface/user-docs`, verify the local Mintlify CLI behavior in your environment before treating it as a content regression.

### Important Rules

**Do not run formatting commands.** This repository intentionally skips `fmt`, `cargo fmt`, and `rustfmt`. Formatting would invalidate caches and is disabled by design.

## Architecture Overview

### Crate Organization

```
crates/
└── chainbot/           # Single crate in this workspace
    └── src/            # Application source
```

### Documentation Topology

Documentation is organized by lifecycle stage:

```text
docs/
|- design/              # Architectural decisions and invariants
|- implementation/      # Implementation records and validation notes
|- research/           # Exploration notes and discarded options
|- interfaces/         # External contracts and adapters
|- user/               # User-facing behavior documentation
`- archive/             # Retired documentation
```

### Key Design Contracts

| Document | Purpose |
|----------|---------|
| `docs/design/CHAINBOT_WORKSPACE_DESIGN.md` | Root workspace layout and boundaries |
| `docs/design/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md` | Workflow/node definitions, trigger kinds, plugin linkage |
| `docs/design/CHAINBOT_CLI_DESIGN.md` | CLI command surface and semantics |

### Implementation Records

Implementation decisions are documented in `docs/implementation/`:

- `WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md` — Initial setup
- `CHAINBOT_V2_MVP_IMPLEMENTATION.md` — MVP release
- `CHAINBOT_V21_CONFIG_IMPLEMENTATION.md` — Configuration system
- `CHAINBOT_V212_CLI_IMPLEMENTATION.md` — CLI v2.1.2
- `CHAINBOT_V213_CLI_IMPLEMENTATION.md` — CLI v2.1.3

## Adding New Features

1. Check existing design docs for relevant context
2. Update design docs if adding new architectural decisions
3. Implement in the appropriate crate
4. Add integration tests in `crates/chainbot/tests/`
5. If the change affects `interface/`, keep Node tooling app-local and avoid creating a root JS workspace
6. Create an implementation record documenting the change
7. Update `AGENTS.md` if adding new critical paths

## Documentation Standards

- **Design docs**: Define invariants, contracts, and architectural decisions
- **Implementation docs**: Record what was built and how to validate it
- **Keep docs concise**: Reference other docs rather than duplicating content
- **Use frontmatter**: Add YAML metadata for machine-readable contracts

## Agent Operating Contract

When working on this codebase:

1. Read root `AGENTS.md` first
2. Read relevant local `AGENTS.md` files (in `crates/`, `docs/`, etc.)
3. Check referenced design docs before making changes
4. Use `fractal-repo` skill for code/doc structure changes
5. Keep the documentation map aligned with the actual code

## Version Management

The current version is tracked in `crates/chainbot/Cargo.toml`. Update the version in both the manifest and the implementation records when releasing.
