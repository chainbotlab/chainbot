# Local Rules

## Architecture
- Position: Rust source tree for the `chainbot` binary crate.
- Logic: Source files define the executable entrypoint and frozen V2 contract modules.
- Constraints: Keep Rust file headers aligned with actual inputs, outputs, and architectural position.

## Members
- `main.rs`: Binary entrypoint that routes CLI stdout/stderr and explicit process exit codes.
- `lib.rs`: Public module map that freezes the V2 boundary for MVP contracts.
- `builtins/`: Unified builtin namespace containing workflow node builtins, trigger builtins, and shared dispatch boundaries.
- `cli.rs`: CLI parser/executor for `help`, `validate`, `list-runs`, `run`, and `serve` with root overrides, restart-only reload policy, runtime recovery, and stable user-facing failures.
- `config.rs`: Explicit root-layout resolver plus v2.1 package-manifest loaders for root config, workflow packages, trigger packages, and shared plugin manifests.
- `workflow.rs`: Workflow package semantic contract with nested manifest-header parsing, DAG/cycle validation, typed runtime variable namespaces and precedence layers, explicit subflow input/output boundaries, and deterministic `when` evaluators.
- `trigger.rs`: Trigger-package runtime contract plus trigger-plane orchestration for builtin/external trigger sources, workflow binding, input mapping, dedup/cooldown coordination, restart-safe duplicate suppression, and normalized run-request emission through builtin trigger dispatch helpers.
- `executor.rs`: Rust-owned execution plane with scheduler node states, dependency/condition/subflow orchestration, and canonical builtin node registry dispatch contracts.
- `plugin/`: Encapsulated plugin subsystem containing the stable public facade plus internal contract and host implementation modules.
- `script_protocol.rs`: Versioned script-worker request/response envelope contract shared by config loading, builtin script execution, and protocol tests.
- `builtins/nodes/script_worker.rs`: Bounded subprocess host for Python/JavaScript builtin script nodes plus timeout cleanup and runtime failure mapping.
- `state.rs`: Run summary schema plus state-tree layout, crash-safe file persistence, append-only workflow/trigger artifacts, minimal SQLite coordination, deterministic run-summary listing, and restart recovery.
- `secrets.rs`: Secret reference parser plus pass-style runtime secret provider, decryptor seam, and redaction helpers that prevent durable plaintext leakage.
- `errors.rs`: Typed contract error variants plus stable CLI-facing error and exit-code mapping.
