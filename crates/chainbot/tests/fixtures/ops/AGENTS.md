# Local Rules

## Architecture
- Position: Namespace fixture root for pass-style secret resolution tests.
- Logic: Mirrors `secret://ops/...` references used by integration tests.
- Constraints: Treat files as placeholders only; never store plaintext secrets.

## Members
- `slack/`: Nested provider-path fixture for `secret://ops/slack/webhook` resolution.
