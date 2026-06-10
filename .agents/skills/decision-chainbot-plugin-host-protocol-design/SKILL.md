---
name: "decision-chainbot-plugin-host-protocol-design"
description: "Load when changing external node plugin host dispatch, node.exec.v2 JSON-RPC, mcp.tool.v1 execution, trigger.exec.v1 listener protocol, or plugin subprocess boundaries. Do not load for plugin source install or workflow DAG semantics."
license: "Proprietary"
metadata:
  generated_by: "decision-capture"
  created: "2026-06-10"
  last_updated: "2026-06-10"
  status: "current"
  affected_modules:
    - "crates/chainbot/src/plugin/"
    - "crates/chainbot/src/app/runtime/execution.rs"
    - "crates/chainbot/src/app/runtime/external_triggers/"
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
`fatal`. The host writes `ack` messages to stdin after staging accepted events
and writes `stop` when fatal or heartbeat timeout handling requires shutdown.

## Boundaries

- `crates/chainbot/src/plugin/contract.rs`: manifest validation, node request
  and response contracts, JSON-RPC envelope types, activation envelope types,
  trigger runtime manifest fields, and protocol constants.
- `crates/chainbot/src/plugin/host.rs`: external node subprocess and MCP host
  execution.
- `crates/chainbot/src/app/runtime/execution.rs`: plugin-node request assembly,
  activation secret resolution, redaction, and dispatch into the node host.
- `crates/chainbot/src/app/runtime/external_triggers/`: trigger process and
  Wasm listener supervision.
- `official-plugins/`: protocol clients that must follow host truth.

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

## Non-goals

- Define source repository install, staging, or overwrite policy.
- Define workflow DAG dependency semantics.
- Move chain-specific signing or provider logic into the host.
- Make MCP sessions long-lived by default for node execution.
