---
title: Migrate protocol-heavy builtin capabilities into official plugins
date: 2026-04-02
category: best-practices
module: chainbot
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - extracting capability logic from builtin runtime paths
  - introducing operator-owned secrets for plugin execution
  - preserving a thin generic runtime while adding new integrations
symptoms:
  - builtin capability owns too much protocol-specific behavior
  - discoverability and installation paths diverge from runtime behavior
  - security policy is hard to enforce consistently across authoring and execution
root_cause: scope_issue
resolution_type: migration
tags:
  - builtin-http
  - http-node
  - official-plugin
  - plugin-activation
  - allowed-origins
  - thin-runtime
  - external-node
---

# Migrate protocol-heavy builtin capabilities into official plugins

## Context

`builtin.http` started as a convenient outbound request node, but it had become the wrong home for the capability.

The problem was no longer just "how do we send an HTTP request". The capability needed its own install identity, its own security policy, its own activation secret boundary, and its own discoverability surface. Keeping that inside `chainbot runtime` would continue to fatten the control plane and create a second, special-case path that bypassed the repo's official plugin model.

The solved migration moved outbound HTTP from:

- `crates/chainbot/src/builtins/nodes/handlers/http.rs`

to:

- `official-plugins/http-node/`

while keeping `chainbot` responsible only for generic orchestration:

- workflow scheduling
- plugin discovery and install
- generic plugin host lifecycle
- execution-time secret resolution and injection
- validation and catalog projection

## Guidance

### 1. Move capability logic, not just code location

If a builtin starts needing all of the following, it should usually become an official plugin package instead of staying in runtime:

- a stable `plugin_id`
- source/install/catalog visibility
- operator-owned activation config
- protocol-specific request/response policy
- security rules that belong to the capability itself

Canonical authoring after the migration:

```toml
[[nodes]]
manifest_version = "2.0.0"
id = "fetch-status"
kind = "plugin"
plugin = "http-node"
operation = "request"
depends_on = []

[[nodes.inputs]]
target = "url"
source = "run.url"

[[nodes.inputs]]
target = "method"
source = "run.method"
```

Avoid keeping the old builtin shape alive:

```toml
[[nodes]]
manifest_version = "2.0.0"
id = "fetch-status"
kind = "builtin"
plugin = "builtin.http"
operation = "https://api.example.com/status"
depends_on = []
```

That shape preserves the wrong mental model even if the implementation is later forwarded somewhere else.

### 2. Keep runtime generic, and push capability semantics into the plugin

The migration only worked cleanly because the runtime already had a generic `external_node` path:

- `crates/chainbot/src/app/runtime/execution.rs`
- `crates/chainbot/src/plugin/contract.rs`
- `crates/chainbot/src/plugin/host.rs`

The HTTP-specific logic stayed inside the package-local plugin:

- request shaping
- destination policy
- response normalization
- header rules
- redirect handling

That division matters. If runtime grows a protocol-specific fallback, the plugin system becomes decorative rather than architectural.

### 3. Put operator-owned auth in `plugin_activation`, not in workflow input

Secrets that belong to operators should not travel through normal workflow `input` fields.

The stable boundary is:

- workflow `input`: business parameters and non-secret request shaping
- `plugin_activation.<plugin_id>.secret_bindings`: operator-owned secret references
- `activation.secrets`: plaintext values injected by the host only at execution time

Example root config:

```toml
[plugin_activation."http-node"]
allowed_origins = ["https://api.example.com"]

[plugin_activation."http-node".secret_bindings]
authorization = "secret://providers/acme#bearer_token"
```

This let the host remain generic while still enforcing a meaningful security boundary.

### 4. Bind secrets to destinations explicitly

Injecting secrets is not enough. They must also be scoped to where they are allowed to go.

The migration made `allowed_origins` part of the activation contract and validated it at multiple layers:

- root config parsing in `crates/chainbot/src/infrastructure/config/mod.rs`
- plugin manifest/request contract in `crates/chainbot/src/plugin/contract.rs`
- request-time enforcement in `official-plugins/http-node/crate/src/client.rs`

This is the key rule:

- anonymous requests may go to destinations that pass the generic outbound policy
- activation-derived auth may only be attached when the request origin matches the configured allowlist

Without that binding, a plugin can easily become a secret exfiltration tunnel.

### 5. Treat safety policy as contract, not as implementation detail

The important security and behavioral rules were locked before implementation drift could start:

- redirects disabled
- `Host` header override rejected
- loopback, private, link-local, metadata, shared-range, and IPv4-mapped IPv6 targets blocked
- response output limited to text-oriented bodies
- sensitive response headers such as `set-cookie` removed from output

These rules live in `official-plugins/http-node/crate/src/client.rs`, but the lesson is broader: if the rule affects trust boundaries or migration semantics, define it early and test it directly.

### 6. Use machine-readable optional inputs

One migration bug came from treating `method`, `headers`, and `body` as optional in plugin code while declaring them all as required in the host contract.

The durable fix was to extend the generic plugin contract with `optional_input_schema` in:

- `crates/chainbot/src/plugin/contract.rs`

This avoided solving the problem with special-case prose or plugin-local guesswork.

Use this pattern whenever a plugin has a small required core and a wider optional request surface.

## Why This Matters

This pattern prevents two long-term failures:

1. `chainbot runtime` quietly accumulating protocol-specific branches again
2. officially supported capabilities bypassing install, catalog, activation, and validation surfaces

The practical payoff is larger than this one migration:

- future official integrations can follow the same package shape
- security boundaries become visible in config and validation, not just at runtime
- migration UX improves because deprecated builtin paths can fail fast with precise diagnostics
- discoverability stays aligned with reality because catalog/source/install all describe the same capability identity

## When to Apply

- A builtin has started to need its own install/upgrade lifecycle.
- A capability needs operator-managed secrets that should not appear in ordinary workflow inputs.
- A feature needs destination-aware or capability-aware security policy.
- A runtime path is accumulating protocol-specific logic that does not belong to the control plane.
- A plugin has a mix of required and optional inputs and needs machine-readable schema support.
- You need to hard-cut a legacy capability path and provide deterministic migration diagnostics.

## Examples

### Canonical package contract

```toml
manifest_version = "2.0.0"
plugin_id = "http-node"
kind = "external_node"
entrypoint = "node.exec.v1"
capabilities = ["node:execute"]
executable = "bin/http-node"

[activation]
optional_secret_slots = ["authorization"]
requires_allowed_origins = true

[[operations]]
name = "request"
summary = "Send an outbound HTTP request"
input_schema = ["url"]
optional_input_schema = ["method", "headers", "body"]
output_schema = ["status", "ok", "url", "body", "headers"]
kind = "read"
```

### Canonical safety boundary inside the plugin

```rust
if !matches!(url.scheme(), "http" | "https") {
    return Err(PluginError::InvalidPolicy(
        "input `url` must use http or https".to_owned(),
    ));
}

if activation_authorization.is_some() {
    validate_allowed_origins(&url, &allowed_origins, true)?;
}
```

### Canonical migration guardrail

```rust
if node.kind == "builtin.http" || node.plugin_id == "builtin.http" {
    return Err(ContractError::InvalidWorkflowNodeField {
        workflow_id: workflow.workflow_id.clone(),
        node_id: node.node_id.clone(),
        field: "node.plugin",
        detail: "builtin.http has been retired; install the official `http-node` plugin...".to_owned(),
    });
}
```

### What didn’t work

These approaches were explicitly rejected during the solved migration:

- keeping `builtin.http` and adding `http-node` beside it
- silently forwarding builtin HTTP to the plugin inside runtime
- letting auth secrets continue to flow through ordinary request headers
- deferring redirect, SSRF, and destination-binding policy until implementation time

Each of those paths preserves ambiguity or weakens the boundary the migration was supposed to create.

## Related

- `docs/archive/2026-04-02-001-refactor-builtin-http-official-plugin-plan.md`
- `docs/decisions/CHAINBOT_OFFICIAL_PLUGIN_DESIGN.md`
- `docs/decisions/CHAINBOT_PLUGIN_ACTIVATION_CONFIG_DESIGN.md`
- `docs/decisions/CHAINBOT_CONFIG_STATE_LAYOUT_DESIGN.md`
- `docs/research/CHAINBOT_CLI_CATALOG_DISCOVERY_PROPOSAL.md`
- `crates/chainbot/src/plugin/contract.rs`
- `crates/chainbot/src/infrastructure/config/mod.rs`
- `crates/chainbot/src/app/definitions/validate.rs`
- `official-plugins/http-node/crate/src/client.rs`
- `examples/http-plugin-integrations/`
