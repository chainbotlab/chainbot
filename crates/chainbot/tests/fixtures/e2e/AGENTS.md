# AGENTS.md

## Scope
- Position: Vertical-slice end-to-end fixture roots.
- Owns: Full root-layout snapshots copied into `target/test-roots/e2e`.
- Excludes: Real credentials and remote dependencies.

## Constraints
- Keep fixtures deterministic, local-only, and credential-free.

## Members
- `success/`: Canonical runnable fixture root with workflow, trigger, plugin packages, and secret references.
- `failure_missing_secret/`: Valid manifests that intentionally fail secret resolution at runtime.
