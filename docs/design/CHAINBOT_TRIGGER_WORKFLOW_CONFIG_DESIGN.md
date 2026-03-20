# ChainBot Trigger Workflow Config Design

## 目标

定义 ChainBot 根目录下 `workflow` 与 `trigger` 的最终稳定配置结构，明确二者边界、目录布局、加载规则和校验约束。

## 设计边界

- `workflow` 是独立可运行的编排单元
- `trigger` 是外部事件到 workflow run request 的适配单元
- `plugin` 是执行宿主与扩展能力的注册单元
- `root config` 是根级路径、默认值和宿主策略入口

`workflow` 不依赖 `trigger` 才能执行。CLI 手动运行、测试运行和子流程调用都直接面向 `workflow`。`trigger` 仅用于 `serve` 场景下把外部事件转换为 `workflow_id + input payload`。

## 根目录布局

```text
<root>/
|- chainbot.toml
|- workflows/
|  |- e2e/
|  |  |- config.toml
|  |  |- scripts/
|  |  `- assets/
|  `- rebalance/
|     `- config.toml
|- triggers/
|  |- manual-e2e/
|  |  `- config.toml
|  `- market-rebalance/
|     |- config.toml
|     |- scripts/
|     |- assets/
|     `- plugins/
|- plugins/
|  |- manifests/
|  `- bin/
|- secrets/
`- state/
```

## 加载顺序

1. `chainbot.toml`
2. `workflows/*/config.toml`
3. `triggers/*/config.toml`
4. `plugins/manifests/*.toml`

兼容性说明：loader 优先读取 `<root>/chainbot.toml`，当该文件缺失时回退读取旧路径 `<root>/config/root.toml`。若两个文件同时存在，则 `<root>/chainbot.toml` 作为唯一生效配置源。

## `chainbot.toml`

`chainbot.toml` 只承载根级稳定配置：

- manifest version
- running ChainBot version
- root profile
- path overrides
- runtime defaults
- plugin discovery settings
- secret references
- all path settings must stay within `<root>` and use root-relative paths

示例：

```toml
manifest_version = "2.0.0"
chainbot_version = "2.1.4"
profile = "prod"
secret_refs = ["secret://ops/slack/webhook#token"]

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

## `workflows/<workflow_id>/config.toml`

workflow manifest 定义独立运行所需的全部内容：

- workflow identity
- runtime defaults
- node DAG
- package-local resources
- subflow contracts

workflow manifest 不定义 trigger。

示例：

```toml
[workflow]
manifest_version = "2.0.0"
id = "e2e"
name = "e2e"
description = "End-to-end verification workflow"

[runtime.defaults]
symbol = "BTCUSDT"
environment = "staging"

[[nodes]]
id = "script-node"
kind = "builtin"
plugin = "builtin.script"
operation = "python:scripts/worker.py"
depends_on = []

[[nodes.inputs]]
target = "symbol"

[nodes.inputs.source]
namespace = "run_scoped"
key = "symbol"

[[nodes]]
id = "call-strategy"
kind = "subflow"
depends_on = ["script-node"]

[nodes.when]
source = "run.enabled"
operator = "truthy"

[nodes.call]
workflow = "strategy-child"

[nodes.call.with]
symbol = "trigger.symbol"
dry_run = "manual.dry_run"

[nodes.call.returns]
decision = "strategy_decision"
```

### Workflow 约束

- canonical entrypoint 是 `workflows/<workflow_id>/config.toml`
- 目录名必须与 `workflow.id` 完全一致
- workflow 内所有相对路径都相对 workflow package root 解析
- workflow identity 只由 `workflow.id` 定义
- workflow node graph 必须是 DAG
- `kind = "subflow"` 的节点必须使用 `[nodes.call]` 声明 child workflow、输入映射与输出导出边界
- `VariableReference` 支持结构化 `{ namespace, key }` 与 `<namespace_alias>.<key>` 简写两种写法；简写 alias 为 `cli`、`manual`、`trigger`、`workflow`、`config`、`node`、`run`，适用于 `nodes.when.source`、`nodes.call.with` 等变量引用位置
- `nodes.call.with` 不允许引用 `subflow_input` 或 `subflow_output`
- `kind = "subflow"` 的节点不得定义 `[[nodes.inputs]]`；subflow 的 author-facing 输入面只有 `nodes.call.with`
- `kind = "subflow"` 的节点若显式声明 `plugin` 与 `operation`，其值必须分别是 `builtin-subflow` 与 `run`
- `depends_mode = "all"` 表示所有依赖节点都必须成功；`depends_mode = "any"` 表示所有依赖节点进入终态后，只要任一成功即可继续
- node 调度顺序固定为：先依据依赖状态判断是否 ready/skip，再对 ready 节点评估 `when`

### Workflow Author Guide

- `depends_mode = "all"` 适合严格 DAG gate；任何依赖失败都会让当前节点变为 `skipped`
- `depends_mode = "any"` 不是 short-circuit OR；它会等待所有依赖进入终态，再根据“是否至少一个成功”决定是否继续
- `when` 是节点级二次 gate：只有节点先通过依赖判定后，才会评估 `when`
- `when` 只支持单条件 `source/operator/expected`，不支持多条件组合、变量对变量比较或嵌套表达式
- `when.operator = "falsy"` 在 source 缺失时也会返回 true；配置作者不能把它理解为单纯的 `!truthy(existing_value)`
- `run.*` 是可变执行态，不是静态输入快照；它会先由优先级层初始化，再在节点执行过程中被新的 `run_scoped` 输出覆盖
- 需要稳定 gating 时，应优先引用 `manual.*`、`trigger.*`、`workflow.*`、`config.*`，而不是 `run.*`
- `node.*` 表示跨节点写入的普通输出命名空间；后写入的同名 key 会覆盖先前值
- `subflow` 的 parent-visible 返回值来自 child workflow 的 `subflow_output` 命名空间，而不是 child 的普通 `node_outputs`
- child workflow 若希望向 parent 返回值，必须显式写入 `subflow_output`，例如通过 `builtin.emit_subflow_output`

## `triggers/<trigger_id>/config.toml`

trigger package 定义事件来源与目标 workflow 的绑定关系，以及 trigger 自己的长时间运行资源。它不承载 DAG、node 或 workflow 内部结构。

为避免每个 builtin trigger 新增参数时都扩展公共顶层 contract，trigger manifest 采用固定 core 字段加 `[params]` 扩展槽：

- core 字段承载稳定路由与运行 contract
- `[params]` 承载 trigger subtype 私有参数
- subtype 私有参数必须由对应 builtin handler 或 external trigger host contract 校验
- `input_mapping` 仍然独立，只负责把 trigger event payload 映射到 workflow run input

示例：

```toml
manifest_version = "2.0.0"
trigger_id = "manual-e2e"
kind = "builtin"
source = "manual"
workflow_id = "e2e"
enabled = true

[input_mapping]
symbol = "payload.symbol"
```

builtin cron 示例：

```toml
manifest_version = "2.0.0"
trigger_id = "cron-rebalance"
kind = "builtin"
source = "cron"
workflow_id = "rebalance"
enabled = true

[params]
schedule = "*/15 * * * *"
timezone = "UTC"

[input_mapping]
scheduled_at = "payload.slot_start_ms"
```

external trigger 示例：

```toml
manifest_version = "2.0.0"
trigger_id = "market-rebalance"
kind = "external_plugin"
plugin = "trigger-market-feed"
source = "market_tick"
workflow_id = "rebalance"
enabled = true

[params]
symbol = "ETHUSDT"

[input_mapping]
symbol = "payload.symbol"
price = "payload.price"
```

兼容性说明：

- canonical builtin 形式是 `kind = "builtin"` + `source = <subtype>`
- canonical external 形式是 `kind = "external_plugin"` + `plugin = <plugin_id>`
- 当前实现仍接受历史 alias 作为兼容输入：builtin 侧包括 `manual`、`market_tick`、`cron`；external 侧包括 `external_trigger` 与 `plugin`
- 新配置应优先使用 canonical 形式，alias 只用于兼容已有 roots

典型 trigger package 目录如下：

```text
triggers/
`- market-rebalance/
   |- config.toml
   |- scripts/
   |  `- watch_market.py
   |- assets/
   |  `- filters.json
   `- plugins/
      `- market_feed.sh
```

其中：

- `config.toml` 是 trigger package 的 canonical entrypoint
- `scripts/` 用于存放长时间运行的监听脚本或辅助脚本
- `assets/` 用于存放 trigger 私有静态资源
- `plugins/` 用于存放 trigger 私有可执行插件或适配器

### Trigger 约束

- trigger 的职责是产出 run request，而不是定义 workflow
- 每个 trigger 必须显式声明 `workflow_id`
- trigger 的 canonical identity 是 `trigger_id`
- trigger 必须引用一个已存在的 `workflow_id`
- trigger top-level schema 保持固定；新增 subtype 私有配置应优先写入 `[params]`
- `input_mapping` 只负责把 trigger payload 映射到 workflow run input
- trigger 不得定义 node、subflow 或 workflow runtime defaults
- canonical entrypoint 是 `triggers/<trigger_id>/config.toml`
- 目录名必须与 `trigger_id` 完全一致
- trigger package 内所有相对路径都相对 trigger package root 解析
- trigger 私有脚本或插件只属于当前 trigger package，不形成全局共享 contract

### Builtin Trigger Params Contract

- builtin trigger subtype 继续由 `kind = "builtin"` + `source = <subtype>` 决定
- `[params]` 是 subtype 私有配置区；不同 subtype 可拥有不同 schema
- `[params]` 中的字段必须在 subtype handler 中做加载期校验，避免把拼写错误或缺失必填项推迟到 `serve`
- `manual` 当前不需要任何 params
- `market_tick` 当前支持可选 `params.symbol`，默认值为 `BTCUSDT`
- `cron` 当前需要 `params.schedule`，并可选 `params.timezone = "UTC"`
- `cron` schedule 语法当前支持五段 UTC cron：`minute hour day month weekday`
  - 支持 `*`
  - 支持 `*/n`
  - 支持 `a,b,c`
  - 支持 `a-b`
  - 支持 `a-b/n`
  - `weekday` 取值为 `0-6`，其中 `0` 表示 Sunday；`7` 也可写作 Sunday alias
- `cron` trigger 只评估当前 serve snapshot 对应的 UTC minute slot，不做 missed-slot backfill
- `cron` trigger 必须基于稳定 slot 生成 `event_id`，以便 restart-safe duplicate suppression 继续成立

### External Trigger Host Contract

- external trigger host keeps the existing stdout output contract: plugin stdout must decode as `TriggerPluginOutput`
- external trigger host keeps `--trigger-id <trigger_id>` argv for compatibility with older plugins
- external trigger host now also writes a JSON `TriggerPluginInput` envelope to plugin stdin
- `TriggerPluginInput` currently contains:
  - `api_version`
  - `trigger_id`
  - `source`
  - `params`
- external trigger plugins may ignore stdin and still work when they only depend on legacy argv-based behavior
- external trigger plugins that want params-aware behavior should read stdin and decode `TriggerPluginInput`

## Workflow Run Model

所有 workflow 执行统一归一到同一个 run request 模型：

- `run_id`
- `workflow_id`
- `manual_invocation_input`
- `trigger_payload_mapping`
- `cli_args`

入口可以不同，但执行面一致：

- `chainbot run` 直接构造手动 run request 并执行 workflow
- `chainbot serve` 先评估 trigger，再把事件归一为 run request 并执行 workflow
- subflow 调用直接构造 child workflow request，不经过 trigger plane

## Runtime Variable Precedence

同名 key 的运行时优先级固定为：

1. `cli_args`
2. `manual_invocation_input`
3. `trigger_payload_mapping`
4. `workflow_defaults`
5. `config_defaults`

这个优先级属于执行 contract，不因 trigger 是否存在而变化。

## `plugins/manifests/*.toml`

plugin manifest 保持根级独立注册，用于共享插件与宿主级能力控制。它不属于 workflow 或 trigger 的业务配置。

示例：

```toml
manifest_version = "2.0.0"
plugin_id = "trigger-market-feed"
kind = "external_trigger"
entrypoint = "trigger.exec.v1"
capabilities = ["trigger.emit.run_request"]
executable = "../bin/market_feed.sh"
```

### Plugin 约束

- plugin manifest 与 workflow / trigger 分离维护
- `executable` 必须相对 manifest 文件自身解析
- external trigger plugin 必须声明 `trigger.emit.run_request`
- 根级 plugin manifest 用于可复用共享插件；trigger package 私有脚本不要求提升为根级共享插件

## Root Plugin Discovery

`chainbot.toml` 的 `[plugins].manifest_globs` 定义共享 plugin manifest 的发现入口。

- 默认值是 `["plugins/manifests/*.toml"]`
- 每一项都必须是 root-relative `<dir>/*.toml` 形式
- 每一项都必须保持在 `<root>` 内，不能使用绝对路径或 `..`
- discovery 只影响共享 plugin manifests，不影响 workflow/trigger package discovery

## 校验规则

- root manifest、workflow manifest、trigger manifest、plugin manifest 都必须通过 major version 校验
- workflow id 全局唯一
- trigger id 全局唯一
- trigger 引用的 `workflow_id` 必须存在
- plugin id 全局唯一
- workflow 与 trigger 的相对路径解析基准必须确定且不可歧义
- root path overrides 与 plugin discovery paths 必须保持在 `<root>` 内

## 稳定约束

- design 文档只记录最终设计，不记录推荐过程、备选方案或迁移步骤
- workflow 与 trigger 是两个独立配置面
- workflow 可以脱离 trigger 独立运行
- trigger 不能拥有 workflow 内部编排语义
- `serve` 消费 trigger，`run` 直接消费 workflow
- workflow 与 trigger 都采用 package/folder 架构

## 变更触发器

出现以下任一情况时，应更新此文档：

- root layout 变化
- workflow manifest 字段变化
- trigger manifest 字段变化
- plugin registry 位置或宿主策略变化
- runtime variable precedence 变化
