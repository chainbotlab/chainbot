# ChainBot

ChainBot is a Rust workspace for the `chainbot` CLI application.

## Workspace Layout

This repository keeps the root as a pure Cargo workspace. Application code lives under `crates/`.

```text
.
|- crates/
|  `- chainbot/
|- docs/
`- postmortem/
```

The primary crate is `crates/chainbot`.

## Documented CLI Surface

The `chainbot` binary currently provides these runtime commands:

- `help`
- `init`
- `status`
- `trigger`
- `validate`
- `run`
- `serve`
- `list-runs`

The v2.1.3 CLI design resolves the ChainBot root from `CHAINBOT_CONFIG_DIR` when it is set to a non-empty path.
If `CHAINBOT_CONFIG_DIR` is unset or empty, the documented contract falls back to the default root at `~/.chainbot`.
The current runtime parser follows that contract directly and no longer exposes `--root` as a public CLI option.
`status` also supports `--json` for structured status snapshots.
`init` bootstraps a minimal root layout with `config/root.toml` and the default package directories.
`trigger` supports `list`, `enable`, and `disable` actions for persisted trigger package state.

## Expected Root Layout

The current runtime and tests expect a ChainBot root with these directories:

```text
<root>/
|- config/
|- workflows/
|- triggers/
|- plugins/
|- secrets/
`- state/
```

## Development

Validate the workspace with:

```bash
cargo metadata --no-deps
cargo check --workspace
cargo test --workspace
```

Do not run formatting commands in this repository.

## Version

The current crate version is `2.1.3`.

## Documentation

- Design constraints: `docs/design/CHAINBOT_WORKSPACE_DESIGN.md`
- Workspace bootstrap record: `docs/implementation/WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md`
- MVP implementation record: `docs/implementation/CHAINBOT_V2_MVP_IMPLEMENTATION.md`
- CLI v2.1.3 implementation record: `docs/implementation/CHAINBOT_V213_CLI_IMPLEMENTATION.md`
