# Symptom

The first OKX official plugin implementation passed local tests but would fail in real private flows:

- `official-plugins/okx-node` signed private REST requests with a fixed `OK-ACCESS-TIMESTAMP`
- `official-plugins/okx-trigger` signed private WebSocket login requests with a fixed login timestamp

That meant `okx_get_account_balance` and `okx_private_stream` could look healthy in mock tests while producing stale signatures against the live OKX API.

# Root Cause

The initial implementation copied the signing shape from the OKX docs but treated the timestamp as a convenient test fixture instead of a runtime input:

- `official-plugins/okx-node/crate/src/provider.rs` implemented `current_timestamp_iso8601()` as a constant string for both test and non-test builds
- `official-plugins/okx-trigger/crate/src/contract.rs` implemented `login_timestamp()` as a constant epoch-seconds string for both test and non-test builds
- tests only asserted header presence and mock-frame success, so they did not fail when the timestamp value itself was stale

The broken assumption was that a hard-coded timestamp was acceptable as long as signing math and request shape were otherwise correct.

# Fix Applied

- changed `official-plugins/okx-node/crate/src/provider.rs` so private REST signing now formats a fresh UTC timestamp at request time using ISO8601 with millisecond precision
- changed `official-plugins/okx-trigger/crate/src/contract.rs` so private login now emits fresh Unix epoch seconds at request time
- corrected OKX manifest artifact names from `bin/binance-*` to `bin/okx-*` so package metadata stays aligned with the actual plugin identity
- strengthened regression coverage:
  - `official-plugins/okx-node/crate/tests/read_rpc.rs` now captures the timestamp header value and verifies it parses as a recent RFC3339 timestamp
  - `official-plugins/okx-trigger/crate/tests/log_listener.rs` now builds a real login request and verifies its timestamp is near current epoch time

# Validation

Targeted tests run after the fix:

- `rtk cargo test` in `official-plugins/okx-node/crate`
- `rtk cargo test` in `official-plugins/okx-trigger/crate`

Observed result:

- OKX node tests passed: `2 passed`
- OKX trigger tests passed: `5 passed`

Live exchange validation:

- Not recorded

# Preventive Safeguards

- never use fixed timestamps in auth/signing helpers outside explicitly test-only seams
- when a plugin signs time-sensitive requests, add a regression test that validates timestamp freshness, not just header presence or payload shape
- during review of exchange integrations, explicitly inspect clock-derived auth fields alongside signature algorithms and secret-slot wiring
