# F4 Scope Fidelity Check (deep)

Verdict: **APPROVE**

## Baseline Compared

- Plan of record: `.sisyphus/plans/mcp-plugin-ecosystem.md`
- Scope: MCP as backward-compatible execution backend for existing `external_node` plugins using internal `mcp.tool.v1` adapter. Phase 1 supports MCP tools only, with local `stdio` and remote Streamable HTTP transports, while preserving the current installed package model.
- Manual QA evidence used as ground truth: `.sisyphus/evidence/f3-manual-qa.txt`

## Pillar-by-Pillar Coverage Map

### 1) MCP manifest contract under existing external node shape

- `crates/chainbot/src/plugin/contract.rs` defines `entrypoint = "mcp.tool.v1"` with additive `[mcp]` block; validation is entrypoint-aware so `node.exec.v1` and `mcp.tool.v1` follow separate rules.
- No new plugin kind introduced; `kind = "external_node"` is preserved.
- Tests in `crates/chainbot/tests/mcp_plugin_host.rs` and `crates/chainbot/tests/node_plugin_host.rs` cover manifest validation for both entrypoints.

### 2) Package-model and install/source guardrails for MCP packages

- `crates/chainbot/src/plugin/source/manifest.rs`, `crates/chainbot/src/plugin/source/discover.rs`, and `crates/chainbot/src/plugin/source/prepare.rs` enforce that MCP packages use the installed package pipeline; HTTP MCP packages bypass local-executable requirement.
- Direct server references are rejected at source metadata validation time.
- Tests in `crates/chainbot/tests/plugin_install_surface.rs` and `crates/chainbot/tests/plugin_source_surface.rs` cover these guardrails.

### 3) Internal node invoker seam and per-invocation MCP lifecycle

- `crates/chainbot/src/plugin/host.rs` implements entrypoint-based invoker selection (`node.exec.v1` vs `mcp.tool.v1`) with per-call MCP session creation and teardown.
- No pooling or long-lived session reuse.
- `crates/chainbot/src/app/runtime/execution.rs` dispatches through `ExternalNodePluginHost::execute_node_invocation` without awareness of MCP concepts.
- Tests in `crates/chainbot/tests/mcp_plugin_host.rs` confirm per-invocation lifecycle.

### 4) rmcp adapter core (discovery validation, schema mapping, protocol normalization)

- `crates/chainbot/src/plugin/host.rs` wraps rmcp client/session behavior behind a local interface.
- Runtime `list_tools` validates manifest-declared operations exist and have callable object-shaped input schema.
- Invalid schema, ambiguous output shapes, and unsupported capabilities fail closed deterministically.
- Tests cover `mcp_tool_discovery_validation`, `mcp_schema_translation`, and `mcp_invalid_tool_schema_rejected`.

### 5) Local stdio MCP transport backend

- `crates/chainbot/src/plugin/host.rs` (stdio branch) uses rmcp child-process transport with executable-path validation and default-deny environment allowlist.
- Tests cover `stdio_tool_call_happy_path`, `stdio_missing_executable_fails_closed`, `stdio_initialize_failure_maps_to_plugin_error`, and `stdio_malformed_payload_rejected`.

### 6) Streamable HTTP MCP transport backend with static auth

- `crates/chainbot/src/plugin/host.rs` (HTTP branch) uses rmcp Streamable HTTP client with static auth resolution at invocation time.
- `401`, stale session, and non-MCP responses are normalized to deterministic plugin errors.
- Tests cover `http_tool_call_happy_path`, `http_401_maps_to_plugin_error`, `http_stale_session_recovers_or_fails_deterministically`, and `http_non_mcp_endpoint_rejected`.

### 7) Runtime execution integration without regressing legacy dispatch

- `crates/chainbot/src/app/runtime/execution.rs` routes `mcp.tool.v1` through MCP invoker and `node.exec.v1` through legacy path.
- Workflow report semantics are preserved unchanged for legacy plugins.
- Tests confirm `runtime_execution_routes_mcp_tool_entrypoint` and `runtime_execution_preserves_legacy_external_node_dispatch`.

### 8) Installed MCP plugin visibility in catalog and CLI

- `crates/chainbot/src/app/cli/view/catalog.rs` surfaces installed MCP plugins with transport details; no new top-level plugin class introduced.
- `help plugin` output remains package-centric.
- Tests in `crates/chainbot/tests/catalog_surface.rs` and `crates/chainbot/tests/cli_surface.rs` cover MCP plugin catalog and CLI output.

## Silent Deferral Check

- No required Phase 1 pillar is missing.
- Explicit out-of-scope items remain deferred: MCP `resources`, `prompts`, `sampling`, `roots`, trigger/event ingestion, OAuth browser flow, session pooling, direct raw URL/server references.
- Scope is additive and backward-compatible; existing `external_node` plugins require no manifest edits.

## Verification Grounding

- Workspace-level manual QA passed: `cargo build -p chainbot`, `cargo test -p chainbot --test mcp_plugin_host`, `cargo test -p chainbot --test node_plugin_host`, `cargo test -p chainbot --test catalog_surface`, `cargo test -p chainbot --test cli_surface`, `cargo run -p chainbot -- catalog list --kind plugin`, `cargo run -p chainbot -- catalog show plugin:mcp-http-plugin`, `cargo run -p chainbot -- help plugin`.
- All evidence recorded in `.sisyphus/evidence/f3-manual-qa.txt`.

Final reviewer decision: **APPROVE**.
