# AGENTS.md

## Scope
- Position: Application-layer boundary for process-facing ChainBot behavior.
- Logic: Owns CLI request parsing and help rendering, root-definition loading and validation, and runtime orchestration that bridges process entrypoints into domain and infrastructure modules.
- Constraints: Keep process-facing orchestration here; do not move backend-agnostic contracts out of `../domain/` or storage and layout adapters out of `../infrastructure/`.

## Members
- `mod.rs`: Application boundary root exporting CLI entrypoints, definition loading, and runtime execution wiring.
- `cli/`: Command parsing, help surfaces, and user-facing read models.
- `definitions/`: Root workspace bundle loading and cross-package validation.
- `runtime/`: Workflow execution, daemon lifecycle, and external-trigger supervision.
