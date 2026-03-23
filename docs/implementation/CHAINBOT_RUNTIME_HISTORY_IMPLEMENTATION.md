# ChainBot Runtime History Implementation

## Goal

补齐 `TODOS.md` 中 runtime history 相关的两项落地工作：

- 为 `status --json`、recent history observation 和 trigger hot path 提供稳定的性能/soak guardrail
- 为 `run_summaries`、`workflow_runtime_logs`、`trigger_event_records` 提供 operator-facing retention + archival contract

## Implemented Changes

- CLI 新增 `observe` 命令：
  - 默认输出最近的 runs / workflow logs / trigger events
  - 支持 `--json`
  - 支持 `--limit`、`--trigger-id`、`--run-id`
  - 输出 archived history 计数，帮助 agent/operator 判断 retention 是否已搬迁旧数据
- root config storage contract 新增 optional `storage.retention`：
  - `enabled`
  - `run_retention_days`
  - `workflow_log_retention_days`
  - `trigger_event_retention_days`
- DB runtime schema 新增 archived tables：
  - `archived_run_summaries`
  - `archived_workflow_runtime_logs`
  - `archived_trigger_event_records`
- `RuntimeStateStore` 新增：
  - recent read APIs for run summaries / workflow logs / trigger events
  - archive count reads
  - retention application that moves expired active rows into archived tables
- runtime schema 额外补充了热路径 index：
  - recent run summary reads
  - recent workflow log reads
  - recent trigger event reads
- runtime load path 在执行型命令里接入 retention hook，使长期运行 root 在正常使用过程中可以持续 compact/archival
- 新增稳定 guardrail harness，而不是脆弱微基准：
  - query-plan assertions for SQLite hot-path reads
  - repeated read-only observation loop assertions to catch accidental write side effects

## Contracts Preserved

- `status` 仍然只读，不执行 workflow，不消费 trigger snapshot
- `list-runs` 仍然输出 raw persisted run summaries
- dedup/cooldown readiness 仍然基于 active `trigger_event_records` 判断，不把 archive tables 混入热路径协调语义
- archived tables 只承担冷历史保留，不改变最近 observation window 的输出结构

## Validation

- `cargo test -p chainbot --test cli_surface --test runtime_state_parity`
- `cargo test -p chainbot --test runtime_guardrails`

## Test Coverage Added or Updated

- `cli_surface.rs`
  - general help now advertises `observe`
  - `help observe` documents the history-observation contract
  - `observe --json` returns recent run/log/event history plus archive counts
- `runtime_state_parity.rs`
  - SQLite/PostgreSQL backends both archive expired history consistently
- `runtime_guardrails.rs`
  - SQLite query plans keep using dedicated hot-path indexes
  - repeated observation loops stay read-only and non-amplifying
