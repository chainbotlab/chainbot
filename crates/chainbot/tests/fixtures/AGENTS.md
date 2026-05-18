# AGENTS.md

## Scope
- Position: Integration-test fixture asset root.
- Owns: Deterministic filesystem inputs copied into test roots.
- Excludes: Real credentials and nondeterministic data.

## Constraints
- Fixtures must stay deterministic and credential-free.

## Members
- `ops/`: Pass-style secret fixture namespace.
- `workers/`: Worker fixture payloads used by host/runtime tests.
- `e2e/`: Full root-layout vertical-slice fixture roots.
