# Local Rules

## Architecture
- Position: Stable design constraints for the repository.
- Logic: Record why the workspace is shaped this way before implementation details drift.
- Constraints: Prefer amendments over rewrites when design intent evolves.

## Members
- `CHAINBOT_WORKSPACE_DESIGN.md`: Defines the Cargo workspace layout, ownership boundaries, and agent-facing repository invariants.
- `CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md`: Defines the stable TOML structure, runtime precedence, and trigger/workflow loading contract for ChainBot roots.
