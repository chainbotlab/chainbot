# ChainBot Ingress Trigger Implementation

## Current Implementation Location

Ingress inbox persistence and DB API are now owned by `infrastructure::state`.
The legacy root shim files (`src/state_db.rs`, `src/state.rs`, `src/cli.rs`) were removed in final tree cleanup.
Current ownership is ingress runtime in `ingress/`, state persistence in `infrastructure::state`, and daemon orchestration in `app::runtime`.
The current ownership map is documented in `crates/chainbot/src/AGENTS.md`.

## Goal

为 ChainBot 新增 listener-backed builtin trigger runtime，使 `webhook` 与 `websocket` trigger 可以在 `serve` daemon 持有 lease 时按配置启动、接收外部输入，并继续复用现有 trigger-plane 的 accepted-event 语义。

## Implemented Changes

- crate `chainbot` 新增 `src/ingress/` 子系统：
  - ingress params contract
  - desired listener reconciliation
  - durable inbox drain helper
  - lease-bound ingress supervisor
  - webhook / websocket transport handlers
- builtin trigger registry 新增 `webhook` 与 `websocket` subtype：
  - 仍通过 trigger-local `[params]` 配置
  - `validate()` 负责参数约束
  - `emit()` 返回空集，listener 运行态不走 poll emitter 路径
- `config.rs` 在 bundle 校验阶段接入 ingress desired-state 构建：
  - enabled ingress trigger route collision 提前失败
  - 继续保持 trigger-local 配置，而不是 root-global ingress config
- `infrastructure::state` 新增 durable ingress inbox record 及 DB API（原 `src/state_db.rs`，已删除）：
  - append inbox row
  - list pending rows by trigger
  - mark processed
- `app::cli` 的 `serve` daemon loop（其原 root-shim位置 `src/cli.rs` 已删除）接入 ingress supervisor：
  - config reload 后 reconcile desired listeners
  - `serve_once_with_lease()` drain ingress inbox
  - drain 后仍通过 `TriggerPlane` 归一化 accepted events
- websocket runtime 进一步补齐：
  - idle timeout 现在在 message loop 中执行真实连接关闭
  - active connection accounting 从全局计数收敛为 per-listener state
  - live websocket integration tests 覆盖 message ingress、connection limit、idle close

## Contracts Preserved

- `TriggerPlane::normalize_emission()` 仍是 accepted trigger event 的唯一写入口。
- dedup / cooldown / trigger snapshot / accepted history 仍保持在现有 trigger-plane + runtime-state 边界。
- ingress inbox 只承担 pre-acceptance staging，不承担 accepted history 真相源。
- `trigger enable|disable` 仍是 operator-facing 唯一开关；只有 enabled ingress triggers 才会启动 listener。
- accepted ingress work 现在通过 persisted `trigger_event_records` 做 restart replay：若 accepted record 已存在但对应 `run_summary` 尚不存在，下一次 `serve` loop 会重建 `TriggerRunRequest` 并执行。
- 该 replay 只覆盖“accepted but never started”窗口；已有 `run_summary` 的 runs 不会通过 replay 重新执行。

## Current Scope

- `webhook`：JSON body ingress, optional header-token auth, request size limit, optional content-type guard, optional idempotency header.
- `websocket`：text JSON message ingress, optional header-token auth, message size limit, connection count limit.
- 不包含 outbound websocket、broadcast、binary frame 或 shared root-level ingress policy。

## Validation

- `cargo check -p chainbot`
- `cargo test -p chainbot build_desired_ingress_state_collects_enabled_listener_triggers -- --exact`
- `cargo test -p chainbot runtime_state_backends_share_core_semantics -- --exact`
- `cargo test -p chainbot help_validate_includes_config_examples -- --exact`
- `cargo test -p chainbot help_serve_includes_ingress_trigger_examples -- --exact`
- `cargo test -p chainbot --test ingress_runtime webhook_trigger_accepts_live_post -- --exact`
- `cargo test -p chainbot --test ingress_runtime websocket_trigger_accepts_live_message_and_enforces_connection_limit -- --exact`
- `cargo test -p chainbot --test ingress_runtime websocket_trigger_closes_idle_connections -- --exact`
- `cargo test -p chainbot --test ingress_runtime accepted_trigger_record_without_run_summary_replays_on_restart -- --exact`
- `cargo test -p chainbot runtime_state_backends_share_core_semantics -- --exact`

## Follow-up Gaps

- 增加端到端 webhook / websocket listener integration tests。
- 补充 CLI help / design docs 中的 canonical ingress trigger examples。
- 细化 structured ingress error logging 和 richer operator-facing diagnostics for listener failures.

## 2026-03-27 Final-Wave Update

- The previously referenced root shim files (`src/state_db.rs`, `src/state.rs`, `src/cli.rs`) were removed during final tree cleanup.
- Ingress ownership remains unchanged: ingress runtime in `ingress/`, state persistence in `infrastructure::state`, and daemon orchestration in `app::runtime`.
