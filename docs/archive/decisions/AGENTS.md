# AGENTS.md

## Scope
- Position: Historical decision archive bucket.
- Owns: Archived decision snapshots and tombstones retained for context after active decision authority moves elsewhere.
- Excludes: Current decision authority and implementation records.

## Constraints
- Active decision authority lives in `.agents/skills/decision-{slug}/SKILL.md`.
- Treat archived copies as historical context, not current project truth.
- Preserve enough rationale to explain why an archived decision existed.

## Members
- Archived decision docs.
