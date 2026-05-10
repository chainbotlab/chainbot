---
title: "Hyperliquid node origin binding was declared but not enforced"
type: postmortem
status: closed
date: "2026-05-11"
tags: [bugfix, postmortem, hyperliquid, official-plugin, activation]
---

# Postmortem: Hyperliquid node origin binding was declared but not enforced

## Summary
`hyperliquid-node` declared origin-bound activation in its manifest, but runtime request handling did not actually enforce the allowlist when callers overrode `base_url`.

## Trigger
This issue was found during REVIEW while comparing the new Hyperliquid node and trigger packages.

## Impact
- A caller could set `input.base_url` to an arbitrary non-blocked public HTTP(S) endpoint.
- The manifest advertised origin-restricted activation, but node runtime behavior did not honor that contract.
- Impact was limited to the new, not-yet-shipped Hyperliquid node package.

## Expected Behavior
When a request overrides `base_url`, the node should only allow destinations whose normalized origin matches an activation allowlist entry.

## Actual Behavior
`RequestContext` stored `allowed_origins`, but `post_info()` and destination validation only checked scheme, blocked hosts, and IP policy. The allowlist was never consulted.

## Root Cause
Direct cause:
- The node implementation copied destination safety checks from the HTTP path but stopped short of adding the trigger-style origin allowlist enforcement.

Contributing factors:
- Review and test coverage initially focused on blocked loopback and successful `/info` reads, not manifest-to-runtime security parity.
- The manifest contract was updated earlier to satisfy host validation, which made the missing runtime enforcement easier to overlook.

Missing guardrails:
- No node test asserted that a mismatched `allowed_origins` value rejects a `base_url` override.

## Fix Applied
- Added runtime allowlist enforcement to `official-plugins/hyperliquid-node/crate/src/provider.rs`.
- Origin binding now runs during `RequestContext::new(...)` and the final `/info` URL validation path.
- Added normalized origin comparison for `http` and `https` destinations.
- Added a regression test that verifies non-allowlisted `base_url` overrides fail.

## Verification
- `rtk cargo test` in `official-plugins/hyperliquid-node/crate`
- The updated test suite now covers:
  - successful allowlisted loopback reads
  - blocked loopback without test guards
  - rejected non-allowlisted `base_url` override

## Prevention / Follow-ups
- For external plugins that declare `requires_allowed_origins = true`, verify runtime enforcement during REVIEW, not just manifest validation.
- Add targeted regression tests whenever a manifest-level security contract is introduced.
- Keep node and trigger destination-binding logic aligned when both expose overrideable endpoints.

## Changed Files
- official-plugins/hyperliquid-node/crate/src/provider.rs
- official-plugins/hyperliquid-node/crate/tests/read_rpc.rs
- docs/postmortem/20260511-hyperliquid-node-origin-binding-not-enforced-en.md

## Notes
The bug was caught before ship, so no production remediation or data repair was required.
