# ChainBot In-Crate Reorganization

## TL;DR
> **Summary**: 在不拆 Cargo crate 的前提下，把 `crates/chainbot` 从顶层大文件布局重组为明确的 `app / domain / infrastructure` 结构，同时保留 `builtins/`、`plugin/`、`ingress/` 这三个已有健康 facade subtree。此次重组允许公开模块路径直接切换到新结构，但必须保持运行时语义、CLI 行为、配置格式、plugin ABI 与 ingress 语义不变。
> **Deliverables**:
> - 新的公开模块树：`app`, `domain`, `infrastructure`, `builtins`, `plugin`, `ingress`, `errors`, `script_protocol`, `secrets`
> - `cli`、`config`、`state`、`executor`、`workflow`、`trigger`、`external_trigger_supervisor`、`catalog` 的最终归位与职责收敛
> - 完整的 first-party import / tests / docs / AGENTS 同步迁移
> - 一套原子提交顺序，确保每次 public-path cut 都有对应测试与文档更新
> **Effort**: XL
> **Parallel**: YES - 3 waves
> **Critical Path**: Task 1 → Task 2 → Task 4 → Task 6 → Task 9 → Task 11 → Task 12 → Final Verification

## Context
### Original Request
- 用户先要求审查“是否该立刻拆多 crate”，随后确认应先做单 crate 内重组。
- 用户进一步要求基于已经讨论出的架构决策，制作一份具体实施文档。
- 用户明确选择：不保留兼容层，允许公开模块路径直接切换到新结构。

### Interview Summary
- 保持 `crates/chainbot` 为单 crate，不做 workspace 级拆分。
- 第一阶段不追求全局对称，而是只重构问题热点；`builtins/`、`plugin/`、`ingress/` 继续保留为 facade subtree。
- `state` 必须拆成 model/contract 与 backend/storage，避免“状态模型 = 基础设施实现”的混合边界。
- `executor` 必须拆成 core contract 与 runtime orchestration，避免“假领域层”。
- `config` 必须拆成 infrastructure loader 与 app composition root，避免 bundle validation 继续滞留在“配置模块”里。
- 用户拒绝兼容层，因此这次计划必须把代码、测试、示例、文档、AGENTS 在同一阶段内整体切换到新路径。

### What Already Exists
- `crates/chainbot/src/builtins/mod.rs:10-14` 已是健康 facade，内部 `nodes/` 与 `triggers/` 子树职责清晰。
- `crates/chainbot/src/plugin/mod.rs:10-23` 已是稳定 facade，不应在本次计划中整体迁移。
- `crates/chainbot/src/ingress/mod.rs:10-23` 已是稳定 facade，不应为追求分层对称而整体重写。
- `crates/chainbot/src/catalog.rs` 已经承担 CLI 发现/状态读模型职责，应复用而不是另造并行 read model。
- `crates/chainbot/src/external_trigger_supervisor.rs:1-140` 已经把长生命周期 external trigger session 从 `TriggerPlane` 中抽出，是 trigger runtime 重组的现成支点。

### Not in Scope
- 不拆分为 `chainbot-core` / `chainbot-runtime` / `chainbot-cli` 等多个 Cargo crates；原因：当前依赖方向仍在抖动。
- 不整体重写 `builtins/`、`plugin/`、`ingress/` 内部实现；原因：这些子树已具备清晰 facade 边界。
- 不改变 `chainbot.toml`、workflow/trigger/plugin manifest 的业务语义；原因：这是结构重组，不是配置 contract 改版。
- 不改变 plugin ABI、worker protocol、ingress transport 语义；原因：这些都属于行为/集成契约，不应与结构迁移耦合。
- 不顺手做 CLI UX 改版或业务语义 cleanup；原因：此次必须限制为结构性重组。

### Metis Review (gaps addressed)
- 计划明确冻结最终公开模块树，而不是边做边决定最终 public surface。
- 计划明确把 `catalog` 与 `external_trigger_supervisor` 的归属写入任务，而不是留作隐式整理。
- 计划加入“Must NOT Change”边界，防止目录重组偷带语义改动。
- 计划要求每个 public-path cut 同时更新代码、测试、docs、AGENTS，避免“代码搬完再补文档”。
- 计划采用原子提交顺序，避免一次性 mega-move 导致回归难以定位。

## Work Objectives
### Core Objective
- 把 `crates/chainbot` 重组为可持续演进的单 crate 结构：`app` 承担 CLI / composition / runtime orchestration，`domain` 承担 workflow/trigger/runtime/state contracts，`infrastructure` 承担 config loader 与 state backends；同时保留 `builtins`、`plugin`、`ingress` 为独立 facade subtree。

### Deliverables
- 最终公开模块树：

```rust
pub mod app;
pub mod domain;
pub mod infrastructure;
pub mod builtins;
pub mod plugin;
pub mod ingress;
pub mod errors;
pub mod script_protocol;
pub mod secrets;
```

- `main.rs` 入口更新为新公开路径，不再调用旧的 `chainbot::cli::run_from_env()`。
- `cli` 拆分为 `app::cli`、`app::runtime`、`app::read_model`、`app::composition`。
- `config` 拆分为 `infrastructure::config` 与 `app::composition`。
- `state` 拆分为 `domain::state` 与 `infrastructure::state`。
- `executor` 拆分为 `domain::runtime` 与 `app::runtime::execution`。
- `workflow`、`trigger` 迁入 `domain` 子树；`external_trigger_supervisor` 与 `trigger_wasm` 迁入 `app::runtime`。
- 所有 first-party tests/examples/docs/AGENTS 切换到最终模块路径。

### Fixed Target Source Tree
```text
crates/chainbot/src/
├─ app/
│  ├─ mod.rs
│  ├─ cli/
│  │  ├─ mod.rs
│  │  ├─ parse.rs
│  │  ├─ help.rs
│  │  └─ commands/
│  ├─ composition/
│  │  ├─ mod.rs
│  │  ├─ root_bundle.rs
│  │  └─ validate.rs
│  ├─ read_model/
│  │  ├─ mod.rs
│  │  ├─ catalog.rs
│  │  ├─ status.rs
│  │  └─ observe.rs
│  └─ runtime/
│     ├─ mod.rs
│     ├─ daemon.rs
│     ├─ execution.rs
│     └─ external_triggers/
│        ├─ mod.rs
│        ├─ supervisor.rs
│        └─ wasmtime.rs
├─ domain/
│  ├─ mod.rs
│  ├─ workflow/
│  │  ├─ mod.rs
│  │  ├─ contract.rs
│  │  ├─ variables.rs
│  │  ├─ when.rs
│  │  └─ subflow.rs
│  ├─ trigger/
│  │  ├─ mod.rs
│  │  ├─ contract.rs
│  │  ├─ emission.rs
│  │  └─ acceptance.rs
│  ├─ runtime/
│  │  ├─ mod.rs
│  │  ├─ contract.rs
│  │  └─ report.rs
│  └─ state/
│     ├─ mod.rs
│     ├─ model.rs
│     ├─ lease.rs
│     └─ records.rs
├─ infrastructure/
│  ├─ mod.rs
│  ├─ config/
│  │  ├─ mod.rs
│  │  ├─ root_layout.rs
│  │  ├─ loader.rs
│  │  └─ package_loader.rs
│  └─ state/
│     ├─ mod.rs
│     ├─ file_store.rs
│     ├─ db_store.rs
│     └─ sqlite_coordination.rs
├─ builtins/
├─ plugin/
├─ ingress/
├─ errors.rs
├─ script_protocol.rs
├─ secrets.rs
├─ lib.rs
└─ main.rs
```

### Explicit File Rehoming Decisions
- `cli.rs` → `app/cli/` + `app/runtime/daemon.rs` + `app/read_model/{status,observe,catalog}.rs`
- `catalog.rs` → `app/read_model/catalog.rs`
- `config.rs` → `infrastructure/config/*` + `app/composition/{root_bundle,validate}.rs`
- `state.rs` → `domain/state/{model,lease,records}.rs` + `infrastructure/state/file_store.rs` + `infrastructure/state/sqlite_coordination.rs`
- `state_db.rs` → `infrastructure/state/db_store.rs`
- `executor.rs` → `domain/runtime/{contract,report}.rs` + `app/runtime/execution.rs`
- `workflow.rs` → `domain/workflow/{contract,variables,when,subflow}.rs`
- `trigger.rs` → `domain/trigger/{contract,emission,acceptance}.rs`
- `external_trigger_supervisor.rs` → `app/runtime/external_triggers/supervisor.rs`
- `trigger_wasm.rs` → `app/runtime/external_triggers/wasmtime.rs`

### Definition of Done (verifiable conditions with commands)
- `cargo check -p chainbot` exits with code `0`.
- `cargo test -p chainbot --lib` exits with code `0`.
- `cargo test -p chainbot --test contract_versions` exits with code `0`.
- `cargo test -p chainbot --test config_loading` exits with code `0`.
- `cargo test -p chainbot --test workflow_dag_semantics` exits with code `0`.
- `cargo test -p chainbot --test execution_scheduler` exits with code `0`.
- `cargo test -p chainbot --test trigger_plane` exits with code `0`.
- `cargo test -p chainbot --test runtime_state_parity` exits with code `0`.
- `cargo test -p chainbot --test runtime_guardrails` exits with code `0`.
- `cargo test -p chainbot --test state_runtime_persistence` exits with code `0`.
- `cargo test -p chainbot --test secrets_runtime` exits with code `0`.
- `cargo test -p chainbot --test worker_host` exits with code `0`.
- `cargo test -p chainbot --test node_plugin_host` exits with code `0`.
- `cargo test -p chainbot --test cli_surface` exits with code `0`.
- `cargo test -p chainbot --test catalog_surface` exits with code `0`.
- `cargo test -p chainbot --test ingress_runtime` exits with code `0`.
- `cargo test -p chainbot --test end_to_end_vertical_slice` exits with code `0`.
- `cargo test -p chainbot` exits with code `0` after all path migrations are complete.
- A repository-wide first-party import audit shows no remaining imports from retired root modules such as `chainbot::cli`, `chainbot::config`, `chainbot::workflow`, `chainbot::executor`, `chainbot::trigger`, `chainbot::state`, `chainbot::state_db`, or `chainbot::external_trigger_supervisor`.

### Must Have
- 明确且唯一的最终公开模块树与 retired module 清单。
- `builtins/`、`plugin/`、`ingress/` 继续保持 facade subtree，不被卷入无收益迁移。
- `domain::state` 不依赖任何 backend/store 实现。
- `domain::runtime` 只持有 execution contracts，不直接依赖 plugin host、builtin registry 实现、或 secret IO。
- `app::composition` 成为唯一 bundle assembly / validation root。
- 每个 public-path cut 都在同一任务内更新 tests/examples/docs/AGENTS。
- 复杂模块补充/更新 ASCII 图注位置说明。

### Must NOT Have (guardrails, AI slop patterns, scope boundaries)
- 不得保留长期兼容层或通过 `pub use` 维持旧 `chainbot::...` 路径。
- 不得把 `state` 的模型/contract 与 backend/store 继续混在一起。
- 不得把 `executor` 整块伪装成纯 domain，实际继续耦合 builtin/plugin/secret adapters。
- 不得把 `config` 继续作为 bundle assembly 与 package loader 的混合缝合点。
- 不得在同一任务里同时做结构迁移与无关语义 cleanup。
- 不得修改 `chainbot.toml`、workflow/trigger/plugin manifest 的业务 contract。
- 不得整体重写 `builtins/`、`plugin/`、`ingress/` 的内部目录，仅允许必要引用调整。

## Verification Strategy
> ZERO HUMAN INTERVENTION — all verification is agent-executed.
- Test decision: tests-after + Rust built-in unit/integration test framework
- QA policy: every task includes implementation plus deterministic QA scenarios
- Evidence: `.sisyphus/evidence/task-{N}-{slug}.{ext}`

## Execution Strategy
### Parallel Execution Waves
- Wave 1: Tasks 1-4 — freeze target surface, create skeleton, split CLI, split config/composition
- Wave 2: Tasks 5-8 — split state, executor, workflow, trigger runtime boundaries
- Wave 3: Tasks 9-12 — rehome read models/runtime control, migrate consumers/docs, remove retired modules, run full regression matrix

### Dependency Matrix (full, all tasks)
- Task 1 blocks Tasks 2-12
- Task 2 blocks Tasks 3-12
- Task 3 blocks Tasks 7-12
- Task 4 blocks Tasks 9-12
- Task 5 blocks Tasks 10-12
- Task 6 blocks Tasks 7-12
- Task 7 blocks Tasks 8-12
- Task 8 blocks Tasks 9-12
- Task 9 blocks Tasks 10-12
- Task 10 blocks Tasks 11-12
- Task 11 blocks Task 12 and Final Verification
- Task 12 blocks Final Verification

### Agent Dispatch Summary (wave → task count → categories)
- Wave 1 → 4 tasks → `deep`, `unspecified-high`
- Wave 2 → 4 tasks → `deep`, `ultrabrain`
- Wave 3 → 4 tasks → `deep`, `writing`, `unspecified-high`

## TODOs
> Implementation + Test = ONE task. Never separate.
> EVERY task MUST have: Agent Profile + Parallelization + QA Scenarios.

- [x] 1. Freeze the final public module tree and establish a minimal compilable skeleton

  **What to do**: Rewrite `crates/chainbot/src/lib.rs` so the final public surface is exactly `app`, `domain`, `infrastructure`, `builtins`, `plugin`, `ingress`, `errors`, `script_protocol`, and `secrets`. In the same task, create the minimal `app/`, `domain/`, and `infrastructure/` module skeleton plus a compilable `app::cli::run_from_env()` entrypoint, then update `crates/chainbot/src/main.rs` to call it. This task freezes the target public surface and creates the minimum file structure required for later tasks to move code into real destinations.
  **Must NOT do**: Do not leave temporary `pub use` compatibility shims for retired root modules. Do not require tests/examples/docs to move in this task; broad consumer migration belongs later.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: this task freezes the target public API and drives every subsequent path migration
  - Skills: [`rust-router`] — Reason: Rust module/public-surface reasoning
  - Omitted: [`fractal-context`] — Reason: file headers/docs updates happen later alongside moved responsibilities

  **Parallelization**: Can Parallel: NO | Wave 1 | Blocks: [2,3,4,5,6,7,8,9,10,11,12] | Blocked By: []

  **References**:
  - Public surface baseline: `crates/chainbot/src/lib.rs:11-26` — current top-level module map to retire/replace
  - Entrypoint baseline: `crates/chainbot/src/main.rs:10-23` — current call to `chainbot::cli::run_from_env()`
  - Blast radius examples: `crates/chainbot/tests/config_loading.rs:15-20`, `crates/chainbot/tests/cli_surface.rs:18-20`, `crates/chainbot/tests/trigger_plane.rs:15-25` — tests that will need new import paths immediately

  **Acceptance Criteria**:
  - [ ] `lib.rs` exposes only the final agreed root modules.
  - [ ] `app::cli::run_from_env()` exists and is the binary entrypoint used by `main.rs`.
  - [ ] The crate compiles with the new root-module skeleton even though broad consumer migration is still pending.

  **QA Scenarios**:
  ```
  Scenario: new root module tree compiles
    Tool: Bash
    Steps: cargo check -p chainbot
    Expected: compilation succeeds with the new root module tree and no temporary compatibility exports
    Evidence: .sisyphus/evidence/task-1-final-module-tree.txt

  Scenario: root-module skeleton is sufficient for subsequent moves
    Tool: Bash
    Steps: cargo test -p chainbot --lib
    Expected: the crate builds and unit tests run with the new root-module skeleton before broad import/doc migration begins
    Evidence: .sisyphus/evidence/task-1-final-module-tree-error.txt
  ```

  **Commit**: YES | Message: `refactor(chainbot): establish final root module skeleton` | Files: `crates/chainbot/src/lib.rs`, `crates/chainbot/src/main.rs`, new `app/**`, `domain/**`, `infrastructure/**` skeleton files

- [x] 2. Move CLI parser/help/command dispatch into `app::cli`

  **What to do**: Move the core CLI entrypoint, argument parsing, help surface, and command dispatch flow from the old `cli.rs` root into `app::cli`. This task should leave status/observe/catalog render logic and daemon-specific orchestration in temporary app-layer locations if needed; those are extracted cleanly in Task 4. The only goal here is to make `app::cli` the true home of parse/help/dispatch without changing shell-visible behavior.
  **Must NOT do**: Do not redesign CLI UX, help text, command names, or shell-visible behavior. Do not move `catalog.rs` or daemon/read-model responsibilities to their final destinations in this task.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: this is the highest-churn path and controls CLI/runtime orchestration boundaries
  - Skills: [`rust-router`] — Reason: Rust module/file moves plus entrypoint rewiring
  - Omitted: [`domain-cli`] — Reason: no new CLI feature design is in scope

  **Parallelization**: Can Parallel: NO | Wave 1 | Blocks: [3,4,5,6,7,8,9,10,11,12] | Blocked By: [1]

  **References**:
  - CLI hotspot: `crates/chainbot/src/cli.rs:1-220`, `crates/chainbot/src/cli.rs:18-56` — current all-in-one command/runtime boundary
  - Binary entrypoint: `crates/chainbot/src/main.rs:10-23` — now points to `app::cli::run_from_env()`
  - CLI surface tests: `crates/chainbot/tests/cli_surface.rs:1-60`, `crates/chainbot/tests/end_to_end_vertical_slice.rs`

  **Acceptance Criteria**:
  - [ ] `app::cli` owns parsing, help, and top-level command dispatch.
  - [ ] `main.rs` and all command entry callers route only through `app::cli`.
  - [ ] CLI integration tests continue to assert the same observable behavior after path updates.

  **QA Scenarios**:
  ```
  Scenario: CLI surface remains stable after rehome
    Tool: Bash
    Steps: cargo test -p chainbot --test cli_surface
    Expected: help, init, validate, trigger toggle, serve/stop, and list-runs behavior matches existing expectations
    Evidence: .sisyphus/evidence/task-2-cli-rehome.txt

  Scenario: end-to-end command dispatch still works after CLI extraction
    Tool: Bash
    Steps: cargo test -p chainbot --test end_to_end_vertical_slice
    Expected: validate/run/serve/list-runs flows still execute with `app::cli` as the command entry boundary
    Evidence: .sisyphus/evidence/task-2-cli-rehome-error.txt
  ```

  **Commit**: YES | Message: `refactor(chainbot): move cli parser and dispatch into app layer` | Files: `crates/chainbot/src/app/cli/**`, moved CLI callers/tests

- [x] 3. Split config into infrastructure loader and app composition root

  **What to do**: Move path/root/package loading logic into `infrastructure::config::{root_layout, package_loader, loader}` and move `RootDefinitionBundle::load()` plus bundle-wide validation/composition responsibilities into `app::composition`. Keep `RootLayout`, storage config, and file decoding in infrastructure; move cross-package assembly and validation orchestration out of infrastructure. Update all callers to use the new composition entrypoint rather than a root `config` module.
  **Must NOT do**: Do not change root-config semantics, path defaults, storage mode behavior, or manifest validation semantics. Do not leave `bundle_validation` half in infrastructure and half in app.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: this task removes the current composition choke point and sets dependency direction for later domain moves
  - Skills: [`rust-router`] — Reason: contract-preserving module split
  - Omitted: [`domain-web`] — Reason: no HTTP API or protocol redesign here

  **Parallelization**: Can Parallel: YES | Wave 1 | Blocks: [7,8,9,10,11,12] | Blocked By: [1,2]

  **References**:
  - Composition hotspot: `crates/chainbot/src/config.rs:494-528` — current `RootDefinitionBundle::load()` composition root
  - Cross-package validation hotspot: `crates/chainbot/src/config.rs:589-631` — current bundle validation plus ingress desired-state validation
  - Config tests: `crates/chainbot/tests/config_loading.rs:15-20`, `crates/chainbot/tests/contract_versions.rs:10-14`

  **Acceptance Criteria**:
  - [ ] Infrastructure config modules own file/path decoding and storage-layout types only.
  - [ ] App composition owns bundle assembly and bundle-wide validation.
  - [ ] `config_loading` and `contract_versions` tests pass with new paths and unchanged behavior.

  **QA Scenarios**:
  ```
  Scenario: config loading boundary remains stable
    Tool: Bash
    Steps: cargo test -p chainbot --test config_loading
    Expected: root layout resolution, env override behavior, package loading, and invalid TOML handling all remain unchanged
    Evidence: .sisyphus/evidence/task-3-config-split.txt

  Scenario: version and contract gating still works
    Tool: Bash
    Steps: cargo test -p chainbot --test contract_versions
    Expected: version-major acceptance/rejection behavior remains unchanged after config/composition split
    Evidence: .sisyphus/evidence/task-3-config-split-error.txt
  ```

  **Commit**: YES | Message: `refactor(chainbot): split config loader from composition root` | Files: `crates/chainbot/src/infrastructure/config/**`, `crates/chainbot/src/app/composition/**`, callers/tests

- [x] 4. Rehome catalog and daemon control into explicit app read-model/runtime modules

  **What to do**: After Task 2 has established `app::cli` as the command boundary, extract the remaining non-CLI responsibilities out of it: move `catalog.rs` plus status/observe/catalog rendering helpers into `app::read_model`, and move daemon/serve/stop orchestration helpers into `app::runtime::daemon`. This task resolves the temporary overlap left intentionally in Task 2.
  **Must NOT do**: Do not leave `catalog` as a root module after Task 1. Do not let daemon control code depend on deprecated root-module paths.

  **Recommended Agent Profile**:
  - Category: `unspecified-high` — Reason: app-layer cleanup tied tightly to CLI/runtime behavior but with narrower blast radius than the core domain splits
  - Skills: [`rust-router`] — Reason: move-only refactor with import rewiring
  - Omitted: [`writing`] — Reason: docs are updated in a later dedicated task

  **Parallelization**: Can Parallel: YES | Wave 1 | Blocks: [9,10,11,12] | Blocked By: [2]

  **References**:
  - Current catalog root module: `crates/chainbot/src/catalog.rs`
  - Current daemon/serve control in CLI: `crates/chainbot/src/cli.rs` (serve/stop/status/observe paths)
  - Runtime-facing tests: `crates/chainbot/tests/cli_surface.rs`, `crates/chainbot/tests/catalog_surface.rs`, `crates/chainbot/tests/ingress_runtime.rs`

  **Acceptance Criteria**:
  - [ ] Catalog/status/observe code no longer lives at the root module level.
  - [ ] Daemon control helpers are isolated under `app::runtime::daemon` or equivalent.
  - [ ] CLI-facing tests still pass with unchanged observable output/behavior.

  **QA Scenarios**:
  ```
  Scenario: runtime-facing CLI commands still pass after app read-model/runtime split
    Tool: Bash
    Steps: cargo test -p chainbot --test cli_surface && cargo test -p chainbot --test catalog_surface
    Expected: status/observe/catalog/serve/stop flows remain behaviorally stable
    Evidence: .sisyphus/evidence/task-4-app-read-model-runtime.txt

  Scenario: ingress-related daemon integration still holds
    Tool: Bash
    Steps: cargo test -p chainbot --test ingress_runtime
    Expected: serve-path orchestration still boots ingress runtime and persists accepted events correctly
    Evidence: .sisyphus/evidence/task-4-app-read-model-runtime-error.txt
  ```

  **Commit**: YES | Message: `refactor(chainbot): isolate app read models and daemon control` | Files: `crates/chainbot/src/app/read_model/**`, `crates/chainbot/src/app/runtime/**`, moved callers/tests

- [x] 5. Split state into domain contracts/models and infrastructure backends

  **What to do**: Extract runtime-state records, lease snapshots, trigger/run/checkpoint models, and any backend-agnostic validation/helpers out of `crates/chainbot/src/state.rs` into `domain::state`. Move file-backed persistence, SQLite coordination, and backend-specific store code into `infrastructure::state`. Keep `RuntimeStateStore` under infrastructure and make it depend on `domain::state` model types, never the reverse.
  **Must NOT do**: Do not leave backend-specific types in `domain::state`. Do not duplicate record types across domain and infrastructure. Do not change persistence semantics while moving code.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: this is the highest-risk domain/infrastructure boundary and underpins trigger/runtime behavior
  - Skills: [`rust-router`] — Reason: Rust type ownership and module-cycle elimination
  - Omitted: [`m12-lifecycle`] — Reason: resource lifecycle redesign is not the goal; ownership boundaries are

  **Parallelization**: Can Parallel: NO | Wave 2 | Blocks: [6,8,9,10,11,12] | Blocked By: [1,2]

  **References**:
  - State models + file backend mix: `crates/chainbot/src/state.rs:47-199`, `crates/chainbot/src/state.rs:190-220`
  - DB backend using state models: `crates/chainbot/src/state_db.rs:19-24`, `crates/chainbot/src/state_db.rs:65-67`, `crates/chainbot/src/state_db.rs:135-203`
  - Parity tests: `crates/chainbot/tests/runtime_state_parity.rs:17-27`, `crates/chainbot/tests/runtime_guardrails.rs:14-16`, `crates/chainbot/tests/state_runtime_persistence.rs:15-16`

  **Acceptance Criteria**:
  - [ ] `domain::state` contains backend-agnostic runtime-state types and contracts only.
  - [ ] `infrastructure::state` owns `RuntimeStateStore`, file-backed persistence, and backend-specific IO.
  - [ ] State parity, persistence, and guardrail tests pass unchanged in behavior.

  **QA Scenarios**:
  ```
  Scenario: DB/file state behavior remains stable after state split
    Tool: Bash
    Steps: cargo test -p chainbot --test runtime_state_parity && cargo test -p chainbot --test state_runtime_persistence
    Expected: lease, run summary, snapshot/checkpoint, staged trigger, and file-backed persistence semantics remain unchanged
    Evidence: .sisyphus/evidence/task-5-state-split.txt

  Scenario: hot-path guardrails remain intact
    Tool: Bash
    Steps: cargo test -p chainbot --test runtime_guardrails
    Expected: read-query and observation guardrails still pass after state boundary split
    Evidence: .sisyphus/evidence/task-5-state-split-error.txt
  ```

  **Commit**: YES | Message: `refactor(chainbot): split state contracts from backends` | Files: `crates/chainbot/src/domain/state/**`, `crates/chainbot/src/infrastructure/state/**`, moved tests/callers

- [x] 6. Split executor into domain runtime contracts and app runtime execution

  **What to do**: Move `NodeDefinition`, `NormalizedRunRequest`, `ScheduledNodeState`, `WorkflowRunReport`, and any other backend-agnostic execution contract types out of `executor.rs` into `domain::runtime`. Move concrete scheduling orchestration, builtin dispatch, plugin host calls, and secret-aware runtime setup into `app::runtime::execution`. Update call sites so workflow depends on domain runtime contracts only, not app runtime implementation.
  **Must NOT do**: Do not keep builtin dispatch or plugin host logic in `domain::runtime`. Do not preserve the old `chainbot::executor` root path.

  **Recommended Agent Profile**:
  - Category: `ultrabrain` — Reason: this task must break the workflow/executor coupling without creating new fake layers
  - Skills: [`rust-router`] — Reason: cycle-breaking Rust refactor
  - Omitted: [`m04-zero-cost`] — Reason: generic abstraction design is not the target; dependency direction is

  **Parallelization**: Can Parallel: NO | Wave 2 | Blocks: [7,8,9,10,11,12] | Blocked By: [1,2,5]

  **References**:
  - Current executor contracts + orchestration mix: `crates/chainbot/src/executor.rs:16-30`, `crates/chainbot/src/executor.rs:35-104`, `crates/chainbot/src/executor.rs:171-219`
  - Workflow dependency on executor contract: `crates/chainbot/src/workflow.rs:22`, `crates/chainbot/src/workflow.rs:713-833`
  - Scheduler tests: `crates/chainbot/tests/execution_scheduler.rs:13-22`, `crates/chainbot/tests/workflow_dag_semantics.rs:13-20`

  **Acceptance Criteria**:
  - [ ] `domain::runtime` owns execution contract types only.
  - [ ] App runtime execution code owns builtin/plugin/secret orchestration.
  - [ ] The direct workflow → old executor root dependency is eliminated.

  **QA Scenarios**:
  ```
  Scenario: scheduler semantics remain stable after executor split
    Tool: Bash
    Steps: cargo test -p chainbot --test execution_scheduler
    Expected: wave planning, depends_mode, when evaluation, builtin/plugin dispatch, and runtime namespace behavior remain unchanged
    Evidence: .sisyphus/evidence/task-6-executor-split.txt

  Scenario: workflow DAG contract still validates against new runtime contracts
    Tool: Bash
    Steps: cargo test -p chainbot --test workflow_dag_semantics
    Expected: DAG cycle detection, variable namespace semantics, and subflow boundary checks still pass
    Evidence: .sisyphus/evidence/task-6-executor-split-error.txt
  ```

  **Commit**: YES | Message: `refactor(chainbot): separate runtime contracts from executor orchestration` | Files: `crates/chainbot/src/domain/runtime/**`, `crates/chainbot/src/app/runtime/execution/**`, callers/tests

- [x] 7. Reorganize workflow into `domain::workflow` without rebuilding healthy facades

  **What to do**: Move `workflow.rs` into `domain::workflow` and split it into contract-focused submodules such as `contract`, `variables`, `when`, and `subflow` only after Task 6 has removed the old executor-cycle pressure. Keep workflow semantics pure: no builtin/plugin/secret/runtime-host logic. Update all workflow consumers to depend on `domain::workflow`.
  **Must NOT do**: Do not move `builtins/` under workflow. Do not let workflow regain a dependency on app runtime implementation or infrastructure state backends.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: workflow semantics are stable but subtle; split must preserve contract behavior exactly
  - Skills: [`rust-router`] — Reason: contract-preserving semantic module split
  - Omitted: [`m09-domain`] — Reason: no new business model is being invented

  **Parallelization**: Can Parallel: YES | Wave 2 | Blocks: [9,10,11,12] | Blocked By: [3,6]

  **References**:
  - Workflow semantic hotspot: `crates/chainbot/src/workflow.rs:25-219`, `crates/chainbot/src/workflow.rs:222-421`, `crates/chainbot/src/workflow.rs:713-833`
  - Config/workflow loader consumers: `crates/chainbot/src/config.rs:503-520`, `crates/chainbot/tests/config_loading.rs:20`, `crates/chainbot/tests/workflow_dag_semantics.rs:14-15`

  **Acceptance Criteria**:
  - [ ] Workflow contracts, namespace logic, `when` evaluation, and subflow boundaries live entirely under `domain::workflow`.
  - [ ] Workflow code no longer imports any app runtime implementation module.
  - [ ] Workflow integration tests pass with the new module layout.

  **QA Scenarios**:
  ```
  Scenario: workflow contract and DAG semantics remain stable
    Tool: Bash
    Steps: cargo test -p chainbot --test workflow_dag_semantics
    Expected: cycle detection, variable namespaces, `when` semantics, and subflow import/export behavior remain unchanged
    Evidence: .sisyphus/evidence/task-7-workflow-domain.txt

  Scenario: workflow loading still composes correctly with config/app split
    Tool: Bash
    Steps: cargo test -p chainbot --test config_loading
    Expected: workflow package decoding and bundle validation continue to work with `domain::workflow`
    Evidence: .sisyphus/evidence/task-7-workflow-domain-error.txt
  ```

  **Commit**: YES | Message: `refactor(chainbot): move workflow semantics into domain layer` | Files: `crates/chainbot/src/domain/workflow/**`, callers/tests

- [x] 8. Reorganize trigger acceptance into `domain::trigger` and move runtime supervision into `app::runtime`

  **What to do**: Move trigger contracts, event normalization, accepted-event suppression, dedup/cooldown, and `TriggerPlane` acceptance behavior into `domain::trigger`. Move `external_trigger_supervisor.rs` and `trigger_wasm.rs` under `app::runtime::external_triggers` (or equivalent) because they are long-lived runtime/session orchestration, not domain contracts. Keep `ingress/` in place, updating only imports needed to point at the new domain/app modules.
  **Must NOT do**: Do not move `ingress/` into the new layered tree. Do not keep supervisor/session runtime inside the trigger domain. Do not alter trigger manifest semantics.

  **Recommended Agent Profile**:
  - Category: `ultrabrain` — Reason: trigger acceptance vs runtime supervision is the second major cycle boundary after executor
  - Skills: [`rust-router`] — Reason: multi-module orchestration split with strong persistence coupling
  - Omitted: [`domain-web`] — Reason: ingress transport semantics are intentionally preserved

  **Parallelization**: Can Parallel: NO | Wave 2 | Blocks: [9,10,11,12] | Blocked By: [5,6,7]

  **References**:
  - Trigger acceptance hotspot: `crates/chainbot/src/trigger.rs:26-35`, `crates/chainbot/src/trigger.rs:49-190`
  - Supervisor seam: `crates/chainbot/src/external_trigger_supervisor.rs:1-140`
  - Ingress facade to preserve: `crates/chainbot/src/ingress/mod.rs:10-23`
  - Trigger tests: `crates/chainbot/tests/trigger_plane.rs:15-25`, `crates/chainbot/tests/runtime_state_parity.rs:17-27`, `crates/chainbot/tests/ingress_runtime.rs`

  **Acceptance Criteria**:
  - [ ] Trigger contracts and acceptance logic live under `domain::trigger`.
  - [ ] External trigger supervision and wasm session hosting live under app runtime, not domain.
  - [ ] Ingress remains a preserved facade subtree with only import/path rewiring.

  **QA Scenarios**:
  ```
  Scenario: trigger-plane semantics remain stable after trigger/runtime split
    Tool: Bash
    Steps: cargo test -p chainbot --test trigger_plane
    Expected: builtin/external normalization, dedup/cooldown, workflow binding, and persistence behavior remain unchanged
    Evidence: .sisyphus/evidence/task-8-trigger-split.txt

  Scenario: runtime supervision and ingress integration still work
    Tool: Bash
    Steps: cargo test -p chainbot --test runtime_state_parity && cargo test -p chainbot --test ingress_runtime
    Expected: supervisor/session lifecycle, state parity, and ingress-driven acceptance flows remain stable
    Evidence: .sisyphus/evidence/task-8-trigger-split-error.txt
  ```

  **Commit**: YES | Message: `refactor(chainbot): split trigger domain from runtime supervision` | Files: `crates/chainbot/src/domain/trigger/**`, `crates/chainbot/src/app/runtime/external_triggers/**`, import updates/tests

- [x] 9. Rewrite first-party consumers to the final module tree and remove retired imports in code/tests/examples

  **What to do**: Update all first-party imports across `crates/chainbot/tests`, any workspace members, `examples/`, and code comments/snippets so they reference the final module tree. This task is responsible for the broad import cut: tests such as `config_loading`, `execution_scheduler`, `trigger_plane`, `runtime_state_parity`, `worker_host`, and `cli_surface` must all switch off the retired root modules. Update any fixture helper code or module comments that still mention the old path map.
  **Must NOT do**: Do not leave mixed old/new module imports in first-party code. Do not postpone example/doc snippet updates after tests compile.

  **Recommended Agent Profile**:
  - Category: `unspecified-high` — Reason: large-scale but explicit consumer migration across code and examples
  - Skills: [`rust-router`] — Reason: import-path and module-surface audit
  - Omitted: [`writing`] — Reason: prose-heavy doc rewriting is concentrated in Task 11

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: [10,11,12] | Blocked By: [4,5,6,7,8]

  **References**:
  - Import blast radius sample: `crates/chainbot/tests/config_loading.rs:15-20`, `crates/chainbot/tests/execution_scheduler.rs:13-22`, `crates/chainbot/tests/trigger_plane.rs:15-25`, `crates/chainbot/tests/runtime_state_parity.rs:17-27`, `crates/chainbot/tests/worker_host.rs:17-21`, `crates/chainbot/tests/cli_surface.rs:18-20`
  - Entrypoint + binary behavior: `crates/chainbot/src/main.rs:10-23`, `crates/chainbot/tests/end_to_end_vertical_slice.rs`

  **Acceptance Criteria**:
  - [ ] No first-party source imports any retired root module path.
  - [ ] All updated tests compile against the final module tree.
  - [ ] Examples and first-party snippets reflect the final module names.

  **QA Scenarios**:
  ```
  Scenario: first-party import migration is complete
    Tool: Bash
    Steps: rg "chainbot::(cli|config|executor|external_trigger_supervisor|state(_db)?|trigger(_wasm)?|workflow)" crates/chainbot examples docs README.md AGENTS.md
    Expected: no first-party import or snippet still references retired root modules
    Evidence: .sisyphus/evidence/task-9-consumer-migration.txt

  Scenario: representative integration suites compile against new paths
    Tool: Bash
    Steps: cargo test -p chainbot --test config_loading && cargo test -p chainbot --test execution_scheduler && cargo test -p chainbot --test trigger_plane
    Expected: representative import-heavy tests pass against the final module tree
    Evidence: .sisyphus/evidence/task-9-consumer-migration-error.txt
  ```

  **Commit**: YES | Message: `refactor(chainbot): migrate first-party consumers to new module tree` | Files: `crates/chainbot/tests/**`, `examples/**`, any in-repo importers/snippets

- [x] 10. Update AGENTS, implementation docs, and required inline ASCII diagrams

  **What to do**: Update `crates/chainbot/AGENTS.md`, `crates/chainbot/src/AGENTS.md`, and any touched subtree `AGENTS.md` files to match new ownership boundaries. Update relevant implementation docs so they describe the new module tree and retired root modules. Add or update inline ASCII diagrams in the most structurally complex files introduced by this plan: `app::composition`, `domain::state`, `app::runtime::execution`, and `domain::trigger` acceptance/runtime boundary comments.
  **Must NOT do**: Do not leave AGENTS or implementation docs describing removed root modules. Do not add stale diagrams that no longer match code responsibility.

  **Recommended Agent Profile**:
  - Category: `writing` — Reason: ownership docs and plan-aligned implementation notes must be updated precisely
  - Skills: [`fractal-repo`] — Reason: AGENTS/doc topology discipline
  - Omitted: [`document-release`] — Reason: this is architecture ownership sync, not post-ship release prose

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: [11,12] | Blocked By: [8,9]

  **References**:
  - Current crate ownership docs: `crates/chainbot/AGENTS.md`, `crates/chainbot/src/AGENTS.md`
  - Healthy subtree ownership docs to preserve: `crates/chainbot/src/builtins/AGENTS.md`, `crates/chainbot/src/plugin/AGENTS.md`, `crates/chainbot/src/ingress/AGENTS.md`
  - Diagram-worthy hotspots: `crates/chainbot/src/external_trigger_supervisor.rs:1-140`, `crates/chainbot/src/config.rs:494-631`, `crates/chainbot/src/executor.rs:171-219`, `crates/chainbot/src/trigger.rs:49-190`

  **Acceptance Criteria**:
  - [ ] AGENTS files and implementation docs describe the new ownership map and module tree accurately.
  - [ ] Required inline ASCII diagrams are added or refreshed in structurally complex modules.
  - [ ] No doc still describes retired root modules as active source of truth.

  **QA Scenarios**:
  ```
  Scenario: ownership docs reflect the new module tree
    Tool: Bash
    Steps: cargo test -p chainbot --test contract_versions
    Expected: doc-only updates do not disturb contract tests, and AGENTS/implementation docs align with current source layout on inspection
    Evidence: .sisyphus/evidence/task-10-docs-agents.txt

  Scenario: stale root-module references are removed from docs
    Tool: Bash
    Steps: rg "(pub mod cli|pub mod config|pub mod executor|pub mod state_db|pub mod workflow|chainbot::cli|chainbot::config|chainbot::workflow)" crates/chainbot/AGENTS.md crates/chainbot/src/AGENTS.md docs README.md examples -g '!target/**'
    Expected: no stale documentation references removed root modules as active public surface
    Evidence: .sisyphus/evidence/task-10-docs-agents-error.txt
  ```

  **Commit**: YES | Message: `docs(chainbot): sync ownership docs with new module tree` | Files: `crates/chainbot/**/AGENTS.md`, relevant `docs/**`, inline ASCII comments in moved code

- [x] 11. Remove retired root modules and enforce the final dependency direction

  **What to do**: Delete the old root modules/files once their replacements are live and all first-party consumers have moved. Ensure the final dependency direction is enforced: `domain` cannot import app/infrastructure implementations; `infrastructure` can depend on `domain` contracts; `app` can depend on both. Remove any temporary alias modules or duplicate definitions that survived earlier commits.
  **Must NOT do**: Do not leave shadow copies of old modules. Do not keep duplicate type definitions to “ease migration”.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: this is the irreversible cleanup point where structural duplication must disappear cleanly
  - Skills: [`rust-router`] — Reason: final dependency and visibility cleanup
  - Omitted: [`rust-refactor-helper`] — Reason: plan already fixes exact ownership/direction; this is not an exploratory rename

  **Parallelization**: Can Parallel: NO | Wave 3 | Blocks: [12] | Blocked By: [9,10]

  **References**:
  - Retired root modules baseline: `crates/chainbot/src/lib.rs:11-26`
  - Domain/app/infrastructure decisions from Tasks 3, 5, 6, 7, 8
  - Test blast radius confirming old roots must be gone: `crates/chainbot/tests/config_loading.rs:15-20`, `crates/chainbot/tests/runtime_state_parity.rs:17-27`, `crates/chainbot/tests/trigger_plane.rs:15-25`

  **Acceptance Criteria**:
  - [ ] Retired root modules/files are removed from the source tree.
  - [ ] No duplicate type definitions remain to bridge old/new layouts.
  - [ ] Domain/app/infrastructure dependency direction matches the agreed architecture.

  **QA Scenarios**:
  ```
  Scenario: final tree builds without retired modules
    Tool: Bash
    Steps: cargo check -p chainbot && cargo test -p chainbot --lib
    Expected: the crate builds and library/unit tests pass with only the final module tree present
    Evidence: .sisyphus/evidence/task-11-retire-old-roots.txt

  Scenario: no duplicate or retired root symbols survive
    Tool: Bash
    Steps: rg "^(pub mod (cli|config|executor|external_trigger_supervisor|state(_db)?|trigger(_wasm)?|workflow));$" crates/chainbot/src
    Expected: no retired root-module declarations survive after cleanup
    Evidence: .sisyphus/evidence/task-11-retire-old-roots-error.txt
  ```

  **Commit**: YES | Message: `refactor(chainbot): remove retired root modules` | Files: removed legacy root files/modules, final dependency cleanup, tests if needed

- [x] 12. Run the full regression matrix and produce the final consumer audit

  **What to do**: Run the complete `chainbot` regression suite named in Definition of Done, capture evidence, and perform a final first-party consumer audit across code, tests, docs, and examples. This task is also responsible for confirming that no behavior changed: CLI UX, config semantics, workflow/executor semantics, trigger acceptance, state persistence, plugin/worker contracts, ingress flows, and vertical-slice behavior must all remain green.
  **Must NOT do**: Do not stop at `cargo check` or a partial subset. Do not claim success if any retired-path reference or stale ownership doc remains.

  **Recommended Agent Profile**:
  - Category: `unspecified-high` — Reason: broad deterministic verification sweep across the whole crate
  - Skills: [`review`] — Reason: pre-landing regression discipline
  - Omitted: [`qa`] — Reason: this is Rust CLI/runtime regression, not browser/UI testing

  **Parallelization**: Can Parallel: NO | Wave 3 | Blocks: [Final Verification] | Blocked By: [11]

  **References**:
  - Regression matrix in `Definition of Done`
  - Import audit commands from Tasks 1 and 9
  - High-risk suites: `crates/chainbot/tests/cli_surface.rs`, `config_loading.rs`, `execution_scheduler.rs`, `trigger_plane.rs`, `runtime_state_parity.rs`, `ingress_runtime.rs`, `end_to_end_vertical_slice.rs`

  **Acceptance Criteria**:
  - [ ] Every command in Definition of Done exits with code `0`.
  - [ ] Final import audit shows no retired root module usage in first-party sources.
  - [ ] Evidence files exist for the complete regression run and consumer audit.

  **QA Scenarios**:
  ```
  Scenario: full chainbot regression matrix passes
    Tool: Bash
    Steps: cargo test -p chainbot --lib && cargo test -p chainbot --test contract_versions && cargo test -p chainbot --test config_loading && cargo test -p chainbot --test workflow_dag_semantics && cargo test -p chainbot --test execution_scheduler && cargo test -p chainbot --test trigger_plane && cargo test -p chainbot --test runtime_state_parity && cargo test -p chainbot --test runtime_guardrails && cargo test -p chainbot --test state_runtime_persistence && cargo test -p chainbot --test secrets_runtime && cargo test -p chainbot --test worker_host && cargo test -p chainbot --test node_plugin_host && cargo test -p chainbot --test cli_surface && cargo test -p chainbot --test catalog_surface && cargo test -p chainbot --test ingress_runtime && cargo test -p chainbot --test end_to_end_vertical_slice
    Expected: all suites pass with exit code 0 under the final module tree
    Evidence: .sisyphus/evidence/task-12-full-regression.txt

  Scenario: final consumer audit is clean
    Tool: Bash
    Steps: rg "chainbot::(cli|config|executor|external_trigger_supervisor|state(_db)?|trigger(_wasm)?|workflow)" crates/chainbot examples docs README.md AGENTS.md
    Expected: no first-party code, docs, or examples refer to retired root modules
    Evidence: .sisyphus/evidence/task-12-full-regression-error.txt
  ```

  **Commit**: NO | Message: `n/a` | Files: verification only; no code changes expected

## Final Verification Wave (MANDATORY — after ALL implementation tasks)
> 4 review agents run in PARALLEL. ALL must APPROVE. Present consolidated results to user and get explicit "okay" before completing.
> **Do NOT auto-proceed after verification. Wait for user's explicit approval before marking work complete.**
> **Never mark F1-F4 as checked before getting user's okay.** Rejection or user feedback -> fix -> re-run -> present again -> wait for okay.
- [x] F1. Plan Compliance Audit — oracle
- [x] F2. Code Quality Review — unspecified-high
- [x] F3. Real Manual QA — unspecified-high
- [x] F4. Scope Fidelity Check — deep

  **Acceptance Criteria**:
  - [ ] All four final-review agents return explicit approval or a zero-critical-findings result that can be presented to the user.
  - [ ] Any review rejection triggers a fix cycle followed by a full re-run of F1-F4.
  - [ ] Work is not considered complete until the user explicitly says the consolidated verification result is acceptable.

  **QA Scenarios**:
  ```
  Scenario: final four-review sweep completes
    Tool: Bash + Agent
    Steps: (1) run the plan compliance audit against .sisyphus/plans/chainbot-in-crate-reorganization.md, (2) run code-quality review over the final diff, (3) run full regression/manual QA using the Task 12 matrix, (4) run a scope-fidelity review comparing final changes against this plan, (5) collect all four outputs into one summary for the user
    Expected: all four reviews approve, or any non-approval is fixed and the full four-review sweep is repeated before asking the user for final okay
    Evidence: .sisyphus/evidence/final-verification-wave.txt

  Scenario: rejection handling preserves final gate discipline
    Tool: Bash + Agent
    Steps: if any one of F1-F4 reports a gap, apply the fix, re-run Task 12 regression matrix, then re-run all four final-review agents and update the consolidated summary
    Expected: no single review is treated as advisory-only; every rejection causes a fix-and-rerun cycle before completion can be proposed
    Evidence: .sisyphus/evidence/final-verification-wave-error.txt
  ```

## Commit Strategy
- 采用 12 个原子提交；每个提交只覆盖一个结构主题，并在同一提交中完成对应代码、测试、示例、文档、AGENTS 更新。
- 禁止 mega-commit；禁止“先搬完代码再统一修测试/文档”。
- 允许在单任务内同时改行为无关的 import/path 以保持编译通过，但禁止顺手做语义 cleanup。

## Success Criteria
- 新公开模块树成为唯一 first-party 使用路径。
- `builtins/`、`plugin/`、`ingress/` 保持健康 facade 结构，无额外无收益搬迁。
- `state`、`executor`、`config` 的职责边界显式可读，不再混合模型/后端、contract/orchestration、loader/composition。
- 所有现有集成测试与端到端测试在新路径下通过。
- AGENTS、implementation doc、examples 与源码职责地图保持一致。
