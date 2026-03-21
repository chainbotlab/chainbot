# ChainBot Workspace Design

## Goal

Initialize the repository as a Rust workspace rooted at `crates/` while keeping repository guidance lightweight and durable for agents.

## Invariants

- The repository root is a pure Cargo workspace.
- Runnable or reusable Rust code lives under `crates/`.
- Repository guidance is indexed from root `AGENTS.md` and refined by local `AGENTS.md` files.
- Agents use `fractal-context` and `fractal-repo` by reference only.
- Agents must not run formatting commands because formatting invalidates cache.

## Target Layout

```text
.
|- .opencode/
|- examples/
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
- A pure workspace root prevents application concerns from leaking into repository governance files.
- Fractal manifests keep local rules close to the files they govern.

## Change Triggers

- Add or remove a workspace member.
- Introduce a new top-level repository area.
- Change the rules around agent execution or repository validation.
