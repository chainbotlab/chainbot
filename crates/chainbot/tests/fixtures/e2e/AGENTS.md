# Local Rules

## Architecture
- Position: Fixture root for ChainBot task-11 vertical-slice integration tests.
- Logic: Contains full root-layout snapshots copied into `target/test-roots/e2e` before command execution.
- Constraints: Keep fixtures deterministic, local-only, and free of real credentials.

## Members
- `success/`: Runnable v3 canonical-only fixture root with one workflow package, one trigger package, one canonical plugin package, plugin executables, and a secret reference.
- `failure_missing_secret/`: Runnable v3 canonical-only fixture root that keeps package manifests valid but fails at runtime secret resolution.
