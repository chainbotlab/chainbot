# ChainBot State Read Model Implementation

## Goal

将运行态读取路径从“全量历史扫描”收敛到更窄的摘要/快照面，同时保持 `state/runs` 与 `state/triggers` 的文件真相来源不变。

## Implemented Changes

- `list-runs` 现在直接读取 committed `state/runs/<run_id>/summary.json`，不再隐式执行 runtime recovery。
- run summary 枚举现在只遍历 `state/runs/*/summary.json`，不再递归扫描 `workflow-logs/*.json`。
- workflow log append 现在使用每个 run 的 `workflow-log-sequence.cursor` 作为 advisory cursor，并在 cursor 缺失时按文件名推导最大序号，而不是反序列化现有日志文件。
- trigger 状态新增 `state/triggers/<trigger_id>/snapshot.json`：
  - 记录最近 accepted event 摘要
  - 记录最近 accepted sequence
  - 记录 accepted event identities 与仍未过期的 dedup/cooldown tokens
- `status` 现在读取 committed trigger snapshots，而不是加载全部 trigger records。
- `TriggerPlane::open` 现在先读取 trigger snapshots，再仅重放 `last_sequence` 之后的 delta records；若 snapshot 缺失，则回退到该 trigger 的全量 records。

## Contracts Preserved

- `state/runs/<run_id>/summary.json` 仍然是 persisted run status 的权威来源。
- `state/runs/<run_id>/workflow-logs/<sequence>.json` 仍然保持 append-only。
- `state/triggers/<trigger_id>/records/*.json` 仍然是 trigger durable truth。
- `state/triggers/<trigger_id>/checkpoint.json` 仍然只承担 resume position 语义。
- `state/triggers/<trigger_id>/snapshot.json` 是 derived state，可由 accepted records 重新构建。
- `coordination.sqlite3` 仍然只是可重建 coordination store，而不是 run/trigger history 的真相来源。

## Validation

- `rtk cargo test -p chainbot --tests`
- `rtk cargo build -p chainbot`

## Test Coverage Added or Updated

- `state_runtime_persistence.rs`
  - trigger snapshot roundtrip and staged recovery
  - workflow log append remains monotonic even with stale cursor state
- `cli_surface.rs`
  - `status --json` reads trigger snapshots without trigger-record side effects
  - `list-runs` stays read-only and does not promote staged summaries
- `trigger_plane.rs`
  - accepted run sequence continues from persisted trigger snapshot after restart
