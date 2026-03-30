# ChainBot MCP Plugin Ecosystem

## TL;DR
> **Summary**: Introduce MCP as a backward-compatible execution backend for existing `external_node` plugins by adding an internal `mcp.tool.v1` adapter under the current plugin facade. Phase 1 supports MCP tools only, with local `stdio` and remote Streamable HTTP transports, while preserving the current installed package model.
> **Deliverables**:
> - Additive plugin manifest support for MCP-backed external node plugins
> - Internal rmcp-based MCP host adapter with `stdio` and Streamable HTTP backends
> - Runtime execution, catalog, and CLI support for installed MCP plugins
> - BDD-style unit and integration coverage for lifecycle, schema, auth, timeout, and compatibility paths
> **Effort**: Large
> **Parallel**: YES - 2 waves
> **Critical Path**: 1 → 3 → 4 → 6 → 7

## Context
### Original Request
当前 plugin 支持多种运行时,引入 MCP 生态作为插件怎么样?

### Interview Summary
- Goal is not a plugin-system rewrite. The target is MCP tool reuse.
- Phase 1 scope is MCP **tools only**.
- Phase 1 must support both local `stdio` MCP servers and remote Streamable HTTP MCP servers.
- Entry must remain the existing installed plugin package model under `plugins/<plugin_id>/config.toml`.
- Full backward compatibility is required for existing manifests and plugins.
- The implementation must use the official Rust SDK / `rmcp`, not a custom MCP protocol stack.
- Test strategy is BDD: unit tests for core adapter/validation logic, integration tests for end-to-end behavior.
- Explicitly out of scope for Phase 1: MCP `resources`, MCP `prompts`, trigger/event ingestion, direct ad-hoc external server references, OAuth browser flow, session pooling.

### Metis Review (gaps addressed)
- Lock the architecture to an internal transport/runtime adapter under the existing plugin facade, not a new workflow/domain abstraction.
- Make package manifests the single source of truth; runtime `list_tools` is validation/diagnostic input, not the authoritative contract.
- Lock the Phase 1 schema policy: manifest `operations[*].input_schema` and `operations[*].output_schema` remain the execution-time contract for `mcp.tool.v1`; runtime MCP tool schemas are compatibility checks only and must fail closed when they cannot be reduced to ChainBot's field-list semantics.
- Decide lifecycle up front: Phase 1 uses one MCP session per node invocation.
- Separate transport config from auth config and keep auth resolution at execution time only.
- Keep the installed-package model explicit for HTTP MCP plugins: `source.entry_artifact` remains required as a package anchor file inside the package, but HTTP runtime execution must not require executable parity or local process launch.
- Lock Phase 1 HTTP auth to one static shape: `[mcp.auth]` requires both `header_name` and `token_secret_ref`; the referenced secret stores the final header value and the host injects it verbatim at execution time.
- Encode failure handling explicitly for invalid schema, timeout, auth errors, stale sessions, and unsupported capabilities.

## Work Objectives
### Core Objective
Enable ChainBot to execute installed MCP-backed node plugins through the existing external-node workflow path, using `rmcp` internally, without breaking existing plugin contracts, install flows, or catalog behavior.

### Deliverables
- Additive `external_node` manifest contract for `entrypoint = "mcp.tool.v1"`
- Internal MCP host seam and rmcp-backed adapter
- Local `stdio` MCP transport backend
- Remote Streamable HTTP MCP transport backend with static secret-backed auth only
- Runtime dispatch support in the existing execution flow
- Installed-plugin catalog and CLI support for MCP-backed plugins
- Unit and integration tests covering happy path and failure path behavior

### Definition of Done (verifiable conditions with commands)
- `cargo test -p chainbot --test node_plugin_host --test plugin_install_surface --test plugin_source_surface --test catalog_surface --test cli_surface` exits `0`
- `cargo test -p chainbot --test mcp_plugin_host` exits `0`
- `cargo test -p chainbot mcp_manifest_validation -- --exact` exits `0`
- `cargo test -p chainbot mcp_tool_discovery_validation -- --exact` exits `0`
- `cargo test -p chainbot mcp_invoker_lifecycle_unit -- --exact` exits `0`
- Existing non-MCP plugin fixtures continue to pass unchanged under `cargo test -p chainbot --tests`

### Must Have
- Preserve `kind = "external_node"`; use `entrypoint = "mcp.tool.v1"` to select MCP runtime semantics
- Keep MCP details inside `crate::plugin`; no `rmcp` types may leak into `app/` or `domain/`
- Treat manifest `operations` as ChainBot’s source of truth; runtime MCP discovery is validation only
- Keep `operations[*].input_schema` and `operations[*].output_schema` authoritative for Phase 1 execution; MCP runtime schemas may only confirm compatibility, never overwrite manifest metadata
- Support both `stdio` and Streamable HTTP in Phase 1
- Resolve secrets/auth only at execution time
- Require `source.entry_artifact` for installed MCP packages, including HTTP packages, but never require HTTP packages to ship a runnable local executable
- For Streamable HTTP auth, support exactly one static header injection contract in Phase 1: `header_name + token_secret_ref`, with the secret value injected verbatim as the header value
- Fail closed on invalid transport config, invalid tool schema, and unsupported capabilities

### Must NOT Have (guardrails, AI slop patterns, scope boundaries)
- Must NOT add a new top-level plugin kind for MCP
- Must NOT redesign the plugin ABI or workflow model
- Must NOT add MCP `resources`, `prompts`, `sampling`, `roots`, or trigger ingestion in Phase 1
- Must NOT support direct raw URL/server references outside installed plugin packages
- Must NOT implement OAuth browser/device flow in Phase 1
- Must NOT treat dynamic `list_tools` results as durable install-time metadata
- Must NOT introduce session pooling or long-lived MCP host reuse before correctness is proven

## Verification Strategy
> ZERO HUMAN INTERVENTION — all verification is agent-executed.
- Test decision: BDD + Rust unit/integration tests via Cargo
- QA policy: Every task includes happy-path and failure-path agent-executed scenarios
- Evidence: `.sisyphus/evidence/task-{N}-{slug}.{ext}`

## Execution Strategy
### Parallel Execution Waves
> Target: 5-8 tasks per wave. <3 per wave (except final) = under-splitting.
> Extract shared dependencies as Wave-1 tasks for max parallelism.

Wave 1: contract and adapter foundation
- Task 1: MCP manifest contract under existing external node shape
- Task 2: package/source/install guardrails for MCP packages
- Task 3: internal node invoker seam and lifecycle boundary
- Task 4: rmcp adapter core for discovery, mapping, and error normalization
- Task 5: local `stdio` transport backend

Wave 2: remote transport and product surfaces
- Task 6: Streamable HTTP transport backend and static auth resolution
- Task 7: runtime execution integration with legacy dispatch preservation
- Task 8: catalog and CLI installed-plugin visibility for MCP packages

### Dependency Matrix (full, all tasks)
| Task | Depends On | Blocks |
|---|---|---|
| 1 | — | 2, 3, 4, 8 |
| 2 | 1 | 6, 8 |
| 3 | 1 | 4, 5, 6, 7 |
| 4 | 1, 3 | 5, 6, 7 |
| 5 | 3, 4 | 7 |
| 6 | 2, 3, 4 | 7 |
| 7 | 5, 6 | 8, F1-F4 |
| 8 | 1, 2, 7 | F1-F4 |

### Agent Dispatch Summary (wave → task count → categories)
- Wave 1 → 5 tasks → `unspecified-high`, `deep`
- Wave 2 → 3 tasks → `unspecified-high`, `deep`
- Final Verification → 4 review tasks → `oracle`, `unspecified-high`, `deep`

## TODOs
> Implementation + Test = ONE task. Never separate.
> EVERY task MUST have: Agent Profile + Parallelization + QA Scenarios.

- [x] 1. Add MCP manifest contract for installed external node plugins

  **What to do**: Extend the plugin manifest contract so MCP-backed plugins remain `kind = "external_node"` but use `entrypoint = "mcp.tool.v1"`. Add an additive `[mcp]` block with transport-specific configuration and an additive `[mcp.auth]` block for execution-time auth references. Make `operations[*].name` equal the MCP tool name in Phase 1. Keep `operations[*].input_schema` and `operations[*].output_schema` as the execution-time contract; runtime MCP schemas may only validate compatibility with that declared field list and must fail closed when the MCP tool shape cannot be reduced to ChainBot's field-list semantics. Keep validation entrypoint-aware so legacy `node.exec.v1` plugins still require their current shape while `mcp.tool.v1` follows MCP-specific rules.
  **Must NOT do**: Do not add a new plugin kind. Do not change `manifest_version` semantics. Do not expose `rmcp` concepts in `domain/` or workflow definitions. Do not add alias mapping for tool names in Phase 1. Do not let runtime `list_tools` schema discovery overwrite manifest-declared operation metadata.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: manifest compatibility and validation rules are high-risk and central to the whole rollout
  - Skills: `[]` — no extra skill required; repo-specific contract work
  - Omitted: `[]` — no omission needed

  **Parallelization**: Can Parallel: NO | Wave 1 | Blocks: 2, 3, 4, 8 | Blocked By: —

  **References**:
  - Pattern: `crates/chainbot/src/plugin/contract.rs` — current plugin manifest, validation logic, and external-node contract
  - Pattern: `crates/chainbot/src/plugin/mod.rs` — stable plugin facade boundary
  - Pattern: `official-plugins/echo-official-plugin/config.toml` — installed plugin package example
  - Pattern: `official-plugins/build-official-plugin/config.toml` — build-required package example
  - Test: `crates/chainbot/tests/node_plugin_host.rs` — existing external node contract coverage
  - Test: `crates/chainbot/tests/catalog_surface.rs` — catalog metadata expectations
  - External: `https://modelcontextprotocol.io/specification/2025-11-25/server/tools` — MCP tools contract
  - External: `https://github.com/modelcontextprotocol/rust-sdk` — official Rust SDK boundary

  **Acceptance Criteria** (agent-executable only):
  - [ ] `cargo test -p chainbot contract::mcp_manifest_validation -- --exact` exits `0`
  - [ ] `cargo test -p chainbot contract::mcp_manifest_rejects_non_field_list_tool_schema -- --exact` exits `0`
  - [ ] `cargo test -p chainbot --test node_plugin_host node_plugin_manifest_validation -- --exact` exits `0`
  - [ ] `cargo test -p chainbot --test plugin_install_surface` exits `0`
  - [ ] Legacy `node.exec.v1` fixtures remain valid without manifest changes

  **QA Scenarios** (MANDATORY — task incomplete without these):
  ```
  Scenario: MCP manifest happy path
    Tool: Bash
    Steps: Run `cargo test -p chainbot contract::mcp_manifest_validation -- --exact`; verify fixture manifest with `entrypoint = "mcp.tool.v1"`, transport `stdio`, and operation `echo` parses and validates.
    Expected: Exit code 0; test asserts additive fields are accepted and operation name is preserved as `echo`.
    Evidence: .sisyphus/evidence/task-1-mcp-manifest.txt

  Scenario: Invalid mixed transport manifest
    Tool: Bash
    Steps: Run `cargo test -p chainbot contract::mcp_manifest_rejects_mixed_transport_fields -- --exact`.
    Expected: Exit code 0; test asserts manifests with both stdio and HTTP transport fields are rejected with a deterministic validation error.
    Evidence: .sisyphus/evidence/task-1-mcp-manifest-error.txt

  Scenario: Non-reducible MCP schema rejected
    Tool: Bash
    Steps: Run `cargo test -p chainbot contract::mcp_manifest_rejects_non_field_list_tool_schema -- --exact`.
    Expected: Exit code 0; test asserts `mcp.tool.v1` manifests fail closed when declared/runtime tool schema cannot be represented as ChainBot's flat field-list contract.
    Evidence: .sisyphus/evidence/task-1-mcp-manifest-schema-error.txt
  ```

  **Commit**: YES | Message: `feat(plugin): add mcp external node manifest contract` | Files: `crates/chainbot/src/plugin/contract.rs`, `crates/chainbot/tests/**/*`

- [x] 2. Enforce package-model and install/source guardrails for MCP plugins

  **What to do**: Keep MCP plugins inside the existing installable package model. Update source/install validation so MCP packages can be installed through the current package pipeline without adding direct server references or install-time network behavior beyond existing source fetches. Keep `source.entry_artifact` required for HTTP MCP packages as a package-local anchor file that survives discover/install/show flows, but do not require it to match `plugin.executable` or to be runnable when `entrypoint = "mcp.tool.v1"` and transport is HTTP.
  **Must NOT do**: Do not add new CLI entrypoints for raw URLs. Do not move runtime auth into `plugin/source/*`. Do not change source manifest version semantics beyond what is required for additive MCP package metadata. Do not make HTTP MCP installability depend on local executable parity.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: package/install safety rules and source contracts are subtle and easy to break
  - Skills: `[]` — no extra skill required
  - Omitted: `[]` — no omission needed

  **Parallelization**: Can Parallel: YES | Wave 1 | Blocks: 6, 8 | Blocked By: 1

  **References**:
  - Pattern: `crates/chainbot/src/plugin/source/manifest.rs` — source contract and version enforcement
  - Pattern: `crates/chainbot/src/plugin/source/prepare.rs` — artifact preparation expectations
  - Pattern: `crates/chainbot/src/plugin/source/install.rs` — staged install and rollback behavior
  - Pattern: `crates/chainbot/src/plugin/source/discover.rs` — source metadata to plugin metadata alignment
  - Test: `crates/chainbot/tests/plugin_install_surface.rs` — install safety and overwrite protection
  - Test: `crates/chainbot/tests/plugin_source_surface.rs` — source list/show behavior
  - External: `docs/design/CHAINBOT_PLUGIN_SOURCE_INSTALL_DESIGN.md` — install/source contract design

  **Acceptance Criteria** (agent-executable only):
  - [ ] `cargo test -p chainbot --test plugin_install_surface` exits `0`
  - [ ] `cargo test -p chainbot --test plugin_source_surface` exits `0`
  - [ ] HTTP MCP packages install successfully without requiring a local executable artifact, while still requiring a safe package-local `source.entry_artifact`
  - [ ] No direct-server configuration path is accepted outside installed plugin packages

  **QA Scenarios**:
  ```
  Scenario: Installed HTTP MCP package succeeds
    Tool: Bash
    Steps: Run `cargo test -p chainbot --test plugin_install_surface mcp_http_package_install_roundtrip -- --exact` using a fixture package whose config declares `entrypoint = "mcp.tool.v1"` and `transport = "streamable_http"`.
    Expected: Exit code 0; install succeeds, the package is materialized under `plugins/<plugin_id>/config.toml`, and `source.entry_artifact` is preserved as package metadata rather than treated as an executable requirement.
    Evidence: .sisyphus/evidence/task-2-mcp-install.txt

  Scenario: Direct server reference rejected
    Tool: Bash
    Steps: Run `cargo test -p chainbot --test plugin_source_surface reject_direct_mcp_server_reference -- --exact`.
    Expected: Exit code 0; source/install path rejects raw external MCP endpoint configuration outside package manifests.
    Evidence: .sisyphus/evidence/task-2-mcp-install-error.txt
  ```

  **Commit**: YES | Message: `feat(plugin): preserve package-only mcp install flow` | Files: `crates/chainbot/src/plugin/source/**/*`, `crates/chainbot/tests/**/*`

- [x] 3. Introduce internal node invoker seam and per-invocation MCP lifecycle policy

  **What to do**: Refactor the plugin subsystem so `app/runtime/execution.rs` dispatches through an internal node-invoker seam with separate implementations for legacy subprocess plugins and MCP-backed plugins. Encode the Phase 1 lifecycle decision explicitly: each MCP node invocation creates one MCP client/session and tears it down after the call. Normalize timeout, cancellation, and protocol failures into existing plugin execution error surfaces.
  **Must NOT do**: Do not let `app/` or `domain/` depend on `rmcp` types. Do not add pooling or long-lived session reuse. Do not rewrite the legacy subprocess host path.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: this is the architectural seam that prevents leakage and future rewrites
  - Skills: `[]` — no extra skill required
  - Omitted: `[]` — no omission needed

  **Parallelization**: Can Parallel: NO | Wave 1 | Blocks: 4, 5, 6, 7 | Blocked By: 1

  **References**:
  - Pattern: `crates/chainbot/src/plugin/host.rs` — current subprocess host behavior and error mapping
  - Pattern: `crates/chainbot/src/app/runtime/execution.rs` — current workflow node dispatch path
  - Pattern: `crates/chainbot/src/domain/runtime/contract.rs` — runtime execution contract boundary
  - Test: `crates/chainbot/tests/node_plugin_host.rs` — current host-level behavior expectations
  - External: `https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/src/service/client.rs` — client lifecycle shape

  **Acceptance Criteria** (agent-executable only):
  - [ ] `cargo test -p chainbot mcp_invoker_lifecycle -- --exact` exits `0`
  - [ ] `cargo test -p chainbot mcp_error_mapping -- --exact` exits `0`
  - [ ] `cargo test -p chainbot --test node_plugin_host` exits `0`
  - [ ] Legacy subprocess plugin execution path remains behaviorally unchanged

  **QA Scenarios**:
  ```
  Scenario: Per-invocation session lifecycle
    Tool: Bash
    Steps: Run `cargo test -p chainbot mcp_invoker_lifecycle -- --exact`; assert one initialize sequence and one teardown per node invocation.
    Expected: Exit code 0; test proves no session reuse across separate invocations.
    Evidence: .sisyphus/evidence/task-3-mcp-invoker.txt

  Scenario: Timeout normalization
    Tool: Bash
    Steps: Run `cargo test -p chainbot mcp_error_mapping_timeout -- --exact` against a hanging mock transport.
    Expected: Exit code 0; timeout surfaces as an existing plugin execution error, with no panic and no leaked task.
    Evidence: .sisyphus/evidence/task-3-mcp-invoker-error.txt
  ```

  **Commit**: YES | Message: `refactor(plugin): add mcp invoker seam` | Files: `crates/chainbot/src/plugin/**/*`, `crates/chainbot/src/app/runtime/execution.rs`, `crates/chainbot/tests/**/*`

- [x] 4. Add rmcp adapter core for discovery validation, schema mapping, and protocol normalization

  **What to do**: Build the internal adapter that wraps `rmcp` client/session behavior behind a local interface. Implement initialize, tool discovery, and tool invocation flow. Use runtime `list_tools` only to validate that the manifest-declared operation exists and is callable. Map tool schema and result shapes into existing ChainBot request/response semantics. Reject unsupported or ambiguous MCP shapes deterministically.
  **Must NOT do**: Do not make dynamic runtime discovery authoritative. Do not add resources/prompts support. Do not pass raw `rmcp` response types across the plugin boundary.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: protocol normalization and shape translation are the core correctness risks
  - Skills: `[]` — no extra skill required
  - Omitted: `[]` — no omission needed

  **Parallelization**: Can Parallel: YES | Wave 1 | Blocks: 5, 6, 7 | Blocked By: 1, 3

  **References**:
  - Pattern: `crates/chainbot/src/plugin/host.rs` — current request/response and error translation patterns
  - Pattern: `crates/chainbot/src/plugin/contract.rs` — operation declaration and schema expectations
  - Test: `crates/chainbot/tests/node_plugin_host.rs` — existing input/output validation style
  - External: `https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/src/service/client.rs` — `list_tools` and `call_tool` APIs
  - External: `https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle` — initialize protocol
  - External: `https://modelcontextprotocol.io/specification/2025-11-25/server/tools` — tool semantics

  **Acceptance Criteria** (agent-executable only):
  - [ ] `cargo test -p chainbot mcp_tool_discovery_validation -- --exact` exits `0`
  - [ ] `cargo test -p chainbot mcp_schema_translation -- --exact` exits `0`
  - [ ] `cargo test -p chainbot mcp_invalid_tool_schema_rejected -- --exact` exits `0`
  - [ ] Unsupported MCP capabilities are ignored safely and do not affect tool execution

  **QA Scenarios**:
  ```
  Scenario: Manifest tool matches discovered MCP tool
    Tool: Bash
    Steps: Run `cargo test -p chainbot mcp_tool_discovery_validation -- --exact` with a mock server exposing tool `echo` and a manifest declaring operation `echo`.
    Expected: Exit code 0; initialize, discovery, and invocation all succeed, and the adapter treats manifest declaration as authoritative.
    Evidence: .sisyphus/evidence/task-4-mcp-adapter.txt

  Scenario: Invalid schema rejected
    Tool: Bash
    Steps: Run `cargo test -p chainbot mcp_invalid_tool_schema_rejected -- --exact` against a mock server whose `echo` tool reports an unsupported input schema.
    Expected: Exit code 0; adapter fails closed with a deterministic validation error before execution.
    Evidence: .sisyphus/evidence/task-4-mcp-adapter-error.txt
  ```

  **Commit**: YES | Message: `feat(plugin): add rmcp adapter core` | Files: `crates/chainbot/src/plugin/**/*`, `crates/chainbot/tests/**/*`

- [x] 5. Implement local stdio MCP transport backend

  **What to do**: Add a local `stdio` transport backend using `rmcp` child-process transport for installed MCP plugins. Reuse the repository’s existing subprocess safety expectations for executable path validation, environment allowlisting, cancellation, and teardown. Support happy-path tool invocation and deterministic failures for missing executable, early process exit, malformed MCP payloads, and initialization failure.
  **Must NOT do**: Do not bypass executable safety checks. Do not silently retry indefinitely. Do not add process pooling in Phase 1.

  **Recommended Agent Profile**:
  - Category: `unspecified-high` — Reason: bounded transport implementation once contract and adapter are fixed
  - Skills: `[]` — no extra skill required
  - Omitted: `[]` — no omission needed

  **Parallelization**: Can Parallel: YES | Wave 1 | Blocks: 7 | Blocked By: 3, 4

  **References**:
  - Pattern: `crates/chainbot/src/plugin/host.rs` — executable-path and env policy patterns
  - Pattern: `crates/chainbot/tests/node_plugin_host.rs` — subprocess-based plugin test style
  - External: `https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/src/transport/child_process.rs` — stdio child-process transport behavior
  - External: `https://github.com/modelcontextprotocol/rust-sdk/blob/main/README.md` — tokio runtime expectations

  **Acceptance Criteria** (agent-executable only):
  - [ ] `cargo test -p chainbot --test mcp_plugin_host stdio_tool_call_happy_path -- --exact` exits `0`
  - [ ] `cargo test -p chainbot --test mcp_plugin_host stdio_missing_executable_fails_closed -- --exact` exits `0`
  - [ ] `cargo test -p chainbot --test mcp_plugin_host stdio_initialize_failure_maps_to_plugin_error -- --exact` exits `0`
  - [ ] `cargo test -p chainbot --test mcp_plugin_host stdio_malformed_payload_rejected -- --exact` exits `0`

  **QA Scenarios**:
  ```
  Scenario: Local stdio happy path
    Tool: Bash
    Steps: Run `cargo test -p chainbot --test mcp_plugin_host stdio_tool_call_happy_path -- --exact` with a fixture MCP server exposing `echo` and input `{"message":"hello"}`.
    Expected: Exit code 0; result is mapped into ChainBot plugin output without leaking rmcp types.
    Evidence: .sisyphus/evidence/task-5-mcp-stdio.txt

  Scenario: Child process exits before initialize
    Tool: Bash
    Steps: Run `cargo test -p chainbot --test mcp_plugin_host stdio_initialize_failure_maps_to_plugin_error -- --exact`.
    Expected: Exit code 0; execution fails with a deterministic plugin error and no zombie child process remains.
    Evidence: .sisyphus/evidence/task-5-mcp-stdio-error.txt
  ```

  **Commit**: YES | Message: `feat(plugin): add stdio mcp transport` | Files: `crates/chainbot/src/plugin/**/*`, `crates/chainbot/tests/mcp_plugin_host.rs`

- [x] 6. Implement Streamable HTTP MCP transport backend with static auth only

  **What to do**: Add a Streamable HTTP transport backend using `rmcp` HTTP client support. Support session establishment, header continuity, stale-session recovery, timeout handling, and static execution-time auth references sourced from the existing secret model. Lock Phase 1 auth to one additive manifest shape: `[mcp.auth]` requires both `header_name` and `token_secret_ref`, and the resolved secret value is injected verbatim as the final HTTP header value at execution time. Keep OAuth/browser-driven auth explicitly out of scope. Normalize `401`, `403`, non-MCP responses, and stale-session failures into deterministic plugin errors.
  **Must NOT do**: Do not add OAuth browser/device flow. Do not persist auth tokens across restarts in Phase 1. Do not move auth concerns into install/source metadata. Do not infer bearer-prefix behavior or alternate auth schemes from secret contents.

  **Recommended Agent Profile**:
  - Category: `unspecified-high` — Reason: transport-specific implementation with meaningful failure handling complexity
  - Skills: `[]` — no extra skill required
  - Omitted: `[]` — no omission needed

  **Parallelization**: Can Parallel: YES | Wave 2 | Blocks: 7 | Blocked By: 2, 3, 4

  **References**:
  - Pattern: `crates/chainbot/src/plugin/host.rs` — current error mapping and timeout policy patterns
  - Pattern: `crates/chainbot/src/secrets/**/*` — existing secret resolution surfaces
  - External: `https://modelcontextprotocol.io/specification/2025-11-25/basic/transports` — Streamable HTTP transport rules
  - External: `https://modelcontextprotocol.io/specification/2025-11-25/basic/security_best_practices` — security guardrails
  - External: `https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/src/transport/streamable_http_client.rs` — session and reconnect behavior
  - External: `https://github.com/modelcontextprotocol/rust-sdk/blob/main/docs/OAUTH_SUPPORT.md` — auth feature boundary to defer

  **Acceptance Criteria** (agent-executable only):
  - [ ] `cargo test -p chainbot --test mcp_plugin_host http_tool_call_happy_path -- --exact` exits `0`
  - [ ] `cargo test -p chainbot --test mcp_plugin_host http_401_maps_to_plugin_error -- --exact` exits `0`
  - [ ] `cargo test -p chainbot --test mcp_plugin_host http_stale_session_recovers_or_fails_deterministically -- --exact` exits `0`
  - [ ] `cargo test -p chainbot --test mcp_plugin_host http_non_mcp_endpoint_rejected -- --exact` exits `0`
  - [ ] `cargo test -p chainbot --test mcp_plugin_host http_auth_requires_header_name_and_secret_ref -- --exact` exits `0`

  **QA Scenarios**:
  ```
  Scenario: Remote HTTP happy path
    Tool: Bash
    Steps: Run `cargo test -p chainbot --test mcp_plugin_host http_tool_call_happy_path -- --exact` against a local fixture server at `http://127.0.0.1:PORT/mcp` with tool `echo` and header-based auth from secrets.
    Expected: Exit code 0; initialize, tool discovery, and invocation succeed through Streamable HTTP.
    Evidence: .sisyphus/evidence/task-6-mcp-http.txt

  Scenario: Auth failure path
    Tool: Bash
    Steps: Run `cargo test -p chainbot --test mcp_plugin_host http_401_maps_to_plugin_error -- --exact` with an invalid secret-backed token.
    Expected: Exit code 0; 401 is normalized to a plugin execution error with no credential leakage.
    Evidence: .sisyphus/evidence/task-6-mcp-http-error.txt

  Scenario: Incomplete auth contract rejected
    Tool: Bash
    Steps: Run `cargo test -p chainbot --test mcp_plugin_host http_auth_requires_header_name_and_secret_ref -- --exact` with a fixture manifest that omits either `header_name` or `token_secret_ref`.
    Expected: Exit code 0; execution fails closed before the HTTP request is sent, with a deterministic plugin contract error and no credential leakage.
    Evidence: .sisyphus/evidence/task-6-mcp-http-contract-error.txt
  ```

  **Commit**: YES | Message: `feat(plugin): add streamable http mcp transport` | Files: `crates/chainbot/src/plugin/**/*`, `crates/chainbot/tests/mcp_plugin_host.rs`, `crates/chainbot/src/secrets/**/*`

- [x] 7. Integrate MCP-backed plugins into runtime execution without regressing legacy dispatch

  **What to do**: Wire the new MCP invoker path into the existing workflow execution engine so `plugin_id + operation` continues to be the only workflow-facing contract. Route `node.exec.v1` plugins through the legacy path and `mcp.tool.v1` plugins through the new MCP path. Preserve existing execution report semantics, timeout behavior, and node error propagation.
  **Must NOT do**: Do not change workflow syntax. Do not make the execution layer aware of transport-specific details. Do not special-case MCP in domain runtime contracts.

  **Recommended Agent Profile**:
  - Category: `deep` — Reason: central execution-path integration with compatibility risk
  - Skills: `[]` — no extra skill required
  - Omitted: `[]` — no omission needed

  **Parallelization**: Can Parallel: NO | Wave 2 | Blocks: 8, F1-F4 | Blocked By: 5, 6

  **References**:
  - Pattern: `crates/chainbot/src/app/runtime/execution.rs` — workflow node dispatch and reporting
  - Pattern: `crates/chainbot/src/plugin/host.rs` — legacy external node behavior to preserve
  - Pattern: `crates/chainbot/src/domain/runtime/report.rs` — runtime report expectations
  - Test: `crates/chainbot/tests/node_plugin_host.rs` — legacy behavior baseline
  - Test: `crates/chainbot/tests/trigger_plane.rs` — adjacent runtime behavior that must not regress

  **Acceptance Criteria** (agent-executable only):
  - [ ] `cargo test -p chainbot --test node_plugin_host --test mcp_plugin_host` exits `0`
  - [ ] `cargo test -p chainbot runtime_execution_routes_mcp_tool_entrypoint -- --exact` exits `0`
  - [ ] `cargo test -p chainbot runtime_execution_preserves_legacy_external_node_dispatch -- --exact` exits `0`
  - [ ] `cargo test -p chainbot --test trigger_plane` exits `0`

  **QA Scenarios**:
  ```
  Scenario: Workflow invokes installed MCP plugin
    Tool: Bash
    Steps: Run `cargo test -p chainbot runtime_execution_routes_mcp_tool_entrypoint -- --exact` using a workflow node that references installed plugin `mcp-echo` and operation `echo`.
    Expected: Exit code 0; workflow execution succeeds without any workflow-schema change.
    Evidence: .sisyphus/evidence/task-7-mcp-runtime.txt

  Scenario: Legacy plugin path unchanged
    Tool: Bash
    Steps: Run `cargo test -p chainbot runtime_execution_preserves_legacy_external_node_dispatch -- --exact` and `cargo test -p chainbot --test node_plugin_host`.
    Expected: Exit code 0; legacy `node.exec.v1` plugin behavior remains unchanged.
    Evidence: .sisyphus/evidence/task-7-mcp-runtime-error.txt
  ```

  **Commit**: YES | Message: `feat(runtime): route mcp plugins through execution engine` | Files: `crates/chainbot/src/app/runtime/execution.rs`, `crates/chainbot/src/plugin/**/*`, `crates/chainbot/tests/**/*`

- [x] 8. Surface installed MCP plugins correctly in catalog and CLI views

  **What to do**: Update catalog and CLI read models so installed MCP plugins show up as installed plugins, not as a separate product concept. Display transport/runtime details accurately enough for operators to understand whether a package is stdio or HTTP backed, while keeping the package-centric catalog/install split intact. Ensure help and status output remain stable for non-MCP plugins.
  **Must NOT do**: Do not add direct-connect management commands. Do not merge remote source view with installed plugin catalog. Do not surface resources/prompts terminology in Phase 1 CLI.

  **Recommended Agent Profile**:
  - Category: `unspecified-high` — Reason: read-model and CLI consistency work with regression risk
  - Skills: `[]` — no extra skill required
  - Omitted: `[]` — no omission needed

  **Parallelization**: Can Parallel: YES | Wave 2 | Blocks: F1-F4 | Blocked By: 1, 2, 7

  **References**:
  - Pattern: `crates/chainbot/src/app/cli/view/catalog.rs` — installed catalog read model
  - Pattern: `crates/chainbot/src/app/cli/view/plugin_source.rs` — source/install read model split
  - Pattern: `crates/chainbot/src/app/cli/commands.rs` — plugin source and catalog wiring
  - Test: `crates/chainbot/tests/catalog_surface.rs` — installed-plugin output expectations
  - Test: `crates/chainbot/tests/cli_surface.rs` — help/status surface expectations
  - External: `docs/design/CHAINBOT_CLI_DESIGN.md` — CLI boundary contract

  **Acceptance Criteria** (agent-executable only):
  - [ ] `cargo test -p chainbot --test catalog_surface` exits `0`
  - [ ] `cargo test -p chainbot --test cli_surface` exits `0`
  - [ ] Installed MCP packages are visible in catalog output without introducing a new top-level plugin class
  - [ ] Existing non-MCP help/status output remains unchanged except additive MCP-specific details

  **QA Scenarios**:
  ```
  Scenario: Installed MCP package appears in catalog
    Tool: Bash
    Steps: Run `cargo test -p chainbot --test catalog_surface catalog_lists_installed_mcp_plugin -- --exact`.
    Expected: Exit code 0; output includes the installed package, transport detail, and declared operations.
    Evidence: .sisyphus/evidence/task-8-mcp-catalog.txt

  Scenario: CLI rejects direct-connect mental model
    Tool: Bash
    Steps: Run `cargo test -p chainbot --test cli_surface help_plugin_does_not_offer_direct_mcp_connect -- --exact`.
    Expected: Exit code 0; CLI/help output preserves the package-centric model and does not expose raw server connection commands.
    Evidence: .sisyphus/evidence/task-8-mcp-catalog-error.txt
  ```

  **Commit**: YES | Message: `feat(cli): surface installed mcp plugins in catalog` | Files: `crates/chainbot/src/app/cli/**/*`, `crates/chainbot/tests/catalog_surface.rs`, `crates/chainbot/tests/cli_surface.rs`

## Final Verification Wave (MANDATORY — after ALL implementation tasks)
> 4 review agents run in PARALLEL. ALL must APPROVE. Present consolidated results to user and get explicit "okay" before completing.
> **Do NOT auto-proceed after verification. Wait for user's explicit approval before marking work complete.**
> **Never mark F1-F4 as checked before getting user's okay.** Rejection or user feedback -> fix -> re-run -> present again -> wait for okay.
- [x] F1. Plan Compliance Audit — oracle
- [x] F2. Code Quality Review — unspecified-high
- [x] F3. Real Manual QA — unspecified-high
- [x] F4. Scope Fidelity Check — deep

## Commit Strategy
- Commit 1: `feat(plugin): add mcp external node manifest contract`
- Commit 2: `feat(plugin): preserve package-only mcp install flow`
- Commit 3: `refactor(plugin): add mcp invoker seam`
- Commit 4: `feat(plugin): add rmcp adapter core`
- Commit 5: `feat(plugin): add stdio mcp transport`
- Commit 6: `feat(plugin): add streamable http mcp transport`
- Commit 7: `feat(runtime): route mcp plugins through execution engine`
- Commit 8: `feat(cli): surface installed mcp plugins in catalog`
- Do not squash transport and contract work together; preserving rollback boundaries matters more than minimizing commit count.

## Success Criteria
- Existing `external_node` plugins keep working with no manifest edits and no workflow syntax changes.
- MCP-backed plugins can be installed through the current package pipeline and executed through the existing node runtime.
- Both `stdio` and Streamable HTTP MCP tools work for happy-path execution.
- Invalid transport config, invalid schema, stale sessions, malformed payloads, and auth failures all fail closed with deterministic plugin errors.
- Catalog and CLI surfaces remain package-centric and display installed MCP plugins without creating a second plugin model.
- No Phase 1 scope leaks into resources/prompts/triggers/OAuth browser flow/direct raw server references.
