# ChainBot Config and State Layout Implementation

## Scope

- Implement canonical plugin package discovery from `plugins/<plugin_id>/config.toml`.
- Implement canonical runtime-state reads and writes under run-scoped and trigger-scoped directories.
- Remove legacy root-config, plugin discovery, and runtime-state compatibility.
- Cut the manifest-bearing config contract to `2.0.0` for canonical-only roots.
- Align integration tests, fixtures, and stable docs with the canonical-only contract.

## Files Changed

- `crates/chainbot/src/config.rs`
- `crates/chainbot/src/state.rs`
- `crates/chainbot/tests/config_loading.rs`
- `crates/chainbot/tests/state_runtime_persistence.rs`
- `crates/chainbot/tests/trigger_plane.rs`
- `crates/chainbot/tests/cli_surface.rs`
- `crates/chainbot/tests/end_to_end_vertical_slice.rs`
- `docs/archive/decisions/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md`
- `docs/archive/decisions/CHAINBOT_CLI_DESIGN.md`
- `docs/engineering/CHAINBOT_CONFIG_STATE_LAYOUT_IMPLEMENTATION.md`
- `docs/engineering/AGENTS.md`
- `docs/research/CHAINBOT_LEGACY_LAYOUT_CONVERGENCE_PROPOSAL.md`
- `docs/research/AGENTS.md`
- `AGENTS.md`
- `crates/chainbot/src/AGENTS.md`
- `crates/chainbot/tests/fixtures/e2e/AGENTS.md`

## Config Loading Changes

- `RootDefinitionBundle::load` now loads plugin definitions only from canonical plugin packages under `plugins/<plugin_id>/config.toml`.
- Canonical plugin package loading validates package identity so the package directory name must match `plugin_id`.
- Legacy root-config fallback from `<root>/config/root.toml` was removed.
- Legacy plugin discovery settings and `manifest_globs` support were removed.
- Manifest-bearing config surfaces now require major `3` rather than accepting the legacy-compatible `2.x` line.

## Plugin Host Compatibility

- Plugin `manifest_path` continues to anchor executable resolution.
- External node and external trigger plugin executables continue to resolve relative to the manifest file.
- Plugins-root containment rules remain unchanged.
- Canonical plugin packages therefore gain local `bin/` support without weakening executable path guards.

## Runtime State Changes

Canonical write paths are now:

- run summaries: `state/runs/<run_id>/summary.json`
- workflow logs: `state/runs/<run_id>/workflow-logs/<sequence>.json`
- trigger records: `state/triggers/<trigger_id>/records/<sequence>-<event>.json`
- trigger checkpoints: `state/triggers/<trigger_id>/checkpoint.json`

Canonical-only runtime-state behavior now applies:

- workflow log recovery scans only `state/runs/<run_id>/workflow-logs/`
- trigger record recovery and loading scan only `state/triggers/<trigger_id>/records/`
- trigger checkpoint reads load only `state/triggers/<trigger_id>/checkpoint.json`

## Trigger-Plane Guarantees Preserved

- `TriggerPlane::open` still rebuilds dedup and cooldown coordination from persisted trigger records.
- External trigger resume still reads persisted checkpoints through `read_trigger_checkpoint()`.
- Trigger records remain correctness-bearing durable state rather than disposable logs.
- `node.manifest_version` semantics were not changed during this implementation.

## Versioning Cut

- `root_config.manifest_version` now requires major `3`
- `workflow.manifest_version` now requires major `3`
- `node.manifest_version` now requires major `3`
- `trigger.manifest_version` now requires major `3`
- `plugin.manifest_version` now requires major `3`
- runtime protocol versions and state record schema versions were intentionally not bumped by this layout-only cut

## Test Coverage Added or Updated

- canonical plugin packages are discovered successfully
- canonical plugin packages are discovered successfully
- v3 canonical root fixtures load successfully
- legacy root-config and legacy plugin/state layouts are no longer part of passing fixtures
- canonical workflow-log and trigger-record paths are asserted directly
- end-to-end persistence checks assert canonical `runs/` and `triggers/` state directories

## Validation

- `lsp_diagnostics` on modified Rust source and test files
- `cargo test -p chainbot --test config_loading`
- `cargo test -p chainbot --test state_runtime_persistence`
- `cargo test -p chainbot --test trigger_plane`
- `cargo test -p chainbot --test cli_surface`
- `cargo test -p chainbot --test end_to_end_vertical_slice`
- `cargo test --workspace`

## Notes

- This implementation is the canonical-only contract cut, not a compatibility bridge.
- Older binaries are not expected to read `3.x` canonical-only roots.
