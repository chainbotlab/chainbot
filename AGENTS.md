# AGENTS.md

> AI agent root index for this repository. Read this file before touching code or docs.

## Project Metadata
- Current Phase: Implementation
- Last Updated: 2026-03-18
- Workspace Layout: `crates/`
- Critical Paths: `Cargo.toml`, `crates/chainbot/`, `docs/design/CHAINBOT_WORKSPACE_DESIGN.md`, `docs/design/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md`, `docs/design/CHAINBOT_CLI_DESIGN.md`, `docs/implementation/WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_V2_MVP_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_V21_CONFIG_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_V212_CLI_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_V213_CLI_IMPLEMENTATION.md`, `docs/implementation/CHAINBOT_BUILTINS_REFACTOR_IMPLEMENTATION.md`, `CONTRIBUTING.md`

## Documentation Topology
```text
.agents/

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
| `docs/design/CHAINBOT_CLI_DESIGN.md` | design | active | Defines the stable CLI command surface, status snapshot semantics, help system, and error navigation contract. |
| `docs/implementation/WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md` | implementation | active | Records bootstrap steps and validation expectations. |
| `docs/implementation/CHAINBOT_V2_MVP_IMPLEMENTATION.md` | implementation | active | Records the MVP artifact rename, evidence cleanup, and release-version alignment. |
| `docs/implementation/CHAINBOT_V21_CONFIG_IMPLEMENTATION.md` | implementation | active | Records the v2.1 package-layout implementation, root-config overrides, and validation coverage. |
| `docs/implementation/CHAINBOT_V212_CLI_IMPLEMENTATION.md` | implementation | active | Records the v2.1.2 CLI root-resolution contract update and release-version alignment. |
| `docs/implementation/CHAINBOT_V213_CLI_IMPLEMENTATION.md` | implementation | active | Records the v2.1.3 CLI init bootstrap, trigger listing surface, and release-version alignment. |
| `docs/implementation/CHAINBOT_BUILTINS_REFACTOR_IMPLEMENTATION.md` | implementation | active | Records the unified builtin namespace refactor, trait-backed registries, and final public API layout. |
| `docs/research/CHAINBOT_V212_CLI_PROPOSAL.md` | research | active | Proposes the v2.1.2 CLI usability surface with environment-based root resolution. |

## Fractal Architecture
- `.agents/`: Project-local agent assets and external skill catalog links.
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
