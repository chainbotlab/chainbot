# Local Rules

## Architecture
- Position: Fixture root for ChainBot task-11 vertical-slice integration tests.
- Logic: Contains full root-layout snapshots copied into `target/test-roots/e2e` before command execution.
- Constraints: Keep fixtures deterministic, local-only, and free of real credentials.

## Members
- `success/`: Runnable fixture root with one workflow, external trigger plugin, external node plugin, script node, builtin node, and secret reference.
- `failure_missing_secret/`: Runnable fixture root that keeps definitions valid but fails at runtime secret resolution.
