# ChainBot V2 Runnable MVP

## TL;DR
> **Summary**: 为 ChainBot V2 生成一个可执行的本地优先 MVP 实施计划：单一 `chainbot` 二进制承载 CLI 与 `serve` 模式，TOML 定义放在 `~/.chainbot/`，运行态放在 `~/.chainbot/state/`，同时支持外部 `trigger`/`node` 插件、子进程 Python/JavaScript 脚本节点、以及 `pass`/PGP 风格机密解析。
> **Deliverables**:
> - 可运行的本地垂直切片：配置加载 -> DAG 校验 -> `serve` -> trigger/plugin -> workflow 执行 -> run 持久化
> - 稳定的版本化契约：config/workflow/plugin manifest/worker protocol/secret reference/state schema
> - 基础内建节点、外部插件宿主、脚本 worker 宿主、机密解析器、最小 SQLite 协调存储、文件化运行日志/触发记录/运行摘要
> - 核心函数边界单元测试与集成验收测试
> **Effort**: XL
> **Parallel**: YES - 6 waves
> **Critical Path**: Task 1 -> Task 2 -> Task 3 -> Task 7 -> Task 11 -> Task 12 -> Final Verification

## Context
### Original Request
- 用户要做 `chainbot v2` 的整体重构，目标是 `cli first`、可扩展 trigger、可组合 DAG workflow 执行器。
- 配置采用 TOML 与纯文本目录布局，默认根目录 `~/.chainbot/`。
- Trigger 要支持内建常用触发器与 WASM 插件扩展，可参考 V1 `chain-bot-monitor`。
- Workflow 要支持运行时变量、并行 DAG、子工作流、条件控制、Python/JavaScript 脚本、内建节点与扩展节点，可参考 V1 `chain-bot-executor`。

### Interview Summary
- 首版不是空架构，而是“可运行最小闭环”。
- 外部插件首版同时覆盖 `trigger` 与 `node`。
- Python/JavaScript 运行时采用 subprocess worker + JSON contract。
- 机密方案不使用 KeePass-NG，改为 `pass` 风格：每个 secret 一个 PGP 加密文件，目录结构表示层级。
- 测试策略为：核心函数边界做单元测试，集成测试承担验收职责。

### Metis Review (gaps addressed)
- 收敛 `serve`：MVP 不默认暴露 HTTP/control plane；若无明确控制面需求，则 `serve` 仅负责本地监听 trigger、执行 workflow、维护状态。
- 收敛状态层：TOML 只存用户定义；SQLite 缩到最小，只承载 lease 与去重/冷却协调；run summary、workflow log、trigger record 改为文件存储。
- 收敛扩展边界：所有插件与脚本 worker 必须走版本化协议，定义超时、输出大小限制、错误映射与 child cleanup 行为。
- 收敛机密边界：配置里只保存 secret reference，不保存 secret value；日志、run output、SQLite 中都不得落地 secret material。

## Work Objectives
### Core Objective
- 在当前 bootstrap-only workspace 上落地一个本地优先、可运行、可验证的 ChainBot V2 MVP，实现最小但完整的产品闭环，而不是分散的子系统样板。

### Deliverables
- `chainbot` crate 被重构为多模块 CLI/serve 程序，具备 `validate`、`serve`、`run`、`list-runs` 等 MVP 命令边界。
- TOML 配置与 workflow/plugin/trigger manifest 契约被冻结并版本化。
- 最小 SQLite 协调存储只负责 `serve` 单实例 lease 与 trigger dedup/cooldown 这类原子协调状态。
- run summary、workflow 运行日志、trigger 触发记录采用文件存储，和最小 SQLite 协调层分层管理。
- Trigger plane 与 execution plane 明确分层；scheduler 语义保留在 Rust 中。
- 外部 `trigger`/`node` 插件支持发现、校验、加载与能力限制。
- Python/JavaScript 脚本节点通过 subprocess worker 运行，采用固定 JSON request/response envelope。
- `pass`/PGP secret provider 可根据 `secret://` 引用解析机密，并保证 redaction。
- 一条端到端 workflow 集成测试覆盖启动、触发、执行、落盘、查询结果。

### Definition of Done (verifiable conditions with commands)
- `cargo metadata --no-deps` 在 workspace 根目录成功执行。
- `cargo check --workspace` 成功执行。
- `cargo test --workspace` 成功执行，包含单元测试与集成测试。
- `cargo test --workspace -- --nocapture` 可产出 MVP 垂直切片的集成证据。
- `cargo run -p chainbot -- validate --root <test-root>` 能校验示例配置并返回成功。
- `cargo run -p chainbot -- serve --root <test-root>` 在测试环境内可启动并处理单次 trigger。
- `cargo run -p chainbot -- list-runs --root <test-root>` 能读取文件化持久化 run summary。
- 端到端测试执行后，`<test-root>/state/logs/` 或等效文件目录中可见 workflow/trigger 记录文件。

### Must Have
- 单一 `chainbot` 二进制，命令模式与 `serve` 模式同宿主。
- `~/.chainbot/` 作为产品约定路径，不隐藏在黑盒库抽象后。
- TOML 定义与运行态严格分层。
- workflow/trigger 的日志、触发记录、run summary 必须采用文件存储，不进入 SQLite。
- `protocol_version` / `schema_version` / `api_version` 明确存在且由宿主校验。
- 插件与脚本 worker 默认 deny 权限，必须经 manifest/host allowlist 开放能力。
- `pass` 风格 PGP 文件机密解析、缺失/解密失败错误与日志脱敏。

### Must NOT Have (guardrails, AI slop patterns, scope boundaries)
- 不要为 MVP 默认加入 HTTP API、远程控制面或多进程分布式协调。
- 不要让 trigger/plugin/script 直接拥有 DAG 调度权或共享可变 runtime 全局状态。
- 不要把 run record、dedup、cooldown、lease 写回 TOML。
- 不要把 workflow/trigger 运行日志、触发记录、run summary 塞进 SQLite；SQLite 只保留最小协调状态。
- 不要把 secret value、完整 worker env、超大 stdout/stderr、原始敏感日志、plugin cache 正文写入 SQLite。
- 不要为了复用 V1 而保留 V1 配置兼容性；V1 只作为模式来源，不作为强兼容目标。

## Verification Strategy
> ZERO HUMAN INTERVENTION — all verification is agent-executed.
- Test decision: tests-after + Rust built-in unit/integration test framework
- QA policy: 每个任务必须同时包含实现与 agent-executed QA 场景
- Evidence: `.sisyphus/evidence/task-{N}-{slug}.{ext}`
- Before any QA command writes evidence: run `mkdir -p .sisyphus/evidence` and preserve command exit status with `set -o pipefail`.

## Execution Strategy
### Parallel Execution Waves
> Target: 5-8 tasks per wave. <3 per wave (except final) = under-splitting.
> This plan uses dependency-safe waves; within a wave, all tasks may run in parallel because every `Blocked By` prerequisite is already complete.

Wave 1: Task 1 only — freeze contracts and module boundaries
Wave 2: Task 2 only — config layout, root path resolution, definition loading
Wave 3: Tasks 3, 4, 5 — minimal coordination store, DAG semantics, CLI surface
Wave 4: Tasks 6, 7, 8, 9, 10 — trigger plane, scheduler, worker host, node plugin host, secret provider
Wave 5: Task 11 — runnable end-to-end vertical slice
Wave 6: Task 12 — restart and failure-boundary hardening

### Dependency Matrix (full, all tasks)
- Task 1 blocks Tasks 2-12
- Task 2 blocks Tasks 3, 4, 6, 7, 8, 9, 10, 11, 12
- Task 3 blocks Tasks 7, 8, 9, 10, 11, 12
- Task 4 blocks Tasks 11, 12
- Task 5 blocks Tasks 6, 11, 12
- Task 6 blocks Tasks 11, 12
- Task 7 blocks Tasks 11, 12
- Task 8 blocks Tasks 11, 12
- Task 9 blocks Tasks 11, 12
- Task 10 blocks Tasks 11, 12
- Task 11 blocks Task 12 and Final Verification
- Task 12 blocks Final Verification

### Agent Dispatch Summary (wave → task count → categories)
- Wave 1 → 1 task → `deep`
- Wave 2 → 1 task → `deep`
- Wave 3 → 3 tasks → `deep`, `ultrabrain`, `unspecified-high`
- Wave 4 → 5 tasks → `deep`, `ultrabrain`
- Wave 5 → 1 task → `deep`
- Wave 6 → 1 task → `unspecified-high`

## TODOs
> Implementation + Test = ONE task. Never separate.
> EVERY task MUST have: Agent Profile + Parallelization + QA Scenarios.

- [x] 1. Freeze V2 module boundaries and versioned contracts

  **What to do**: Restructure `crates/chainbot/src/` into explicit modules for `cli`, `config`, `workflow`, `state`, `trigger`, `executor`, `plugin`, `worker`, `secrets`, and `errors`. Define versioned Rust types for config root, workflow definition, trigger definition, node definition, plugin manifest, worker request/response envelope, run record summary, and secret reference syntax. Make `schema_version` / `api_version` / `protocol_version` mandatory in the relevant contracts and reject unknown future major versions.
  **Must NOT do**: 不要在这一任务引入真正的执行逻辑；不要开始实现插件加载、PGP 解密、或 SQLite 持久化。

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: 这是后续所有任务的契约基线，错误会放大到全系统。
  - Skills: [`fractal-context`] — 约束模块边界和文件职责；[`m05-type-driven`] — 冻结版本化契约类型。
  - Omitted: [`m10-performance`] — 首要目标是正确性与边界，而不是性能优化。

  **Parallelization**: Can Parallel: NO | Wave 1 | Blocks: 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12 | Blocked By: none

  **References** (executor has NO interview context — be exhaustive):
  - Pattern: `docs/design/CHAINBOT_WORKSPACE_DESIGN.md:3` — 当前仓库的 workspace 形状与边界约束。
  - Pattern: `docs/implementation/WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md:11` — 当前仓库默认验证矩阵。
  - Pattern: `crates/chainbot/src/main.rs:1` — 当前入口仍是占位，需要重构为模块化入口。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/core/data_types.rs` — V1 的 workflow/node/trigger 核心类型来源，V2 需简化并重新版本化。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/nodes/contract.rs` — V1 的节点契约与回退策略经验。

  **Acceptance Criteria** (agent-executable only):
  - [ ] `cargo check --workspace` succeeds after module extraction and contract type introduction.
  - [ ] `cargo test --workspace contract_versions` succeeds for version parsing/rejection tests.
  - [ ] Invalid future-major config/plugin/worker envelope fixtures are rejected by unit tests.

  **QA Scenarios** (MANDATORY — task incomplete without these):
  ```text
  Scenario: Contract fixtures validate and reject unknown majors
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace contract_versions -- --nocapture | tee .sisyphus/evidence/task-1-contracts.txt`
    Expected: tests include both success fixtures and explicit rejection of unsupported future major versions
    Evidence: .sisyphus/evidence/task-1-contracts.txt

  Scenario: Broken contract fixture fails validation
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace invalid_contract_fixtures_are_rejected -- --nocapture | tee .sisyphus/evidence/task-1-contracts-error.txt`
    Expected: malformed or version-mismatched fixtures are rejected without panic
    Evidence: .sisyphus/evidence/task-1-contracts-error.txt
  ```

  **Commit**: YES | Message: `feat(contract): freeze v2 core schemas` | Files: `crates/chainbot/src/**`, `crates/chainbot/Cargo.toml`

- [x] 2. Implement TOML root layout, path resolution, and definition loaders

  **What to do**: Implement explicit root path resolution for `~/.chainbot/` and test roots, then parse and validate TOML definitions for root config, workflows, triggers, and plugin manifests. Encode default filesystem layout for `config/`, `workflows/`, `triggers/`, `plugins/`, `secrets/`, and `state/`. Add secret reference syntax (for example `secret://ops/slack/webhook`) as data only; resolution happens later.
  **Must NOT do**: 不要在这一任务读 secret 文件内容；不要把 mutable state 放回 TOML；不要隐藏路径逻辑在第三方 convenience crate 中。

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: 路径与定义加载是 CLI、serve、tests、state 的公共基座。
  - Skills: [`fractal-context`] — 保持配置目录与模块职责一致；[`m06-error-handling`] — 明确验证错误与用户提示。
  - Omitted: [`domain-web`] — 此任务不涉及网络服务。

  **Parallelization**: Can Parallel: YES | Wave 2 | Blocks: 3, 4, 5, 6, 7, 8, 9, 10, 11, 12 | Blocked By: 1

  **References**:
  - Pattern: `docs/design/CHAINBOT_WORKSPACE_DESIGN.md:15` — 根目录与扩展布局的现有设计意图。
  - Pattern: `docs/implementation/WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md:17` — 当前仓库强调最小结构变化和明确验证。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-monitor/src/config.rs` — V1 配置组织经验，可参考但不要照搬 DB/monitor 专用字段。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/config.rs` — V1 executor 配置入口。

  **Acceptance Criteria**:
  - [ ] `cargo test --workspace config_root_layout` succeeds for default root and overridden test root.
  - [ ] `cargo test --workspace toml_definition_validation` succeeds for valid fixtures and invalid path/version cases.
  - [ ] `cargo run -p chainbot -- validate --root target/test-roots/basic` returns success for a valid fixture set.

  **QA Scenarios**:
  ```text
  Scenario: Validate fixture tree under explicit root
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo run -p chainbot -- validate --root target/test-roots/basic | tee .sisyphus/evidence/task-2-validate.txt`
    Expected: command exits 0 and reports all workflow/trigger/plugin TOML files valid
    Evidence: .sisyphus/evidence/task-2-validate.txt

  Scenario: Invalid TOML fixture is rejected
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace invalid_toml_fixture_rejected -- --nocapture | tee .sisyphus/evidence/task-2-validate-error.txt`
    Expected: test fails the bad fixture with a structured validation error and no panic
    Evidence: .sisyphus/evidence/task-2-validate-error.txt
  ```

  **Commit**: YES | Message: `feat(config): add v2 toml loaders and root layout` | Files: `crates/chainbot/src/config/**`, `crates/chainbot/src/cli/**`, `crates/chainbot/tests/**`

- [x] 3. Implement minimal SQLite coordination, file-backed run summaries, and runtime logs

  **What to do**: Add a minimal local coordination store under `~/.chainbot/state/`, using SQLite only for single-owner `serve` lease and trigger dedup/cooldown tokens that require atomic updates. Store run summaries, workflow runtime logs, and trigger event records as files under the state tree. Implement migrations only for the minimal SQLite schema, define file naming/layout rules for run summaries and logs, and specify crash-safe transitions and recovery expectations so file-backed run state stays consistent with the coordination store.
  **Must NOT do**: 不要在 SQLite 中保存 run summary 正文、secret material、完整 worker env、超大原始日志；不要引入远程数据库依赖；不要把文件布局规则做成隐式不可测试行为。

  **Recommended Agent Profile**:
  - Category: `ultrabrain` — Reason: 混合文件状态 + 最小协调数据库的边界更容易出一致性问题。
  - Skills: [`m12-lifecycle`] — 资源、lease、文件写入时序设计；[`m13-domain-error`] — 区分文件层与协调层错误。
  - Omitted: [`domain-web`] — 不需要网络数据库。

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: 6, 7, 11, 12 | Blocked By: 1, 2

  **References**:
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/core/run_record.rs` — V1 run lifecycle store 模式来源，V2 需改成文件化 run summary。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-monitor/src/trigger/dedup.rs` — dedup 存储抽象参考。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-monitor/src/trigger/cooldown.rs` — cooldown 存储抽象参考。
  - Pattern: `docs/implementation/WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md:19` — 当前仓库已有“root-level durable state expectations”的最小约束。

  **Acceptance Criteria**:
  - [ ] `cargo test --workspace sqlite_coordination_migrations` succeeds.
  - [ ] `cargo test --workspace file_backed_runtime_logs` succeeds.
  - [ ] `cargo test --workspace serve_single_owner_lease` succeeds, including reject-second-owner behavior.
  - [ ] `cargo test --workspace file_backed_run_summary_recovery` succeeds for crash-before-commit and restart recovery fixtures.

  **QA Scenarios**:
  ```text
  Scenario: Serve acquires and releases single-owner lease
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace serve_single_owner_lease -- --nocapture | tee .sisyphus/evidence/task-3-lease.txt`
    Expected: first owner acquires lease, second owner is rejected, lease is released on shutdown
    Evidence: .sisyphus/evidence/task-3-lease.txt

  Scenario: Runtime logs and trigger records are written to files
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace file_backed_runtime_logs -- --nocapture | tee .sisyphus/evidence/task-3-logs.txt`
    Expected: workflow logs, trigger records, and run summaries are written under the configured file tree and linked by run identifier
    Evidence: .sisyphus/evidence/task-3-logs.txt

  Scenario: Crash recovery preserves consistent run state
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace file_backed_run_summary_recovery -- --nocapture | tee .sisyphus/evidence/task-3-lease-error.txt`
    Expected: interrupted runs are recovered or marked consistently in file-backed summaries without duplicate success records
    Evidence: .sisyphus/evidence/task-3-lease-error.txt
  ```

  **Commit**: YES | Message: `feat(state): add minimal sqlite coordination and file runtime state` | Files: `crates/chainbot/src/state/**`, `crates/chainbot/tests/**`, migration files under crate resources

- [x] 4. Build DAG model, validation, runtime variable namespaces, and subflow contracts

  **What to do**: Implement workflow graph parsing and validation, including cycle detection, `depends_on`, `depends_mode`, conditional `when`, subflow input/output boundaries, runtime variable namespaces, and deterministic precedence (`CLI args > manual invocation input > trigger payload mapping > workflow defaults > config defaults`). Use a graph library only for DAG validation/traversal assistance; scheduler semantics stay custom in Rust.
  **Must NOT do**: 不要在这一任务执行节点；不要让 subflow 共享可变全局状态；不要把条件表达式设计成宿主难以约束的任意调度脚本。

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: 这决定 workflow 语义是否可预测、可测试、可扩展。
  - Skills: [`m09-domain`] — 建模 workflow/subflow/value precedence；[`m05-type-driven`] — 用类型限制非法状态。
  - Omitted: [`m10-performance`] — 先冻结正确语义。

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: 7, 8, 9, 11, 12 | Blocked By: 1, 2

  **References**:
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/dag/mod.rs` — V1 `Dag` trait 的最小抽象。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/dag/petgraph_impl.rs` — V1 使用 graph 库的经验来源。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/control_flow/if_scope.rs` — 条件分支语义参考。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/control_flow/nested_workflow.rs` — 子工作流边界参考。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/control_flow/foreach_scope.rs` — 循环与局部 scope 经验。

  **Acceptance Criteria**:
  - [ ] `cargo test --workspace dag_cycle_validation` succeeds.
  - [ ] `cargo test --workspace runtime_variable_precedence` succeeds.
  - [ ] `cargo test --workspace subflow_contract_boundaries` succeeds for input/output isolation.

  **QA Scenarios**:
  ```text
  Scenario: Valid DAG with subflow and conditions passes validation
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace dag_cycle_validation -- --nocapture | tee .sisyphus/evidence/task-4-dag.txt && cargo test --workspace runtime_variable_precedence -- --nocapture | tee -a .sisyphus/evidence/task-4-dag.txt && cargo test --workspace subflow_contract_boundaries -- --nocapture | tee -a .sisyphus/evidence/task-4-dag.txt`
    Expected: valid graphs pass; precedence order is deterministic; subflow boundaries are enforced
    Evidence: .sisyphus/evidence/task-4-dag.txt

  Scenario: Cyclic graph and invalid variable references are rejected
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace invalid_dag_and_variable_fixtures_rejected -- --nocapture | tee .sisyphus/evidence/task-4-dag-error.txt`
    Expected: cycle and bad variable namespace fixtures are rejected with typed errors
    Evidence: .sisyphus/evidence/task-4-dag-error.txt
  ```

  **Commit**: YES | Message: `feat(workflow): add dag validation and runtime variable model` | Files: `crates/chainbot/src/workflow/**`, `crates/chainbot/tests/**`

- [x] 5. Implement CLI surface and user-visible error model

  **What to do**: Replace placeholder `main.rs` with a real CLI entrypoint providing at least `validate`, `serve`, `run`, and `list-runs`. Define structured user-facing errors, exit codes, and root path overrides. `validate` performs definition-only checks, `run` performs single-shot execution without long-lived lease, `serve` acquires lease and loops triggers, `list-runs` queries file-backed persisted run summaries.
  **Must NOT do**: 不要在这一任务做复杂 trigger/executor 逻辑；不要把内部错误直接裸露给用户；不要引入 HTTP 命令面。

  **Recommended Agent Profile**:
  - Category: `unspecified-high` — Reason: 需要把多个内部子系统收敛成清晰的产品边界。
  - Skills: [`domain-cli`] — 设计命令面、退出码与根目录覆盖；[`clarify`] — 错误文案与提示边界。
  - Omitted: [`domain-web`] — 不需要 API server。

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: 11, 12 | Blocked By: 1, 2

  **References**:
  - Pattern: `crates/chainbot/src/main.rs:1` — 当前占位入口，必须彻底替换。
  - Pattern: `docs/design/CHAINBOT_WORKSPACE_DESIGN.md:9` — repo root 仍是纯 workspace，命令面必须落在 crate 内。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/main.rs` — V1 服务入口组织方式，仅作结构参考。

  **Acceptance Criteria**:
  - [ ] `cargo run -p chainbot -- --help` lists all MVP commands.
  - [ ] `cargo run -p chainbot -- validate --root target/test-roots/basic` exits 0 for valid fixtures.
  - [ ] `cargo run -p chainbot -- list-runs --root target/test-roots/basic` exits 0 and prints empty list for a fresh file-backed run summary store.

  **QA Scenarios**:
  ```text
  Scenario: CLI exposes the MVP command surface
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo run -p chainbot -- --help | tee .sisyphus/evidence/task-5-cli.txt`
    Expected: output includes `validate`, `serve`, `run`, and `list-runs`
    Evidence: .sisyphus/evidence/task-5-cli.txt

  Scenario: Invalid root path returns structured error
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo run -p chainbot -- validate --root target/test-roots/missing 2>&1 | tee .sisyphus/evidence/task-5-cli-error.txt`
    Expected: command exits non-zero and prints a stable, user-facing error without panic backtrace
    Evidence: .sisyphus/evidence/task-5-cli-error.txt
  ```

  **Commit**: YES | Message: `feat(cli): add validate serve run and list-runs commands` | Files: `crates/chainbot/src/main.rs`, `crates/chainbot/src/cli/**`, `crates/chainbot/tests/**`

- [x] 6. Implement trigger plane with builtin triggers and external trigger plugin host

  **What to do**: Build the trigger plane that loads enabled builtin triggers and external trigger plugins, normalizes emitted events, applies dedup/cooldown logic against the minimal SQLite coordination store, writes file-backed trigger records, and hands off normalized run requests to the execution plane. Plugin discovery must validate manifest allowlist, capability set, executable path, and supported `api_version`.
  **Must NOT do**: 不要让 trigger plane 直接执行 DAG 节点；不要跳过 manifest 校验；不要信任任意目录下的可执行文件；不要只在内存里保留触发记录而不落文件。

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: trigger plane 是事件归一化、节流、防重与执行入口边界。
  - Skills: [`m13-domain-error`] — 区分 transport/config/runtime 错误；[`m07-concurrency`] — 处理 serve 循环与事件调度。
  - Omitted: [`m10-performance`] — 先锁定正确边界和失败模式。

  **Parallelization**: Can Parallel: YES | Wave 4 | Blocks: 11, 12 | Blocked By: 1, 2, 3, 5

  **References**:
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-monitor/src/trigger/mod.rs` — V1 `TriggerPlaneRuntime` 边界与重载模式。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-monitor/src/trigger/evaluator.rs` — dedup/cooldown + dispatch decision 逻辑。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-monitor/src/trigger/reconcile.rs` — runtime snapshot/reconcile 模式。

  **Acceptance Criteria**:
  - [ ] `cargo test --workspace trigger_plugin_manifest_validation` succeeds.
  - [ ] `cargo test --workspace trigger_dedup_and_cooldown` succeeds.
  - [ ] `cargo test --workspace builtin_and_external_trigger_emit_run_requests` succeeds.
  - [ ] `cargo test --workspace trigger_records_are_file_backed` succeeds.

  **QA Scenarios**:
  ```text
  Scenario: Builtin and external trigger paths both emit normalized run requests
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace builtin_and_external_trigger_emit_run_requests -- --nocapture | tee .sisyphus/evidence/task-6-trigger.txt`
    Expected: both trigger kinds yield normalized run requests and respect manifest validation
    Evidence: .sisyphus/evidence/task-6-trigger.txt

  Scenario: Invalid trigger plugin manifest is rejected before execution
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace trigger_plugin_manifest_validation -- --nocapture | tee .sisyphus/evidence/task-6-trigger-error.txt`
    Expected: unsupported capability, bad executable path, or version mismatch is rejected without spawn
    Evidence: .sisyphus/evidence/task-6-trigger-error.txt

  Scenario: Trigger records persist to files with run linkage
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace trigger_records_are_file_backed -- --nocapture | tee .sisyphus/evidence/task-6-trigger-files.txt`
    Expected: each accepted trigger produces a file-backed record and a stable link to the corresponding run identifier
    Evidence: .sisyphus/evidence/task-6-trigger-files.txt
  ```

  **Commit**: YES | Message: `feat(trigger): add builtin and external trigger host` | Files: `crates/chainbot/src/trigger/**`, `crates/chainbot/tests/**`

- [x] 7. Implement Rust-owned scheduler, execution plane, and builtin node registry

  **What to do**: Implement the execution plane that accepts normalized run requests, resolves workflow definitions, evaluates dependencies/conditions/subflows, and executes builtin nodes under a Rust-owned scheduler. Add a builtin node registry with stable dispatch contracts and ensure scheduler semantics remain separate from plugin/script execution engines.
  **Must NOT do**: 不要把调度逻辑下放到脚本或插件；不要绕过 DAG 验证；不要把 failure handling 写成隐式 side effects。

  **Recommended Agent Profile**:
  - Category: `ultrabrain` — Reason: 这是系统核心调度器，涉及状态机、并发、跳过/失败语义。
  - Skills: [`m07-concurrency`] — 并行调度；[`m05-type-driven`] — 用类型表达节点状态和 dispatch 结果。
  - Omitted: [`domain-web`] — 无网络控制面。

  **Parallelization**: Can Parallel: YES | Wave 4 | Blocks: 11, 12 | Blocked By: 1, 2, 3, 4

  **References**:
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/core/executor.rs` — V1 `WorkflowExecutor` 与 node state 管理经验。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/nodes/registry.rs` — 节点注册与版本选择。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/control_flow/if_scope.rs` — 条件分支执行边界。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/control_flow/nested_workflow.rs` — subflow 调度边界。

  **Acceptance Criteria**:
  - [ ] `cargo test --workspace scheduler_parallel_ready_nodes` succeeds.
  - [ ] `cargo test --workspace scheduler_when_and_depends_mode` succeeds.
  - [ ] `cargo test --workspace builtin_node_registry_dispatch` succeeds.

  **QA Scenarios**:
  ```text
  Scenario: Scheduler runs ready nodes in parallel and respects dependencies
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace scheduler_parallel_ready_nodes -- --nocapture | tee .sisyphus/evidence/task-7-scheduler.txt && cargo test --workspace scheduler_when_and_depends_mode -- --nocapture | tee -a .sisyphus/evidence/task-7-scheduler.txt`
    Expected: independent nodes run concurrently; conditional and any/all dependency modes behave deterministically
    Evidence: .sisyphus/evidence/task-7-scheduler.txt

  Scenario: Unknown builtin node kind is rejected cleanly
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace builtin_node_registry_dispatch -- --nocapture | tee .sisyphus/evidence/task-7-scheduler-error.txt`
    Expected: unsupported builtin node resolution yields typed error instead of panic
    Evidence: .sisyphus/evidence/task-7-scheduler-error.txt
  ```

  **Commit**: YES | Message: `feat(executor): add scheduler and builtin node registry` | Files: `crates/chainbot/src/executor/**`, `crates/chainbot/src/nodes/**`, `crates/chainbot/tests/**`

- [x] 8. Implement subprocess worker host for Python and JavaScript script nodes

  **What to do**: Build a subprocess worker host that launches Python/JavaScript workers with versioned JSON request/response envelopes, strict timeout controls, stdout/stderr size limits, malformed-output handling, and orphan child cleanup. Map script outputs back into node outputs without giving workers scheduler authority.
  **Must NOT do**: 不要嵌入解释器；不要信任无限输出；不要让 worker 拿到未筛选宿主环境。

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: 子进程协议、资源限制与错误边界很容易失控。
  - Skills: [`m06-error-handling`] — 错误映射与 redaction；[`m12-lifecycle`] — child process 生命周期与 cleanup。
  - Omitted: [`m10-performance`] — 先正确、可控。

  **Parallelization**: Can Parallel: YES | Wave 4 | Blocks: 11, 12 | Blocked By: 1, 2, 4

  **References**:
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/nodes/logic/if_rhai_node.rs` — V1 脚本/表达式节点经验，仅参考错误边界，不继承 Rhai 方案。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/core/context.rs` — V1 context/value 传递经验，V2 需简化为 JSON envelope。

  **Acceptance Criteria**:
  - [ ] `cargo test --workspace worker_protocol_version_negotiation` succeeds.
  - [ ] `cargo test --workspace python_and_javascript_worker_roundtrip` succeeds.
  - [ ] `cargo test --workspace worker_timeout_and_oversized_output` succeeds.

  **QA Scenarios**:
  ```text
  Scenario: Python and JavaScript workers round-trip structured data
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace python_and_javascript_worker_roundtrip -- --nocapture | tee .sisyphus/evidence/task-8-worker.txt`
    Expected: both worker kinds accept JSON input and return structured outputs through the versioned envelope
    Evidence: .sisyphus/evidence/task-8-worker.txt

  Scenario: Malformed or runaway worker is terminated safely
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace worker_timeout_and_oversized_output -- --nocapture | tee .sisyphus/evidence/task-8-worker-error.txt`
    Expected: timeout, malformed JSON, or oversized output causes controlled failure and child cleanup
    Evidence: .sisyphus/evidence/task-8-worker-error.txt
  ```

  **Commit**: YES | Message: `feat(worker): add subprocess script runtime host` | Files: `crates/chainbot/src/worker/**`, `crates/chainbot/tests/**`, fixture workers under test resources

- [x] 9. Implement external node plugin host and manifest validation

  **What to do**: Implement discovery and execution for external node plugins, using a manifest + executable contract parallel to trigger plugins. Validate allowed capabilities, executable location, declared input/output schema, supported `api_version`, and plugin kind. Keep host/plugin contract stable and versioned. Integrate external node execution into the scheduler without giving plugins direct write access to either the file-backed runtime state or the minimal SQLite coordination store.
  **Must NOT do**: 不要让 node plugin 与 trigger plugin 共用“万能执行路径”而丢失类别边界；不要允许插件直接写 SQLite 或读取 secret store 原文。

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: 这是 MVP 里最容易扩散边界的扩展点。
  - Skills: [`m04-zero-cost`] — 设计宿主侧 trait/dispatch；[`m13-domain-error`] — 插件兼容性和执行错误分类。
  - Omitted: [`unsafe-checker`] — 方案应坚持进程/协议隔离，不走 unsafe ABI。

  **Parallelization**: Can Parallel: YES | Wave 4 | Blocks: 11, 12 | Blocked By: 1, 2, 4

  **References**:
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/nodes/registry.rs` — V1 节点注册思路，用于宿主侧 kind/version 路由。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/nodes/mod.rs` — V1 节点 dispatch 入口经验。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-monitor/src/dispatch/executor_client.rs` — 远离内联执行、保持边界的经验来源。

  **Acceptance Criteria**:
  - [ ] `cargo test --workspace node_plugin_manifest_validation` succeeds.
  - [ ] `cargo test --workspace external_node_plugin_roundtrip` succeeds.
  - [ ] `cargo test --workspace node_plugin_capability_restrictions` succeeds.

  **QA Scenarios**:
  ```text
  Scenario: External node plugin executes through host contract
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace external_node_plugin_roundtrip -- --nocapture | tee .sisyphus/evidence/task-9-node-plugin.txt`
    Expected: host loads a valid node plugin manifest, executes the plugin, and maps outputs into scheduler-visible node outputs
    Evidence: .sisyphus/evidence/task-9-node-plugin.txt

  Scenario: Node plugin with invalid capability request is rejected
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace node_plugin_capability_restrictions -- --nocapture | tee .sisyphus/evidence/task-9-node-plugin-error.txt`
    Expected: plugin requesting unsupported capability or wrong API version is rejected before execution
    Evidence: .sisyphus/evidence/task-9-node-plugin-error.txt
  ```

  **Commit**: YES | Message: `feat(plugin): add external node plugin host` | Files: `crates/chainbot/src/plugin/**`, `crates/chainbot/tests/**`, plugin fixtures under test resources

- [x] 10. Implement pass-style PGP secret provider and redaction guarantees

  **What to do**: Implement a `SecretProvider` backed by `pass`-style directory semantics: one encrypted file per secret, nested folders as namespaces, and late runtime resolution from `secret://` references. Define lookup rules, file naming conventions, decryption invocation, caching boundaries, and redaction policy across CLI errors, run summaries, scheduler logs, and worker requests. Hide the actual decrypt command behind a provider interface so unit/integration tests can use a deterministic fake decryptor when `gpg` is unavailable, while production wiring still targets `pass`/PGP files.
  **Must NOT do**: 不要在数据库、evidence、stdout、panic、或 worker env 中持久化 secret value；不要提前把所有 secret 批量解密进内存；不要把测试成功建立在本机一定安装 `gpg` 的假设上。

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: 机密边界一旦设计错误，后面很难补救。
  - Skills: [`m06-error-handling`] — 缺失/解密失败/权限错误分类；[`m12-lifecycle`] — late binding 与缓存边界。
  - Omitted: [`domain-web`] — 不涉及网络 secret manager。

  **Parallelization**: Can Parallel: YES | Wave 4 | Blocks: 11, 12 | Blocked By: 1, 2

  **References**:
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/core/storage.rs` — V1 regular vs secret storage 的经验来源，V2 需改为 `pass` 文件解析而不是内嵌 secret store。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/core/context.rs` — V1 context/store 边界，可参考但避免 secret 常驻。

  **Acceptance Criteria**:
  - [ ] `cargo test --workspace secret_reference_resolution` succeeds.
  - [ ] `cargo test --workspace secret_decryption_failure_redaction` succeeds.
  - [ ] `cargo test --workspace secret_values_never_persisted` succeeds.
  - [ ] tests pass with a fake decryptor fixture and do not require a globally installed `gpg` binary.

  **QA Scenarios**:
  ```text
  Scenario: Secret reference resolves from pass-style hierarchy at runtime
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace secret_reference_resolution -- --nocapture | tee .sisyphus/evidence/task-10-secrets.txt`
    Expected: `secret://` references resolve only at execution time and yield plaintext only in-memory for the active operation
    Evidence: .sisyphus/evidence/task-10-secrets.txt

  Scenario: Decryption failure is redacted everywhere
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace secret_decryption_failure_redaction -- --nocapture | tee .sisyphus/evidence/task-10-secrets-error.txt && cargo test --workspace secret_values_never_persisted -- --nocapture | tee -a .sisyphus/evidence/task-10-secrets-error.txt`
    Expected: failure messages contain metadata only, and tests prove secret content is absent from logs, state rows, and worker payload snapshots
    Evidence: .sisyphus/evidence/task-10-secrets-error.txt
  ```

  **Commit**: YES | Message: `feat(secrets): add pass-style pgp secret provider` | Files: `crates/chainbot/src/secrets/**`, `crates/chainbot/tests/**`

- [x] 11. Wire one runnable end-to-end vertical slice with builtin, plugin, and script execution

  **What to do**: Build a minimal but complete fixture tree under test resources that includes one workflow, one builtin trigger or manual invocation path, one external trigger plugin, one external node plugin, one script node, one builtin node, and one secret reference. Wire `validate`, `serve`, `run`, and `list-runs` so the vertical slice can be executed and verified end-to-end against a temporary test root, with workflow logs and trigger records emitted to files.
  **Must NOT do**: 不要扩展额外产品能力；不要为了测试方便绕过 manifest/secret/state/worker 的真实路径。

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: 要把所有关键子系统串成真实闭环并保证验证稳定。
  - Skills: [`fractal-context`] — 保持 fixture 与真实目录契约一致；[`m06-error-handling`] — 统一跨边界错误证据。
  - Omitted: [`m10-performance`] — 目标是跑通且可验收。

  **Parallelization**: Can Parallel: NO | Wave 5 | Blocks: 12, Final Verification | Blocked By: 2, 3, 4, 5, 6, 7, 8, 9, 10

  **References**:
  - Pattern: `docs/implementation/WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md:13` — 当前 repo 的基础验证命令集合。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-monitor/src/dispatch/run_request.rs` — V1 trigger 到 run request 的桥接思路。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-monitor/src/dispatch/executor_client.rs` — plane 间边界参考。

  **Acceptance Criteria**:
  - [ ] `cargo run -p chainbot -- validate --root target/test-roots/e2e` succeeds.
  - [ ] `cargo test --workspace end_to_end_vertical_slice` succeeds.
  - [ ] `cargo run -p chainbot -- list-runs --root target/test-roots/e2e` shows the persisted completed run after the test flow.
  - [ ] end-to-end test proves file-backed workflow logs and trigger records exist under the configured state tree.

  **QA Scenarios**:
  ```text
  Scenario: End-to-end vertical slice succeeds from trigger to persisted run
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace end_to_end_vertical_slice -- --nocapture | tee .sisyphus/evidence/task-11-e2e.txt`
    Expected: fixture workflow validates, runs through builtin + plugin + script nodes, persists run state, writes workflow/trigger files, and exposes result through CLI query
    Evidence: .sisyphus/evidence/task-11-e2e.txt

  Scenario: Broken vertical slice fixture fails with bounded errors
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace end_to_end_vertical_slice_failure_modes -- --nocapture | tee .sisyphus/evidence/task-11-e2e-error.txt`
    Expected: invalid plugin, missing secret, or worker error fails the run cleanly and leaves consistent state
    Evidence: .sisyphus/evidence/task-11-e2e-error.txt
  ```

  **Commit**: YES | Message: `feat(mvp): wire chainbot v2 vertical slice` | Files: `crates/chainbot/src/**`, `crates/chainbot/tests/**`, fixture resources

- [x] 12. Harden restart behavior, reload policy, and failure boundaries

  **What to do**: Add the remaining hardening needed for MVP stability: restart recovery, crash-safe cleanup, explicit config reload policy (`restart only` unless already implemented safely), worker cancellation cleanup, duplicate trigger prevention after restart, plugin executable disappearance, bounded logging, and file log/trigger record/run summary rotation or append-safety rules. Lock in `serve` startup/shutdown behavior and document restart-only reload semantics in code/tests.
  **Must NOT do**: 不要在 hardening 阶段新增新特性类别；不要把 reload 扩成热更新控制面；不要把“暂时没实现”留成隐式行为。

  **Recommended Agent Profile**:
  - Category: `unspecified-high` — Reason: 需要系统性扫尾并验证所有 failure boundary。
  - Skills: [`m13-domain-error`] — 稳定失败语义；[`m07-concurrency`] — shutdown/restart/cleanup 行为。
  - Omitted: [`domain-web`] — 仍然不引入网络控制面。

  **Parallelization**: Can Parallel: NO | Wave 6 | Blocks: Final Verification | Blocked By: 3, 5, 6, 7, 8, 9, 10, 11

  **References**:
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-monitor/src/trigger/reconcile.rs` — runtime reconcile / reload 经验来源。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-executor/src/core/run_record.rs` — 运行态恢复与状态一致性模式。
  - Pattern: `/Users/cyouguang/Documents/codework/ChainBot/chain-bot-rs/crates/chain-bot-monitor/src/trigger/mod.rs` — snapshot activation 和 reload 边界参考。

  **Acceptance Criteria**:
  - [ ] `cargo test --workspace serve_restart_recovery` succeeds.
  - [ ] `cargo test --workspace duplicate_trigger_after_restart` succeeds.
  - [ ] `cargo test --workspace config_reload_requires_restart` succeeds.
  - [ ] `cargo test --workspace file_log_recovery_and_rotation_policy` succeeds.

  **QA Scenarios**:
  ```text
  Scenario: Serve restarts cleanly and resumes without duplicate execution
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace serve_restart_recovery -- --nocapture | tee .sisyphus/evidence/task-12-hardening.txt && cargo test --workspace duplicate_trigger_after_restart -- --nocapture | tee -a .sisyphus/evidence/task-12-hardening.txt`
    Expected: restart re-acquires lease, recovers state, and avoids duplicate trigger dispatch after crash/restart
    Evidence: .sisyphus/evidence/task-12-hardening.txt

  Scenario: Config changes are not hot-reloaded silently
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace config_reload_requires_restart -- --nocapture | tee .sisyphus/evidence/task-12-hardening-error.txt`
    Expected: changed config is ignored until restart, with explicit test coverage proving restart-only reload policy
    Evidence: .sisyphus/evidence/task-12-hardening-error.txt

  Scenario: File-backed logs remain consistent across restart and rollover
    Tool: Bash
    Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo test --workspace file_log_recovery_and_rotation_policy -- --nocapture | tee .sisyphus/evidence/task-12-hardening-logs.txt`
    Expected: restart does not corrupt workflow/trigger log files, and rollover or append rules remain deterministic
    Evidence: .sisyphus/evidence/task-12-hardening-logs.txt
  ```

  **Commit**: YES | Message: `fix(runtime): harden restart and failure boundaries` | Files: `crates/chainbot/src/**`, `crates/chainbot/tests/**`

## Final Verification Wave (4 parallel agents, ALL must APPROVE)
- [x] F1. Plan Compliance Audit — oracle
  - Tool: `task(subagent_type="oracle")`
- Steps: review delivered implementation against `.sisyphus/plans/chainbot-v2-mvp.md`, verify every completed task preserved stated guardrails and no hidden scope expansion occurred
  - Expected: oracle approves that implementation matches plan decisions or returns a concrete blocking drift list
  - Evidence: `.sisyphus/evidence/f1-plan-compliance.md`
- [x] F2. Code Quality Review — unspecified-high
  - Tool: `task(category="unspecified-high")`
  - Steps: review changed source for architectural consistency, error boundaries, persistence separation, and maintainability regressions
  - Expected: reviewer approves or returns a concrete defect list tied to file paths and acceptance criteria
  - Evidence: `.sisyphus/evidence/f2-code-quality.md`
- [x] F3. Real Manual QA — unspecified-high (+ playwright if UI)
  - Tool: `Bash`
  - Steps: run `mkdir -p .sisyphus/evidence && set -o pipefail && cargo metadata --no-deps | tee .sisyphus/evidence/f3-manual-qa.txt && cargo check --workspace | tee -a .sisyphus/evidence/f3-manual-qa.txt && cargo test --workspace | tee -a .sisyphus/evidence/f3-manual-qa.txt && cargo run -p chainbot -- validate --root target/test-roots/e2e | tee -a .sisyphus/evidence/f3-manual-qa.txt`
  - Expected: all commands exit 0 and e2e fixtures validate without hidden manual intervention
  - Evidence: `.sisyphus/evidence/f3-manual-qa.txt`
- [x] F4. Scope Fidelity Check — deep
  - Tool: `task(category="deep")`
  - Steps: compare final implementation to original user goals, confirm CLI-first shape, plugin coverage, subprocess script runtime, pass/PGP secrets, file-backed logs, and minimal SQLite coordination all exist with no missing pillar
  - Expected: reviewer confirms all original product goals are represented and no required subsystem was silently deferred
  - Evidence: `.sisyphus/evidence/f4-scope-fidelity.md`

## Commit Strategy
- 采用原子提交，按契约冻结 -> 测试基线 -> CLI/config/state -> scheduler/plugin/worker/secrets -> vertical slice -> hardening 顺序推进。
- 每个提交必须保持 `cargo check --workspace` 通过；关键里程碑提交需保持 `cargo test --workspace` 通过。
- 不做 V1 兼容迁移提交；所有提交围绕 V2 本地优先 MVP。

## Success Criteria
- 从空的测试根目录可以初始化、校验并运行一个最小 workflow。
- `serve` 模式能获得单实例 lease，拒绝第二个 owner，并在正常退出后释放。
- 一个外部 trigger 插件与一个外部 node 插件都能通过 manifest 校验并被宿主调用。
- 一个 Python 或 JavaScript 脚本节点可以通过 subprocess worker 执行并返回结构化输出。
- 一个 `secret://` 引用可以解析到 `pass`/PGP 文件，且失败日志不泄漏 secret 内容。
- workflow 运行日志、trigger 触发记录、run summary 以文件形式落地，并可通过 run identifier 关联。
- SQLite 只承载最小协调状态：single-owner lease 与 dedup/cooldown token。
- 集成测试覆盖 trigger -> run -> node/plugin/script -> file-backed result -> CLI 查询结果的闭环。
