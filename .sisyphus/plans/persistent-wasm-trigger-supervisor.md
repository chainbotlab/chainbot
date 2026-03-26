# Persistent Wasm Trigger Supervisor

## TL;DR
> **Summary**: Replace short-lived wasm trigger polling with daemon-owned persistent trigger sessions. Wasm guests push events into a durable staging boundary, receive ACK only after durable persistence, and existing `TriggerPlane::normalize_emission()` remains the acceptance/replay/dedup/cooldown authority.
> **Deliverables**:
> - A daemon-owned `WasmTriggerSupervisor` that keeps one `Store` / `Instance` alive per active wasm trigger session
> - Durable staging + ACK semantics for guest-pushed events before acceptance
> - Explicit backpressure / lease-lost / shutting-down error contract for push callbacks
> - Process and wasm external triggers converged onto one supervisor/session abstraction
> - `catalog` / help / examples that clearly distinguish process trigger lifecycle from wasm persistent trigger lifecycle
> - Full session-oriented test matrix covering start/stop, durable ACK, crash windows, lease/reconcile, fairness, and recovery
> **Effort**: XL
> **Parallel**: YES - 3 waves
> **Critical Path**: Task 1 → Task 4 → Task 7 → Task 9 → Task 12 → Task 15 → Final Verification

## Context
### Original Request
- 用户要把 wasm trigger 从短命 `start/drain/stop` 提升到面向长时间运行事件源的正式模型。
- 用户明确提出：长期目标不是大 backlog 排空，而是“插件长期运行并持续产生 event”。
- 用户进一步要求主动讨论 push vs poll，并最终选择 **guest 主动 push**，而不是 host 定时 poll。

### Interview Summary
- 首版 scope 收敛到“会话核心”，不在这一轮把完整 capability wiring 扩到所有 host runtime 细节。
- Session ownership 不放进 `TriggerPlane`，而是放在 daemon-owned supervisor。
- `TriggerPlane` 继续只做 accepted-event 归一化、dedup/cooldown、checkpoint/replay 语义。
- Guest push 成功语义必须等于 durable acceptance；不能先收到就 ACK。
- Backpressure 必须是显式错误码，guest 负责退避/停止。
- 发现面必须把 process trigger 与 wasm persistent trigger 生命周期分叉展示，不再继续统一文案。
- 用户最终将 “future process external trigger convergence” 也并入当前方案，而不是留作后续 TODO。

### Metis Review (gaps addressed)
- 明确 supervisor 必须持有 session 生命周期、reconcile、teardown，而不是让 `trigger.rs` 继续膨胀。
- 明确 durable staging 是 guest push 到 acceptance 语义之间的防撞层，避免 crash window 变脏。
- 明确实现计划必须单独覆盖 manifest/runtime discoverability 漂移、session fairness、lease loss、manifest change reconcile。
- 明确 current process external trigger 收敛进入统一 supervisor 抽象现在就做，而不是口头承诺“以后再统一”。

## Work Objectives
### Core Objective
- 在不破坏当前 accepted-event 归一化/持久化语义的前提下，把 wasm trigger runtime 升级为 daemon-owned persistent session 模型，并将 process external trigger 同步收敛到同一 supervisor abstraction。

### Deliverables
- 新增 daemon-owned external trigger supervisor module，持有 process 与 wasm external trigger sessions。
- 新增 durable staging/inbox 边界，供 persistent session 把 raw external events 先 durable 写入，再交由 acceptance layer 消费。
- 新增 guest push callback contract，只有 durable 写入成功才返回 ACK。
- 新增显式 backpressure / lease-lost / shutting-down 错误 contract。
- 更新 `catalog`、CLI help、examples、research docs，使 process trigger 与 wasm persistent trigger 生命周期差异对外可见。
- 过程与 wasm external trigger 在 supervisor/session lifecycle、lease/reconcile、fairness 上共享一套模型。
- 完整 session matrix 测试通过，包括 restart recovery 和 scheduler fairness。

### Definition of Done (verifiable conditions with commands)
- `cargo check -p chainbot` succeeds.
- `cargo test -p chainbot --test trigger_plane` succeeds with persistent session and durable staging coverage.
- `cargo test -p chainbot --test end_to_end_vertical_slice` succeeds with supervisor-owned external trigger lifecycle coverage.
- `cargo test -p chainbot --test catalog_surface` succeeds with process trigger vs wasm persistent trigger runtime visibility coverage.
- `cargo test -p chainbot --lib` succeeds with new durable staging / session / budget unit coverage.
- `cargo test -p chainbot -- --ignored` succeeds for heavy backlog / fairness / reconcile scenarios if any ignored stress tests are added.
- `chainbot serve` with a wasm persistent trigger fixture produces staged events, accepted trigger records, and run requests without re-instantiating the session each tick.

### Must Have
- Daemon-owned `WasmTriggerSupervisor` or equivalent external trigger supervisor module.
- Durable staging before `TriggerPlane::normalize_emission()`.
- Guest push callback returns success only after durable staging succeeds.
- Explicit host error contract for backpressure, lease loss, and shutdown.
- `Store` limiter + session/poll budget from day one.
- `catalog` and examples visibly differentiate process external trigger lifecycle from wasm persistent trigger lifecycle.
- Process external trigger path converged into the same supervisor/session abstraction in this plan.

### Must NOT Have (guardrails, AI slop patterns, scope boundaries)
- Do not let persistent live session state live inside `TriggerPlane`.
- Do not ACK guest-pushed events before durable staging writes complete.
- Do not let guest infer backpressure from generic string failures.
- Do not keep process and wasm trigger lifecycle differences hidden behind a single generic catalog protocol display.
- Do not introduce a second business contract source beyond manifest.
- Do not expand this plan into full wasm SDK/template tooling or marketplace work.

## Verification Strategy
> ZERO HUMAN INTERVENTION — all verification is agent-executed.
- Test decision: tests-after + Rust built-in unit/integration test framework
- QA policy: every task includes implementation plus deterministic QA scenarios
- Evidence: `.sisyphus/evidence/task-{N}-{slug}.{ext}`

## Execution Strategy
### Parallel Execution Waves
Wave 1: Tasks 1-5 — contract, staging, supervisor boundaries, discoverability split
Wave 2: Tasks 6-10 — session runtime, push callback, backpressure, reconcile, process convergence
Wave 3: Tasks 11-15 — fairness/budgets, restart recovery, full matrix tests, examples/docs, final cleanup

### Dependency Matrix (full, all tasks)
- Task 1 blocks Tasks 2-15
- Task 2 blocks Tasks 7-15
- Task 3 blocks Tasks 6-15
- Task 4 blocks Tasks 6-15
- Task 5 blocks Tasks 14-15
- Task 6 blocks Tasks 7-15
- Task 7 blocks Tasks 8-15
- Task 8 blocks Tasks 11-15
- Task 9 blocks Tasks 12-15
- Task 10 blocks Tasks 12-15
- Task 11 blocks Tasks 12-15
- Task 12 blocks Tasks 13-15
- Task 13 blocks Tasks 15 and Final Verification
- Task 14 blocks Task 15 and Final Verification
- Task 15 blocks Final Verification

### Agent Dispatch Summary (wave → task count → categories)
- Wave 1 → 5 tasks → `deep`, `unspecified-high`
- Wave 2 → 5 tasks → `deep`, `ultrabrain`
- Wave 3 → 5 tasks → `deep`, `unspecified-high`, `writing`

## TODOs
> Implementation + Test = ONE task. Never separate.
> EVERY task MUST have: Agent Profile + Parallelization + QA Scenarios.

- [x] 1. Define persistent trigger runtime contract

  **What to do**: Extend `crates/chainbot/src/plugin/contract.rs` and related decoding paths so external trigger manifests can distinguish process short-lived trigger runtime from daemon-supervised wasm persistent trigger runtime, including explicit push callback semantics, durable ACK semantics, and required host error categories. Reject ambiguous or mixed trigger runtime declarations.
  **Must NOT do**: Do not create a second business schema source outside manifest. Do not overload `capabilities` with permission or lifecycle semantics.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: contract changes affect runtime, CLI discovery, and tests
  - Skills: [`rust-router`] — reason about Rust contract evolution and type boundaries
  - Omitted: [`domain-web`] — no HTTP surface is being designed here

  **Parallelization**: Can Parallel: YES | Wave 1 | Blocks: [2,3,4,5,6,7,8,9,10,11,12,13,14,15] | Blocked By: []

  **References**:
  - Contract: `crates/chainbot/src/plugin/contract.rs:1-220` — current nested runtime, operations, event_schema, host_permissions baseline
  - Discoverability: `crates/chainbot/src/catalog.rs:1-260` — current runtime-aware plugin rendering surface
  - Proposal: `docs/research/CHAINBOT_WASM_PLUGIN_RUNTIME_PROPOSAL.md:137-251` — reviewed runtime config and validation baseline

  **Acceptance Criteria**:
  - [ ] External trigger manifests can encode persistent-session lifecycle metadata without flat-field ambiguity.
  - [ ] Validation rejects mixed or missing lifecycle fields with stable `ContractError` mapping.
  - [ ] Process external trigger manifests and wasm persistent trigger manifests are distinguishable in decoded contract output.

  **QA Scenarios**:
  ```
  Scenario: process vs persistent trigger manifest validation
    Tool: Bash
    Steps: cargo test -p chainbot --test config_loading
    Expected: process trigger, wasm persistent trigger, and invalid mixed trigger manifests all match expected outcomes
    Evidence: .sisyphus/evidence/task-1-trigger-runtime-contract.txt

  Scenario: invalid mixed runtime contract rejected
    Tool: Bash
    Steps: cargo test -p chainbot --lib plugin::contract::tests -- --nocapture
    Expected: invalid mixed trigger runtime declarations fail with stable field-specific errors
    Evidence: .sisyphus/evidence/task-1-trigger-runtime-contract-error.txt
  ```

  **Commit**: YES | Message: `feat(trigger): define persistent session runtime contract` | Files: `crates/chainbot/src/plugin/contract.rs`, `crates/chainbot/src/config.rs`, related tests

- [x] 2. Add durable staging schema and APIs

  **What to do**: Add durable staging/inbox records to `crates/chainbot/src/state_db.rs` (and any state layer mirrors required for parity) so external trigger supervisors can persist pushed raw events before acceptance. Include APIs for append, list pending, mark accepted, and reconcile on restart.
  **Must NOT do**: Do not bypass existing accepted-event persistence paths. Do not store only in-memory staging queues.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: DB/state changes affect crash recovery and replay semantics
  - Skills: [`rust-router`] — stateful Rust API design
  - Omitted: [`domain-cloud-native`] — no distributed queue here

  **Parallelization**: Can Parallel: YES | Wave 1 | Blocks: [7,9,12,13,14,15] | Blocked By: [1]

  **References**:
  - Runtime state: `crates/chainbot/src/state_db.rs` — existing trigger checkpoints/snapshots/event records
  - Trigger acceptance boundary: `crates/chainbot/src/trigger.rs:69-172` — normalized trigger event / run request structures
  - Precedent: `crates/chainbot/src/ingress/supervisor.rs` — long-lived runtime to durable staging precedent

  **Acceptance Criteria**:
  - [ ] Durable staging records survive daemon restart.
  - [ ] Supervisor can append staged events without going through acceptance layer first.
  - [ ] Acceptance layer can enumerate and consume staged external trigger events deterministically.

  **QA Scenarios**:
  ```
  Scenario: staged event survives restart
    Tool: Bash
    Steps: cargo test -p chainbot --test runtime_state_parity -- --nocapture
    Expected: staged trigger event and checkpoint survive restart and can be reloaded
    Evidence: .sisyphus/evidence/task-2-durable-staging.txt

  Scenario: duplicate staging write guarded
    Tool: Bash
    Steps: cargo test -p chainbot --test state_runtime_persistence -- --nocapture
    Expected: duplicate or conflicting staging persistence behaves deterministically and does not corrupt replay state
    Evidence: .sisyphus/evidence/task-2-durable-staging-error.txt
  ```

  **Commit**: YES | Message: `feat(state): add durable staging for external trigger sessions` | Files: `crates/chainbot/src/state_db.rs`, related tests

- [x] 3. Introduce external trigger supervisor module

  **What to do**: Add a dedicated daemon-owned external trigger supervisor module (for example `crates/chainbot/src/external_trigger_supervisor.rs`) responsible for owning live process/wasm sessions, not acceptance semantics. Define session registry, session state, lease-bound startup/teardown, and reconcile hooks.
  **Must NOT do**: Do not keep growing `trigger.rs` with long-lived runtime orchestration. Do not hide session ownership inside `TriggerPlane`.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: lifecycle boundaries and module ownership are the heart of this plan
  - Skills: [`rust-router`] — module boundaries and Rust ownership
  - Omitted: [`domain-web`] — lifecycle boundary is internal

  **Parallelization**: Can Parallel: YES | Wave 1 | Blocks: [6,7,8,9,10,11,12,13,14,15] | Blocked By: [1]

  **References**:
  - Existing trigger plane: `crates/chainbot/src/trigger.rs:182-203` — current TriggerPlane owns acceptance layer state
  - Current daemon hook: `crates/chainbot/src/cli.rs:3199-3240` — current trigger host policy wiring at serve construction time
  - Precedent: `crates/chainbot/src/ingress/supervisor.rs` — existing daemon-owned long-lived runtime boundary

  **Acceptance Criteria**:
  - [ ] Long-lived external trigger session ownership exists outside `TriggerPlane`.
  - [ ] Supervisor API can start, stop, and reconcile sessions by trigger ID.
  - [ ] `trigger.rs` remains focused on acceptance and normalization semantics.

  **QA Scenarios**:
  ```
  Scenario: supervisor module composes with daemon loop
    Tool: Bash
    Steps: cargo test -p chainbot --test cli_surface -- --nocapture
    Expected: daemon-oriented trigger setup still succeeds with supervisor in the construction path
    Evidence: .sisyphus/evidence/task-3-trigger-supervisor.txt

  Scenario: session registry tears down on explicit stop
    Tool: Bash
    Steps: cargo test -p chainbot --lib trigger_supervisor -- --nocapture
    Expected: explicit stop removes session state without touching accepted-event persistence
    Evidence: .sisyphus/evidence/task-3-trigger-supervisor-error.txt
  ```

  **Commit**: YES | Message: `feat(trigger): add daemon-owned external trigger supervisor` | Files: new supervisor module, `crates/chainbot/src/lib.rs`, related tests

- [x] 4. Define guest push ABI and typed host errors

  **What to do**: Replace the short-lived `start/drain/stop` wasm trigger transport with a guest-driven push ABI that uses host imports/callbacks. Define explicit success and error results for durable ACK, backpressure, lease-lost, and shutting-down outcomes. Keep manifest as business schema truth; WIT only defines transport.
  **Must NOT do**: Do not keep using `drain` heuristics for this plan. Do not reduce durable ack to a generic string success/failure contract.

  **Recommended Agent Profile**:
  - Category: `ultrabrain` — Reason: transport ABI choices have long-lived blast radius
  - Skills: [`rust-router`] — Rust/WIT integration reasoning
  - Omitted: [`domain-web`] — no HTTP API contract here

  **Parallelization**: Can Parallel: YES | Wave 1 | Blocks: [6,7,8,9,10,11,12,13,14,15] | Blocked By: [1]

  **References**:
  - Current WIT: `wit/trigger-plugin.wit` — current short-lived transport baseline
  - Current wasm host: `crates/chainbot/src/trigger_wasm.rs:50-185` — current `start`/`drain`/`stop` implementation
  - Reviewed direction: Wasmtime guidance — Store/Instance live for session lifetime

  **Acceptance Criteria**:
  - [ ] WIT/host callback API explicitly models guest push and typed host responses.
  - [ ] Durable ACK vs retryable host failure vs terminal shutdown are distinguishable.
  - [ ] No business field schema is duplicated into WIT definitions.

  **QA Scenarios**:
  ```
  Scenario: WIT transport compiles and host bindings generate cleanly
    Tool: Bash
    Steps: cargo check -p chainbot
    Expected: WIT/bindgen integration compiles with typed callback results
    Evidence: .sisyphus/evidence/task-4-push-abi.txt

  Scenario: host callback error categories remain machine-distinguishable
    Tool: Bash
    Steps: cargo test -p chainbot --lib trigger_wasm -- --nocapture
    Expected: guest push tests can assert backpressure, lease-lost, and shutting-down separately
    Evidence: .sisyphus/evidence/task-4-push-abi-error.txt
  ```

  **Commit**: YES | Message: `feat(trigger): define push callback ABI for persistent wasm sessions` | Files: `wit/trigger-plugin.wit`, `crates/chainbot/src/trigger_wasm.rs`, related tests

- [ ] 5. Split trigger discoverability by lifecycle

  **What to do**: Update `catalog.rs`, CLI help, examples, and docs so process external trigger and wasm persistent trigger lifecycle are visibly different. Show runtime/lifecycle/backpressure semantics instead of one generic trigger protocol block.
  **Must NOT do**: Do not leave the current unified trigger protocol text in place for both runtimes.

  **Recommended Agent Profile**:
  - Category: `writing` — Reason: this is runtime-surface documentation and CLI presentation work
  - Skills: [`rust-router`] — keep Rust display structures aligned
  - Omitted: [`frontend-design`] — no UI work

  **Parallelization**: Can Parallel: YES | Wave 1 | Blocks: [14,15] | Blocked By: [1]

  **References**:
  - Catalog rendering: `crates/chainbot/src/catalog.rs:300-520` — current runtime-aware plugin display
  - CLI help: `crates/chainbot/src/cli.rs` — current catalog/help text surfaces
  - Examples: `examples/plugin-integrations/plugins/` and `examples/README.md`

  **Acceptance Criteria**:
  - [ ] `catalog show plugin:<process-trigger>` and `catalog show plugin:<wasm-trigger>` present different lifecycle/runtime descriptions.
  - [ ] Help/examples no longer imply one unified trigger runtime protocol.
  - [ ] Docs/examples match the chosen supervisor + durable ACK architecture.

  **QA Scenarios**:
  ```
  Scenario: catalog shows lifecycle split clearly
    Tool: Bash
    Steps: cargo test -p chainbot --test catalog_surface
    Expected: process trigger and wasm persistent trigger fixtures render different runtime/lifecycle detail blocks
    Evidence: .sisyphus/evidence/task-5-trigger-catalog.txt

  Scenario: example roots validate with updated lifecycle docs
    Tool: Bash
    Steps: cargo test -p chainbot --test cli_surface
    Expected: curated example roots remain valid and discoverable after docs/help updates
    Evidence: .sisyphus/evidence/task-5-trigger-catalog-error.txt
  ```

  **Commit**: YES | Message: `docs(trigger): split process and wasm persistent trigger discovery` | Files: `crates/chainbot/src/catalog.rs`, `crates/chainbot/src/cli.rs`, `examples/README.md`, example manifests

- [x] 6. Add `WasmTriggerSession` runtime and Store lifetime ownership

  **What to do**: Implement a `WasmTriggerSession` type inside the supervisor boundary that owns the long-lived `Store`, instantiated component handles, guest state, and lifecycle bookkeeping for one trigger ID. Reuse the same session across repeated daemon turns until reconcile/stop.
  **Must NOT do**: Do not recreate the Wasm instance each serve cycle. Do not leak `Store` lifetime outside the supervisor boundary.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: Wasmtime store lifetime and Rust ownership need careful modeling
  - Skills: [`rust-router`] — lifetime/resource modeling
  - Omitted: [`domain-web`] — internal runtime concern

  **Parallelization**: Can Parallel: YES | Wave 2 | Blocks: [7,8,9,10,11,12,13,14,15] | Blocked By: [3,4]

  **References**:
  - Current wasm host: `crates/chainbot/src/trigger_wasm.rs:32-240` — current short-lived host baseline
  - Wasmtime guidance: one `Store<T>` / `Instance` per long-lived session
  - Engine/cache: `crates/chainbot/src/wasm/engine.rs`, `crates/chainbot/src/wasm/cache.rs`

  **Acceptance Criteria**:
  - [ ] One active wasm trigger session reuses one `Store` / `Instance` across repeated turns.
  - [ ] Session teardown drops owned runtime state cleanly on reconcile/stop.
  - [ ] Session lifecycle is daemon-owned, not acceptance-layer owned.

  **QA Scenarios**:
  ```
  Scenario: repeated turns reuse one session
    Tool: Bash
    Steps: cargo test -p chainbot --lib trigger_supervisor -- --nocapture
    Expected: repeated supervisor ticks do not recreate the wasm trigger session unless explicitly reconciled
    Evidence: .sisyphus/evidence/task-6-wasm-session.txt

  Scenario: session teardown releases state on stop
    Tool: Bash
    Steps: cargo test -p chainbot --lib trigger_wasm -- --nocapture
    Expected: stop/reconcile drops the session and no stale Store/Instance handles remain registered
    Evidence: .sisyphus/evidence/task-6-wasm-session-error.txt
  ```

  **Commit**: YES | Message: `feat(trigger): add long-lived wasm trigger sessions` | Files: supervisor module, `crates/chainbot/src/trigger_wasm.rs`, related tests

- [x] 7. Implement durable push callback and ACK semantics

  **What to do**: Implement host callback(s) that accept guest-pushed events, write them into durable staging/inbox storage, and only then return success to the guest. Make the callback return retryable or terminal errors according to the chosen backpressure contract.
  **Must NOT do**: Do not enqueue only in memory before ACK. Do not ACK before durable write succeeds.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: correctness boundary between guest and host
  - Skills: [`rust-router`] — host/runtime and error handling
  - Omitted: [`domain-web`] — this is not a network API

  **Parallelization**: Can Parallel: YES | Wave 2 | Blocks: [8,9,10,12,13,14,15] | Blocked By: [2,4,6]

  **References**:
  - Accepted-event structures: `crates/chainbot/src/trigger.rs:69-172`
  - State APIs: `crates/chainbot/src/state_db.rs`
  - WIT push contract from Task 4

  **Acceptance Criteria**:
  - [ ] Guest-visible success means durable staging succeeded.
  - [ ] Durable failure paths return explicit retryable/terminal host errors.
  - [ ] Crash between push and acceptance does not lose already-ACKed events.

  **QA Scenarios**:
  ```
  Scenario: guest gets success only after durable write
    Tool: Bash
    Steps: cargo test -p chainbot --test trigger_plane -- --nocapture
    Expected: staged event record exists before guest-visible ACK success is observed in the test harness
    Evidence: .sisyphus/evidence/task-7-durable-ack.txt

  Scenario: durable write failure returns retryable host error
    Tool: Bash
    Steps: cargo test -p chainbot --lib trigger_supervisor -- --nocapture
    Expected: guest push receives a machine-distinguishable failure class when durable staging fails
    Evidence: .sisyphus/evidence/task-7-durable-ack-error.txt
  ```

  **Commit**: YES | Message: `feat(trigger): ack pushed events after durable staging` | Files: supervisor module, `state_db.rs`, tests

- [x] 8. Add explicit backpressure, lease-lost, and shutting-down host errors

  **What to do**: Implement typed host-side callback responses that tell guest code whether it should retry, back off, or stop. Wire these outcomes to queue saturation, session budget exhaustion, daemon shutdown, and lease loss.
  **Must NOT do**: Do not collapse all failure into one generic string. Do not leave guest retry behavior undefined.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: control-plane error semantics affect long-lived stability
  - Skills: [`rust-router`] — typed error design
  - Omitted: [`domain-web`] — not HTTP semantics

  **Parallelization**: Can Parallel: YES | Wave 2 | Blocks: [9,10,12,13,14,15] | Blocked By: [4,7]

  **References**:
  - `crates/chainbot/src/errors.rs` — current stable error taxonomy
  - Wasmtime session guidance — long-lived store should not be abused as unlimited queue

  **Acceptance Criteria**:
  - [ ] Guest push can distinguish backpressure vs lease-lost vs shutting-down.
  - [ ] Host error mapping stays stable and testable.
  - [ ] Retry/backoff expectations are documented in examples/help.

  **QA Scenarios**:
  ```
  Scenario: queue/budget exhaustion returns backpressure
    Tool: Bash
    Steps: cargo test -p chainbot --lib trigger_supervisor -- --nocapture
    Expected: callback returns the specific retryable backpressure result class
    Evidence: .sisyphus/evidence/task-8-backpressure.txt

  Scenario: lease loss and shutdown return terminal stop errors
    Tool: Bash
    Steps: cargo test -p chainbot --test trigger_plane -- --nocapture
    Expected: callback returns distinct non-retryable outcomes for lease-lost and shutting-down conditions
    Evidence: .sisyphus/evidence/task-8-backpressure-error.txt
  ```

  **Commit**: YES | Message: `feat(trigger): add explicit push backpressure error contract` | Files: supervisor module, errors, tests, docs/help surfaces

- [x] 9. Reconcile sessions on lease loss, disable, and manifest change

  **What to do**: Implement reconcile logic so the supervisor stops/restarts/removes sessions when daemon lease is lost, a trigger is disabled, or its manifest/runtime config changes. Ensure stale sessions cannot continue pushing after reconcile.
  **Must NOT do**: Do not leave session teardown only for process exit. Do not let stale sessions keep holding Store/Instance state after manifest changes.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: reconcile semantics define correctness under long-lived runtime changes
  - Skills: [`rust-router`] — stateful lifecycle transitions
  - Omitted: [`domain-cloud-native`] — single-node daemon scope

  **Parallelization**: Can Parallel: YES | Wave 2 | Blocks: [12,13,14,15] | Blocked By: [3,6,8]

  **References**:
  - Daemon lifecycle: `crates/chainbot/src/cli.rs:3199-3240` and surrounding `serve_once_with_lease()` logic
  - Lease state: `crates/chainbot/src/state_db.rs:633-759`
  - Supervisor precedent: `crates/chainbot/src/ingress/supervisor.rs`

  **Acceptance Criteria**:
  - [ ] Disabled triggers stop their active sessions.
  - [ ] Manifest/runtime change tears down and recreates the affected session deterministically.
  - [ ] Lease loss stops session activity before new pushes are ACKed.

  **QA Scenarios**:
  ```
  Scenario: disable trigger tears down session
    Tool: Bash
    Steps: cargo test -p chainbot --test trigger_plane -- --nocapture
    Expected: disabling a trigger stops the session and future pushes are rejected/ignored
    Evidence: .sisyphus/evidence/task-9-reconcile.txt

  Scenario: manifest change restarts session cleanly
    Tool: Bash
    Steps: cargo test -p chainbot --test end_to_end_vertical_slice -- --nocapture
    Expected: manifest change causes one clean teardown/start cycle without duplicate accepted-event corruption
    Evidence: .sisyphus/evidence/task-9-reconcile-error.txt
  ```

  **Commit**: YES | Message: `feat(trigger): reconcile persistent sessions on runtime changes` | Files: supervisor module, `cli.rs`, `trigger.rs`, tests

- [x] 10. Converge process external trigger onto supervisor abstraction

  **What to do**: Refactor current process external trigger handling so it also runs under the new external trigger supervisor/session abstraction, even if the actual session implementation differs from wasm. Keep one lifecycle and reconcile model across both runtimes.
  **Must NOT do**: Do not leave two unrelated orchestration stacks after this plan. Do not silently change process trigger acceptance semantics.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: this is the scope expansion the user chose; it affects two runtime families
  - Skills: [`rust-router`] — refactor/runtime convergence
  - Omitted: [`domain-web`] — no frontend or HTTP surface

  **Parallelization**: Can Parallel: YES | Wave 2 | Blocks: [12,13,14,15] | Blocked By: [3,7,8,9]

  **References**:
  - Current process host state machine: `crates/chainbot/src/trigger.rs:89-160` and process message handling below
  - Current policy build: `crates/chainbot/src/cli.rs:3219-3240`

  **Acceptance Criteria**:
  - [ ] Process external triggers are supervised/reconciled through the same top-level abstraction as wasm triggers.
  - [ ] Process trigger acceptance/replay behavior is unchanged.
  - [ ] Process and wasm trigger lifecycle docs can now describe one supervisor model with runtime-specific adapters.

  **QA Scenarios**:
  ```
  Scenario: process external trigger still produces accepted run requests under supervisor
    Tool: Bash
    Steps: cargo test -p chainbot --test trigger_plane -- --nocapture
    Expected: existing process external trigger fixtures keep passing under the new supervisor path
    Evidence: .sisyphus/evidence/task-10-process-convergence.txt

  Scenario: process and wasm trigger parity stays aligned
    Tool: Bash
    Steps: cargo test -p chainbot --test runtime_state_parity -- --nocapture
    Expected: checkpoint/snapshot semantics remain aligned across trigger runtimes
    Evidence: .sisyphus/evidence/task-10-process-convergence-error.txt
  ```

  **Commit**: YES | Message: `refactor(trigger): converge process and wasm trigger supervision` | Files: supervisor module, `trigger.rs`, tests

- [x] 11. Add Store limiter and session/poll budgets

  **What to do**: Add `Store` limiter configuration and explicit session/poll budgets for persistent trigger sessions, including max events per callback cycle, max wall time per supervisor tick, and fairness yield rules.
  **Must NOT do**: Do not leave long-lived sessions unbounded. Do not hide fairness behind undocumented heuristics.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: long-lived runtime resource control is first-order correctness for this plan
  - Skills: [`rust-router`] — resource/lifecycle management
  - Omitted: [`m10-performance`] — no benchmarking-first tuning yet

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: [12,13,14,15] | Blocked By: [6,8,9,10]

  **References**:
  - Wasmtime guidance: `Store` should own long-lived session state and use explicit resource limiting
  - Current wasm engine/cache: `crates/chainbot/src/wasm/engine.rs`, `crates/chainbot/src/wasm/cache.rs`

  **Acceptance Criteria**:
  - [ ] Long-lived sessions have an explicit memory limiter.
  - [ ] Supervisor tick budget is explicit and testable.
  - [ ] Fairness/yield behavior prevents one hot trigger from monopolizing the daemon.

  **QA Scenarios**:
  ```
  Scenario: hot trigger yields under budget
    Tool: Bash
    Steps: cargo test -p chainbot --lib external_trigger_supervisor -- --nocapture
    Expected: a hot session stops within budget and is re-scheduled without starving peers
    Evidence: .sisyphus/evidence/task-11-session-budget.txt

  Scenario: memory limiter stops runaway session growth
    Tool: Bash
    Steps: cargo test -p chainbot --lib wasm::engine wasm::cache -- --nocapture
    Expected: guest memory growth beyond configured limit fails with controlled host error
    Evidence: .sisyphus/evidence/task-11-session-budget-error.txt
  ```

  **Commit**: YES | Message: `feat(trigger): add session budgets and store limits` | Files: supervisor module, wasm engine/support, tests

- [x] 12. Wire supervisor into daemon serve loop and acceptance path

  **What to do**: Integrate the new supervisor into daemon startup and lease-bound loop so staged events are consumed, normalized, and turned into run requests through existing acceptance codepaths. Keep `TriggerPlane` as the acceptance layer, not the session owner.
  **Must NOT do**: Do not bypass `normalize_emission()`. Do not introduce a second accepted-event path.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: this is the bridge between long-lived runtime and existing acceptance semantics
  - Skills: [`rust-router`] — runtime orchestration
  - Omitted: [`domain-web`] — no UI/web API

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: [13,14,15] | Blocked By: [2,3,7,8,9,10,11]

  **References**:
  - Daemon serve loop: `crates/chainbot/src/cli.rs:3199-3240` and surrounding `serve_once_with_lease()` orchestration
  - Trigger acceptance: `crates/chainbot/src/trigger.rs`
  - Durable staging APIs from Task 2

  **Acceptance Criteria**:
  - [ ] Daemon loop starts supervisor once per lease owner.
  - [ ] Staged external trigger events are consumed through existing acceptance logic.
  - [ ] No duplicate acceptance path is introduced.

  **QA Scenarios**:
  ```
  Scenario: persistent session produces accepted run requests through daemon loop
    Tool: Bash
    Steps: cargo test -p chainbot --test end_to_end_vertical_slice -- --nocapture
    Expected: supervisor-owned persistent trigger sessions produce run requests via the same accepted-event path as existing triggers
    Evidence: .sisyphus/evidence/task-12-daemon-wireup.txt

  Scenario: crash between staging and acceptance remains replay-safe
    Tool: Bash
    Steps: cargo test -p chainbot --test trigger_plane -- --nocapture
    Expected: events durably staged before crash are replayed exactly once into acceptance on restart
    Evidence: .sisyphus/evidence/task-12-daemon-wireup-error.txt
  ```

  **Commit**: YES | Message: `feat(cli): wire persistent trigger supervisor into serve loop` | Files: `crates/chainbot/src/cli.rs`, `crates/chainbot/src/trigger.rs`, supervisor module, tests

- [x] 13. Add complete persistent session matrix

  **What to do**: Expand tests to cover session start/stop, durable ACK, backpressure, lease loss, restart recovery, fairness, process/wasm convergence, and supervisor reconcile. Include real process and wasm persistent runtime paths where practical.
  **Must NOT do**: Do not settle for happy-path-only session coverage. Do not leave crash-window semantics implicit.

  **Recommended Agent Profile**:
  - Category: `unspecified-high` — Reason: this is the highest-yield correctness net for the new lifecycle model
  - Skills: [`rust-router`] — test architecture for Rust runtime behavior
  - Omitted: [`qa`] — we need code-level Rust tests, not browser QA

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: [15] | Blocked By: [2,3,4,6,7,8,9,10,11,12]

  **References**:
  - Existing trigger tests: `crates/chainbot/tests/trigger_plane.rs`, `runtime_state_parity.rs`, `state_runtime_persistence.rs`, `end_to_end_vertical_slice.rs`
  - Test plan artifact: `~/.gstack/projects/feature-built-in-function/cyouguang-feature-built-in-function-eng-review-test-plan-20260326-023058.md`

  **Acceptance Criteria**:
  - [ ] Session matrix covers start/stop, durable ACK, backpressure, lease loss, restart recovery, fairness.
  - [ ] Process and wasm trigger supervision are both covered under the unified abstraction.
  - [ ] Crash-window semantics are verified, not just described.

  **QA Scenarios**:
  ```
  Scenario: full persistent session matrix
    Tool: Bash
    Steps: cargo test -p chainbot --test trigger_plane --test runtime_state_parity --test state_runtime_persistence --test end_to_end_vertical_slice -- --nocapture
    Expected: all persistent session lifecycle and recovery cases pass under one aggregated matrix
    Evidence: .sisyphus/evidence/task-13-session-matrix.txt

  Scenario: fairness and reconcile paths are stable under repeated runs
    Tool: Bash
    Steps: cargo test -p chainbot --lib external_trigger_supervisor -- --nocapture
    Expected: repeated hot-trigger and reconcile scenarios stay deterministic and bounded
    Evidence: .sisyphus/evidence/task-13-session-matrix-error.txt
  ```

  **Commit**: YES | Message: `test(trigger): add persistent session matrix` | Files: integration tests, e2e fixtures, supervisor unit tests

- [x] 14. Update examples, help, and user-facing docs

  **What to do**: Update process and wasm trigger examples, CLI help, `examples/README.md`, and any user-facing docs so the chosen lifecycle and backpressure semantics are discoverable from the repo and CLI.
  **Must NOT do**: Do not leave old short-lived trigger wording in persistent-session examples. Do not hide durable-ack/backpressure semantics from operators or AI agents.

  **Recommended Agent Profile**:
  - Category: `writing` — Reason: user-facing contract alignment
  - Skills: [`rust-router`] — keep CLI/docs aligned with implementation
  - Omitted: [`design-review`] — no UI here

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: [15] | Blocked By: [5,12,13]

  **References**:
  - `README.md`
  - `examples/README.md`
  - `docs/research/CHAINBOT_WASM_PLUGIN_RUNTIME_PROPOSAL.md`
  - `crates/chainbot/src/cli.rs`

  **Acceptance Criteria**:
  - [ ] Examples show daemon supervisor + persistent session semantics explicitly.
  - [ ] CLI help and catalog output match the implemented lifecycle split.
  - [ ] User-facing docs explain durable ACK and explicit backpressure semantics.

  **QA Scenarios**:
  ```
  Scenario: curated examples and help stay aligned
    Tool: Bash
    Steps: cargo test -p chainbot --test cli_surface --test catalog_surface
    Expected: docs/help-facing fixtures continue to match the implemented runtime surfaces
    Evidence: .sisyphus/evidence/task-14-docs.txt

  Scenario: examples validate under new lifecycle shape
    Tool: Bash
    Steps: cargo run -p chainbot -- validate
    Expected: curated process and wasm persistent trigger roots validate with the updated documentation model
    Evidence: .sisyphus/evidence/task-14-docs-error.txt
  ```

  **Commit**: YES | Message: `docs(trigger): document persistent supervisor lifecycle` | Files: README, examples, CLI help, research/design docs as needed

- [x] 15. Harden and clean final runtime behavior

  **What to do**: Remove transitional drift, update stale backlog/ignored fixtures, and ensure supervisor shutdown, staging drain, and fairness behavior are the only remaining lifecycle paths. No stale process/wasm divergence or placeholder supervisor code should remain.
  **Must NOT do**: Do not leave ignored flaky fixtures without explicit rationale or replacement. Do not leave duplicate codepaths for acceptance or session teardown.

  **Recommended Agent Profile**:
  - Category: `unspecified-high` — Reason: this is the final consistency sweep for a runtime/platform change
  - Skills: [`rust-router`] — final Rust runtime polish
  - Omitted: [`clean-code-reviewer`] — structural correctness matters more than style commentary here

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: [Final Verification] | Blocked By: [12,13,14]

  **References**:
  - current runtime/tests/modules touched by Tasks 1-14
  - ignored/stress fixtures in `crates/chainbot/tests/trigger_plane.rs`

  **Acceptance Criteria**:
  - [ ] No stale runtime/callback wording remains in code or docs.
  - [ ] Ignored backlog fixtures are either replaced with stable coverage or explicitly justified.
  - [ ] Process and wasm trigger session paths share one top-level orchestration model.

  **QA Scenarios**:
  ```
  Scenario: final trigger runtime verification set
    Tool: Bash
    Steps: cargo check -p chainbot && cargo test -p chainbot --lib --test config_loading --test catalog_surface --test node_plugin_host --test trigger_plane --test cli_surface --test end_to_end_vertical_slice
    Expected: the final persistent trigger implementation passes the full repository verification slice for this scope
    Evidence: .sisyphus/evidence/task-15-final-runtime.txt

  Scenario: no stale process/wasm lifecycle drift remains
    Tool: Bash
    Steps: cargo test -p chainbot --test catalog_surface --test cli_surface -- --nocapture
    Expected: process trigger vs wasm persistent trigger lifecycle differences are explicit and stable in CLI/user-facing surfaces
    Evidence: .sisyphus/evidence/task-15-final-runtime-error.txt
  ```

  **Commit**: YES | Message: `refactor(trigger): finalize persistent supervisor runtime` | Files: touched runtime modules, tests, docs

## Final Verification Wave (MANDATORY — after ALL implementation tasks)
> 4 review agents run in PARALLEL. ALL must APPROVE. Present consolidated results to user and get explicit "okay" before completing.
> **Do NOT auto-proceed after verification. Wait for user's explicit approval before marking work complete.**
> **Never mark F1-F4 as checked before getting user's okay.** Rejection or user feedback -> fix -> re-run -> present again -> wait for okay.
- [ ] F1. Plan Compliance Audit — oracle
- [ ] F2. Code Quality Review — unspecified-high
- [ ] F3. Real Manual QA — unspecified-high
- [ ] F4. Scope Fidelity Check — deep

## Commit Strategy
- Commit once per task when the task materially changes contract/runtime/test boundaries.
- Keep process/wasm convergence commits narrow; do not mix docs-only cleanup into contract/runtime commits.
- Hold README/example lifecycle wording changes until the runtime shape is already stable in code.

## Success Criteria
- Persistent wasm trigger sessions are daemon-owned and survive repeated serve turns without re-instantiation.
- Guest push ACK means durable staging succeeded.
- Explicit backpressure / lease-lost / shutting-down responses exist and are tested.
- `TriggerPlane` remains the sole acceptance/replay/dedup/cooldown authority.
- Process and wasm external triggers share one supervisor/session orchestration model.
- CLI/catalog/examples clearly distinguish process trigger lifecycle from wasm persistent trigger lifecycle.
- Full persistent session matrix passes without human intervention.
