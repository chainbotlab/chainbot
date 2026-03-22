# ChainBot DB-Primary Runtime Implementation

## Goal

将 ChainBot 主运行路径从 file-authoritative 状态切换到 DB-primary：

- local mode 以 SQLite 为权威状态
- postgres mode 以 PostgreSQL 为权威状态
- `status` / `list-runs` / `run` / `serve` / trigger execution 不再依赖 `summary.json`、`records/*.json`、`checkpoint.json`、`snapshot.json` 作为主路径真相来源

## Implemented Changes

- root config schema 扩展 storage contract：
  - `storage.mode = "local" | "postgres"`
  - `storage.local.database_path`（local mode required）
  - `storage.postgres.database_url`（postgres mode required）
  - `storage.raw_debug` shape 保留，默认可关闭
- 新增 `src/state_db.rs`：
  - 统一 DB runtime store（SQLite + PostgreSQL）
  - 统一 runtime tables：
    - `run_summaries`
    - `workflow_runtime_logs`
    - `trigger_event_records`
    - `trigger_checkpoints`
    - `trigger_snapshots`
    - `serve_leases`
- CLI 主路径改为 DB-primary：
  - `status` 从 DB 读取 run summaries + trigger snapshots + serve lease
  - `list-runs` 从 DB 读取 run summaries
  - `run` / `serve` 执行期间写入 DB run summary 与 workflow log
- TriggerPlane 主路径改为 DB-primary：
  - accepted trigger record 写入 `trigger_event_records`
  - checkpoint 写入 `trigger_checkpoints`
  - snapshot 写入 `trigger_snapshots`
  - dedup/cooldown 通过 DB trigger history 查询保持 restart-safe
- legacy file-backed state code 保留在 `state.rs`，但不再是上述命令的主运行路径

## Validation

- `rtk cargo check -p chainbot`
- `rtk cargo test -p chainbot --test config_loading --test trigger_plane --test cli_surface`
- `rtk cargo build -p chainbot`

## Updated Test Coverage

- `config_loading.rs`
  - 新增 storage mode required-field validation
  - fixture root configs 补充 storage section
- `trigger_plane.rs`
  - trigger record persistence assertion 改为 DB table validation
  - restart dedup/cooldown test 改为 DB history contract
- `cli_surface.rs`
  - status/list-runs related setup 改为 DB runtime store
  - fixture reset 增加 `state/runtime.sqlite3` 清理

## Notes

- 本次明确不提供 legacy file state 到 DB-primary runtime 的迁移合同；旧 file-backed state 仅保留兼容、检查、与测试用途，不作为升级路径的一部分。
- raw debug capture 保持默认关闭，仅保留配置形状与运行时可扩展接口。

## Follow-up Hardening

- `TriggerPlane` 的 legacy `StateLayout` 打开路径已改为显式测试兼容入口，避免新主路径代码继续从 file-backed authority 入口接入。
- 新增 `runtime_state_parity.rs`，对 SQLite local mode 与 PostgreSQL mode 的 lease、run summary、trigger snapshot/checkpoint 与 trigger history 语义做共享测试。
