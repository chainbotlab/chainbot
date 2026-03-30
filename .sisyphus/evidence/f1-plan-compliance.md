# F1 Plan Compliance Audit (oracle)

Verdict: **APPROVE**

Checked against the current repository contents and the MCP plugin ecosystem implementation scope.

## Guardrail Verdict

- No new top-level plugin kind added. MCP-backed plugins remain `kind = "external_node"` with `entrypoint = "mcp.tool.v1"`.
- No `rmcp` types leak into `app/` or `domain/`. All MCP details are confined to `crate::plugin`.
- Manifest `operations` is treated as ChainBot's source of truth; runtime `list_tools` is validation only.
- No Phase 1 scope leaks: MCP `resources`, `prompts`, `sampling`, `roots`, trigger ingestion, OAuth browser flow, session pooling, and direct raw URL references are all absent.
- Transport config is separate from auth config; auth resolution happens only at execution time.
- No direct server references are accepted outside installed plugin packages.

## Plan-to-Code Mapping

### Wave 1 Tasks

- Task 1 is present. `crates/chainbot/src/plugin/contract.rs` defines the MCP manifest contract with `entrypoint = "mcp.tool.v1"`, additive `[mcp]` block, and entrypoint-aware validation (`node.exec.v1` vs `mcp.tool.v1`).
- Task 2 is present. `crates/chainbot/src/plugin/source/manifest.rs`, `crates/chainbot/src/plugin/source/discover.rs`, and `crates/chainbot/src/plugin/source/prepare.rs` enforce package-model and install/source guardrails for MCP packages; HTTP MCP packages bypass local-executable requirement.
- Task 3 is present. `crates/chainbot/src/plugin/host.rs` introduces the internal node invoker seam with per-invocation MCP session lifecycle; `crates/chainbot/src/app/runtime/execution.rs` dispatches through `ExternalNodePluginHost::execute_node_invocation`.
- Task 4 is present. `crates/chainbot/src/plugin/host.rs` implements the rmcp adapter core for discovery validation, schema mapping, and protocol normalization; runtime `list_tools` is used only to validate declared operations.
- Task 5 is present. `crates/chainbot/src/plugin/host.rs` (stdio branch) implements local stdio transport backend using rmcp child-process transport with executable-path validation and default-deny environment allowlist.

### Wave 2 Tasks

- Task 6 is present. `crates/chainbot/src/plugin/host.rs` (HTTP branch) implements Streamable HTTP transport backend with static auth resolution, `401` normalization, stale-session recovery, and non-MCP endpoint rejection.
- Task 7 is present. `crates/chainbot/src/app/runtime/execution.rs` routes `mcp.tool.v1` through the MCP invoker path and `node.exec.v1` through the legacy path; workflow report semantics are preserved.
- Task 8 is present. `crates/chainbot/src/app/cli/view/catalog.rs` surfaces installed MCP plugins in catalog output with transport details; `help plugin` output remains package-centric.

## Final Verification Cross-Check

- Task tests all pass: `cargo test -p chainbot --test mcp_plugin_host`, `cargo test -p chainbot --test node_plugin_host`, `cargo test -p chainbot --test catalog_surface`, `cargo test -p chainbot --test cli_surface`, `cargo test -p chainbot runtime_execution_routes_mcp_tool_entrypoint`, `cargo test -p chainbot runtime_execution_preserves_legacy_external_node_dispatch`, and `cargo build -p chainbot` all confirmed passing.
- CLI checks confirmed: `catalog list --kind plugin`, `catalog show plugin:mcp-http-plugin`, and `help plugin` produce correct output.

## Hidden Scope Expansion Check

- No hidden scope expansion found. The implementation adds MCP as an internal transport under the existing plugin facade with no redesign of the plugin ABI or workflow model.

## Blocking Drift List

- None.
