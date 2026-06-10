---
type: archive
status: superseded
updated: 2026-06-10
replacement:
  - ../../../.agents/skills/decision-chainbot-workspace-boundary/SKILL.md
---

# ChainBot Workspace Design

Archive Note: Active decision authority moved to `.agents/skills/decision-chainbot-workspace-boundary/SKILL.md` on 2026-06-10. This file is retained as a historical snapshot.

## Goal

Initialize the repository as a Rust workspace rooted at `crates/` while allowing standalone interface surfaces under `interface/` without turning the root into a JavaScript workspace.

## Invariants

- The repository root is a pure Cargo workspace.
- Runnable or reusable Rust code lives under `crates/`.
- Frontend and documentation apps live under `interface/` with app-local tooling.
- Repository guidance is indexed from root `AGENTS.md` and refined by local `AGENTS.md` files.
- Agents use `fractal-context` and `fractal-repo` by reference only.
- Agents must not run formatting commands because formatting invalidates cache.

## Target Layout

```text
.
|- .opencode/
|- examples/
|- interface/
|  |- land-page/
|  `- user-docs/
|- crates/
|  `- chainbot/
|     `- src/
|- docs/
|  |- design/
|  |- implementation/
|  |- research/
|  |- interfaces/
|  |- user/
|  `- archive/
`- postmortem/
```

## Rationale

- `crates/` gives the repository a stable expansion point for future Rust members.
- `interface/` keeps frontend and docs tooling isolated from the Cargo workspace root.
- A pure workspace root prevents application concerns from leaking into repository governance files.
- Fractal manifests keep local rules close to the files they govern.

## Change Triggers

- Add or remove a workspace member.
- Introduce a new top-level repository area.
- Change the boundary between Rust workspace members and app-local interface tooling.
- Change the rules around agent execution or repository validation.
