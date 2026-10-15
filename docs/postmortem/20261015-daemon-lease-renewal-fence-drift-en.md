---
title: "Daemon lease renewal could diverge from the execution fence"
type: postmortem
status: closed
date: "2026-10-15"
tags: [runtime, lease, fencing]
---

# Daemon lease renewal could diverge from the execution fence

## Symptom

Architecture review candidate 2 identified separate lease ownership in
`ServeLeaseSupervisor` and `ServeLeaseHeartbeat`. Source inspection also found
that heartbeat renewal accepted both `Acquired` and `Renewed` grants while the
supervisor retained the initial grant used to generate execution fences. No
production incident or data corruption was recorded.

## Root Cause

`RuntimeStateStore::try_acquire_serve_lease` creates a new generation when an
expired lease is acquired, including acquisition by the same owner. Treating
that result as renewal let the heartbeat continue under the new generation
while run persistence used the old generation. Fenced writes could then fail
despite the heartbeat reporting success.

A boolean selected which component renewed the lease, but did not give those
components one shared ownership state. Progress callbacks additionally mapped
lease errors to `InvalidTriggerEmission`, obscuring runtime ownership failures.

## Fix Applied

A private `DaemonLeaseSession` now owns a persistent control store and shared
renewal state. Foreground checks and background heartbeat work use the same
renewal implementation. An expired session cancels work instead of reacquiring,
and renewal must retain the execution fence generation. Transient storage
failures retry within the existing lease lifetime; expiry retains the last
storage error in its diagnostic. Shutdown cancels work, joins heartbeat work,
and releases the session lease. Progress callbacks preserve runtime errors.

## Validation

The regression `expired_session_cannot_reacquire_a_new_generation` uses explicit
timestamps and a real SQLite store to verify expiry cancels the session without
changing the persisted generation. Other session tests cover same-generation
renewal, acquisition rejection, successor ownership, storage failure/recovery,
stop requests, automatic release, and background cancellation.

`trigger_progress_preserves_lease_loss_as_a_runtime_conflict` exercises a serve
turn and verifies callback failure returns `serve_lease_lost` without creating
a run summary.

`cargo test --workspace` passed: 334 tests, 0 failures. Rust Analyzer reported
no diagnostics for the three changed Rust files, and `git diff --check` passed.
`CHAINBOT_TEST_POSTGRES_URL` was unset, so database validation used SQLite only.
The postmortem retrieval link was not added to an `AGENTS.md` index because the
project-local fractal scope config and checker are absent.

## Preventive Safeguards

Keep the heartbeat and serve-turn tests on the same lease-session boundary. A
new lease generation requires a new session and execution fence. Keep lease
failures classified as runtime conflicts at trigger progress boundaries.
