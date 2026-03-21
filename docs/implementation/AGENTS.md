# Local Rules

## Architecture
- Position: Implementation records for repository-level work.
- Logic: Capture what was changed, how it is validated, and what constraints matter during execution.
- Constraints: Keep this focused on actionable implementation detail, not long-lived design rationale.
- Constraints: Treat implementation records as append-only history; do not rewrite older implementation docs to match newer work.
- Constraints: If an implementation approach becomes obsolete, redundant, or superseded, move that material to `docs/archive/` instead of mutating historical implementation records.

## Members
- `WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md`: Bootstrap checklist and validation matrix for the initial repository setup.
- `CHAINBOT_V2_MVP_IMPLEMENTATION.md`: Records the MVP rename, artifact alignment, and release-version update for ChainBot V2.
- `CHAINBOT_V21_CONFIG_IMPLEMENTATION.md`: Records the v2.1 package-layout implementation, root-config overrides, and validation coverage.
- `CHAINBOT_V212_CLI_IMPLEMENTATION.md`: Records the v2.1.2 CLI root-resolution contract update and release-version alignment.
- `CHAINBOT_V213_CLI_IMPLEMENTATION.md`: Records the v2.1.3 CLI init bootstrap, trigger list command, and release-version alignment.
- `CHAINBOT_BUILTINS_REFACTOR_IMPLEMENTATION.md`: Records the unified builtin namespace refactor, trait-backed registries, and final public API layout.
- `CHAINBOT_TRIGGER_PARAMS_IMPLEMENTATION.md`: Records the params-backed trigger extension model, builtin cron implementation, and validation coverage.
- `CHAINBOT_CONFIG_STATE_LAYOUT_IMPLEMENTATION.md`: Records canonical plugin/state layout writes, legacy compatibility reads, and the validation coverage for the redesign.
