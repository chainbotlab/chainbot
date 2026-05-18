# AGENTS.md

## Scope
- Position: Integration test boundary for the `chainbot` crate.
- Owns: Contract compatibility, workflow semantics, runtime-state persistence, scheduler behavior, and host/runtime guardrail tests.
- Excludes: Flaky or external side-effect-dependent test coverage.

## Constraints
- Keep tests deterministic.
- Focus coverage on contract, runtime-state, and execution semantics.

## Members
- `contract_versions.rs`: Versioned contract compatibility checks.
- `catalog_surface.rs`: Catalog surface behavior.
- `cli_surface.rs`: CLI integration expectations.
- `config_loading.rs`: Root and package loading behavior.
- `runtime_state_parity.rs`: Runtime-state parity guarantees.
- `runtime_guardrails.rs`: Runtime guardrail assertions.
- `state_runtime_persistence.rs`: Persistence round-trip behavior.
- `workflow_dag_semantics.rs`: Workflow graph semantics.
- `execution_scheduler.rs`: Scheduler execution semantics.
- `mcp_plugin_host.rs`, `node_plugin_host.rs`, `chain_node_plugin_host.rs`: Plugin host boundaries.
- `trigger_plane.rs`, `chain_trigger_runtime.rs`: Trigger-plane runtime behavior.
- `secrets_runtime.rs`, `worker_host.rs`, `end_to_end_vertical_slice.rs`: Secrets, worker host, and vertical-slice coverage.
- `fixtures/`: Deterministic fixture roots and secret placeholders.
