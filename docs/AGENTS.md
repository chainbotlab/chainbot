# Local Rules

## Architecture
- Position: Repository documentation root.
- Logic: Root policies -> category documents -> folder-local manifests.
- Constraints: Keep docs concise, durable, and linked from root context.

## Members
- `decisions/`: Long-lived architectural decisions and invariants.
- `engineering/`: Execution notes, validation expectations, and bootstrapping details.
- `research/`: Exploration notes and discarded options when they become necessary.
- `postmortem/`: Durable incident records and debugging learnings.
- `archive/`: Retired docs and tombstones.

## Conventions
- Add new durable knowledge to the narrowest suitable bucket.
- Update root `AGENTS.md` when a new top-level doc area is introduced.
