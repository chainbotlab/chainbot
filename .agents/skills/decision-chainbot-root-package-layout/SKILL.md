---
name: "decision-chainbot-root-package-layout"
description: "Load when changing ChainBot root layout, chainbot.toml, workflows/triggers/plugins package identity, storage config, DB-primary runtime state, or manifest-version boundaries. Do not load for CLI copy changes alone."
license: "Proprietary"
metadata:
  generated_by: "decision-capture"
  created: "2026-06-10"
  last_updated: "2026-06-10"
  status: "current"
  affected_modules:
    - "crates/chainbot/src/domain/config/"
    - "crates/chainbot/src/domain/workflow/"
    - "crates/chainbot/src/domain/trigger/"
    - "crates/chainbot/src/plugin/"
    - "crates/chainbot/src/infrastructure/state/"
    - "examples/"
  supersedes:
    - "docs/archive/decisions/CHAINBOT_CONFIG_STATE_LAYOUT_DESIGN.md"
    - "docs/archive/decisions/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md"
  superseded_by: []
---

# Decision: ChainBot Root Package Layout

## Context

ChainBot roots need stable package discovery, manifest compatibility gates, and
durable runtime state boundaries. Workflow, trigger, plugin, and state files
must not depend on sibling-directory convenience or legacy root-config paths.

## Decision

The canonical root layout is:

```text
<root>/
|- chainbot.toml
|- workflows/<workflow_id>/config.toml
|- triggers/<trigger_id>/config.toml
|- plugins/<plugin_id>/config.toml
|- secrets/
`- state/
```

Stable loading order is:

1. `chainbot.toml`
2. `workflows/<workflow_id>/config.toml`
3. `triggers/<trigger_id>/config.toml`
4. `plugins/<plugin_id>/config.toml`

`chainbot.toml` is the only supported root config entrypoint. `config/root.toml`
is not part of the stable contract.

Directory name and manifest identity must match for workflow, trigger, and
plugin packages. Relative paths inside each package resolve from that package
root. Root path overrides remain root-relative and must stay within `<root>`.

Runtime state is DB-primary:

- `storage.mode = "local"` uses `state/runtime.sqlite3` or the configured local
  database path as authority.
- `storage.mode = "postgres"` uses the configured PostgreSQL database as
  authority.
- Legacy file-backed state is compatibility/import material, not authority for
  `status`, `observe`, `list-runs`, `run`, `serve`, or trigger execution.

## Boundaries

- `chainbot.toml`: manifest version, ChainBot version, profile, path overrides,
  runtime defaults, secret refs, plugin activation, and storage settings.
- `workflows/<workflow_id>/config.toml`: package-local execution graph and
  runtime defaults.
- `triggers/<trigger_id>/config.toml`: package-local event source and workflow
  binding.
- `plugins/<plugin_id>/config.toml`: package-local extension registration and
  executable boundary.
- `state/`: durable runtime database artifacts and optional debug output.

Manifest versions are compatibility gates for on-disk contract shape. They do
not select runtime implementation. Runtime protocol versions remain separate
from manifest compatibility.

## Implications

`run_summaries` and `trigger_event_records` are primary durable history.
`trigger_snapshots` is a derived read model. Archived runtime tables retain old
history outside the hot query window. Raw debug artifacts never decide
correctness.

Trigger dedup, cooldown, checkpoints, serve leases, and restart recovery must
come from persisted DB truth, not ad hoc files.

Examples and generated roots must follow the same package model as production
roots.

## Non-goals

- Reintroduce `config/root.toml`.
- Treat package manifest versions as implementation selectors.
- Make legacy file-backed state authoritative again.
- Define command help text or operator-facing CLI wording.
