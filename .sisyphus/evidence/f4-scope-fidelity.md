# F4 Scope Fidelity Check (deep)

Verdict: **APPROVE**

## Baseline Compared

- Plan of record: `.sisyphus/plans/chainbot-v2-mvp.md`
- Original MVP goals: CLI-first runtime, trigger/node plugin extensibility, subprocess script runtime, pass/PGP secret resolution with redaction, file-backed runtime artifacts, SQLite limited to minimal coordination.
- Manual QA evidence used as ground truth for execution viability: `.sisyphus/evidence/f3-manual-qa.txt`

## Pillar-by-Pillar Coverage Map

### 1) CLI-first shape

- Command surface is implemented in one binary with explicit `validate`, `run`, `serve`, `list-runs` routing and `--root` override: `crates/chainbot/src/cli.rs`.
- Process entrypoint and user-facing exit behavior are wired in the binary host: `crates/chainbot/src/main.rs`.
- CLI integration tests cover help, valid paths, invalid root error model, and bounded runtime behavior: `crates/chainbot/tests/cli_surface.rs`.
- Manual QA confirms CLI paths execute in workspace verification run: `.sisyphus/evidence/f3-manual-qa.txt`.

### 2) Plugin coverage (external trigger + external node)

- Trigger plugin host policy, allowlist/capability checks, executable confinement, and run-request emission are implemented in trigger plane: `crates/chainbot/src/trigger.rs`.
- Node plugin manifest validation and external node process contract are implemented: `crates/chainbot/src/plugin.rs`.
- Runtime wiring from scheduler builtin nodes to external node host exists: `crates/chainbot/src/cli.rs`.
- Tests validate both plugin classes and rejection paths: `crates/chainbot/tests/trigger_plane.rs`, `crates/chainbot/tests/node_plugin_host.rs`.
- E2E fixture includes both trigger and node manifests plus executables: `crates/chainbot/tests/fixtures/e2e/success/plugins/trigger_e2e.toml`, `crates/chainbot/tests/fixtures/e2e/success/plugins/node_e2e.toml`, `crates/chainbot/tests/fixtures/e2e/success/plugins/bin/external_trigger.sh`, `crates/chainbot/tests/fixtures/e2e/success/plugins/bin/external_node.sh`.

### 3) Subprocess script runtime (Python/JavaScript)

- Worker protocol envelopes, timeout handling, stdout/stderr caps, and child cleanup are implemented: `crates/chainbot/src/worker.rs`.
- Script node dispatch from execution path into worker host is implemented with runtime parser and interpreter resolution: `crates/chainbot/src/cli.rs`.
- Worker-host integration covers protocol negotiation, python/javascript roundtrip, timeout, malformed output, and oversized stream limits: `crates/chainbot/tests/worker_host.rs`.
- E2E fixture uses a real script node operation: `crates/chainbot/tests/fixtures/e2e/success/workflows/e2e.toml`, `crates/chainbot/tests/fixtures/e2e/success/scripts/e2e_worker.py`.

### 4) pass/PGP secrets + redaction

- `secret://` reference parsing, pass-style file mapping, and decryptor seam (GPG + test decryptor) are implemented: `crates/chainbot/src/secrets.rs`.
- Runtime secret resolution is integrated into node input materialization path: `crates/chainbot/src/cli.rs`.
- Redaction helpers and decryption-failure redacted error semantics are present and covered: `crates/chainbot/src/secrets.rs`, `crates/chainbot/src/errors.rs`, `crates/chainbot/tests/secrets_runtime.rs`.
- E2E config/workflow carries secret references and encrypted fixture file: `crates/chainbot/tests/fixtures/e2e/success/config/root.toml`, `crates/chainbot/tests/fixtures/e2e/success/workflows/e2e.toml`, `crates/chainbot/tests/fixtures/e2e/success/secrets/ops/slack/webhook.gpg`.

### 5) File-backed runtime artifacts

- Run summaries, workflow logs, and trigger records are persisted under deterministic file tree with staged atomic writes and recovery: `crates/chainbot/src/state.rs`.
- CLI `list-runs` reads file-backed summaries, and `run`/`serve` write run/log artifacts: `crates/chainbot/src/cli.rs`.
- State tests verify append-only behavior, crash recovery, staged promotion/removal, and file persistence contracts: `crates/chainbot/tests/state_runtime_persistence.rs`.
- E2E tests assert artifact directories are populated and linked to run execution: `crates/chainbot/tests/end_to_end_vertical_slice.rs`.

### 6) Minimal SQLite coordination only

- SQLite schema is limited to `serve_leases`, `coordination_tokens`, and migration ledger; no run summary/log/trigger payload tables: `crates/chainbot/src/state.rs`.
- Lease acquisition/release and dedup/cooldown token coordination are isolated in coordination store APIs: `crates/chainbot/src/state.rs`.
- Tests explicitly assert minimal-table shape and lease exclusivity semantics: `crates/chainbot/tests/state_runtime_persistence.rs`.

### 7) Restart-only reload + restart dedup rebuild (scope-fidelity critical)

- `serve` help and runtime semantics state restart-only config reload behavior: `crates/chainbot/src/cli.rs`.
- Trigger-plane startup rebuilds coordination tokens from durable trigger records, preventing restart drift: `crates/chainbot/src/trigger.rs`, `crates/chainbot/src/state.rs`.
- Tests validate restart reload policy, restart recovery, duplicate suppression after restart, and trigger-coordination rebuild: `crates/chainbot/src/cli.rs` (unit tests), `crates/chainbot/tests/end_to_end_vertical_slice.rs`, `crates/chainbot/tests/trigger_plane.rs`.

## Silent Deferral Check

- No required MVP pillar is missing.
- Scope reductions are explicit (not silent): no HTTP/control plane expansion, and `serve` behavior is intentionally bounded to deterministic snapshot processing in current MVP command semantics (`crates/chainbot/src/cli.rs` + test evidence above).

## Verification Grounding

- Workspace-level manual QA already passed end-to-end verification chain (`cargo metadata`, `cargo check --workspace`, `cargo test --workspace`, `cargo run -p chainbot -- validate --root target/test-roots/e2e`): `.sisyphus/evidence/f3-manual-qa.txt`.
- The pass includes direct success records for pillar-specific tests (`trigger_*`, `worker_*`, `secret_*`, `state_*`, `end_to_end_vertical_slice*`) in `.sisyphus/evidence/f3-manual-qa.txt`.

Final reviewer decision: **APPROVE**.
