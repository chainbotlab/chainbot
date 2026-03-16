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

## Current CLI Surface

The `chainbot` binary currently provides these commands:

- `help`
- `validate`
- `run`
- `serve`
- `list-runs`

The command parser supports `--root <path>` for command execution against an explicit ChainBot root.

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

The current crate version is `2.0.0`.

## Documentation

- Design constraints: `docs/design/CHAINBOT_WORKSPACE_DESIGN.md`
- Workspace bootstrap record: `docs/implementation/WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md`
- MVP implementation record: `docs/implementation/CHAINBOT_V2_MVP_IMPLEMENTATION.md`
