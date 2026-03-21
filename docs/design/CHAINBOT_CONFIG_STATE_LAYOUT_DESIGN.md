# ChainBot Config and State Layout Design

## Goal

Define the stable root layout that aligns ChainBot contract versioning, package discovery, plugin packaging, and durable runtime state boundaries.

## Design Boundaries

- `root config` defines root-relative paths, runtime defaults, plugin discovery, and secret references
- `workflow` defines a package-local execution graph and runtime contract
- `trigger` defines a package-local event source and workflow binding contract
- `plugin` defines a package-local extension registration and executable boundary
- `state` defines durable runtime artifacts and restart-safe coordination boundaries

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
   |- coordination.sqlite3
   |- runs/
   |  `- <run_id>/
   |     |- summary.json
   |     `- workflow-logs/
   |        |- 00000000000000000001.json
   |        `- 00000000000000000002.json
   `- triggers/
      `- <trigger_id>/
         |- checkpoint.json
         `- records/
            |- 00000000000000000001-<event>.json
            `- 00000000000000000002-<event>.json
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

### Run-Scoped Artifacts

`state/runs/<run_id>/` owns one workflow execution record.

- `summary.json` is the durable run summary
- `workflow-logs/*.json` are append-only runtime log entries

Run-scoped artifacts are retained, inspected, and cleaned up as one lifecycle unit.

### Trigger-Scoped Artifacts

`state/triggers/<trigger_id>/` owns one trigger listener state record.

- `checkpoint.json` is the durable resume position
- `records/*.json` are append-only accepted trigger records

Trigger-scoped artifacts are retained, inspected, and cleaned up as one lifecycle unit.

Accepted trigger records are not ordinary logs. They are durable trigger-plane state used to rebuild dedup and cooldown coordination after restart.

### Coordination-Scoped Artifacts

`state/coordination.sqlite3` owns narrow mutable coordination state.

- serve lease ownership
- dedup tokens
- cooldown tokens

The coordination database must remain rebuild-friendly and must not become the sole source of run history or accepted trigger history.

## State Invariants

- run summaries are authoritative for persisted run status
- workflow logs remain append-only
- trigger records remain append-only
- trigger checkpoints remain the last acknowledged trigger progress
- trigger dedup and cooldown rebuild must succeed from persisted trigger records after restart
- run cleanup must not delete trigger-scoped artifacts implicitly
- trigger cleanup must treat accepted trigger records as correctness-bearing state

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
