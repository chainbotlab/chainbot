# Examples

This folder contains curated ChainBot roots you can copy, inspect, or adapt.

## Which example should I start with?

| Example | Start here when | Shows | Quick check |
|---|---|---|---|
| `single-workflow/` | you want the smallest possible ChainBot root | one workflow package, builtin node wiring, canonical root layout | `CHAINBOT_CONFIG_DIR=examples/single-workflow target/debug/chainbot validate` |
| `core-builtins/` | you want to learn the builtin flow/data node toolbox | JSON parsing, data shaping, fallback selection, comparison, and assertion without plugins | `CHAINBOT_CONFIG_DIR=examples/core-builtins target/debug/chainbot validate` |
| `builtin-triggers/` | you want simple trigger cases in one place | builtin `manual`, `market_tick`, and `cron` packages sharing one workflow | `CHAINBOT_CONFIG_DIR=examples/builtin-triggers target/debug/chainbot validate` |
| `workflow-composition/` | you want to learn subflow boundaries | parent/child workflows, `nodes.call`, subflow input and output flow | `CHAINBOT_CONFIG_DIR=examples/workflow-composition target/debug/chainbot validate` |
| `plugin-integrations/` | you want external plugin setup | process + wasm external trigger plugins, external node plugin, package-local `bin/` executables | `CHAINBOT_CONFIG_DIR=examples/plugin-integrations target/debug/chainbot validate` |
| `custom-paths/` | you need non-default root-relative directories | `[paths]` overrides for workflows, triggers, plugins, secrets, and state | `CHAINBOT_CONFIG_DIR=examples/custom-paths target/debug/chainbot validate` |

## Example Roots

### `single-workflow/`

Use this when you want the minimum base root before adding triggers or plugins.

Highlights:

- `chainbot.toml` root config
- one workflow package
- one builtin node
- no trigger or plugin package requirements beyond empty root directories

### `builtin-triggers/`

Use this when you want the common builtin trigger contracts in one reviewable root.

Highlights:

- builtin `manual` trigger
- builtin `market_tick` trigger
- builtin `cron` trigger
- shared workflow consuming trigger-derived input

### `core-builtins/`

Use this when you want to learn the builtin flow/data nodes before reaching for scripts or plugins.

Highlights:

- `builtin.data.parse_json` and `builtin.data.stringify_json`
- `builtin.data.get`, `builtin.data.coalesce`, and `builtin.data.compare`
- `builtin.data.pick`, `builtin.data.merge`, and `builtin.data.template`
- `builtin.data.math` and `builtin.flow.assert`

### `workflow-composition/`

Use this when you want to understand workflow-to-workflow composition without external plugin setup.

Highlights:

- parent workflow calling a child workflow
- `nodes.call.with`
- `subflow_output` return mapping
- builtin-only child workflow

### `plugin-integrations/`

Use this when you want to understand root-level plugin registration and external host boundaries.

Highlights:

- external trigger plugin packages showing two distinct lifecycle models:
  - `process_short_lived`: poll-based, supervisor-orchestrated short-lived process adapters
  - `wasm_daemon_persistent_session`: long-lived daemon sessions with persistent Wasmtime ownership
- external node plugin package
- plugin-local `bin/` executables
- use `chainbot catalog show plugin:<name>` to see lifecycle details for each trigger plugin

### `custom-paths/`

Use this when you want to see how the root contract behaves away from the default folder names.

Highlights:

- root-relative `[paths]` overrides
- relocated workflow and trigger package directories
- relocated plugin, secret, and state directories

## Notes

- All examples follow the current `chainbot.toml` contract.
- No example uses legacy `plugins/manifests/` layout.
- Secrets and runtime state are represented as empty directories or placeholders, not real credentials.
- All six curated roots are covered by the `validate_accepts_curated_examples` smoke test in `crates/chainbot/tests/cli_surface.rs`.
