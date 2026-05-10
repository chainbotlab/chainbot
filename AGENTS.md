# AGENTS.md

> AI agent root index for this repository. Read this file before touching code or docs.

## Project
- Repository: ChainBot Cargo workspace for workflow automation, trigger orchestration, and plugin-backed runtime execution.
- Entry rule: Read root `AGENTS.md`, then the nearest local `AGENTS.md`, then any linked design or implementation docs.

docs/
|- decisions/
|- engineering/
|- research/
|- postmortem/
`- archive/
## Topology
- `.agents/`: Project-local agent assets and repository-specific skills.
- `crates/`: Cargo workspace members and crate-local manifests.
- `docs/`: Long-lived decisions, engineering records, research, postmortems, and archive knowledge.
- `examples/`: Copyable workspace roots aligned with the stable ChainBot contract.
- `official-plugins/`: Repository-local official remote plugin source catalog used for discoverability and install contract fixtures.

## Local Maps
- `docs/AGENTS.md`: Documentation taxonomy and bucket-level ownership.
- `crates/AGENTS.md`: Workspace member inventory.
- `crates/chainbot/AGENTS.md`: `chainbot` crate boundary and test surface.
- `crates/chainbot/src/AGENTS.md`: Frozen source-tree module map and root Rust file constraints.
- `official-plugins/AGENTS.md`: Official plugin package inventory and local source-package constraints.
- `docs/postmortem/AGENTS.md`: Postmortem naming rules, retained structure, and writing triggers.

## Crates
- `crates/chainbot/`: Primary CLI, runtime, builtin, ingress, and plugin host crate.

## Source Catalog
- `chainbot-plugin-index.toml`: Repository-local official remote plugin source index.
- `official-plugins/`: Official remote plugin source packages used as first-party catalog examples.

## Global Constraints
- Required pre-read checks: root `AGENTS.md`, then nearest local `AGENTS.md`, then referenced docs in `docs/`.
- Required skills: use `fractal-context` and `fractal-repo` for code or doc structure changes; do not copy skill contents into this file.
- Workspace rule: treat the repository root as a pure Cargo workspace; application code lives under `crates/`.
- Formatting rule: do not run `fmt`, `cargo fmt`, or `rustfmt`; formatting invalidates cache and is intentionally skipped in this repository.

## Review Triggers
- Update root or nearest local `AGENTS.md` when modules move, ownership changes, contracts change, or a new top-level area appears.
- Record a postmortem when debugging exposes a reusable design mistake, an upstream/local contract mismatch, or a failure pattern worth preventing from recurring.
