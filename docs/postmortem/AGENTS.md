# AGENTS.md

## Scope
- Position: Durable incident and debugging knowledge base for the repository.
- Owns: Postmortem records that preserve root cause, fix validation, and recurrence-prevention guidance after a non-trivial failure.
- Excludes: Temporary debugging notes, speculative hypotheses that were never validated, and implementation changelogs better kept in `docs/engineering/`.

## Constraints
- Write a postmortem only when the outcome is worth preserving beyond the immediate fix.
- Name files `YYYYMMDD-bug-description-en.md`.
- Each postmortem should retain incident metadata plus `Symptom`, `Root Cause`, `Fix Applied`, `Validation`, and `Preventive Safeguards` sections.
- Evidence should point to concrete logs, tests, code paths, or explicit placeholders such as `Not recorded`.

## Members
- `AGENTS.md`: Folder manifest for postmortem naming, retained structure, and writing triggers.

## Review Triggers
- Add or update a postmortem when debugging takes non-trivial effort, reveals a broken assumption, or yields safeguards that should be reused.
