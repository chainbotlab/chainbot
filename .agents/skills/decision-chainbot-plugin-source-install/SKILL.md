---
name: "decision-chainbot-plugin-source-install"
description: "Load when changing plugin source list/show/install behavior, source repo metadata, chainbot-plugin-index.toml, package staging, overwrite, backup, or rollback semantics. Do not load for installed plugin execution protocol changes."
license: "Proprietary"
metadata:
  generated_by: "decision-capture"
  created: "2026-06-10"
  last_updated: "2026-06-10"
  status: "current"
  affected_modules:
    - "crates/chainbot/src/plugin/source/"
    - "crates/chainbot/src/cli/"
    - "official-plugins/"
    - "chainbot-plugin-index.toml"
  supersedes:
    - "docs/archive/decisions/CHAINBOT_PLUGIN_SOURCE_INSTALL_DESIGN.md"
  superseded_by: []
---

# Decision: ChainBot Plugin Source Install

## Context

ChainBot separates installed local capabilities from remote installable source
repositories. Discovery must be readable without mutating a root, while install
must be explicit and safe because it writes package contents into the current
root.

## Decision

`catalog` describes the current root's installed capabilities.

`plugin source list` and `plugin source show` describe remote source
repositories and are read-only.

`plugin install` is the only operator-facing command that writes a remote
source package into the current root.

Source repositories use one of two contracts:

- Single-plugin repository: repo root contains `config.toml`, with install
  metadata inside `config.toml[source]`.
- Multi-plugin repository: repo root contains `chainbot-plugin-index.toml`.

Legacy `source.toml` is not supported. Runtime execution continues to depend on
`<root>/plugins/<plugin_id>/config.toml` as the canonical installed package
entrypoint.

Install must use prepare, staging, swap, and root revalidation. Existing target
directories are not overwritten by default; `--force` is required for
replacement. Replacement must include backup and rollback semantics.

## Boundaries

- `crates/chainbot/src/plugin/source/`: source repository discovery,
  validation, staging, swap, and install orchestration.
- `crates/chainbot/src/cli/`: operator-facing `plugin source` and
  `plugin install` command surfaces.
- `official-plugins/` and `chainbot-plugin-index.toml`: repository-local
  official source catalog.
- `<root>/plugins/<plugin_id>/config.toml`: installed package entrypoint for
  runtime execution.

## Implications

Official packages use the same source/install contract as third-party packages;
they are not special runtime branches.

Install metadata is authoring/install metadata, not execution-time runtime
state. Installed manifests may retain install-only `[source]` metadata, but the
runtime uses package identity and execution fields for dispatch.

Source discoverability must keep remote installable packages distinct from the
current-root installed catalog.

## Non-goals

- Support legacy `source.toml`.
- Make `plugin source list/show` write root state.
- Define node or trigger host wire protocols.
- Bypass backup/rollback semantics for official packages.
