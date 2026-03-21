# Local Rules

## Architecture
- Position: Stable design constraints for the repository.
- Logic: Record why the workspace is shaped this way before implementation details drift.
- Constraints: Design docs in this folder must contain only the final stable design; do not include recommendation history, migration plans, or exploratory thinking.
- Constraints: Keep design docs focused on final goals, stable boundaries, invariants, and settled decisions; do not use this folder for implementation plans, execution steps, or discussion transcripts.

## Members
- `CHAINBOT_WORKSPACE_DESIGN.md`: Defines the Cargo workspace layout, ownership boundaries, and agent-facing repository invariants.
- `CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md`: Defines the stable TOML structure, runtime precedence, and trigger/workflow loading contract for ChainBot roots.
- `CHAINBOT_CLI_DESIGN.md`: Defines the stable CLI command surface, help system, status snapshot contract, and error-navigation semantics.
- `CHAINBOT_CONFIG_STATE_LAYOUT_DESIGN.md`: Defines the stable package-aligned plugin layout, manifest version roles, and durable runtime state boundaries.
