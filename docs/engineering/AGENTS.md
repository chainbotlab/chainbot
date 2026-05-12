# Local Rules

## Architecture
- Position: Engineering records for repository-level work.
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
- `CHAINBOT_HELP_DIAGNOSTICS_IMPLEMENTATION.md`: Records the richer CLI help cards, canonical config examples, and precise TOML/argv diagnostics.
- `CHAINBOT_CURATED_EXAMPLES_IMPLEMENTATION.md`: Records the five-root curated examples refactor and validation coverage.
- `CHAINBOT_BUILTINS_REFACTOR_IMPLEMENTATION.md`: Records the unified builtin namespace refactor, trait-backed registries, and final public API layout.
- `CHAINBOT_CORE_BUILTIN_NODES_IMPLEMENTATION.md`: Records the first-wave core builtin flow/data nodes, their registry wiring, and validation coverage.
- `CHAINBOT_TRIGGER_PARAMS_IMPLEMENTATION.md`: Records the params-backed trigger extension model, builtin cron implementation, and validation coverage.
- `CHAINBOT_CONFIG_STATE_LAYOUT_IMPLEMENTATION.md`: Records the canonical-only plugin/state layout cut, removed legacy compatibility, and the validation coverage for the redesign.
- `CHAINBOT_STATE_READ_MODEL_IMPLEMENTATION.md`: Records the narrower run-summary reads, trigger snapshot read-model, and validation coverage for the state-read optimizations.
- `CHAINBOT_DB_PRIMARY_RUNTIME_IMPLEMENTATION.md`: Records the storage-mode schema extension and DB-primary runtime cut for CLI and trigger execution paths.
- `CHAINBOT_RUNTIME_HISTORY_IMPLEMENTATION.md`: Records the observe command, runtime history retention/archive contract, and stable guardrail coverage for hot-path runtime reads.
- `CHAINBOT_GATE_OFFICIAL_PLUGIN_IMPLEMENTATION.md`: Records the Gate spot official node/trigger plugin rollout, host-bound activation coverage, and catalog discoverability validation.
- `builtin-http-to-official-http-node-plugin-migration.md`: Records the migration of the builtin HTTP node to an official external HTTP node plugin, covering removal strategy, plugin packaging, catalog registration, and compatibility guardrails.
- `CHAINBOT_HYPERLIQUID_OFFICIAL_PLUGINS_IMPLEMENTATION.md`: Records the Hyperliquid official plugin rollout, scope boundaries, package registration, and validation coverage.
