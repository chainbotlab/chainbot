# AGENTS.md

## Scope
- Position: Concrete adapter layer.
- Owns: Layout resolution, package loading, and runtime-state persistence adapters.
- Excludes: Backend-agnostic validation, scheduling, and CLI orchestration.

## Constraints
- Keep concrete side effects here.
- Do not duplicate contracts owned by `../domain/` or orchestration owned by `../app/`.

## Members
- `mod.rs`: Infrastructure namespace root.
- `config/`: Configuration and root-layout adapters.
- `state/`: Runtime-state persistence adapters.
