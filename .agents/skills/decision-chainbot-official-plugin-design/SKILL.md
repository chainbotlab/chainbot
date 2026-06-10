---
name: "decision-chainbot-official-plugin-design"
description: "Load when modifying official-plugins packages, official plugin manifests, chain-specific node or trigger packages, or thin runtime/plugin ownership boundaries. Do not load for third-party plugin authoring or unrelated workflow DAG changes."
license: "Proprietary"
metadata:
  generated_by: "decision-capture"
  created: "2026-06-10"
  last_updated: "2026-06-10"
  status: "current"
  affected_modules:
    - "official-plugins/"
    - "chainbot-plugin-index.toml"
    - "crates/chainbot/src/plugin/"
    - "crates/chainbot/src/app/runtime/"
  supersedes:
    - "docs/archive/decisions/CHAINBOT_OFFICIAL_PLUGIN_DESIGN.md"
  superseded_by: []
---

# Decision: ChainBot Official Plugin Design

## Context

Official plugins are repository-local capability packages, but they must not
become hidden runtime branches. Chain-specific behavior changes often tempt
authors to add Ethereum, Solana, or provider-specific cases inside the generic
runtime. That would make the runtime grow with every chain and weaken package
containment.

## Decision

Official packages under `official-plugins/` remain normal ChainBot plugin
packages and are not Cargo workspace members.

Official node packages use the external node contract, normally
`entrypoint = "node.exec.v2"`. Official trigger packages use the external
trigger contract, normally `entrypoint = "trigger.exec.v1"`.

Official packages prefer Rust package-local Cargo binary crates with this
install shape:

- `install_mode = "build_required"`
- `build.kind = "cargo"`
- `runtime = "bin"`
- `entry_artifact` points inside package-local `bin/`

The runtime stays a thin generic control plane. It owns package discovery,
source install orchestration, workflow scheduling, process/MCP host lifecycle,
execution-time secret resolution, redaction, accepted-event durability,
listener supervision, CLI surfaces, and catalog read models.

Chain-specific logic stays inside official plugin packages, including RPC
request shaping, chain data decoding, transaction assembly, signing payload
construction, signing algorithm selection, confirmation polling, listener
cursor encoding, event filtering, retry/backoff constants, and provider
compatibility handling.

## Boundaries

- `official-plugins/`: package-local code, manifests, assets, build outputs,
  and chain/provider semantics.
- `chainbot-plugin-index.toml`: source catalog entries for official packages.
- `crates/chainbot/src/plugin/`: generic manifest and host contracts only.
- `crates/chainbot/src/app/runtime/`: scheduling, dispatch, durability, secret
  resolution, and listener supervision only.

Runtime changes are allowed only when a guarantee cannot be expressed in
plugin-local code, when all official plugins would otherwise duplicate the same
host-side safety logic, or when host enforcement is required for correctness.

## Implications

Official package identity is directory name plus package-local `config.toml`.
Reinstall and upgrade flows must preserve package containment and keep build
artifacts under the package root.

Secret resolution by the runtime does not imply chain-aware signing in the
runtime. The host may inject resolved activation secrets; the plugin consumes
them and performs signing or provider auth locally.

Non-Rust official plugin exceptions must be explicitly recorded before
implementation, not decided ad hoc during coding.

## Non-goals

- Move chain business workflows into `chainbot runtime`.
- Add Ethereum-only, Solana-only, or provider-only branches to generic runtime
  scheduling, dispatch, or trigger acceptance.
- Require third-party plugins to inherit the official implementation preference.
- Define official plugin shared retry/backoff constants in the runtime.
