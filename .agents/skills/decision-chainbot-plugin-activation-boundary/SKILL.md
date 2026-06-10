---
name: "decision-chainbot-plugin-activation-boundary"
description: "Load when changing plugin_activation config, activation.secrets or allowed_origins envelopes, secret binding resolution, or operator-owned plugin runtime config. Do not load for generic secret store changes unrelated to plugin activation."
license: "Proprietary"
metadata:
  generated_by: "decision-capture"
  created: "2026-06-10"
  last_updated: "2026-06-10"
  status: "current"
  affected_modules:
    - "crates/chainbot/src/domain/config/"
    - "crates/chainbot/src/app/runtime/execution.rs"
    - "crates/chainbot/src/app/runtime/external_triggers/"
    - "crates/chainbot/src/plugin/"
    - "docs/archive/decisions/CHAINBOT_PLUGIN_ACTIVATION_CONFIG_DESIGN.md"
  supersedes:
    - "docs/archive/decisions/CHAINBOT_PLUGIN_ACTIVATION_CONFIG_DESIGN.md"
  superseded_by: []
---

# Decision: ChainBot Plugin Activation Boundary

## Context

Installed plugin packages need local operator bindings for secrets and allowed
origins, but package manifests must stay package-owned. Reinstalling a plugin
must not overwrite root-local operational configuration, and runtime secret
injection must not be confused with business input.

## Decision

Plugin activation config is root-owned and lives in `<root>/chainbot.toml` under
`plugin_activation.<plugin_id>`.

`plugins/<plugin_id>/config.toml` is package-owned manifest state. Install,
reinstall, and upgrade flows must not overwrite operator-owned activation
config.

`secret_bindings` is a `slot -> secret ref` map. `allowed_origins` is an
operator-owned activation boundary for plugins that declare it. The runtime
does not interpret slot names or origin meaning beyond contract validation and
transport.

Resolved activation data is injected through a dedicated envelope:

```json
{
  "activation": {
    "secrets": {
      "signer": "<plaintext secret>",
      "rpc_token": "<plaintext secret>"
    },
    "allowed_origins": ["https://example.invalid"]
  }
}
```

Node plugin requests and trigger start commands use the same activation shape.
Workflow node `input` and trigger `params` remain business inputs and do not
carry activation secret bindings.

## Boundaries

- `chainbot.toml`: operator-owned activation config.
- `plugins/<plugin_id>/config.toml`: package-owned manifest.
- `crates/chainbot/src/app/runtime/execution.rs`: execution-time resolution and
  injection for node plugins.
- `crates/chainbot/src/app/runtime/external_triggers/`: execution-time
  resolution and injection for trigger plugins.
- `crates/chainbot/src/plugin/`: activation envelope and manifest activation
  contract.

## Implications

Root loading validates activation structure and secret-ref syntax, but secret
existence, decryption, and keyed lookup happen at execution time.

Plaintext secret material must not be written back to root config, plugin
packages, runtime state, checkpoints, or user-visible read models. Surfaced
plugin errors must be redacted against resolved activation values.

Plugins fail closed according to their own contract when required activation
slots are absent. The runtime must not add chain-specific activation logic.

## Non-goals

- Store activation config inside plugin package directories.
- Prefer environment variables as the activation secret transport.
- Merge activation secrets into workflow `input` or trigger `params`.
- Decrypt or persist plaintext secrets at install time.
