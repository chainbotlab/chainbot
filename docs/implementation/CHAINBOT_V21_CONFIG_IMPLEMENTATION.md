# ChainBot V2.1 Config Implementation

## Scope

- Implement workflow package discovery from `workflows/*/config.toml`.
- Implement trigger package discovery from `triggers/*/config.toml`.
- Implement shared plugin manifest discovery from `plugins/manifests/*.toml`.
- Align Rust contracts with v2.1 manifest fields and runtime boundaries.
- Add root-config support for `paths` overrides and plugin manifest discovery settings.

## Files Changed

- `crates/chainbot/src/config.rs`
- `crates/chainbot/src/workflow.rs`
- `crates/chainbot/src/trigger.rs`
- `crates/chainbot/src/plugin.rs`
- `crates/chainbot/src/executor.rs`
- `crates/chainbot/src/cli.rs`
- `crates/chainbot/tests/config_loading.rs`
- `crates/chainbot/tests/contract_versions.rs`
- `crates/chainbot/tests/execution_scheduler.rs`
- `crates/chainbot/tests/workflow_dag_semantics.rs`
- `crates/chainbot/tests/trigger_plane.rs`
- `crates/chainbot/tests/node_plugin_host.rs`
- `crates/chainbot/tests/fixtures/e2e/`

## Implemented Contract Changes

- `WorkflowDefinition` now parses v2.1 package manifests with `[workflow]` headers and no embedded triggers.
- `TriggerDefinition` now owns `workflow_id`, optional `plugin`, `params`, and `input_mapping`.
- `PluginManifest` now resolves executable paths relative to the manifest file while enforcing containment under the shared `plugins/` root.
- `RootDefinitionBundle::load` now validates cross-manifest invariants during load:
  - unique workflow ids
  - unique trigger ids
  - unique plugin ids
  - trigger-to-workflow references must resolve
  - package directory names must match workflow/trigger identities

## Root Config Additions

`config/root.toml` now supports:

- `[paths]` overrides for `workflows_dir`, `triggers_dir`, `plugins_dir`, `secrets_dir`, and `state_dir`
- `[plugins]` discovery settings via `manifest_globs`
- `[runtime_defaults]` as root-level execution defaults

Root-config path/discovery constraints:

- all `[paths]` values must be root-relative and stay within `<root>`
- `manifest_globs` currently supports root-relative `<dir>/*.toml` entries only
- plugin discovery paths are validated before manifest loading

The loader now resolves the root in two phases:

1. Read `config/root.toml` from the bootstrap layout.
2. Apply root-config overrides to build the effective runtime layout.

## Runtime Behavior Changes

- `chainbot run` now refuses ambiguous multi-workflow roots instead of silently picking the first workflow.
- `chainbot serve` still runs through `TriggerPlane`, but builtin trigger emissions now target the workflow declared on each trigger definition.
- Trigger payload mapping now preserves non-object payloads by binding them under `payload` in `trigger_payload_mapping`.

## Fixture Alignment

- End-to-end fixture roots now use package layout for workflows and triggers.
- Shared plugin manifests moved under `plugins/manifests/`.
- Legacy flat workflow/trigger fixture files were removed.

## Validation

- `cargo check --workspace`
- `cargo test --workspace`
- `lsp_diagnostics` on modified Rust source files

## Follow-Up Constraints

- Keep `docs/design/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md` as the final contract source.
- Record future implementation-only behavior changes here instead of expanding the design doc.
