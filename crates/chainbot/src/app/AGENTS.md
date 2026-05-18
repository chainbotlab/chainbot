# AGENTS.md

## Scope
- Position: Application-layer boundary for process-facing ChainBot behavior.
- Owns: CLI request parsing and help rendering, root-definition loading and validation, and runtime orchestration that bridges process entrypoints into domain and infrastructure modules.
- Excludes: Backend-agnostic contracts and storage/layout adapters.

## Constraints
- Keep process-facing orchestration in `app/`.
- Keep backend-agnostic contracts in `../domain/` and storage/layout adapters in `../infrastructure/`.

## Members
- `mod.rs`: Application boundary root exporting CLI entrypoints, definition loading, and runtime execution wiring.
- `cli/`: Command parsing, help surfaces, and user-facing read models.
- `definitions/`: Root workspace bundle loading and cross-package validation.
- `runtime/`: Workflow execution, daemon lifecycle, and external-trigger supervision.
