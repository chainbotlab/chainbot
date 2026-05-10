---
title: Add Hyperliquid official node and trigger plugins
date: 2026-05-11
category: implementation
module: chainbot
problem_type: feature
component: official-plugins
severity: medium
applies_when:
  - adding official exchange integrations without widening the generic runtime
  - packaging read-only market data and trigger listeners as standalone plugins
  - extending catalog and source discovery surfaces for new official plugins
symptoms:
  - Hyperliquid market data is unavailable through official plugin install surfaces
  - catalog and source listing do not expose Hyperliquid plugin packages
  - exchange-specific listener logic risks leaking into generic runtime paths
root_cause: missing_integration
resolution_type: implementation
tags:
  - hyperliquid
  - official-plugin
  - external-node
  - external-trigger
  - catalog
  - source-discovery
---

# Add Hyperliquid official node and trigger plugins

## Context

This change adds a first-cut Hyperliquid integration as official plugins instead of expanding `crates/chainbot` with exchange-specific runtime branches.

The chosen scope stays intentionally narrow:

- `hyperliquid-node` provides read-only `/info` operations
- `hyperliquid-trigger` provides market websocket listeners for `trades` and `l2Book`
- no signed actions
- no generic runtime refactor

That keeps the control plane generic while still making Hyperliquid installable, discoverable, and testable through the same official-plugin surfaces used by other integrations.

## Implementation

### 1. Added `official-plugins/hyperliquid-node/`

The node package is a standalone build-required plugin with `node.exec.v2` entrypoint and executable artifact `bin/hyperliquid-node`.

Implemented operations:

- `hyperliquid_get_all_mids`
- `hyperliquid_get_l2_book`
- `hyperliquid_get_candle_snapshot`

Package behavior includes:

- request parsing for legacy and JSON-RPC envelopes
- request validation per operation
- outbound `/info` POST execution
- destination safety policy with test-only loopback gates
- activation manifest declares an optional `origin_binding` slot so origin-restricted installs satisfy the current host contract
- deterministic tests for normalized read responses and blocked unsafe destinations

### 2. Added `official-plugins/hyperliquid-trigger/`

The trigger package is a standalone build-required plugin with `trigger.exec.v1` entrypoint and executable artifact `bin/hyperliquid-trigger`.

Implemented trigger sources:

- `hyperliquid_trades`
- `hyperliquid_l2_book`

Package behavior includes:

- start-envelope parsing and source validation
- Hyperliquid websocket subscription request shaping
- subscription ack handling on `subscriptionResponse`
- normalized ready/event frame emission
- reconnect-on-close loop
- endpoint allowlist enforcement with safe test overrides
- activation manifest declares an optional `origin_binding` slot so origin-restricted installs satisfy the current host contract
- deterministic mock and websocket listener tests

### 3. Registered official discovery surfaces

Updated the top-level source catalog in `chainbot-plugin-index.toml` to include:

- `hyperliquid-node`
- `hyperliquid-trigger`

Extended integration coverage so source and catalog surfaces can project Hyperliquid packages alongside the prior Ethereum and Solana official fixtures.

## Validation

Executed:

```text
rtk cargo test                         # official-plugins/hyperliquid-node/crate
rtk cargo test                         # official-plugins/hyperliquid-trigger/crate
```

Follow-up integration validation for catalog/source surfaces should cover:

- `crates/chainbot/tests/plugin_source_surface.rs`
- `crates/chainbot/tests/catalog_surface.rs`

## Constraints Kept

- kept provider semantics inside plugin packages
- kept runtime/plugin contract boundaries aligned with `node.exec.v2` and `trigger.exec.v1`
- avoided signed Hyperliquid actions in the first cut
- avoided generic runtime branches for Hyperliquid-specific behavior
- kept tests deterministic and loopback-gated
