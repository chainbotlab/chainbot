---
name: "decision-chainbot-plugin-host-protocol-design"
description: "Load when changing plugin host dispatch, host limits, node.exec.v2, mcp.tool.v1, trigger.exec.v1 process lifecycles, or Wasm Component Model bindings. Do not load for plugin source install or workflow DAG semantics."
license: "Proprietary"
metadata:
  generated_by: "decision-capture"
  created: "2026-06-10"
  last_updated: "2026-07-14"
  status: "current"
  affected_modules:
    - "crates/chainbot/src/plugin/"
    - "crates/chainbot/src/app/runtime/execution.rs"
    - "crates/chainbot/src/app/runtime/external_triggers/"
    - "crates/chainbot/wit/"
    - "official-plugins/"
  supersedes:
    - "docs/archive/decisions/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md"
    - "docs/archive/decisions/CHAINBOT_OFFICIAL_PLUGIN_DESIGN.md"
  superseded_by: []
---

# Decision: ChainBot Plugin Host Protocol Design

## Context

Plugins need stable host protocols without turning entrypoint strings,
JSON-RPC methods, and listener messages into interchangeable names. Current
runtime truth is split across node subprocess hosts, MCP node hosts, and
external trigger listener hosts.

## Decision

External node plugin manifests select host behavior by `entrypoint`.

`entrypoint = "node.exec.v2"` means JSON-RPC 2.0 over subprocess stdin/stdout.
The manifest entrypoint is `node.exec.v2`, but the JSON-RPC request method is
`node.execute`. These names are intentionally different and must not be
collapsed.

`entrypoint = "mcp.tool.v1"` means external node execution through MCP with a
per-invocation session. The host initializes the MCP session, discovers tools,
validates the manifest operation against discovered tools, calls the selected
tool, normalizes structured output, and then validates output schema.

External trigger plugins use `entrypoint = "trigger.exec.v1"` semantics. The
process host writes one line-delimited `TriggerHostMessage::Start` JSON command
to stdin and retains `--trigger-id <trigger_id>` argv for compatibility. The
plugin emits line-delimited stdout messages: `ready`, `event`, `heartbeat`, or
`fatal`. The host writes `ack` messages to stdin only after durable staging and
writes `stop` during reconciliation, shutdown, fatal handling, or heartbeat
timeout.

Process triggers have two explicit lifecycles. `process_short_lived` is a
bounded turn with a hard deadline and bounded event/output budgets.
`process_daemon_session` is a long-lived listener owned by the external trigger
supervisor, including child handles, bounded channels, cancellation, restart,
and graceful-stop/forced-kill behavior. Long-lived official listeners use
`process_daemon_session`; bounded pollers may retain `process_short_lived`.

Wasm persistent trigger plugins use the WIT Component Model contract in
`crates/chainbot/wit/trigger-plugin.wit` as the single ABI authority. The host
uses generated component bindings, fuel or epoch interruption, store limits,
and fallible compilation/instantiation. The legacy core-Wasm pointer ABI
remains behind a compatibility adapter for one release, then is removed.

Every host adapter has explicit execution time, output-size, shutdown, and
cancellation limits. A plugin process or guest must not block daemon lease
renewal, reconciliation, or stop handling.

## Boundaries

- `crates/chainbot/src/plugin/contract.rs`: manifest validation, node request
  and response contracts, JSON-RPC envelope types, activation envelope types,
  trigger runtime manifest fields, and protocol constants.
- `crates/chainbot/src/plugin/host.rs`: external node subprocess and MCP host
  execution.
- `crates/chainbot/src/app/runtime/execution.rs`: plugin-node request assembly,
  activation secret resolution, redaction, and dispatch into the node host.
- `crates/chainbot/src/app/runtime/external_triggers/`: managed process
  lifecycles, bounded listener channels, cancellation, and Component Model
  Wasm hosting.
- `crates/chainbot/wit/trigger-plugin.wit`: authoritative Wasm trigger ABI.
- `official-plugins/`: protocol clients that must follow host truth and declare
  the lifecycle matching their actual execution behavior.

## Implications

Helper crates or official plugin clients that use `node.exec.v2` as the
JSON-RPC method are stale relative to current host truth. Record that drift and
fix it in implementation work; do not document `node.exec.v2` as the current
method name.

Plugin executable paths are package-local and must not escape the configured
plugin root. Host subprocess environments remain allowlisted.

Trigger listener ordering is stateful: `ready` must come before `event` or
`heartbeat`; duplicate `ready` is a protocol violation; messages after stopped
state are protocol violations.

The supervisor, not a daemon poll stack frame, owns long-lived child and Wasm
session resources. Lease loss and daemon shutdown propagate cancellation to all
hosts; final process cleanup escalates from protocol `stop` to forced process
group termination after the configured grace period.

Host resource defaults may vary by adapter, but every adapter must expose the
same typed timeout, cancellation, and output-limit failure categories.

## Non-goals

- Define source repository install, staging, or overwrite policy.
- Define workflow DAG dependency semantics.
- Move chain-specific signing or provider logic into the host.
- Make MCP sessions long-lived by default for node execution.
- Keep long-lived process listeners labeled or executed as short-lived turns.
- Maintain two independent authoritative Wasm ABI definitions.
- Treat trusted installation as permission for an unbounded runtime process.
