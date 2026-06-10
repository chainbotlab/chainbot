---
name: "decision-chainbot-cli-contract"
description: "Load when changing ChainBot CLI command surface, CHAINBOT_CONFIG_DIR root resolution, status or observe JSON contracts, trigger enable/disable semantics, or help/error navigation. Do not load for runtime internals without CLI impact."
license: "Proprietary"
metadata:
  generated_by: "decision-capture"
  created: "2026-06-10"
  last_updated: "2026-06-10"
  status: "current"
  affected_modules:
    - "crates/chainbot/src/cli/"
    - "crates/chainbot/src/app/"
    - "crates/chainbot/src/infrastructure/state/"
    - "README.md"
    - "CONTRIBUTING.md"
  supersedes:
    - "docs/archive/decisions/CHAINBOT_CLI_DESIGN.md"
  superseded_by: []
---

# Decision: ChainBot CLI Contract

## Context

Operators and agents need one stable command surface for root validation,
status observation, trigger toggles, and execution. CLI commands must make clear
which operations observe state and which mutate root config or runtime state.

## Decision

The stable command surface is:

```text
chainbot help [command]
chainbot version
chainbot init
chainbot status [--json]
chainbot observe [--json] [--limit <n>] [--trigger-id <id>] [--run-id <id>]
chainbot stop
chainbot trigger <list|enable|disable> [trigger-id]
chainbot validate
chainbot list-runs
chainbot run
chainbot serve
chainbot plugin source <list|show>
chainbot plugin install
```

CLI root resolution uses `CHAINBOT_CONFIG_DIR`. If it is unset or empty, the
default root is `~/.chainbot`. The value must point to a complete ChainBot root,
not to a single config file.

`status` is a non-executing snapshot. It does not start workflow execution,
consume trigger snapshots, recover incomplete runs, append logs, or create or
migrate the coordination database. It may return human output or `--json`.

`observe` is read-only runtime history over committed run summaries, workflow
runtime logs, and trigger event records. It supports `--json`, `--limit`,
`--trigger-id`, and `--run-id`; it does not trigger recovery, replay triggers,
or mutate active/archived history.

`trigger enable` and `trigger disable` persistently mutate the trigger package
`enabled` field. They do not mutate runtime trigger records, dedup/cooldown
coordination, or serve leases.

## Boundaries

- `crates/chainbot/src/cli/`: command parsing, help, output mode, root
  resolution, and user-facing error navigation.
- `crates/chainbot/src/app/`: command handlers and execution/serve boundaries
  surfaced through CLI.
- `crates/chainbot/src/infrastructure/state/`: read models used by `status`,
  `observe`, and `list-runs`.
- `README.md` and `CONTRIBUTING.md`: public navigation to stable CLI contract.

## Implications

Help text describes command contracts, not implementation details. Command help
should include use-when, usage, reads or writes, execution/non-execution
boundary, outputs, root resolution when relevant, failure navigation, examples,
and see-also links.

CLI errors should point to the next action: unsupported command suggestions,
unexpected argv token location, missing root directory, missing root config, and
TOML decode file/line/source context.

`run` executes a single manually selected workflow. If multiple top-level
workflows are plausible, it returns a usage error rather than guessing.

`serve` starts the long-running daemon; daemon health is persisted as DB truth
and surfaced by `status` as `idle`, `active`, or `stale`.

## Non-goals

- Define root package layout beyond relying on the root package layout decision.
- Define workflow DAG scheduling internals.
- Turn every command into JSON output.
- Make trigger enable/disable a runtime-only toggle.
