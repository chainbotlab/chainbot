---
name: "decision-chainbot-workflow-dag-design"
description: "Load when changing workflow DAG validation, dependency scheduling, node output references, ready-wave execution, depends_mode, when gates, or subflow contracts. Do not load for trigger config or plugin host protocol changes."
license: "Proprietary"
metadata:
  generated_by: "decision-capture"
  created: "2026-06-10"
  last_updated: "2026-07-14"
  status: "current"
  affected_modules:
    - "crates/chainbot/src/domain/workflow/"
    - "crates/chainbot/src/domain/runtime/"
    - "crates/chainbot/src/app/runtime/execution.rs"
    - "crates/chainbot/tests/workflow_dag_semantics.rs"
  supersedes:
    - "docs/archive/decisions/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md"
  superseded_by: []
---

# Decision: ChainBot Workflow DAG Design

## Context

Workflow execution must be deterministic across manual runs, trigger-driven
runs, and subflow calls. Authors need stable DAG semantics, while the runtime
needs validation strong enough to reject ambiguous graphs before scheduling.

## Decision

Workflow manifests define a node DAG. Validation rejects duplicate node IDs,
self dependencies, unknown dependency IDs, missing graph indexes, and cycles.

Dependency evaluation is terminal-state based:

- `depends_mode = "all"` waits for all dependencies to reach a terminal state;
  the node is ready only when every dependency succeeded, otherwise it is
  skipped.
- `depends_mode = "any"` also waits for all dependencies to reach a terminal
  state; the node is ready when at least one dependency succeeded, otherwise it
  is skipped. It is not a short-circuit OR.

`when` is a second gate. The scheduler first decides dependency readiness or
skip, and evaluates `when` only for dependency-ready nodes. A false `when`
marks the node skipped.

Node output references migrate to producer-addressed form:
`node.<node_id>.<output_key>`. The referenced producer must belong to the
consumer's transitive dependency closure. Legacy flat `node.<output_key>`
references remain accepted for one compatibility release with a validation
warning, then are removed.

A ready wave observes one immutable namespace snapshot. Nodes in the same wave
cannot observe each other's outputs. Wave results are committed only after all
nodes in that wave finish, in deterministic node-ID order. Real parallel wave
execution is enabled only after producer-addressed references and deterministic
commit semantics are in place.

Subflow nodes use `kind = "subflow"` plus `[nodes.call]`. Their author-facing
input surface is `nodes.call.with`; they must not define `[[nodes.inputs]]`.
If a subflow node explicitly sets `plugin` or `operation`, the only valid
values are `builtin-subflow` and `run`.

## Boundaries

- `crates/chainbot/src/domain/workflow/`: manifest lowering, node contract
  validation, producer-addressed variable references, subflow contract
  validation, and DAG validation.
- `crates/chainbot/src/domain/runtime/`: scheduled node states and run report
  contract.
- `crates/chainbot/src/app/runtime/execution.rs`: dependency decision,
  immutable wave snapshots, deterministic result commit, `when` evaluation,
  node dispatch, and subflow recursion.
- `crates/chainbot/tests/workflow_dag_semantics.rs`: regression coverage for
  graph and scheduler semantics.

## Implications

Changing dependency or output-addressing behavior changes workflow author
expectations and must be treated as a contract change. In particular, `any`
must keep waiting for every dependency to finish so later dependency failures
and outputs are not hidden by early success.

Flat node-output compatibility is temporary. New examples, docs, and generated
workflows use producer-addressed references immediately; compatibility warnings
must identify the workflow, consumer node, and legacy key.

`when` cannot be used to make an invalid dependency graph valid. Graph
validation happens before scheduling and before condition evaluation.

Subflow recursion is runtime orchestration, not a plugin host protocol. It
must preserve depth limits and cycle detection across workflow frames.

## Non-goals

- Define trigger payload mapping or trigger package schema.
- Define external plugin host protocols.
- Add multi-condition expression language semantics for `when`.
- Treat skipped nodes as failed nodes for final workflow status.
- Preserve ambiguous flat node-output references as a permanent contract.
- Allow same-wave nodes to communicate through mutable runtime namespaces.
