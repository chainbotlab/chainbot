# Local Rules

## Scope
- Position: Application definition-loading and validation boundary.
- Logic: Owns root bundle assembly from workspace packages and cross-package contract validation.
- Constraints: Keep storage and path adapter concerns in `../../infrastructure/`.

## Members
- `mod.rs`: Definition-loading module boundary.
- `root_bundle.rs`: Root workspace bundle loading and assembly.
- `validate.rs`: Cross-package validation for workflows, triggers, and plugin manifests.
