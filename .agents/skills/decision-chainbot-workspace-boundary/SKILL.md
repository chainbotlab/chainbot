---
name: "decision-chainbot-workspace-boundary"
description: "Load when changing repository topology, Cargo workspace membership, crates/interface/official-plugins boundaries, AGENTS.md maps, or formatting command policy. Do not load for ordinary code edits inside an existing crate."
license: "Proprietary"
metadata:
  generated_by: "decision-capture"
  created: "2026-06-10"
  last_updated: "2026-06-10"
  status: "current"
  affected_modules:
    - "AGENTS.md"
    - "Cargo.toml"
    - "crates/"
    - "interface/"
    - "official-plugins/"
    - "docs/"
  supersedes:
    - "docs/archive/decisions/CHAINBOT_WORKSPACE_DESIGN.md"
  superseded_by: []
---

# Decision: ChainBot Workspace Boundary

## Context

The repository hosts Rust runtime code, standalone interface surfaces, examples,
official plugin packages, and durable documentation. These areas need clear
ownership so tooling for one area does not leak into the root or another area.

## Decision

The repository root is a pure Cargo workspace. Runnable or reusable Rust code
lives under `crates/`.

Frontend and documentation apps live under `interface/` with app-local tooling.
The root is not a JavaScript workspace.

Official plugin packages live under `official-plugins/` as standalone source
packages and are not root Cargo workspace members unless a separate workspace
boundary decision changes that.

Repository guidance is indexed from root `AGENTS.md` and refined by local
`AGENTS.md` files. Agents use fractal skills by reference; skill contents are
not inlined into repository docs.

Formatting commands are intentionally skipped in this repository. Do not run
`fmt`, `cargo fmt`, or `rustfmt` because formatting invalidates cache.

## Boundaries

- `Cargo.toml`: pure Cargo workspace membership and Rust crate topology.
- `crates/`: application/runtime Rust workspace members.
- `interface/`: app-local frontend or documentation tooling.
- `official-plugins/`: standalone source-installed plugin packages.
- `docs/`: durable knowledge buckets and archive lifecycle.
- `AGENTS.md` files: current topology, ownership, and constraints.

## Implications

Adding or removing a workspace member is a repository topology change and must
update local maps. Adding interface tooling must stay app-local under
`interface/`.

Official plugin build outputs and package metadata remain package-local; they
do not become root workspace artifacts by default.

When topology, ownership, or contract boundaries change, update the nearest
relevant `AGENTS.md`.

## Non-goals

- Define workflow root config semantics.
- Define plugin host wire protocols.
- Move interface apps into the root workspace.
- Run formatting as part of documentation or skill migration.
