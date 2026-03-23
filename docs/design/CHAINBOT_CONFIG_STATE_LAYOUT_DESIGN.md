# ChainBot Config and State Layout Design

## Goal

Define the stable root layout that aligns ChainBot contract versioning, package discovery, plugin packaging, and durable runtime state boundaries.

## Design Boundaries

- `root config` defines root-relative paths, runtime defaults, plugin discovery, and secret references
- `workflow` defines a package-local execution graph and runtime contract
- `trigger` defines a package-local event source and workflow binding contract
- `plugin` defines a package-local extension registration and executable boundary
- `state` defines durable runtime artifacts and restart-safe coordination boundaries

`state` 在主运行路径采用 DB-primary 模型：

- `storage.mode = "local"` 时，`state/runtime.sqlite3`（或配置的 local DB 路径）是本地权威状态
- `storage.mode = "postgres"` 时，配置的 PostgreSQL 数据库是权威状态
- legacy file-backed state 仅保留为兼容/导入准备，不再作为 `status`、`list-runs`、`run`、`serve`、trigger execution 的权威来源

## Root Layout

```text
<root>/
|- chainbot.toml
|- workflows/
|  `- <workflow_id>/
|     |- config.toml
|     |- scripts/
|     `- assets/
|- triggers/
|  `- <trigger_id>/
|     |- config.toml
|     |- scripts/
|     `- assets/
|- plugins/
|  `- <plugin_id>/
|     |- config.toml
|     |- bin/
|     `- assets/
|- secrets/
`- state/
   `- runtime.sqlite3
```

## Contract Boundary

### Manifest Versions

All `manifest_version` fields are compatibility gates for on-disk manifest contracts.

- `root_config.manifest_version`
- `workflow.manifest_version`
- `node.manifest_version`
- `trigger.manifest_version`
- `plugin.manifest_version`

These fields define accepted contract shape and semantics. They do not select runtime implementations.

### Runtime Protocol Versions

Runtime protocol negotiation remains separate from manifest compatibility.

- node plugin execution uses request and response contract versions
- external trigger listener startup uses trigger protocol version

### Node Version Rule

`node.manifest_version` defines the embedded node block contract only.

Builtin implementation selection must be modeled through execution identity such as `plugin_id`, capability, or a dedicated protocol version, not through `node.manifest_version`.

## Package Boundary

### Workflow Packages

- canonical entrypoint: `workflows/<workflow_id>/config.toml`
- package directory name must equal `workflow_id`
- workflow-local relative paths resolve from the workflow package root

### Trigger Packages

- canonical entrypoint: `triggers/<trigger_id>/config.toml`
- package directory name must equal `trigger_id`
- trigger-local relative paths resolve from the trigger package root

### Plugin Packages

- canonical entrypoint: `plugins/<plugin_id>/config.toml`
- package directory name must equal `plugin_id`
- plugin-local relative paths resolve from the plugin package root
- plugin executables must remain within the configured `plugins_root`

### Plugin Package Contents

Each plugin package may contain:

- `config.toml`
- `bin/`
- `assets/`

Manifest and executable locality are part of the package contract.

## State Boundary

### Runtime DB Tables

权威运行态持久化写入统一逻辑表：

- `run_summaries`
- `workflow_runtime_logs`
- `trigger_event_records`
- `trigger_checkpoints`
- `trigger_snapshots`（derived read model）
- `serve_leases`
- `archived_run_summaries`
- `archived_workflow_runtime_logs`
- `archived_trigger_event_records`

其中：

- `run_summaries` 与 `trigger_event_records` 是 run/trigger history 的主权威来源
- `trigger_snapshots` 是可由 accepted records 重建的 read model
- archived tables 保留超出 retention window 的 durable history，不再参与热路径 `status` / `observe` 最近窗口查询
- raw debug artifact 不参与正确性判定

## State Invariants

- run summaries are authoritative for persisted run status
- workflow runtime logs remain append-only by `(run_id, sequence)`
- trigger event records remain append-only by `(trigger_id, sequence)`
- archived runtime tables remain append-only by `(archived_at_ms, primary identity...)`
- trigger checkpoints remain the last acknowledged trigger progress
- trigger snapshots remain derived, replaceable state
- trigger dedup and cooldown decisions must remain restart-safe from persisted trigger history
- retention may move old history into archive tables, but must not silently drop the most recent retained observation window

## Discovery Rules

Stable loading order:

1. `chainbot.toml`
2. `workflows/<workflow_id>/config.toml`
3. `triggers/<trigger_id>/config.toml`
4. `plugins/<plugin_id>/config.toml`

All path settings remain root-relative and must stay within `<root>`.

## Rationale

- contract versioning becomes independent from runtime implementation selection
- workflow, trigger, and plugin discovery share one package model
- runtime durability aligns with lifecycle ownership rather than sibling directory convenience
- restart-safe coordination stays narrow and rebuildable

## Change Triggers

- change plugin discovery contract or plugin package identity rules
- change state directory ownership or durable artifact semantics
- change the role of any `manifest_version` field
- change restart recovery, dedup, cooldown, or checkpoint guarantees
