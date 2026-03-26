# ChainBot

ChainBot is a Rust-based CLI application for managing automation workflows, triggers, and plugins.

This repository now also includes app-local interface surfaces under `interface/`:

- `interface/land-page/` — Astro marketing site for the product narrative
- `interface/user-docs/` — Mintlify user-facing documentation site

## What is ChainBot?

ChainBot is a workspace-based automation framework that lets you define and run triggered workflows. It provides a plugin architecture for extensibility and supports persistent trigger states.

## Quick Start

```bash
# Install from source
cargo install --path crates/chainbot

# Initialize a new workspace
chainbot init

# Check status
chainbot status

# Run the inferred top-level workflow
chainbot run

# Start the background daemon control plane
chainbot serve

# Request graceful shutdown
chainbot stop

# Manage triggers
chainbot trigger list
chainbot trigger enable <trigger-id>
chainbot trigger disable <trigger-id>
```

## Workspace Structure

When you run `chainbot init`, it creates this minimal canonical layout:

```text
<root>/
|- chainbot.toml  # Root configuration file
|- workflows/    # Workflow definitions
|- triggers/     # Trigger state persistence
|- plugins/
|  `- bin/       # Shared plugin executables bootstrap directory
|- secrets/      # Secret management
`- state/        # Runtime state
```

Installed plugin packages live under `plugins/<plugin_id>/config.toml` when present. The older shared `plugins/manifests/` layout is not created by `chainbot init` and is not part of the current stable bootstrap contract.

The workspace root defaults to `~/.chainbot`. Set `CHAINBOT_CONFIG_DIR` to override.

## Repository Surfaces

```text
.
|- Cargo.toml
|- crates/
|- interface/
|  |- land-page/
|  `- user-docs/
|- docs/
`- examples/
```

The repository root remains a pure Cargo workspace. Frontend tooling is isolated inside each app under `interface/` and does not introduce a root JavaScript workspace.

## CLI Commands

| Command | Description |
|---------|-------------|
| `chainbot help` | Show help information |
| `chainbot version` | Show the running ChainBot version |
| `chainbot init` | Initialize a new workspace |
| `chainbot status` | Show workspace status (use `--json` for structured output) |
| `chainbot observe` | Inspect persisted trigger events, workflow logs, and runs |
| `chainbot stop` | Request graceful shutdown of the background daemon |
| `chainbot trigger` | Manage triggers: `list`, `enable`, `disable` |
| `chainbot validate` | Validate workspace configuration |
| `chainbot run` | Execute a workflow |
| `chainbot serve` | Start the background daemon control plane |
| `chainbot list-runs` | List workflow run history |

## Configuration

Main configuration file: `chainbot.toml`

For complete copyable roots, see [`examples/`](examples/README.md).

Use `chainbot help validate`, `chainbot help trigger`, and `chainbot help run` for CLI-embedded config examples.

### Root config example

```toml
manifest_version = "2.0.0"
chainbot_version = "2.2.0"
profile = "basic"
secret_refs = ["secret://ops/slack/webhook#token"]

[paths]
workflows_dir = "workflows"
triggers_dir = "triggers"
plugins_dir = "plugins"
secrets_dir = "secrets"
state_dir = "state"

[runtime_defaults]
timezone = "UTC"
```

### Workflow package example

`workflows/wf-alpha/config.toml`

```toml
[workflow]
manifest_version = "2.0.0"
id = "wf-alpha"
name = "alpha"
description = "Normalize a quote payload"

[runtime.defaults]
symbol = "BTCUSDT"

[[nodes]]
manifest_version = "2.0.0"
id = "normalize"
kind = "plugin"
plugin = "quote-plugin"
operation = "normalize"
depends_on = []
```

### Trigger package example

`triggers/tr-market/config.toml`

```toml
manifest_version = "2.0.0"
trigger_id = "tr-market"
kind = "builtin"
source = "market_tick"
workflow_id = "wf-alpha"
enabled = true

[params]
symbol = "BTCUSDT"

[input_mapping]
symbol = "payload.symbol"
price = "payload.price"
```

### Plugin package example

`plugins/quote-plugin/config.toml`

```toml
manifest_version = "2.0.0"
plugin_id = "quote-plugin"
kind = "external_node"
entrypoint = "node.exec.v1"
capabilities = ["normalize"]
executable = "bin/quote-plugin.sh"
```

## Version

Current version: **2.2.0**

## Links

- [Contributing Guide](CONTRIBUTING.md) — Development setup and architecture
- [CLI Design](docs/design/CHAINBOT_CLI_DESIGN.md) — CLI command reference
- [Workspace Design](docs/design/CHAINBOT_WORKSPACE_DESIGN.md) — Root layout specification
- [Trigger & Workflow Design](docs/design/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md) — Workflow definition format
- [Landing Page Source](interface/land-page/src/pages/index.astro) — Product-introduction interface
- [User Docs Config](interface/user-docs/docs.json) — Mintlify navigation and branding surface
