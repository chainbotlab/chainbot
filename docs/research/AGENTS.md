# Local Rules

## Architecture
- Position: Storage for exploratory notes that may influence later implementation.
- Logic: Short-lived research can land here before it graduates into design or is discarded.
- Constraints: Do not create placeholder research docs without a real question to capture.
- Constraints: Use this folder for problem framing, option comparison, risk analysis, and interim research conclusions; final stable decisions belong in `docs/design/`, and concrete execution history belongs in `docs/implementation/`.

## Members
- `CHAINBOT_V212_CLI_PROPOSAL.md`: Proposal for the v2.1.2 CLI surface covering `status`, `init`, environment-based root resolution, and skill-oriented `help`.
- `CHAINBOT_CONFIG_STATE_LAYOUT_PROPOSAL.md`: Proposal for aligning plugin package layout, version semantics, and runtime state boundaries under a single root model.
- `CHAINBOT_LEGACY_LAYOUT_CONVERGENCE_PROPOSAL.md`: Proposal for retiring legacy plugin and runtime-state layout reads after canonical layout adoption is proven safe.
