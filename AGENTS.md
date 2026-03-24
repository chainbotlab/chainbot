# AGENTS.md

> AI agent root index for this repository. Read this file before touching code or docs.

## Project Metadata
- Current Phase: Implementation
- Last Updated: 2026-03-24
- Workspace Layout: `crates/`
- Critical Paths: `Cargo.toml`, `crates/chainbot/`, `docs/design/CHAINBOT_WORKSPACE_DESIGN.md`, `docs/design/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md`, `docs/design/CHAINBOT_CONFIG_STATE_LAYOUT_DESIGN.md`, `docs/design/CHAINBOT_CLI_DESIGN.md`, `docs/implementation/WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_V2_MVP_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_V21_CONFIG_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_V212_CLI_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_V213_CLI_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_BUILTINS_REFACTOR_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_CORE_BUILTIN_NODES_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_TRIGGER_PARAMS_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_CONFIG_STATE_LAYOUT_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_STATE_READ_MODEL_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_DB_PRIMARY_RUNTIME_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_RUNTIME_HISTORY_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_INGRESS_TRIGGER_IMPLEMENTATION.md`, `docs/user/CHAINBOT_STORAGE_OPERATOR_GUIDE.md`, `docs/research/CHAINBOT_CLI_CATALOG_DISCOVERY_PROPOSAL.md`, `docs/research/CHAINBOT_LEGACY_LAYOUT_CONVERGENCE_PROPOSAL.md`, `CONTRIBUTING.md`

## Documentation Topology
```text
.agents/
examples/

docs/
|- design/
|- implementation/
|- research/
|- interfaces/
|- user/
`- archive/

postmortem/
crates/
```

## Active Context
| Doc | Type | Status | Summary |
|-----|------|--------|---------|
| `docs/design/CHAINBOT_WORKSPACE_DESIGN.md` | design | active | Defines the root workspace layout and repository boundaries. |
| `docs/design/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md` | design | active | Defines the ChainBot root TOML layout, workflow/node fields, trigger kinds, and plugin linkage. |
| `docs/design/CHAINBOT_CONFIG_STATE_LAYOUT_DESIGN.md` | design | active | Defines the stable package-aligned plugin layout, manifest version roles, and durable runtime state boundaries. |
| `docs/design/CHAINBOT_CLI_DESIGN.md` | design | active | Defines the stable CLI command surface, status snapshot semantics, help system, and error navigation contract. |
| `docs/implementation/WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md` | implementation | active | Records bootstrap steps and validation expectations. |
| `docs/implementation/CHAINBOT_V2_MVP_IMPLEMENTATION.md` | implementation | active | Records the MVP artifact rename, evidence cleanup, and release-version alignment. |
| `docs/implementation/CHAINBOT_V21_CONFIG_IMPLEMENTATION.md` | implementation | active | Records the v2.1 package-layout implementation, root-config overrides, and validation coverage. |
| `docs/implementation/CHAINBOT_V212_CLI_IMPLEMENTATION.md` | implementation | active | Records the v2.1.2 CLI root-resolution contract update and release-version alignment. |
| `docs/implementation/CHAINBOT_V213_CLI_IMPLEMENTATION.md` | implementation | active | Records the v2.1.3 CLI init bootstrap, trigger listing surface, and release-version alignment. |
| `docs/implementation/CHAINBOT_HELP_DIAGNOSTICS_IMPLEMENTATION.md` | implementation | active | Records the richer help cards, canonical config examples, and precise TOML/argv diagnostics. |
| `examples/README.md` | user-facing examples | active | Indexes curated single-workflow, builtin-triggers, workflow-composition, plugin-integrations, and custom-paths example roots. |
| `docs/implementation/CHAINBOT_BUILTINS_REFACTOR_IMPLEMENTATION.md` | implementation | active | Records the unified builtin namespace refactor, trait-backed registries, and final public API layout. |
| `docs/implementation/CHAINBOT_CORE_BUILTIN_NODES_IMPLEMENTATION.md` | implementation | active | Records the first-wave core builtin flow/data nodes, their stable registry surface, and validation coverage. |
| `docs/implementation/CHAINBOT_TRIGGER_PARAMS_IMPLEMENTATION.md` | implementation | active | Records the params-backed trigger extension model, builtin cron subtype, and validation coverage. |
| `docs/implementation/CHAINBOT_CONFIG_STATE_LAYOUT_IMPLEMENTATION.md` | implementation | active | Records the v3 canonical-only plugin/state layout cut, removed legacy compatibility, and validation coverage for the redesign. |
| `docs/implementation/CHAINBOT_STATE_READ_MODEL_IMPLEMENTATION.md` | implementation | active | Records the narrower run-summary reads, trigger snapshot read-model, and incremental trigger-state recovery inputs. |
| `docs/implementation/CHAINBOT_DB_PRIMARY_RUNTIME_IMPLEMENTATION.md` | implementation | active | Records the storage-mode extension and DB-primary runtime cut for CLI and trigger execution paths. |
| `docs/implementation/CHAINBOT_RUNTIME_HISTORY_IMPLEMENTATION.md` | implementation | active | Records the observe command, runtime history retention/archive contract, and hot-path guardrail coverage. |
| `docs/implementation/CHAINBOT_INGRESS_TRIGGER_IMPLEMENTATION.md` | implementation | active | Records the webhook/websocket ingress runtime, durable inbox staging seam, and serve-lifecycle listener wiring. |
| `docs/user/CHAINBOT_STORAGE_OPERATOR_GUIDE.md` | user | active | Explains operator-facing `local` and `postgres` storage modes, runtime boundaries, and the current no-legacy-import stance. |
| `docs/research/CHAINBOT_V212_CLI_PROPOSAL.md` | research | active | Proposes the v2.1.2 CLI usability surface with environment-based root resolution. |
| `docs/research/CHAINBOT_CLI_CATALOG_DISCOVERY_PROPOSAL.md` | research | active | Expands the reviewed baseline into an implementation-oriented spec for `catalog list/show`, plugin metadata enrichment, status plugin summaries, and test rollout order. |
| `docs/research/CHAINBOT_CONFIG_STATE_LAYOUT_PROPOSAL.md` | research | active | Proposes the one-step package and runtime-state layout that aligns plugin discovery, version semantics, and durable state boundaries. |
| `docs/research/CHAINBOT_LEGACY_LAYOUT_CONVERGENCE_PROPOSAL.md` | research | active | Proposes the phased retirement of legacy plugin and runtime-state layout support after canonical adoption is observable and safe. |

## Fractal Architecture
- `.agents/`: Project-local agent assets and external skill catalog links.
- `examples/`: Copyable root-level configuration cases aligned with the stable ChainBot root contract.
- `crates/`: Rust workspace members and crate-local manifests.
- `docs/`: Long-lived repository knowledge split by design, implementation, interfaces, research, user-facing behavior, and archive state.
- `postmortem/`: Durable debugging and incident learnings.

> Keep the map aligned with the terrain, or the terrain will be lost.

## Upstream / Downstream Map
```yaml
submodules: {}
upstream_forks: []
local_adapters: []
```

## Agent Operating Contract
- Required pre-read checks: root `AGENTS.md`, then nearest local `AGENTS.md`, then referenced docs in `docs/`.
- Required skills: use `fractal-context` and `fractal-repo` for code/doc structure changes; do not copy skill contents into this file.
- Workspace rule: treat the repository root as a pure Cargo workspace; application code lives under `crates/`.
- Formatting rule: do not run `fmt`, `cargo fmt`, or `rustfmt`; formatting invalidates cache and is intentionally skipped in this repository.
- Doc update triggers: new module, moved file, changed responsibility, changed contract, or new long-lived operational knowledge.

## Ambiguity Flags
- None.
