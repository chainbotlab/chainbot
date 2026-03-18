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
|- config/
|  `- root.toml
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

1. `config/root.toml`
2. `workflows/*/config.toml`
3. `triggers/*/config.toml`
4. `plugins/manifests/*.toml`

## `config/root.toml`

`config/root.toml` 只承载根级稳定配置：

- manifest version
- root profile
- path overrides
- runtime defaults
- plugin discovery settings
- secret references
- all path settings must stay within `<root>` and use root-relative paths

示例：

```toml
manifest_version = "2.0.0"
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
```

### Workflow 约束

- canonical entrypoint 是 `workflows/<workflow_id>/config.toml`
- 目录名必须与 `workflow.id` 完全一致
- workflow 内所有相对路径都相对 workflow package root 解析
- workflow identity 只由 `workflow.id` 定义
- workflow node graph 必须是 DAG

## `triggers/<trigger_id>/config.toml`

trigger package 定义事件来源与目标 workflow 的绑定关系，以及 trigger 自己的长时间运行资源。它不承载 DAG、node 或 workflow 内部结构。

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

external trigger 示例：

```toml
manifest_version = "2.0.0"
trigger_id = "market-rebalance"
kind = "external_plugin"
plugin = "trigger-market-feed"
source = "market_tick"
workflow_id = "rebalance"
enabled = true

[input_mapping]
symbol = "payload.symbol"
price = "payload.price"
```

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
- `input_mapping` 只负责把 trigger payload 映射到 workflow run input
- trigger 不得定义 node、subflow 或 workflow runtime defaults
- canonical entrypoint 是 `triggers/<trigger_id>/config.toml`
- 目录名必须与 `trigger_id` 完全一致
- trigger package 内所有相对路径都相对 trigger package root 解析
- trigger 私有脚本或插件只属于当前 trigger package，不形成全局共享 contract

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

`config/root.toml` 的 `[plugins].manifest_globs` 定义共享 plugin manifest 的发现入口。

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
