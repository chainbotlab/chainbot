Task 1 scope confirmation: freeze the final public module tree and establish a minimal compilable skeleton.

Constraints that apply:
- Root AGENTS: repository is a pure Cargo workspace; code lives under crates/; formatting commands must not run; doc updates are triggered by new modules, moved files, or contract changes.
- crates/AGENTS: only workspace members belong under crates/.
- crates/chainbot/AGENTS: the crate boundary is Cargo-manifest owned; src/ holds executable code and should keep responsibilities local.
- crates/chainbot/src/AGENTS: lib.rs is the public module map; the frozen public surface is app, domain, infrastructure, builtins, plugin, ingress, errors, script_protocol, secrets; main.rs should call app::cli::run_from_env(); builtins/plugin/ingress remain facade subtrees; no compatibility shims are allowed for retired root modules.
- crates/chainbot/src/builtins/AGENTS, nodes/AGENTS, triggers/AGENTS, plugin/AGENTS, ingress/AGENTS: preserve those subsystem boundaries and keep builtin dispatch, plugin contract, and ingress lifecycle inside their folders.
- docs/design/CHAINBOT_WORKSPACE_DESIGN.md, docs/design/CHAINBOT_CONFIG_STATE_LAYOUT_DESIGN.md, docs/design/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md, docs/design/CHAINBOT_CLI_DESIGN.md: define the stable workspace layout, root/package contracts, storage/state boundaries, and CLI surface that the skeleton must respect.
- docs/implementation/CHAINBOT_BUILTINS_REFACTOR_IMPLEMENTATION.md and docs/implementation/CHAINBOT_CONFIG_STATE_LAYOUT_IMPLEMENTATION.md: record settled builtin and layout decisions; treat them as append-only history, not rewrite targets.

Deferred to Task 10: broad consumer migration (tests, examples, docs, AGENTS sync), retired-import cleanup across the repository, and later documentation refreshes tied to moved files and finalized paths.

2026-03-27 Task 1 implementation decisions:
- Freeze `lib.rs` public map to exactly: `app`, `domain`, `infrastructure`, `builtins`, `plugin`, `ingress`, `errors`, `script_protocol`, `secrets`.
- Keep legacy modules (`catalog`, `cli`, `config`, `executor`, `external_trigger_supervisor`, `state`, `state_db`, `trigger`, `trigger_wasm`, `workflow`) as crate-private `mod` declarations to avoid premature code moves.
- Route binary startup through `chainbot::app::cli::run_from_env()` in `main.rs`, with `app::cli` forwarding into legacy CLI internals.

2026-03-27 Task 2 implementation decisions:
- Move CLI parse/help/top-level dispatch ownership into `src/app/cli.rs`, including `CliCommand`, `HelpTopic`, `CliRequest`, `CliOutput`, trigger/catalog/observe request types, parser helpers, help text builders, and version rendering.
- Keep status/observe rendering and daemon/runtime orchestration in `src/cli.rs` for this cut; expose command executors as `pub(crate)` `CliRequest` methods that `app::cli::dispatch()` calls.
- Add `app::cli::integration_test_support` as a temporary test bridge for runtime storage/state types consumed by `cli_surface` and `end_to_end_vertical_slice`, instead of restoring retired root-module exports.

2026-03-27 Task 2 rejection fix decisions:
- Remove `app::cli::integration_test_support` completely to restore the intended Task-2 ownership boundary and avoid public API surface creep.
- Keep `app::cli` parser/help/top-level dispatch ownership unchanged; repair tests by replacing internal-runtime type imports with test-local SQLite setup/query helpers only.

2026-03-27 Task 3 implementation decisions:
- Create `src/infrastructure/config/mod.rs` as the infrastructure-owned contract root for root/storage types, with `root_layout.rs`, `package_loader.rs`, and `loader.rs` split exactly by path resolution, file/package decoding, and side-effectful loader concerns.
- Move `RootDefinitionBundle::load` and bundle-wide validation out of infrastructure into `src/app/composition/{root_bundle.rs,validate.rs}`, and expose a new app entrypoint `load_root_definition_bundle()` for direct callers.
- Keep `src/config.rs` as a crate-private transitional re-export shim so existing internal `crate::config::*` type imports remain compile-safe while ownership moves to `infrastructure::config`.
- Rewire immediate Task-3 callers/tests (`src/cli.rs`, `tests/config_loading.rs`, `tests/contract_versions.rs`) to consume `app::composition` + `infrastructure::config` paths instead of the legacy root config choke point.

2026-03-27 Task 4 implementation decisions:
- Create `src/app/read_model/mod.rs` and move catalog/status/observe read-model responsibilities into `src/app/read_model/{catalog,status,observe}.rs`.
- Keep `src/catalog.rs` as a crate-private shim re-exporting `app::read_model::catalog` to minimize direct-caller churn while root ownership is already moved.
- Create `src/app/runtime/{mod,daemon}.rs` and move serve/stop/internal daemon orchestration there; keep `app::cli` as parse/help/dispatch boundary that delegates to runtime/read-model modules.
- Expose only the minimum `pub(crate)` runtime helpers/types from `src/cli.rs` (`RuntimeContext`, `SingleRunResult`, runtime helper fns) required by `app::runtime::daemon` to avoid pulling unrelated responsibilities into Task 4.

2026-03-27 Task 4 rejection fix decision:
- Keep the existing Task-4 ownership split and restore/retain status count semantics via data-path correctness only: `execute_status` must pass persisted `run_summaries` through unchanged, and `app::read_model::status` must derive `summary.run_count` from that persisted slice instead of any derived subset.

2026-03-27 Task 5 implementation decisions:
- Create `src/domain/state/mod.rs` as the backend-agnostic runtime-state contract root and move pure record/status/snapshot/lease models plus pure helper logic (`accepted_trigger_key`, token retention/upsert, default schema version) there.
- Introduce `src/infrastructure/state/{mod,file_store,sqlite_coordination,db_store}.rs` as infrastructure ownership entrypoints, and route runtime callers/imports to `crate::infrastructure::state::*` and model imports to `crate::domain::state::*`.
- Keep `src/state.rs` and `src/state_db.rs` as compatibility implementation surfaces for this task boundary while re-exporting/consuming domain contracts, so persistence semantics remain unchanged and required suites stay green.

2026-03-27 Task 5 rejection-fix decisions:
- Replace `infrastructure::state::{file_store,sqlite_coordination,db_store}` thin `pub use` facades with full implementation ownership by moving code bodies from legacy root files into these infrastructure files.
- Downgrade `src/state.rs` and `src/state_db.rs` to shim-only modules that re-export domain/infrastructure APIs for staged internal-call-site compatibility.
- Remove stray runtime artifact `examples/single-workflow/state/runtime.sqlite3` from the working diff to keep Task-5 scope clean.

2026-03-27 Task 6 implementation decisions:
- Create `src/domain/runtime/mod.rs` as the backend-agnostic execution-contract owner and move scheduler contract models/helpers there (`NodeDefinition` validation, `NormalizedRunRequest::new`, `ScheduledNodeState::is_terminal`).
- Create `src/app/runtime/execution.rs` as the concrete orchestration owner and move scheduler wave execution, builtin/plugin dispatch wiring, subflow recursion/depth/cycle guards, and plugin secret-aware runtime setup there.
- Keep `src/executor.rs` as a crate-private compatibility shim that re-exports moved execution/domain contract surfaces for internal continuity while callers are rewired.
- Rewire immediate call sites/tests to public app/domain/infrastructure paths (`app::ExecutionPlane`, `domain::runtime::*`, `infrastructure::config::RootLayout`) so required semantic suites run without reopening private root modules.

2026-03-27 Task 7 implementation decisions:
- Create `src/domain/workflow/mod.rs` with explicit `contract`, `variables`, `when`, and `subflow` modules, and move pure workflow semantics from legacy `src/workflow.rs` into those domain-owned files.
- Keep `src/workflow.rs` as a minimal compatibility shim (`pub use crate::domain::workflow::*;`) without retaining semantic logic, so staged internal continuity remains while ownership is explicit.
- Rewire direct consumers and tests to `crate::domain::workflow` / `chainbot::domain::workflow` imports, and update `domain::runtime` re-exports to source from `domain::workflow` instead of legacy root `workflow.rs`.

2026-03-27 Task 8 implementation decisions:
- Create `src/app/runtime/external_triggers/mod.rs` and physically move runtime supervision code to `src/app/runtime/external_triggers/supervisor.rs` plus wasm runtime host code to `src/app/runtime/external_triggers/wasmtime.rs`; keep `src/external_trigger_supervisor.rs` and `src/trigger_wasm.rs` as compatibility-only re-export shims.
- Add `src/domain/trigger/{mod,contract,emission,acceptance}.rs` as Task-8 domain ownership entrypoints and route direct trigger imports in runtime/config/composition/ingress/builtins/tests to `domain::trigger` paths.
- Rebind `tests/trigger_plane.rs` and `tests/ingress_runtime.rs` away from private legacy roots (`config/state/state_db`) to current public `infrastructure::{config,state}` and `domain::state` paths so required Task-8 suites remain runnable under the frozen root-module surface.

2026-03-27 Task 8 rejection-fix decisions:
- Replace facade-only `domain::trigger::{contract,emission,acceptance}` with real ownership by moving concrete definitions/impls from legacy `src/trigger.rs` into those files.
- Keep `src/trigger.rs` as shim-only (`pub use crate::domain::trigger::*;`) for staged continuity while removing direct logic ownership from the root module.
- Keep external session supervision ownership in `app::runtime::external_triggers` and avoid pulling long-lived daemon/session runtime code back into domain during the trigger-plane domain move.

2026-03-27 Task 8 runtime-boundary rejection fix decisions:
- Move remaining process listener runtime management out of `domain::trigger::acceptance` into `app::runtime::external_triggers::process_listener` (process spawn/stdin-stdout protocol loop/heartbeat timeout/ack handling/plugin manifest runtime validation).
- Keep `domain::trigger::acceptance` focused on pure acceptance semantics and runtime-state writes only; no process/thread/io/plugin-host runtime management imports remain there.
- Preserve old `TriggerPlane` API behavior through a shim-level collector in `src/trigger.rs`: open-time manifest validation remains intact, and external listener protocol errors continue surfacing during request collection.

2026-03-27 Task 9 implementation decisions:
- Treat Task-9 as strict consumer-path migration only: update imports in `tests/secrets_runtime.rs` and avoid any semantic/runtime fixture changes.
- Rebind state imports by ownership boundary (`domain::state` for runtime records/status, `infrastructure::state` for persistence stores/layout/coordination) and move root layout import to `infrastructure::config::RootLayout`.
- Keep scope narrow after audit confirmation: no other first-party tests/examples/snippets matched retired root imports in the Task-9 search pattern.

2026-03-27 Task 10 implementation decisions:
- Rewire only the four real `crate::config` dependencies called out by the blocker map and keep behavior unchanged via import-only updates.
- Keep shim files in place for Task-11 deletion readiness, but scrub source/helper text that still points at retired root-module names or obsolete legacy root-config guidance.
- Treat helper-text audit matches in `tests/cli_surface.rs` as intentional negative assertions rather than migration blockers.

2026-03-27 Task 10 rejection-fix decisions:
- Keep verification scope unchanged but explicitly clean `examples/single-workflow/state/runtime.sqlite3` after test execution so Task-10 remains source/helper-text-only with no example artifact drift.

2026-03-27 Task 10 final-sync decisions:
- `crates/chainbot/src/AGENTS.md` rewrite replaced the stale root-era member list with an explicit "Public Module Map" + "Legacy Compatibility Shims" table that accurately describes current ownership.
- Inline ASCII diagrams added using `//` style comments (not `//! ```text` blocks) to avoid LSP/rustc triple-backtick parsing issues in module-level doc comments.
- Implementation docs (`CHAINBOT_DB_PRIMARY_RUNTIME_IMPLEMENTATION.md`, `CHAINBOT_INGRESS_TRIGGER_IMPLEMENTATION.md`, etc.) were NOT modified because their "Files Changed" sections are historical records of past implementation work, not current-state claims; stale references in those sections refer to files that WERE changed at that time and still exist as compatibility shims.
- The stale-reference grep audit (pattern: retired root module paths in AGENTS/docs/examples) returns zero matches after the src/AGENTS.md update, confirming the AGENTS is the single source of stale ownership claims.

2026-03-27 final-wave decisions:
- Keep staging payload writes raw and preserve mapping ownership in `domain::trigger::emission::map_trigger_payload`; do not duplicate mapping in daemon/supervisor staging paths.
- Keep TriggerPlane compatibility constructors in `app::runtime` (`mod.rs`) and remove `app/runtime/trigger_plane.rs` to match final ownership shape.
- Keep `app/cli_runtime.rs` removed; runtime helper ownership remains under app-layer CLI/runtime modules without restoring root-style shim naming.
- Keep domain module split explicit with declaration/re-export `mod.rs` only for `domain::state` and `domain::runtime`.

2026-03-27 reviewer-hardening decisions:
- Choose directory-module shape for CLI (`app/cli/mod.rs`) rather than oversized single file to remove out-of-plan sibling `app/cli_impl.rs` without changing `app::cli` import paths.
- Keep daemon-facing helper visibility exactly at `app::cli` boundary via `pub(crate) use runtime::{...}`; only move file location, no behavior edits.
- Fix contradictory current-state wording only in top-level implementation doc ownership notes; keep historical implementation sections untouched.

2026-03-27 final reviewer-closure decisions:
- Replace `app/cli/runtime.rs` with plan-friendly `app/cli/commands.rs` and split parse/help into dedicated files (`parse.rs`, `help.rs`) to satisfy fixed-tree objections with minimal risk.
- Keep `CliRequest::dispatch` in `commands.rs` and keep `run_from_env` in `mod.rs` so external entrypoint semantics remain unchanged.
- Keep `.sisyphus/boulder.json` tracked state neutralized (not deleted in diff) to clear F4 without touching plan/notepad artifacts.
