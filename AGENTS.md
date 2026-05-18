# AGENTS.md

> AI agent root index for this repository. Read this file before touching code or docs.

## Project
- Repository: ChainBot Cargo workspace for workflow automation, trigger orchestration, and plugin-backed runtime execution.
- Entry rule: Read root `AGENTS.md`, then the nearest local `AGENTS.md`, then any linked design or implementation docs.
- Primary domains: CLI/runtime orchestration, workflow and trigger contracts, runtime-state persistence, and source-installed plugin packages.

## Topology
- `.agents/`: Project-local agent assets and repository-specific skills.
- `crates/`: Cargo workspace members and crate-local manifests.
- `docs/`: Durable decisions, engineering records, research notes, postmortems, and archive knowledge.
- `examples/`: Copyable workspace roots aligned with the stable ChainBot contract.
- `interface/`: Repo-adjacent frontend surfaces that stay outside the Cargo workspace.
- `official-plugins/`: Repository-local official remote plugin source catalog and standalone plugin packages.

## Local Maps
- `crates/AGENTS.md`: Workspace member inventory and crate-container boundary.
- `crates/chainbot/AGENTS.md`: `chainbot` crate boundary, source tree, and integration-test surface.
- `crates/chainbot/src/AGENTS.md`: Source-tree module boundary and frozen public surface.
- `docs/AGENTS.md`: Documentation taxonomy and bucket-level ownership.
- `docs/postmortem/AGENTS.md`: Postmortem retention, naming, and evidence rules.
- `examples/AGENTS.md`: Canonical example-root constraints and package layout expectations.
- `interface/AGENTS.md`: Shared constraints for repo-adjacent interface apps.
- `official-plugins/AGENTS.md`: Official plugin package inventory and source-install contract boundary.

## Global Constraints
- Required pre-read order is root `AGENTS.md`, then the nearest local `AGENTS.md`, then referenced docs.
- Use `fractal-context` and `fractal-repo` for code or doc structure changes; do not inline skill contents into repository docs.
- Treat the repository root as a pure Cargo workspace; application code lives under `crates/` and interface tooling stays app-local.
- Do not run `fmt`, `cargo fmt`, or `rustfmt`; formatting invalidates cache and is intentionally skipped in this repository.
- Update the nearest relevant `AGENTS.md` when topology, ownership, or contract boundaries change.
- Record a postmortem when debugging exposes a reusable design mistake, a contract mismatch, or a safeguard worth preserving.
