# Local Rules

## Architecture
- Position: Integration-test fixture assets for the `chainbot` crate.
- Logic: Fixture files model filesystem inputs consumed by runtime tests.
- Constraints: Keep fixtures deterministic and free of real credentials.

## Members
- `ops/`: Pass-style fixture hierarchy for secret-provider runtime tests.
- `workers/`: Deterministic script worker fixtures for subprocess worker-host tests.
- `e2e/`: Full root-layout snapshots for task-11 end-to-end vertical-slice validation.
