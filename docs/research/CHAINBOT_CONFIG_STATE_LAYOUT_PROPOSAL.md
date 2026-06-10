# ChainBot Config and State Layout Proposal

## Goal

Define a one-step target layout that aligns ChainBot package discovery, plugin packaging, and runtime state boundaries under a single stable mental model.

This proposal captures the full `Approach C` direction before any stable design doc is amended.

## Problem Statement

The current ChainBot root has three different organizational models at the same time:

- workflows use package folders
- triggers use package folders
- plugins use manifest glob discovery plus a separate executable directory
- runtime state mixes run-scoped, trigger-scoped, and coordination-scoped artifacts under sibling directories

This creates two kinds of drift:

1. package discovery is not modeled consistently across workflow, trigger, and plugin contracts
2. durable runtime artifacts are not grouped by lifecycle boundary even though recovery logic depends on those boundaries

## Scope

- clarify the semantic role of `manifest_version` across root, workflow, node, trigger, and plugin contracts
- define a package-folder target layout for plugins
- define a run-scoped plus trigger-scoped target layout for durable runtime state
- define compatibility expectations for moving from the current root layout to the target layout

## Non-Goals

- redesign the CLI command surface
- redesign builtin node or builtin trigger registries
- replace file-backed runtime artifacts with a database-only storage model
- define implementation-ready migration code or task breakdown

## Current Constraints

The proposal must preserve these observed repository contracts:

- `workflow.manifest_version`, `node.manifest_version`, `trigger.manifest_version`, `plugin.manifest_version`, and `root_config.manifest_version` are all active major-version compatibility gates
- workflow and trigger packages are discovered from `workflows/*/config.toml` and `triggers/*/config.toml`
- plugin manifests are currently discovered from `plugins/manifests/*.toml`
- external plugin executables are resolved relative to `manifest_path` and must remain within `plugins_root`
- `trigger-records` are a durable source for dedup and cooldown rebuild during `TriggerPlane::open`
- `trigger-checkpoints` are the durable resume source for external trigger listeners
- `coordination.sqlite3` is authoritative for live serve lease ownership and coordination tokens, while dedup and cooldown token contents are rebuildable from persisted trigger records

## Target Mental Model

ChainBot should expose three aligned boundaries:

```text
contract boundary   -> version fields
package boundary    -> workflows / triggers / plugins
lifecycle boundary  -> runs / triggers / coordination
```

Each boundary should answer one question only:

- contract boundary: which manifest or wire formats are accepted?
- package boundary: where does this unit live on disk?
- lifecycle boundary: which durable artifacts must survive restart and cleanup?

## Proposed Target Root Layout

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

## Contract Clarifications

### Version Semantics

`manifest_version` fields remain contract gates, not runtime dispatch selectors.

- `root_config.manifest_version`: root config contract version
- `workflow.manifest_version`: workflow package contract version
- `node.manifest_version`: embedded node block contract version
- `trigger.manifest_version`: trigger package contract version
- `plugin.manifest_version`: plugin manifest contract version

Runtime protocol selection remains separate:

- node plugin stdin/stdout compatibility uses `node_plugin_request.contract_version` and `node_plugin_response.contract_version`
- external trigger listener compatibility uses `trigger_start_command.protocol_version`

### Node Versioning Rule

`node.manifest_version` must not be used to select between multiple builtin implementations.

Builtin coexistence, if introduced later, should be modeled through a separate execution identity such as `plugin_id`, capability, or explicit protocol version.

## Plugin Package Design

### Discovery Rule

Canonical plugin discovery becomes:

- `plugins/<plugin_id>/config.toml`

The package directory name is the plugin identity boundary in the same way that workflow and trigger package directories already encode package identity.

### Plugin Package Contents

Each plugin package may contain:

- `config.toml`
- `bin/` for executables or wrappers
- `assets/` for schemas, fixtures, or static resources

### Executable Resolution Rule

Plugin executable paths continue to resolve relative to the plugin manifest file and must remain inside the configured `plugins_root`.

This preserves the existing containment guarantee while making manifest and executable locality explicit.

## Runtime State Design

### State Axes

Durable runtime state is split along lifecycle boundaries.

#### Run-scoped

- `runs/<run_id>/summary.json`
- `runs/<run_id>/workflow-logs/*.json`

These artifacts describe one workflow execution and can be retained, exported, or cleaned up as a unit.

#### Trigger-scoped

- `triggers/<trigger_id>/checkpoint.json`
- `triggers/<trigger_id>/records/*.json`

These artifacts describe trigger listener progress and accepted event history. They must not be treated as disposable logs because accepted trigger records are the durable source for dedup and cooldown rebuild.

#### Coordination-scoped

- `coordination.sqlite3`

The coordination database remains intentionally narrow:

- serve lease ownership
- dedup tokens
- cooldown tokens

It must stay rebuild-friendly rather than becoming the sole source of run history or trigger ledger data.

## State Invariants

- run summaries remain authoritative for `list-runs` and run-status recovery
- workflow logs remain append-only
- trigger records remain append-only
- trigger checkpoints remain last-acknowledged trigger progress
- trigger dedup and cooldown rebuild must succeed from persisted trigger records after restart
- cleanup of run artifacts must not implicitly delete trigger artifacts
- cleanup of trigger artifacts must be explicitly aware that records participate in duplicate suppression correctness

## Compatibility Direction

Because this document is a proposal, compatibility is defined as design intent rather than implementation steps.

The future implementation is expected to support a transition period where:

- old plugin manifest discovery and new plugin package discovery can coexist
- old state paths and new state paths can both be read during recovery
- new writes eventually converge on the target layout

The stable design doc should only be updated once that compatibility policy is accepted and the final canonical layout is decided.

## Key Risks

### High Risk

- treating `trigger-records` as ordinary logs and losing dedup or cooldown correctness after restart
- mixing manifest contract versioning with runtime implementation version selection
- changing state path identity without preserving staged-write and recovery scans

### Medium Risk

- keeping two plugin discovery models active too long and creating ambiguous canonical layout rules
- introducing plugin package identity rules that conflict with existing shared-manifest roots

### Low Risk

- clarifying `node.manifest_version` semantics in docs and contract tests without changing runtime behavior

## Open Questions

- Should trigger record filenames remain keyed by accepted sequence plus event identity, or should the target layout prefer event-first naming within each trigger package?
- Should plugin package discovery remain configurable via globs after package layout becomes canonical, or should plugins adopt the same fixed package discovery contract as workflows and triggers?
- Should `runs/<run_id>/workflow-logs/` stay JSON-per-entry, or should a future design introduce segmented archives after this layout change is settled?

## Promotion Criteria

This proposal is ready to graduate into stable design only when all of the following are true:

- the canonical plugin discovery contract is accepted
- the canonical state layout is accepted
- compatibility expectations for legacy roots are accepted
- no remaining open question changes the target on-disk identities

## Recommended Follow-Up

- amend `.agents/skills/decision-chainbot-root-package-layout/SKILL.md` once the target layout becomes the chosen stable contract
- add a dedicated implementation record when migration behavior is implemented
- keep recovery, dedup, and checkpoint semantics verified by integration tests before promoting the design
