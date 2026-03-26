# Local Rules

## Architecture
- Position: Integration test boundary for the `chainbot` crate.
- Logic: Validate contract compatibility plus workflow-semantic DAG/namespace boundaries, runtime-state persistence behavior, and subprocess worker-host safety boundaries.
- Constraints: Keep tests deterministic and scoped to contract/runtime-state boundaries plus execution-plane scheduler semantics without external side-effect dependencies.

## Members
- `contract_versions.rs`: Verifies accepted current versions and rejects unsupported future majors.
- `catalog_surface.rs`: Verifies the catalog list/show CLI surface, JSON payload stability, and plugin metadata fallback behavior.
- `cli_surface.rs`: Verifies help output, validate/list-runs success paths, and bounded user-facing failures for unimplemented commands.
- `config_loading.rs`: Verifies v2.1 root layout resolution plus package-manifest loader behavior for valid fixtures, missing paths, and invalid TOML.
- `runtime_state_parity.rs`: Verifies DB-primary runtime-state semantics stay aligned between SQLite local mode and PostgreSQL mode for leases, run summaries, trigger snapshots/checkpoints, and trigger history visibility.
- `runtime_guardrails.rs`: Guards SQLite runtime read-query plans and repeated read-only observation loops for status/observe hot paths.
- `state_runtime_persistence.rs`: Verifies legacy file-backed runtime persistence plus SQLite coordination behavior that remains available outside the DB-primary main runtime path.
- `workflow_dag_semantics.rs`: Verifies DAG cycle/dependency validation, deterministic runtime variable precedence, and explicit subflow import/export boundary enforcement.
- `execution_scheduler.rs`: Verifies Rust-owned scheduler wave planning, `depends_mode`/`when` behavior, and builtin node registry typed dispatch failures.
- `node_plugin_host.rs`: Verifies external node plugin manifest guards, stdin/stdout roundtrip contract, and capability/version rejection before spawn.
- `trigger_plane.rs`: Verifies trigger package manifest policy checks, builtin/external run-request normalization, workflow binding, dedup/cooldown persistence, and DB-backed trigger records.
- `secrets_runtime.rs`: Verifies pass-style secret resolution, decryption-failure redaction, and non-persistence guarantees for secret material.
- `fixtures/`: Deterministic pass-style filesystem fixtures consumed by runtime secret-provider tests.
- `worker_host.rs`: Verifies subprocess worker protocol negotiation, Python/JavaScript roundtrip behavior, timeout cleanup, malformed output handling, and stdout/stderr size limits.
- `end_to_end_vertical_slice.rs`: Verifies the task-11 runnable vertical slice for `validate`/`run`/`serve`/`list-runs`, persisted artifacts, bounded failure-mode behavior, and task-12 restart-hardening flows.
- `fixtures/workers/`: Deterministic Python and JavaScript worker scripts used by worker-host integration tests.
- `fixtures/e2e/`: Deterministic v2.1 package-layout fixture roots used by task-11 end-to-end vertical-slice tests and e2e root bootstrapping.
