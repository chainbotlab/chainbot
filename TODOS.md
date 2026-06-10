# TODOS

## Infrastructure

### Pre-existing test failures in `crates/chainbot/src/app/cli/commands.rs`

**What:** Fix 4 pre-existing test failures in serve/cli command tests.

**Why:** These tests fail due to SQLite "readonly database" and missing test-root directory environment issues, causing CI noise on every branch.

**Context:**
- `serve_bridges_pending_staged_external_events` — missing test-root directory at target/test-roots/cli-serve-bridges-staged-external-events/plugins/trigger-e2e-plugin
- `serve_lease_supervisor_renews_same_owner_lease` — assertion failed on lease acquire (LeaseAcquireResult::Acquired)
- `serve_drains_all_requests_from_snapshot` — SQLite readonly database at target/test-roots/cli-serve-drains-all-requests/state/runtime.sqlite3
- `config_reload_requires_restart` — SQLite readonly database at target/test-roots/cli-config-reload-requires-restart/state/runtime.sqlite3
- Noticed on branch `cross-chain-bridge-plugin-selection` by gstack /ship on 2026-06-09

**Effort:** M
**Priority:** P0

## Completed
