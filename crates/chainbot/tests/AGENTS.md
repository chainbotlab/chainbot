# Local Rules

## Architecture
- Position: Integration test boundary for the `chainbot` crate.
- Logic: Validate contract compatibility plus workflow-semantic DAG/namespace boundaries, runtime-state persistence behavior, and subprocess worker-host safety boundaries.
- Constraints: Keep tests deterministic and scoped to contract/runtime-state boundaries plus execution-plane scheduler semantics without external side-effect dependencies.

## Members
- `contract_versions.rs`: Verifies accepted current versions and rejects unsupported future majors.
- `cli_surface.rs`: Verifies help output, validate/list-runs success paths, and bounded user-facing failures for unimplemented commands.
- `config_loading.rs`: Verifies root layout resolution plus TOML loader behavior for valid fixtures, missing paths, and invalid TOML.
- `state_runtime_persistence.rs`: Verifies minimal SQLite migrations, single-owner serve lease rules, append-safe file-backed runtime logs/trigger records, and crash recovery for staged run summaries.
- `workflow_dag_semantics.rs`: Verifies DAG cycle/dependency validation, deterministic runtime variable precedence, and explicit subflow import/export boundary enforcement.
- `execution_scheduler.rs`: Verifies Rust-owned scheduler wave planning, `depends_mode`/`when` behavior, and builtin node registry typed dispatch failures.
- `node_plugin_host.rs`: Verifies external node plugin manifest guards, stdin/stdout roundtrip contract, and capability/version rejection before spawn.
- `trigger_plane.rs`: Verifies trigger plugin manifest policy checks, builtin/external run-request normalization, dedup/cooldown coordination, and file-backed trigger records.
- `secrets_runtime.rs`: Verifies pass-style secret resolution, decryption-failure redaction, and non-persistence guarantees for secret material.
- `fixtures/`: Deterministic pass-style filesystem fixtures consumed by runtime secret-provider tests.
- `worker_host.rs`: Verifies subprocess worker protocol negotiation, Python/JavaScript roundtrip behavior, timeout cleanup, malformed output handling, and stdout/stderr size limits.
- `end_to_end_vertical_slice.rs`: Verifies the task-11 runnable vertical slice for `validate`/`run`/`serve`/`list-runs`, persisted artifacts, bounded failure-mode behavior, and task-12 restart-hardening flows.
- `fixtures/workers/`: Deterministic Python and JavaScript worker scripts used by worker-host integration tests.
- `fixtures/e2e/`: Deterministic fixture roots used by task-11 end-to-end vertical-slice tests and e2e root bootstrapping.
