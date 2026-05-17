# AGENTS.md

## Scope
- Position: Leaf fixture folder for pass-style encrypted secret files.
- Owns: Encrypted placeholder files mapped from `secret://ops/slack/...` references.
- Excludes: Arbitrary filenames and plaintext payloads.

## Constraints
- Filenames must follow secret lookup semantics as `<name>.gpg`.

## Members
- `webhook.gpg`: Placeholder encrypted secret used by Slack-oriented tests.
