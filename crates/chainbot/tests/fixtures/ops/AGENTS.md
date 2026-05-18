# AGENTS.md

## Scope
- Position: Pass-style secret fixture namespace root.
- Owns: Filesystem trees that mirror `secret://ops/...` lookups in tests.
- Excludes: Plaintext secrets.

## Constraints
- Only placeholder fixture material belongs here.

## Members
- `slack/`: Slack-oriented fake secret material.
- `chain/`: Reserved chain-oriented secret fixture namespace.
