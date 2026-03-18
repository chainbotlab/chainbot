# ChainBot V2.1.2 CLI Proposal

## Goal

针对 `v2.1.2`，将 `chainbot` CLI 从“最小可用命令集合”提升到“可发现、可自举、可观察”的操作面，并把 root 选择统一收敛到环境变量驱动的契约，重点覆盖三个方向：

- 新增 `status`，可读取 trigger / workflow 的运行状态与最近活动。
- 新增 `init`，可快速初始化一个可校验的 ChainBot root。
- 重做 `help`，把帮助系统从静态说明提升为面向 agent / operator 的 command skill。

方案必须遵守当前实现与设计约束：

- root layout 与加载顺序仍然以 `docs/design/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md` 为准。
- CLI 边界仍以 `crates/chainbot/src/cli.rs` 为唯一入口。
- root config 仍沿用两阶段加载：先读 `config/root.toml`，再应用 path overrides。

## Current Baseline

当前 CLI 命令面定义在 `crates/chainbot/src/cli.rs`，已有命令如下：

- `help`
- `validate`
- `run`
- `serve`
- `list-runs`

当前持久化状态能力已经存在，不需要为 `status` 额外引入新的 runtime plane：

- `RunRecordSummary` 提供 workflow run 的持久化摘要。
- `WorkflowRuntimeLogEntry` 提供运行日志。
- `TriggerEventRecord` 提供 trigger 接受记录。
- `CoordinationStore` 提供 `serve` lease 的占用状态。

这意味着 `status` 的第一版应优先走“非执行式状态快照 + 合成视图”，而不是新增一套实时状态协议。

这里的“非执行式”比“严格只读”更准确：在当前 `state.rs` 能力下，状态查询流程可能触发 recovery helper 或 coordination store 初始化，因此第一版目标是不启动 workflow execution，而不是承诺完全零副作用的底层 I/O 行为。若后续补出 no-recovery query API，可再把语义收紧为严格只读。

## CLI Design Direction

参考 `cli-agent-design`，`v2.1.2` 的 CLI 设计目标如下：

- 渐进式帮助发现：`chainbot help` 给出技能目录，`chainbot help <command>` 给出命令级 skill。
- 错误即导航：错误文案不仅说明失败，还给出下一步动作。
- 人类默认可读，机器显式可读：默认输出适合终端阅读；需要结构化消费时，显式提供 `--json`。
- 单命令职责清晰：`list-runs` 保留机器化 run 列表职责，`status` 负责操作态观察。

## Proposed Command Surface

```text
chainbot help [command]
chainbot init [--force] [--json]
chainbot status [--json]
chainbot validate
chainbot list-runs
chainbot run
chainbot serve
```

说明：

- `list-runs` 保留，定位为低层 JSON run summary 查询。
- `status` 新增，定位为高层运行态概览。
- `init` 新增，定位为 root bootstrap。
- `help` 保留命令名，但语义升级为 skill-oriented help。
- root 路径统一从 `CHAINBOT_CONFIG_DIR` 读取；变量未设置或为空时回退到 `~/.chainbot`。

## Command Proposal: `status`

### Intent

`status` 的目标不是启动执行，而是回答以下操作问题：

- 当前 root 是否有效、是否可加载。
- `serve` 当前是否被占用。
- 当前有哪些 workflow / trigger 被配置。
- 最近有哪些 workflow run，最新状态是什么。
- 最近哪些 trigger 产生活动。

### Data Sources

`status` 第一版只基于已有数据源构建快照：

- 配置面：`RootDefinitionBundle::load`
- 运行摘要：`FileBackedStateStore::list_run_summaries`
- trigger 记录：`FileBackedStateStore::load_trigger_records`
- lease 状态：`CoordinationStore`

### Output Model

默认输出为 human-readable，多段摘要；`--json` 输出稳定结构。

关于无效 root，`v2.1.2` 应明确沿用当前 CLI 的 fail-fast 风格：如果 root config 或 definition bundle 无法加载，`status` 直接返回 validation error，而不是同时支持 partial JSON `valid=false`。

建议 JSON 结构：

```json
{
  "root": {
    "path": "/tmp/demo-root",
    "profile": "dev",
    "valid": true
  },
  "serve": {
    "state": "idle",
    "owner": null,
    "expires_at_ms": null
  },
  "workflows": [
    {
      "workflow_id": "wf-alpha",
      "last_run_status": "failed",
      "last_run_id": "manual-wf-alpha-1710300000000",
      "last_started_at_ms": 1710300000000,
      "last_finished_at_ms": 1710300005000
    }
  ],
  "triggers": [
    {
      "trigger_id": "tr-market",
      "enabled": false,
      "workflow_id": "wf-alpha",
      "last_event_id": null,
      "last_accepted_at_ms": null
    }
  ],
  "summary": {
    "workflow_count": 1,
    "trigger_count": 1,
    "run_count": 3,
    "running_run_count": 0
  }
}
```

### Human Output

建议默认输出：

```text
Root
  path: /tmp/demo-root
  profile: dev
  serve: idle

Workflows
  wf-alpha  failed  last_run=manual-wf-alpha-1710300000000

Triggers
  tr-market  disabled  workflow=wf-alpha  last_event=none

Summary
  workflows=1 triggers=1 runs=3 running=0
```

### Scope Boundary

`status` 第一版不做以下事情：

- 不提供实时 node-level 进度追踪。
- 不引入常驻状态采样器。
- 不替代 `list-runs` 的完整 run 明细查询。
- 不承担 root repair 或 state repair 的管理职责。

### Serve Lease Semantics

`status` 不能把数据库里是否存在 lease row 直接等价为 `serve` 是否活跃。第一版必须复用与 `try_acquire_serve_lease` 一致的 stale-owner 判定语义，否则 operator 在 `status` 里看到的结果会和后续 `serve` 的决策不一致。

建议对外统一为三态：

- `idle`: 当前没有有效 lease owner
- `active`: 当前存在有效 lease owner
- `stale`: 存在历史 lease 记录，但 owner 已不活跃，不应阻塞下一次 `serve`

### Required Internal Changes

- `crates/chainbot/src/cli.rs`
  - 新增 `CliCommand::Status`
  - 新增 `HelpTopic::Status`
  - 新增 `execute_status`
- `crates/chainbot/src/state.rs`
  - 新增读取 serve lease 快照的查询函数，避免 CLI 直接拼 SQL
  - 新增实现需复用现有 stale-owner 判定语义
  - 可新增“latest run by workflow”与“latest trigger record by trigger”的聚合 helper
- `crates/chainbot/tests/cli_surface.rs`
  - 新增 `status` 基础输出测试
- `crates/chainbot/tests/state_runtime_persistence.rs`
  - 新增 lease 查询与状态聚合 helper 测试

## Command Proposal: `init`

### Intent

`init` 的目标是零到一创建一个“立即可 validate 的 root”，而不是生成完整业务样例。

### Default Behavior

在 `CHAINBOT_CONFIG_DIR` 指向目标 root，或默认 root 为目标路径时，执行 `chainbot init` 后创建：

```text
<root>/
|- config/
|  `- root.toml
|- workflows/
|- triggers/
|- plugins/
|  |- manifests/
|  `- bin/
|- secrets/
`- state/
```

默认 `config/root.toml` 建议内容：

```toml
manifest_version = "2.0.0"
profile = "default"
secret_refs = []

[paths]
workflows_dir = "workflows"
triggers_dir = "triggers"
plugins_dir = "plugins"
secrets_dir = "secrets"
state_dir = "state"

[plugins]
manifest_globs = ["plugins/manifests/*.toml"]

[runtime_defaults]
timezone = "UTC"
```

### Behavior Rules

- 默认幂等：如果目标目录不存在则创建；如果为空目录则补齐结构。
- 默认保守：若关键文件已存在且内容可能被覆盖，则返回冲突错误。
- `--force` 才允许覆盖由 `init` 管理的模板文件。
- `--json` 输出创建结果与冲突明细，方便脚本集成。

`--force` 的覆盖边界应保持很窄：`v2.1.2` 只建议覆盖 CLI-managed template file，例如 `config/root.toml`。对于 `workflows/`, `triggers/`, `plugins/`, `secrets/`, `state/` 下的已有业务内容，默认仍返回冲突，而不是递归覆盖。

### Why Minimal Bootstrap

不建议在第一版 `init` 默认生成 workflow / trigger sample，原因是：

- 当前设计文档把 workflow 与 trigger 视为业务包，而不是 CLI 自带样板资产。
- 最小 root 已可被 `validate` 与未来 `status` 正确消费。
- 样例模板会带来额外的长期维护成本与版本耦合。

若后续确有需要，可在 `v2.1.2+` 增加 `chainbot init --example basic`。

### Required Internal Changes

- `crates/chainbot/src/cli.rs`
  - 新增 `CliCommand::Init`
  - 新增 `HelpTopic::Init`
  - 新增 `execute_init`
  - 扩展参数解析支持 `--force` 和 `--json`
- 可选新增 `crates/chainbot/src/init.rs`
  - 封装 bootstrap tree 和模板写入逻辑，避免 `cli.rs` 继续膨胀
- `crates/chainbot/tests/cli_surface.rs`
  - 新增 `init` 成功创建 root 测试
  - 新增 `init` 冲突与 `--force` 行为测试

## Command Proposal: `help` As Skill

### Intent

当前 `help` 还是静态 usage 文本。`v2.1.2` 应把它升级为“命令技能系统”，让每个命令都能回答：

- 什么时候用我。
- 我会读什么。
- 我会写什么。
- 常见失败后下一步怎么做。
- 相关命令是什么。

### Help Levels

#### Level 1: General Skill Index

`chainbot help` 输出建议：

```text
ChainBot command skills

  init       Bootstrap a minimal ChainBot root.
  status     Inspect trigger/workflow runtime status.
  validate   Validate config and package contracts.
  list-runs  Print persisted run summaries as JSON.
  run        Execute one manual workflow run.
  serve      Drain one trigger snapshot under a serve lease.

Use `chainbot help <command>` for command-specific guidance.
```

#### Level 2: Command Skill

`chainbot help status` 输出建议结构：

```text
status - Inspect runtime state without executing workflows

Use when:
  - you want to know whether serve is active
  - you want the latest workflow run result
  - you want trigger activity without opening state files

Reads:
  - configured root config
  - configured workflow packages
  - configured trigger packages
  - configured state runs directory
  - configured trigger record directory
  - configured coordination store

Does not mutate:
  - workflows
  - triggers
  - run state

Examples:
  chainbot status
  CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot status
  chainbot status --json

See also:
  validate, list-runs, serve
```

### Error As Navigation

帮助与错误文案要统一升级：

- Unknown command -> 给出最接近命令和 `chainbot help`
- Missing root config -> 引导检查 `CHAINBOT_CONFIG_DIR` 或在目标 root 执行 `chainbot init`
- Existing root conflict during `init` -> 提示 `--force` 或改路径

示例：

```text
Unsupported command `stats`.
Did you mean `status`?
Run `chainbot help` to see available command skills.
```

## Recommended Output Strategy

为兼顾 operator 与 automation，建议统一策略：

- `help`: human only
- `init`: human default, `--json` optional
- `status`: human default, `--json` optional
- `list-runs`: 保持 JSON only

这样可以避免 `status` 和 `list-runs` 的职责重叠。

## Suggested Implementation Order

### Phase 1

- `help` skill 化
- `status` human output + `--json`

理由：

- 复用现有只读状态面，改动风险最低。
- 先把可观察性和可发现性补齐，能直接改善 CLI 使用体验。

### Phase 2

- `init` 最小 bootstrap
- `validate` / `status` 与 `init` 的联动提示

理由：

- `init` 涉及文件写入与冲突策略，需要更严格的幂等语义。

## Acceptance Criteria

### `status`

- 可在有效 root 上输出 workflow / trigger / serve / run 摘要。
- 无 run 数据时输出稳定空状态，而不是错误。
- `--json` 输出稳定字段结构。

### `init`

- 在空目录中生成可通过 `chainbot validate` 的最小 root。
- 不带 `--force` 时不覆盖已存在模板文件。
- 输出明确说明创建了哪些目录与文件。

### `help`

- `chainbot help` 显示命令技能目录。
- `chainbot help <command>` 显示 use-when / reads / mutates / examples / see-also。
- 常见 usage error 文案包含下一步动作建议。

## Risks And Tradeoffs

### `status` 数据精度

由于当前状态面主要是持久化摘要，`status` 第一版反映的是“最近已知状态”，不是实时执行图。这是有意 tradeoff，用更低复杂度换更快落地。

同样，第一版应避免把“基于现有状态构建快照”误写成“严格无副作用读取”，直到 state 层提供专门的 no-recovery query API。

### `init` 的模板边界

若模板过重，会让 CLI 与业务样板绑定；若模板过轻，则首次上手仍需文档辅助。当前建议先选择轻模板，并把样例生成留给后续版本。

### `help` 与 README 的重复

`help` skill 化后，README 只保留命令总览；命令细节应以 CLI 内 help 为准，避免双份文案长期漂移。

## File Hotspots

- `crates/chainbot/src/cli.rs`
- `crates/chainbot/src/state.rs`
- `crates/chainbot/src/errors.rs`
- `crates/chainbot/tests/cli_surface.rs`
- `crates/chainbot/tests/state_runtime_persistence.rs`
- `README.md`
- `docs/implementation/` 下未来对应实现记录文档

## Recommendation

`v2.1.2` 建议定义为一次“CLI usability release”，而不是运行时架构升级。优先通过 `status + init + help-as-skill` 三件事，把 ChainBot 从“能跑”推进到“能发现、能观察、能起步”，并用环境变量统一 root 选择入口。
