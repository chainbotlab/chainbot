## 2026-03-29 Task 1 decisions

- Preserved `kind = "external_node"` and selected runtime contract via `entrypoint` only.
- Added additive MCP contract under `PluginManifest.mcp` with nested `mcp.auth`, keeping all MCP details inside `crate::plugin`.
- Chose entrypoint-aware validation policy:
  - `entrypoint = "mcp.tool.v1"`: require `mcp` block, require operations, forbid `plugin.executable`, and enforce transport-specific shape.
  - non-`mcp.tool.v1` external-node entrypoints: keep legacy executable-based validation and reject `mcp` block.
- Phase 1 no-alias rule is represented by contract shape: operations keep only `name` and no separate tool alias field is introduced.

## 2026-03-29 Task 2 decisions

- Kept MCP plugins inside the existing package install model by changing only `plugin/source/*` validation/preparation logic; no parallel direct-server install path was introduced.
- Scoped the executable/artifact bypass narrowly to `entrypoint = "mcp.tool.v1"` with `mcp.transport = "streamable_http"` so legacy executable-based plugin installs and MCP stdio behavior remain unchanged.
- Enforced a package-only source/install contract by rejecting URL-like `source.entry_artifact` values, preventing raw server references from entering source metadata while still allowing server endpoints in the canonical `plugin.mcp.streamable_http.url` manifest field.

## 2026-03-29 Task 3 decisions

- Introduced an internal node-invoker seam in `plugin/host.rs` via entrypoint dispatch and changed runtime orchestration to call `ExternalNodePluginHost::execute_node_invocation`, removing the implicit subprocess-only assumption at the app-layer call site.
- Kept legacy subprocess behavior unchanged by moving existing execution logic into the dedicated `LegacySubprocess` branch.
- Encoded Phase 1 MCP lifecycle as `per_invocation_session` with an explicit session guard created and dropped inside each MCP node invocation.
- Deferred transport wiring intentionally: MCP invocations currently terminate at the seam with a normalized `NodePluginProtocolContractViolation` that preserves existing plugin error surfaces for runtime/reporting.
- Added targeted lifecycle and error-mapping tests (`mcp_invoker_lifecycle`, `mcp_error_mapping`) without extending into Task 4 rmcp transport discovery/call work.

## 2026-03-29 Task 4 decisions

- Implemented the MCP adapter core inside `plugin/host.rs` with a local session interface that models official MCP lifecycle verbs (`initialize`, `list_tools`, `call_tool`) while keeping transport specifics deferred to Tasks 5/6.
- Enforced manifest-authoritative operation policy: runtime discovery is used only to validate that the declared operation exists uniquely and that the discovered input schema is a supported object shape compatible with manifest-declared inputs.
- Normalized MCP call outputs into `NodePluginExecutionResult` by accepting only deterministic JSON-object forms (`structured_content` or a single JSON content item), and rejecting unsupported/ambiguous shapes with `NodePluginProtocolContractViolation`.

## 2026-03-29 Task 4 retry decisions

- Kept `per_invocation_session` lifecycle semantics unchanged and fixed only test instrumentation reliability by replacing process-wide test globals with thread-local registries for MCP lifecycle events and injected test adapters.

## 2026-03-29 Task 5 decisions

- Wired `mcp.transport = stdio` directly in `plugin/host.rs` with an rmcp child-process adapter that owns a per-invocation `RunningService<RoleClient, ()>` and closes it with bounded teardown on drop.
- Reused existing subprocess guardrails by resolving `plugin.mcp.stdio.command` through the host’s installed-plugin executable-path policy and by sharing the same host environment allowlist for both legacy subprocess and rmcp stdio startup.
- Kept `node_plugin_host` focused on the still-unwired transport seam by moving its Task 3/4 fail-closed assertions to `streamable_http`, and added a dedicated `mcp_plugin_host` integration surface for stdio happy-path plus deterministic missing-executable / initialize-failure / malformed-payload coverage.

## 2026-03-29 Task 6 decisions

- Added a dedicated rmcp Streamable HTTP adapter in `plugin/host.rs` and kept all HTTP/session/auth logic inside `crate::plugin`, preserving the existing transport-agnostic adapter seam for the app/domain layers.
- Extended `ExternalNodePluginHost` with execution-time secret-runtime wiring so Streamable HTTP auth resolves `plugin.mcp.auth.token_secret_ref` only at invocation time using the existing secret provider/decryptor model.
- Normalized Streamable HTTP auth, stale-session, timeout, and non-MCP endpoint failures onto existing node-plugin error surfaces, while letting rmcp handle transparent same-invocation session reinitialization for recoverable `404` session expiry.
- Retired the old `node_plugin_host` placeholder assertions for unwired HTTP transport and moved real Streamable HTTP behavior coverage into `mcp_plugin_host` with deterministic local fixture scenarios.

## 2026-03-29 Task 7 decisions

- Kept runtime integration backend-agnostic by preserving `ExecutionPlane` -> `ExternalNodePluginHost::execute_node_invocation` dispatch, with no MCP transport types introduced in `app/` or `domain/`.
- Added explicit execution-engine regressions in `tests/execution_scheduler.rs` with exact test names:
  - `runtime_execution_routes_mcp_tool_entrypoint`
  - `runtime_execution_preserves_legacy_external_node_dispatch`
- Chose workflow-report parity as the compatibility gate: both MCP and legacy plugin paths must preserve existing node failure/outputs/state aggregation semantics at runtime.

## 2026-03-29 Task 8 decisions

- Kept installed MCP packages inside the existing catalog `plugins` collection and `Installed plugins` text section; no new top-level MCP class or CLI mode was introduced.
- Exposed MCP-specific installed metadata additively via `transport` plus per-invocation `runtime` labels only for `entrypoint = "mcp.tool.v1"`, avoiding any raw server-management or source-runtime mental model in the installed catalog surface.
- Locked the package-centric CLI/help boundary with surface tests that assert help continues to steer users toward `plugin source` / `plugin install` / `catalog` flows rather than direct-connect or server-management commands.
