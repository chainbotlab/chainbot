# ChainBot Legacy Layout Convergence Proposal

## Goal

Define a safe follow-up path for converging away from legacy plugin and runtime-state layouts after canonical package discovery and canonical state writes have already shipped.

## Problem Statement

ChainBot now writes canonical plugin and state layouts, but it still reads:

- legacy shared plugin manifests from `plugins/manifests/*.toml`
- legacy workflow logs from `state/workflow-logs/<run_id>/`
- legacy trigger records from `state/trigger-records/<run_id>/`
- legacy trigger checkpoints from `state/trigger-checkpoints/<trigger_id>.json`

This compatibility is correct for the transition period, but it cannot remain indefinite without creating ambiguity about:

- which layout is canonical
- what operators may safely delete
- which paths must remain covered by tests

## Scope

- define the conditions for retiring legacy plugin discovery
- define the conditions for retiring legacy state reads
- define required observability, migration, and rollback expectations
- define exit criteria for promoting the canonical layout to the only supported layout

## Non-Goals

- remove compatibility in the current implementation phase
- redesign runtime-state semantics
- redesign plugin executable policy
- replace file-backed state with a database-only model

## Current Baseline

The current implementation already guarantees:

- canonical plugin packages are discoverable from `plugins/<plugin_id>/config.toml`
- canonical state writes land under `state/runs/` and `state/triggers/`
- legacy plugin and state layouts remain readable
- trigger dedup and cooldown rebuild continue to depend on persisted trigger records
- trigger checkpoint resume continues to depend on persisted checkpoint state

## Convergence Principles

- canonical writes must precede legacy read removal
- no legacy read path may be removed until an explicit migration path exists
- trigger records must be treated as correctness-bearing state throughout migration
- cleanup and migration tooling must distinguish run-scoped artifacts from trigger-scoped artifacts
- rollback must preserve the ability to reopen old roots without duplicate trigger acceptance

## Proposed Phases

### Phase 1: Measure and Warn

- add explicit runtime or CLI visibility when a root still depends on legacy plugin or state paths
- surface legacy-path usage in validation output or operator diagnostics
- document which legacy inputs remain supported and why

### Phase 2: Add Explicit Migration Tooling

- add a read-only inspection mode that reports legacy-path usage per root
- add an explicit migration command or documented manual procedure for:
  - plugin manifests
  - workflow logs
  - trigger records
  - trigger checkpoints
- migration output must be idempotent and leave rollback breadcrumbs

### Phase 3: Dual-Read with Deprecation Gate

- keep canonical writes only
- keep legacy reads behind a clearly documented compatibility gate
- fail validation for new roots that introduce fresh legacy-only artifacts
- make mixed-root conflict behavior explicit before any legacy reader is removed:
  - duplicate `plugin_id` across canonical and legacy discovery remains a hard failure
  - canonical checkpoint wins when both canonical and legacy checkpoints exist
  - accepted trigger record history must not be double-counted across canonical and legacy locations

### Phase 4: Remove Legacy Reads

- remove legacy plugin discovery
- remove legacy workflow-log scanning
- remove legacy trigger-record scanning
- remove legacy checkpoint fallback

This phase should occur only after migration tooling and compatibility telemetry show that supported roots have converged.

## Downgrade Boundary

The current compatibility direction is forward-read compatibility:

- new runtimes can read legacy layouts during the transition period

It must not be described as bidirectional compatibility by default.

Before legacy read removal, operator guidance must explicitly state whether older binaries are expected to understand canonical-only plugin packages and canonical-only state paths. If not, downgrade behavior must be documented as unsupported.

## Required Migration Guarantees

### Plugin Discovery

- migration must preserve `plugin_id`
- migration must preserve manifest-relative executable resolution
- migration must fail if canonical and legacy sources disagree on `plugin_id` contents

### Workflow Logs

- migration must preserve append-only ordering by `sequence`
- migration must preserve restart recovery for staged files where applicable

### Trigger Records

- migration must preserve dedup and cooldown semantics exactly
- migration must preserve event identity and accepted ordering
- migration must avoid double-loading the same accepted trigger record after migration

### Trigger Checkpoints

- migration must preserve the last acknowledged checkpoint for each trigger
- migration must not regress external trigger resume behavior

## Risks

### High Risk

- treating trigger records as disposable logs during migration
- removing legacy reads before mixed-layout roots are explicitly migrated
- migrating trigger records in a way that duplicates accepted-event history

### Medium Risk

- removing legacy plugin discovery before operators have a reliable way to detect old roots
- leaving legacy compatibility active without observability, which hides convergence progress

### Low Risk

- adding documentation-only deprecation notices before behavior changes

## Exit Criteria

Legacy layout support is ready for removal only when all of the following are true:

- canonical plugin packages are the only plugin layout generated by bootstrap flows
- supported roots can be inspected for legacy usage deterministically
- an explicit migration path exists for plugin manifests and state artifacts
- integration coverage proves no duplicate trigger acceptance after migration
- rollback from a partially migrated root is defined and tested
- mixed-root conflict semantics are documented and stable before compatibility code is removed

## Suggested Validation for the Future Removal Phase

- mixed-layout recovery tests
- migrated-root restart recovery tests
- duplicate-acceptance regression tests across migration boundaries
- plugin discovery conflict tests between migrated and non-migrated roots
- workspace test pass after legacy readers are disabled

## Recommended Follow-Up

- keep the current compatibility implementation documented in `docs/engineering/CHAINBOT_CONFIG_STATE_LAYOUT_IMPLEMENTATION.md`
- add operator-visible legacy-layout inspection before planning removal
- treat legacy-layout retirement as its own implementation milestone rather than bundling it into unrelated feature work
