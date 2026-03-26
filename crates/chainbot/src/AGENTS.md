# Local Rules

## Architecture
- Position: Rust source tree for the `chainbot` binary crate.
- Logic: Source files define the executable entrypoint and the canonical-only V2-line config contract modules.
- Constraints: Keep Rust file headers aligned with actual inputs, outputs, and architectural position.

## Members
- `main.rs`: Binary entrypoint that routes CLI stdout/stderr and explicit process exit codes.
- `lib.rs`: Public module map that freezes the V2 boundary for MVP contracts.
- `catalog.rs`: CLI-facing capability discovery read model plus human/JSON renderers for builtin and plugin catalog output.
- `builtins/`: Unified builtin namespace containing workflow node builtins, trigger builtins, and shared dispatch boundaries.
- `cli.rs`: CLI parser/executor for `help`, `status`, `observe`, `catalog`, `trigger`, `validate`, `list-runs`, `run`, and `serve` with root overrides, canonical bootstrap, runtime retention hooks, and stable user-facing failures.
- `config.rs`: Explicit root-layout resolver plus canonical package loaders for root config, workflow packages, trigger packages, and plugin packages.
- `workflow.rs`: Workflow package semantic contract with nested manifest-header parsing, DAG/cycle validation, typed runtime variable namespaces and precedence layers, explicit subflow input/output boundaries, and deterministic `when` evaluators.
- `trigger.rs`: Trigger-package runtime contract plus trigger-plane orchestration for builtin/external trigger sources, workflow binding, input mapping, dedup/cooldown coordination, restart-safe duplicate suppression, and normalized run-request emission through builtin trigger dispatch helpers.
- `executor.rs`: Rust-owned execution plane with scheduler node states, dependency/condition/subflow orchestration, canonical builtin registry dispatch, and direct external-node plugin execution.
- `ingress/`: Listener-backed trigger ingress runtime containing webhook/websocket params contracts, route reconciliation, durable inbox bridging, and lease-bound server supervision.
- `plugin/`: Encapsulated plugin subsystem containing the stable public facade plus internal contract and host implementation modules.
- `script_protocol.rs`: Versioned script-worker request/response envelope contract shared by config loading, builtin script execution, and protocol tests.
- `builtins/nodes/script_worker.rs`: Bounded subprocess host for Python/JavaScript builtin script nodes plus timeout cleanup and runtime failure mapping.
- `state.rs`: Legacy file-backed runtime-state and SQLite coordination implementation retained for compatibility, inspection, and future import paths.
- `state_db.rs`: DB-primary runtime-state implementation for local SQLite and PostgreSQL backends, including recent-history observation reads, archival retention moves, and hot-path index guards for CLI and trigger execution paths.
- `secrets.rs`: Secret reference parser plus pass-style runtime secret provider, decryptor seam, and redaction helpers that prevent durable plaintext leakage.
- `errors.rs`: Typed contract error variants plus stable CLI-facing error and exit-code mapping.
