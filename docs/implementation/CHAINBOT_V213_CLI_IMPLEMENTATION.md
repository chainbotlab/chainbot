# ChainBot V2.1.3 CLI Implementation

## Scope

- Add `chainbot init` to bootstrap a minimal, immediately valid ChainBot root.
- Extend `chainbot trigger` with `list` alongside the existing `enable` and `disable` actions.
- Bump the crate release version to `2.1.3`.
- Align README and stable CLI design docs with the new operator surface.

## Files Changed

- `crates/chainbot/src/cli.rs`
- `crates/chainbot/tests/cli_surface.rs`
- `docs/design/CHAINBOT_CLI_DESIGN.md`
- `docs/implementation/CHAINBOT_V213_CLI_IMPLEMENTATION.md`
- `docs/implementation/AGENTS.md`
- `AGENTS.md`
- `README.md`
- `crates/chainbot/Cargo.toml`
- `Cargo.lock`

## Runtime Changes

- `init` now creates the resolved root directory plus `config/`, `workflows/`, `triggers/`, `plugins/`, `plugins/manifests/`, `plugins/bin/`, `secrets/`, and `state/`.
- `init` writes `config/root.toml` when it is missing and reuses existing bootstrap files or directories without recursive overwrite.
- `trigger list` now renders the configured trigger packages from the current root config and trigger package definitions.
- `trigger enable` and `trigger disable` keep their existing persisted-state semantics.

## Bootstrap Template

The generated `config/root.toml` uses this minimal contract-compatible template:

```toml
manifest_version = "2.0.0"
profile = "default"
secret_refs = []

[paths]
workflows_dir = "workflows"
triggers_dir = "triggers"
plugins_dir = "plugins"
secrets_dir = "secrets"
state_dir = "state"

[plugins]
manifest_globs = ["plugins/manifests/*.toml"]

[runtime_defaults]
timezone = "UTC"
```

## Validation

- CLI integration coverage in `crates/chainbot/tests/cli_surface.rs` for `help init`, `init`, idempotent `init`, and `trigger list`.
- Stable CLI contract alignment in `docs/design/CHAINBOT_CLI_DESIGN.md`.
- Crate manifest and lockfile version alignment at `2.1.3`.

## Notes

- `init` intentionally bootstraps only the root layout and root config; it does not generate workflow or trigger business templates.
- `trigger list` is config-driven operator output and does not inspect runtime trigger records or serve lease coordination.
