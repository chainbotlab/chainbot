# Local Rules

## Scope
- Position: Curated root-level ChainBot configuration examples for humans and agents.
- Logic: Each example root demonstrates the current canonical config contract without test-only noise or legacy layouts.
- Constraints: Keep examples deterministic, credential-free, and aligned with `../.agents/skills/decision-chainbot-root-package-layout/SKILL.md` plus `../.agents/skills/decision-chainbot-workflow-dag-design/SKILL.md`.

## Constraints
- Prefer canonical `chainbot.toml` + package-directory layouts.
- Do not add `plugins/manifests/` legacy examples.
- Do not add failure-only or secret-blob fixtures.
- Keep examples copyable as standalone roots.

## Members
- `README.md`: Index of available example roots and what each one demonstrates.
- `single-workflow/`: Smallest standalone workflow-only root for the base package layout.
- `builtin-triggers/`: Shared workflow root showing builtin manual, market-tick, and cron trigger packages.
- `workflow-composition/`: Parent/child workflow root focused on subflow contracts without external plugin setup.
- `plugin-integrations/`: External trigger and external node plugin root focused on package-aligned plugin integration.
- `http-plugin-integrations/`: Official outbound HTTP node root showing the installed-plugin mirror after the canonical source/install path.
- `eth-plugin-integrations/`: Official Ethereum node and trigger root with activation secret bindings and live-only listener examples.
- `solana-plugin-integrations/`: Official Solana node and trigger root with activation secret bindings and live-only listener examples.
- `custom-paths/`: Root showing non-default `[paths]` overrides while staying inside the configured root.
