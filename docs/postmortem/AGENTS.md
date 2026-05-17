# AGENTS.md

## Scope
- Position: Durable incident and debugging knowledge base for the repository.
- Owns: Postmortem records that preserve root cause, fix validation, and recurrence-prevention guidance after non-trivial failures.
- Excludes: Temporary debugging notes, speculative hypotheses that were never validated, and implementation changelogs better kept in `docs/engineering/`.

## Constraints
- Write a postmortem only when the outcome is worth preserving beyond the immediate fix.
- Name files `YYYYMMDD-bug-description-en.md`.
- Each postmortem should retain incident metadata plus `Symptom`, `Root Cause`, `Fix Applied`, `Validation`, and `Preventive Safeguards` sections.
- Evidence should point to concrete logs, tests, code paths, or explicit placeholders such as `Not recorded`.

## Members
- `AGENTS.md`: Folder manifest for postmortem naming, retained structure, and evidence rules.
- `20260511-hyperliquid-node-origin-binding-not-enforced-en.md`: Records the pre-ship Hyperliquid node bug where manifest-declared origin binding was not enforced at runtime.
