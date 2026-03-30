## 2026-03-29 Task 1 learnings

- `PluginManifest` validation is currently kind-scoped first; adding MCP support safely required making `external_node` validation entrypoint-aware (`node.exec.v1` vs `mcp.tool.v1`) instead of introducing a new plugin kind.
- Existing integration surfaces (`node_plugin_host`, `plugin_install_surface`) rely on `PluginManifest` struct literals in tests; adding manifest fields requires updating those literals to keep deterministic compile-time coverage.
- The most stable deterministic mixed-transport guard is to require `mcp.transport` and then reject opposite transport blocks (`transport=stdio` rejects `mcp.streamable_http`, `transport=streamable_http` rejects `mcp.stdio`).

## 2026-03-29 Task 2 learnings

- Source/install alignment in `plugin/source/discover.rs` was the primary blocker for MCP HTTP package install because it still required `plugin.executable` for `python|node|bin` runtimes regardless of `entrypoint`.
- Install preparation in `plugin/source/prepare.rs` also required a local executable artifact for non-wasm runtimes; MCP streamable HTTP must bypass that artifact check to remain package-installed without introducing network behavior at install time.
- A deterministic guardrail against direct server references is easiest at source metadata validation time: rejecting URL-shaped `source.entry_artifact` preserves package-relative artifact semantics and keeps raw server endpoints authoritative only under `plugin.mcp.*`.

## 2026-03-29 Task 3 learnings

- `execution.rs` can stay backend-agnostic by switching only the host call to a plugin-owned seam (`ExternalNodePluginHost::execute_node_invocation`), which avoids leaking MCP client/session concepts into app or domain runtime contracts.
- The cleanest Task 3 split is entrypoint-based invoker selection inside `plugin/host.rs` (`node.exec.v1` vs `mcp.tool.v1`) while preserving `PluginKind::ExternalNode` behavior from Task 1.
- Phase-1 MCP lifecycle can be made explicit before transport implementation by creating a per-call session guard in the MCP branch and proving it in tests without introducing pooling or long-lived runtime state.

## 2026-03-29 Task 4 learnings

- The MCP adapter seam can stay transport-agnostic by introducing a local session interface (`initialize` / `list_tools` / `call_tool`) and keeping all protocol-shape translation in `plugin/host.rs`, so app/domain layers remain unchanged.
- Runtime `list_tools` validation works best as a strict gate: manifest operation names remain authoritative, while discovery only proves that the declared operation exists exactly once and has a callable object-shaped input schema.
- Deterministic fail-closed behavior for MCP output normalization requires rejecting ambiguous payloads (e.g. conflicting `structured_content` vs content JSON) instead of guessing which result shape to trust.

## 2026-03-29 Task 4 retry learnings

- Full-suite parallel test execution exposed hidden cross-test interference from test-only global lifecycle/adapter registries in `plugin/host.rs`; this can drop the recorded `start:*` event while still retaining `stop:*`.
- Converting those test-only registries to thread-local storage removes inter-test contamination without changing production lifecycle behavior.

## 2026-03-29 Task 5 learnings

- The safest stdio startup policy is to treat `plugin.mcp.stdio.command` the same way as legacy `plugin.executable`: resolve it relative to the installed plugin manifest root, reject absolute/escaping paths, and apply the same default-deny environment allowlist before spawn.
- `tokio::process::Command` for the rmcp child-process transport must be spawned from inside a live Tokio runtime; constructing the transport outside `runtime.block_on(...)` panics with the missing-reactor error even though the overall host entrypoint is synchronous.
- rmcp stdio uses newline-delimited JSON-RPC framing, so deterministic host integration tests can use a tiny local script fixture that speaks only the required MCP lifecycle/tool methods without introducing a second Rust test binary.

## 2026-03-29 Task 6 learnings

- `rmcp 1.3.0` Streamable HTTP should be wired through `StreamableHttpClientTransport::from_config(...)` instead of passing the crate's own `reqwest 0.12` client into `with_client(...)`, because rmcp's default reqwest backend is compiled against its internal `reqwest 0.13` feature surface.
- Static MCP auth is safest when resolved entirely inside `plugin/host.rs` at invocation time and injected as a sensitive custom header; this preserves the existing secret model and avoids persisting plaintext or pushing auth concerns into install metadata.
- Deterministic HTTP transport tests are easiest with a tiny in-process Axum fixture that returns real `Mcp-Session-Id` headers, records auth/session continuity, and forces one-session `404` expiry to exercise rmcp's transparent stale-session reinitialization path.

## 2026-03-29 Task 7 learnings

- Runtime routing integration can remain app-layer agnostic when `ExecutionPlane` continues to call `ExternalNodePluginHost::execute_node_invocation`; the entrypoint split (`node.exec.v1` vs `mcp.tool.v1`) is fully enforced inside `plugin/host.rs`.
- The most deterministic proof for runtime MCP routing is a workflow-level execution test using `entrypoint = "mcp.tool.v1"` with no legacy `executable` field, because accidental legacy dispatch would fail closed before spawn.
- Legacy-dispatch regression safety is best covered by asserting unchanged workflow report semantics (`status`, `schedule_waves`, `node_states`, `node_outputs`, and `runtime_namespaces.run_scoped`) for a `node.exec.v1` plugin node.

## 2026-03-29 Task 8 learnings

- Installed catalog surfaces cannot safely reuse source-phase `runtime = python|node|bin|wasm` metadata because installed plugin manifests only preserve the runtime-relevant MCP facts `entrypoint = "mcp.tool.v1"` and `mcp.transport`; the additive installed view must therefore describe host session/runtime behavior instead of source packaging runtime.
- The least disruptive presentation change is to append MCP-only transport/runtime details to the existing installed plugin summary/detail paths, leaving non-MCP plugin text and section structure unchanged.

## 2026-03-29 Evidence Refresh

- Stale final-wave evidence artifacts refreshed to reference `mcp-plugin-ecosystem` plan instead of old `chainbot-v2-mvp` plan.
- F1 now cites correct implementation files: `plugin/contract.rs`, `plugin/host.rs`, `plugin/source/*`, `app/runtime/execution.rs`, `app/cli/view/catalog.rs`.
- F2 now covers MCP-specific code quality concerns (lifecycle boundary, transport-agnostic facade, deterministic failure handling) instead of legacy MVP quality issues.
- F3 now records actual MCP test commands and CLI checks: stdio happy path, HTTP happy path, HTTP 401/stale-session/non-MCP rejection, runtime routing tests, catalog list/show, and help plugin.
- F4 now maps Phase 1 scope pillars against `mcp-plugin-ecosystem` deliverables (Tasks 1-8) instead of MVP pillars.
- No source or test files were modified.
