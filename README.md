# ChainBot

ChainBot is a Rust-based CLI application for managing automation workflows, triggers, and plugins.

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

# Run a workflow
chainbot run <workflow-name>

# Serve the web interface
chainbot serve

# Manage triggers
chainbot trigger list
chainbot trigger enable <trigger-id>
chainbot trigger disable <trigger-id>
```

## Workspace Structure

When you run `chainbot init`, it creates this layout:

```text
<root>/
|- config/        # Configuration files
|- workflows/    # Workflow definitions
|- triggers/     # Trigger state persistence
|- plugins/      # Plugin binaries and configs
|- secrets/      # Secret management
`- state/        # Runtime state
```

The workspace root defaults to `~/.chainbot`. Set `CHAINBOT_CONFIG_DIR` to override.

## CLI Commands

| Command | Description |
|---------|-------------|
| `chainbot help` | Show help information |
| `chainbot init` | Initialize a new workspace |
| `chainbot status` | Show workspace status (use `--json` for structured output) |
| `chainbot trigger` | Manage triggers: `list`, `enable`, `disable` |
| `chainbot validate` | Validate workspace configuration |
| `chainbot run` | Execute a workflow |
| `chainbot serve` | Start the web interface |
| `chainbot list-runs` | List workflow run history |

## Configuration

Main configuration file: `config/root.toml`

## Version

Current version: **2.1.3**

## Links

- [Contributing Guide](CONTRIBUTING.md) — Development setup and architecture
- [CLI Design](docs/design/CHAINBOT_CLI_DESIGN.md) — CLI command reference
- [Workspace Design](docs/design/CHAINBOT_WORKSPACE_DESIGN.md) — Root layout specification
- [Trigger & Workflow Design](docs/design/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md) — Workflow definition format
