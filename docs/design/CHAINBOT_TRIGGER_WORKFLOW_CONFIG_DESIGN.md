# ChainBot Trigger Workflow Config Design

## 目标

把 ChainBot 根目录下与 `trigger` / `workflow` 相关的 TOML 配置结构整理为稳定设计约束，避免后续只能通过测试 fixture 或源码反推目录布局、字段语义与校验规则。

## 适用范围

- `config/root.toml`
- `workflows/*.toml`
- `triggers/*.toml`
- `plugins/*.toml` 中与 external trigger 关联的清单字段

## 根目录布局

ChainBot 运行时从显式 `--root` 或默认 `~/.chainbot` 解析根目录，并要求以下目录全部存在：

```text
<root>/
|- config/
|  `- root.toml
|- workflows/
|- triggers/
|- plugins/
|- secrets/
`- state/
```

加载顺序固定为：

1. `config/root.toml`
2. `workflows/*.toml`
3. `triggers/*.toml`
4. `plugins/*.toml`

目录下所有 `.toml` 文件都会被收集并按路径排序后解码。

## `config/root.toml`

`config/root.toml` 是根级配置入口，当前字段结构如下：

```toml
schema_version = "1.0.0"
profile = "e2e-success"
secret_refs = ["secret://ops/slack/webhook#api_token"]
```

- `schema_version`: 根配置 schema 版本；当前只接受 major `1`
- `profile`: 可选的根级 profile 标识
- `secret_refs`: 预声明的 secret 引用列表；每个值都必须能被 `SecretReference::parse` 正确解析

## `workflows/*.toml`

每个 workflow 文件映射到 `WorkflowDefinition`，顶层结构由 workflow 元信息、运行时默认值、内联 trigger 定义和 node DAG 组成。

### 顶层字段

```toml
api_version = "1.0.0"
workflow_id = "wf-e2e"
name = "wf-e2e"

[runtime.workflow_defaults]
symbol = "BTCUSDT"
secret_token_ref = "secret://ops/slack/webhook#api_token"
```

- `api_version`: workflow contract 版本；当前只接受 major `1`
- `workflow_id`: workflow 唯一标识
- `name`: 展示名称
- `runtime`: 运行时变量层；当前支持 `cli_args`、`manual_invocation_input`、`trigger_payload_mapping`、`workflow_defaults`、`config_defaults`

### 运行时变量优先级

相同 key 的解析优先级固定为：

1. `cli_args`
2. `manual_invocation_input`
3. `trigger_payload_mapping`
4. `workflow_defaults`
5. `config_defaults`

解析后的值会物化到 `run_scoped` 命名空间，供 node 输入绑定继续引用。

### 内联 `[[triggers]]`

workflow 内可以内联 trigger 定义，结构与 `triggers/*.toml` 相同：

```toml
[[triggers]]
api_version = "1.0.0"
trigger_id = "manual-inline"
kind = "builtin"
source = "manual"
enabled = true
```

这些定义参与 workflow 合约校验，适合把 workflow 与其触发方式一起建模。

### `[[nodes]]`

每个 node 映射到 `NodeDefinition`：

```toml
[[nodes]]
api_version = "1.0.0"
node_id = "script-node"
kind = "builtin"
plugin_id = "builtin.script"
operation = "python:scripts/e2e_worker.py"
depends_mode = "all"
depends_on = ["builtin-start"]
```

- `api_version`: node contract 版本；当前只接受 major `1`
- `node_id`: workflow 内唯一标识
- `kind`: 当前代码允许任意非空字符串进入后续执行平面；常见值包括 `builtin`、`plugin`、`subflow`
- `plugin_id`: builtin 或 external node plugin 标识
- `operation`: 具体操作名或 worker 调用入口
- `depends_mode`: `all` 或 `any`，默认 `all`
- `depends_on`: 依赖的上游 node 列表

#### Node 输入绑定

```toml
[[nodes.inputs]]
target = "symbol"

[nodes.inputs.source]
namespace = "run_scoped"
key = "symbol"
```

- `target`: 传给当前 node 的输入名
- `source.namespace`: 变量来源命名空间
- `source.key`: 来源 key

当前可用命名空间为：

- `cli_args`
- `manual_invocation_input`
- `trigger_payload_mapping`
- `workflow_defaults`
- `config_defaults`
- `node_outputs`
- `run_scoped`
- `subflow_input`
- `subflow_output`

#### Node 条件执行

```toml
[nodes.when]
operator = "truthy"

[nodes.when.source]
namespace = "manual_invocation_input"
key = "enabled"
```

- `operator`: `exists`、`equals`、`not_equals`、`truthy`、`falsy`
- `expected`: 仅 `equals` / `not_equals` 允许设置，其他 operator 必须省略

#### Subflow 合约

当 `kind = "subflow"` 时，必须提供 `subflow` 段：

```toml
[nodes.subflow]
workflow_id = "wf-child"

[[nodes.subflow.imports]]
child_key = "ticker"

[nodes.subflow.imports.source]
namespace = "trigger_payload_mapping"
key = "symbol"

[[nodes.subflow.exports]]
child_key = "decision"
parent_key = "subflow_decision"
```

- `workflow_id`: 被调用的子 workflow
- `imports`: 从父级命名空间导入到子流程输入；不允许引用 `subflow_input` 或 `subflow_output`
- `exports`: 把子流程输出映射回父级 `run_scoped`

## `triggers/*.toml`

每个文件映射到 `TriggerDefinition`：

```toml
api_version = "1.0.0"
trigger_id = "external-trigger-e2e"
kind = "external_plugin"
source = "trigger-e2e-plugin"
enabled = true
```

- `api_version`: trigger contract 版本；当前只接受 major `1`
- `trigger_id`: trigger 唯一标识
- `kind`: trigger 类型别名
- `source`: builtin source 名或 external trigger plugin id
- `enabled`: 是否启用

### Trigger kind 归一化

代码会把下列 kind 归一到两个执行类别：

- builtin: `builtin`、`manual`、`market_tick`
- external plugin: `external_plugin`、`external_trigger`、`plugin`

未知 kind 会在加载阶段被拒绝。

## `plugins/*.toml` 与 trigger 的关系

当 `triggers/*.toml` 使用 external plugin 类别时，`source` 必须指向一个 external trigger plugin manifest：

```toml
api_version = "1.0.0"
plugin_id = "trigger-e2e-plugin"
kind = "external_trigger"
entrypoint = "trigger.exec.v1"
capabilities = ["trigger.emit.run_request"]
executable = "bin/external_trigger.sh"
```

external trigger plugin 至少需要：

- `kind = "external_trigger"` 或其别名
- 非空 `executable`
- capability `trigger.emit.run_request`

## 设计约束

- 所有 `api_version` / `schema_version` 当前只支持 major `1`
- `workflows/*.toml` 与 `triggers/*.toml` 是目录级集合，不是单文件聚合
- workflow node id 在单个 workflow 内必须唯一
- node 依赖图必须是 DAG，禁止自依赖与环
- 输入绑定、`when`、subflow import/export 中的 key 不能为空
- `equals` / `not_equals` 必须显式提供 `expected`
- 当前 `serve` 触发面直接消费 `triggers/*.toml`，并通过 `plugins/*.toml` 解析 external trigger plugin
- workflow 内联 `[[triggers]]` 仍然是稳定 contract 的一部分，但当前主要承担 workflow 合约建模与校验职责

## 最小可运行示例

```text
<root>/
|- config/
|  `- root.toml
|- workflows/
|  `- e2e.toml
|- triggers/
|  `- external_trigger.toml
|- plugins/
|  `- trigger_e2e.toml
|- secrets/
`- state/
```

这个结构覆盖了当前代码中的根布局解析、TOML 加载、workflow DAG 校验、trigger plane 装载和 external trigger plugin 绑定。

## 变更触发器

出现以下任一情况时，应增补此文档而不是只改 fixture：

- `RootConfigDefinition` 字段变化
- `WorkflowDefinition` / `NodeDefinition` / `TriggerDefinition` 字段变化
- runtime variable 命名空间或优先级变化
- trigger kind / plugin kind 别名变化
- `serve` 对 trigger/workflow 绑定方式变化
