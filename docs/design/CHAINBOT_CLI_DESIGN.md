# ChainBot CLI Design

## Goal

定义 ChainBot CLI 的稳定命令面、帮助系统、状态观察语义和错误导航契约，使 operator 与 agent 都能通过统一命令面完成 root 校验、状态观察与执行操作。

## Command Surface

当前稳定命令面如下：

```text
chainbot help [command]
chainbot version
chainbot init
chainbot status [--json]
chainbot trigger <list|enable|disable> [trigger-id]
chainbot validate
chainbot list-runs
chainbot run
chainbot serve
```

全局规则：

- CLI 通过 `CHAINBOT_CONFIG_DIR` 环境变量解析 ChainBot root。
- 当 `CHAINBOT_CONFIG_DIR` 未设置或为空字符串时，默认 root 为 `~/.chainbot`。
- `CHAINBOT_CONFIG_DIR` 必须指向一个完整 ChainBot root，而不是单个配置文件。
- `status` 支持 `--json`；其他命令保持既有输出模式。
- `trigger` 负责持久化变更 trigger package 的 `enabled` 字段，不引入独立运行态开关面。

## Command Roles

### `help`

- 提供命令技能目录与命令级帮助。
- `chainbot help` 返回完整命令目录。
- `chainbot help <command>` 返回该命令的 use-when、I/O 边界、示例和相关命令。

### `version`

- 输出当前运行中的 ChainBot CLI 版本号。
- `chainbot version` 与 `chainbot --version` 等价。

### `status`

- 提供非执行式状态快照。
- 不启动 workflow execution。
- 不消费 trigger snapshot。
- 不执行 runtime recovery。
- 输出 root、serve lease、workflow 最近运行状态、trigger 最近活动与摘要计数。

### `validate`

- 校验 root layout、root config、workflow package、trigger package 与 plugin manifest contract。
- 不启动执行。

### `trigger`

- 提供 trigger package 的 operator 级查看与开关操作。
- `chainbot trigger list` 输出当前配置面里的 trigger 列表。
- `chainbot trigger enable <trigger-id>` 将目标 trigger package 的 `enabled` 置为 `true`。
- `chainbot trigger disable <trigger-id>` 将目标 trigger package 的 `enabled` 置为 `false`。
- 开关结果在下一次 `status`、`validate`、`serve` 或任意重新加载定义的命令中生效。
- 不修改运行态 trigger record、dedup/cooldown coordination 或 serve lease。

### `init`

- 初始化一个最小可校验的 ChainBot root。
- 若目标 root 不存在则创建默认 canonical 目录布局。
- 若 `chainbot.toml` 缺失则写入默认模板。
- `init` 只引导当前稳定 root contract：`chainbot.toml`、`workflows/`、`triggers/`、`plugins/<plugin_id>/config.toml`、`secrets/`、`state/`。
- `init` 不创建旧版 shared manifest 目录，也不把 legacy root-config 路径视为稳定 bootstrap 输出的一部分。
- 已存在的目录与 root config 默认按幂等方式复用，不递归覆盖业务内容。

### `list-runs`

- 输出持久化 run summary 的低层 JSON 列表。
- 不承担高层操作态聚合职责。

### `run`

- 执行一次手动 workflow run。
- 仅适用于可唯一确定 workflow 的 root。
- 当 root 中只有一个 workflow package 时，直接执行该 workflow。
- 当 root 中存在多个 workflow package 时，若只能推导出一个未被任何 subflow 引用的 top-level workflow，则执行该 workflow。
- 若 top-level workflow 仍然不唯一，则返回 usage error，而不是猜测执行目标。

### `serve`

- 在 serve lease 保护下评估一次 trigger snapshot。
- 负责 runtime recovery、trigger-plane coordination 与 accepted run 执行。

## Help System Contract

帮助系统采用两层结构：

### General Help

`chainbot help` 或 `chainbot --help` 返回：

- 当前可用命令目录
- 每个命令的一行职责说明
- 引导用户继续使用 `chainbot help <command>`

### Command Help

`chainbot help <command>` 返回稳定结构化帮助信息，包含：

- `Use when`
- `Usage`
- `Reads` 或 `Writes`
- `Does not execute` 或等价约束说明
- `Outputs`
- `Root resolution`（当命令依赖 root 时）
- `Config examples`（当命令依赖配置 contract 时）
- `Failure navigation`
- `Examples`
- `See also`

帮助文本描述的是命令 contract，而不是实现细节。

当命令依赖 root / workflow / trigger / plugin contract 时，帮助系统应直接内嵌 canonical 示例片段，避免 operator 或 agent 需要跳转到源码或测试夹具才能理解配置形状。

## Status Contract

### Data Sources

`status` 通过以下稳定数据面构建状态快照：

- resolved root config
- resolved workflow packages
- resolved trigger packages
- committed run summaries
- committed trigger records
- existing coordination database

### Output Modes

`status` 有两种输出模式：

- 默认 human-readable summary
- `--json` structured snapshot

### JSON Shape

`status --json` 返回稳定顶层结构：

```json
{
  "root": {
    "path": "/tmp/demo-root",
    "profile": "basic"
  },
  "serve": {
    "state": "idle",
    "owner": null,
    "expires_at_ms": null
  },
  "workflows": [
    {
      "workflow_id": "wf-alpha",
      "last_run_status": null,
      "last_run_id": null,
      "last_started_at_ms": null,
      "last_finished_at_ms": null
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
    "run_count": 0,
    "running_run_count": 0
  }
}
```

字段规则：

- `root.path` 是当前有效 root 路径。
- `root.profile` 来自 root config；缺失时为 `null`。
- `serve.state` 取值为 `idle`、`active` 或 `stale`。
- workflow 与 trigger 列表来自当前配置定义，不依赖是否已有运行记录。
- 最近活动字段在无持久化记录时返回 `null`。

### Human Output

默认文本输出必须包含以下段落：

- `Root`
- `Workflows`
- `Triggers`
- `Summary`

文本输出优先面向终端 operator 阅读，不要求与 JSON 完全同构，但必须表达相同语义。

### State Boundaries

`status` 是 non-executing snapshot，具有以下约束：

- 不触发 workflow execution
- 不触发 trigger collection
- 不修改 incomplete run 状态
- 不追加 workflow log
- 不创建或迁移 coordination database；若数据库不存在，则 serve lease 视为 `idle`

`status` 仅观察已提交 durable state，不观察 staged-but-uncommitted recovery candidates。

## Serve Lease Semantics

`serve` 状态对外统一为三态：

- `idle`: 没有有效 lease row，或 coordination database 不存在
- `active`: lease row 存在、未过期，且 owner 被视为存活
- `stale`: lease row 存在，但该 lease 不应继续阻塞下一次 `serve`

lease 存活判定必须与实际 lease acquisition 语义一致，避免 `status` 与 `serve` 给出冲突结论。

## Error Navigation Contract

CLI 错误不仅描述失败，还必须给出下一步动作。

稳定规则：

- unsupported command -> 尽量给出最近命令建议，并引导 `chainbot help`
- unexpected argv token -> 指出命令路径与参数位置，避免用户猜测是哪一个 token 触发失败
- missing root directory -> 引导设置 `CHAINBOT_CONFIG_DIR` 或检查默认 `~/.chainbot` 是否指向有效 root
- missing root config file -> 引导检查 `CHAINBOT_CONFIG_DIR` 指向的 root，确认 `chainbot.toml` 存在且可加载
- invalid config -> 返回 validation error，不降级为 partial status payload；当失败来自 TOML decode 时，错误应包含 file path、line/column 与 source snippet

## Output Strategy

- `help`: human-readable only
- `version`: human-readable only
- `init`: human-readable bootstrap result
- `status`: human-readable by default, `--json` optional
- `trigger`: human-readable list or mutation result
- `validate`: human-readable success / validation failure
- `list-runs`: JSON output
- `run`: human-readable execution result
- `serve`: human-readable execution result

这组策略用于保持 `status` 与 `list-runs` 的职责分离：

- `status` 面向操作态观察
- `list-runs` 面向原始持久化摘要消费

## Root Assumptions

CLI 假设 ChainBot root 继续遵守 `docs/design/CHAINBOT_TRIGGER_WORKFLOW_CONFIG_DESIGN.md` 中定义的加载顺序与目录结构：

- `chainbot.toml`
- `workflows/*/config.toml`
- `triggers/*/config.toml`
- `plugins/<plugin_id>/config.toml`

`status`、`trigger`、`validate`、`run` 与 `serve` 都基于该 root contract 工作，并统一遵守 `CHAINBOT_CONFIG_DIR` 优先、`~/.chainbot` 回退的 root 解析顺序。

## Change Triggers

出现以下任一情况时，应更新此文档：

- CLI command surface 变化
- `status` JSON shape 变化
- help system structure 变化
- serve lease 对外状态语义变化
- default output strategy 变化
