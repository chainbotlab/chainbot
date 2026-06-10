# AGENTS.md

## Scope
- Position: Repository documentation root.
- Owns: Documentation bucket taxonomy and routing for durable project knowledge.
- Excludes: Source-code ownership, runtime implementation, and workspace member boundaries.

## Constraints
- Keep docs concise, durable, and linked from root context.
- Add new durable knowledge to the narrowest suitable bucket.
- Update root `AGENTS.md` when a new top-level documentation area appears.

## Members
- `decisions/`: Migrated legacy redirect bucket; active decision authority lives in `.agents/skills/decision-{slug}/SKILL.md`.
- `engineering/`: Implementation records, execution details, and validation notes.
- `research/`: Exploratory notes, option analysis, and interim conclusions.
- `postmortem/`: Durable incident records and debugging learnings.
- `specs/`: AI-generated task specification docs.
- `archive/`: Retired docs, historical decision snapshots, and tombstones.
