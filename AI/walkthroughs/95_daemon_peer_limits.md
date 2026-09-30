# Walkthrough 95 — Daemon Per-Peer Connection Limits and Event Quota

- **Date**: 2026-09-30
- **Issues**: Review finding DMN-04 (GitHub #157); PAM-04 / DMN-03 (GitHub #175)
  — **Branch**: `fix/daemon-peer-limits`
- **Matrix criteria**: DPL1–DPL6 (new)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, area Privileged Daemon)
confirmed that `ConnectionDispatcher::handle_connection` guarded the socket with one global
`Semaphore(max_concurrent_connections = 8)` acquired before the peer was even identified, and that
a persistent connection had neither a request cap nor a lifetime cap. A single `soos`-group account
opening 8 connections and sending `Status` every 900 ms held every permit forever: every gdm, sudo
or login face attempt got EOF and fell back to the password, for every user, with no log naming
the offender.

GitHub #175 reported that `PasswordFailed` events trust the payload `uid` without an
`SO_PEERCRED` cross-check and are not rate limited, so any `soos`-group process can trigger
evidence snapshots at will (when evidence capture is enabled) and attribute them to any UID.

Objectives: per-UID admission keyed on the kernel UID, permits reserved for root (PAM) peers,
bounded connection lifetime and request count, one-shot `Auth` connections, a per-peer event quota,
refusals logged with `peer_uid`, and no unbounded wait for a refused peer.

## 2. Architect Design

- New module `crates/daemon/src/limits.rs` (pure, no I/O, no clock):
  - `PeerLimitsConfig { max_connections_per_uid, reserved_root_connections,
    max_requests_per_connection, max_connection_lifetime, max_events_per_window, event_window }`
    with defaults 2 / 2 / 1024 / 30 s / 5 / 10 s (`DEFAULT_*` constants), `validate(capacity)` and
    `event_rate_limit_config()` (256 tracked UIDs).
  - `PeerConnectionLimiter::new(capacity, &limits)` / `try_acquire(peer_uid)` returning an RAII
    `PeerConnectionPermit` or a `ConnectionRejection` (`GlobalCapacity`, `ReservedForPrivileged`,
    `PerUidCap`). Root is exempt from the per-UID cap; unprivileged peers together use at most
    `capacity - reserved`. The reservation is clamped to `capacity - 1` in the library.
- `DaemonConfig.peer_limits` parsed from a new `[peer_limits]` TOML section and validated against
  `[dispatcher] max_concurrent_connections` at load time. `DispatcherConfig` is unchanged (its
  struct literals in existing tests stay valid).
- `ConnectionDispatcher::with_peer_limits(limits)` / `peer_limits()`; `main.rs` wires the
  configured limits. `available_permits()` now reports the limiter's free permits.
- `handle_connection`: reads `SO_PEERCRED` once, acquires the per-peer permit (refusal => warn with
  `peer_uid` and reason, stream dropped), then loops with the request cap and lifetime checks
  before each read; `ProcessedOutput.one_shot` closes the connection after an `Auth` response.
- `handle_event`: `event_within_quota(peer_uid)` (per-peer `RateLimiter`, root included) gates the
  snapshot; the clock failing or the table being full drops the event (fail closed).

## 3. Trust Model for PasswordFailed (GitHub #175)

The only legitimate emitter is `pam_soos.so event=password-failed`, running inside the process
that calls `pam_authenticate`. `SO_PEERCRED` reports that process's effective UID: 0 for sudo,
su, login, gdm-session-worker and polkit-agent-helper-1; the user's own UID for screen lockers
authenticating their own user. The rule is therefore "`peer_uid == 0`, or `event.uid` is
`None` / `Some(peer_uid)`", recorded as an ADR in `AI/DECISIONS.md` and enforced in
`handle_event` before the per-peer event quota; a violating event is dropped with a `warn` naming
`peer_uid` and the claimed UID.

Contract migration: the setup of
`pipeline_integration_tests::test_12_4_password_failed_event_captures_evidence_snapshot` had an
unprivileged test peer report for `current_uid + 42`, i.e. it encoded the vulnerable behaviour.
The cross-check was first held back (test-integrity invariant); with explicit user approval
(2026-09-30) the one-line setup changed to `target_uid = fixture.current_uid`, assertions
unchanged. The new negative test `test_unprivileged_peer_cannot_report_event_for_foreign_uid`
covers the refused path (foreign UID and UID 0 claimed by an unprivileged peer).

## 4. Tester Contract (TDD Red)

`crates/daemon/tests/peer_limits_tests.rs` (16 tests). First run: compile failure on the missing
API (`soos_daemon::limits`, `with_peer_limits`, `DaemonConfig.peer_limits`). With the API skeleton
in place and the dispatcher not yet enforcing it, the six behavioural tests failed on assertions:
`test_same_uid_connection_over_cap_is_closed_without_blocking_others` (a third same-UID connection
received a payload), `test_idle_polling_peer_cannot_exhaust_global_capacity`,
`test_auth_request_is_one_shot_per_connection`, `test_connection_closed_after_max_requests`,
`test_connection_closed_after_max_lifetime`,
`test_password_failed_events_are_rate_limited_per_peer_uid` (4 snapshots instead of 2).

Cross-check (GitHub #175): without the `handle_event` check,
`test_unprivileged_peer_cannot_report_event_for_foreign_uid` failed ("Foreign-UID events from an
unprivileged peer must not create snapshots"). With the check but before the approved setup
migration, `pipeline_integration_tests::test_12_4_password_failed_event_captures_evidence_snapshot`
failed ("Evidence store must contain 1 snapshot after PasswordFailed event"), confirming it encoded
the vulnerable behaviour. Both pass after the migration.

Root reservation cannot be exercised through sockets by an unprivileged test process, so it is
proven on the pure limiter (`test_limiter_reserves_permits_for_root_peers`: 8 unprivileged
attempts leave the 9th root connection served) instead of adding a production hook that overrides
`SO_PEERCRED`.

## 5. Audit Constraints

1. No `unwrap`/`expect`; the limiter mutex recovers from poisoning (`PoisonError::into_inner`) and
   every counter update is saturating.
2. The limiter table holds only UIDs with a permit in use (removed at zero): bounded by capacity.
   The event limiter tracks at most 256 UIDs; a full table drops events.
3. Peer credentials are read before any payload byte; a failure closes the connection.
4. A refused peer is never queued: its stream is closed at once (PAM: EOF => `PAM_IGNORE`, covered
   by `ipc_tests::test_ipc_daemon_crash_immediate_disconnect_returns_ignore`).
5. Logs carry only `peer_uid`, the rejection reason and request counts: no payload, frame,
   embedding or credential.
6. Socket mode and ownership (`0660 root:soos`) untouched.
7. Invalid `[peer_limits]` values fail startup instead of silently disabling a bound.

## 6. Developer Notes

- Worst-case connection lifetime is `max_connection_lifetime` plus one read/process/write cycle,
  because the check runs before each read.
- `soos-gui` preview polls at ~30 requests/s on one connection; the lifetime cap closes it about
  every 30 s and the GUI reconnects after its 200 ms `RECONNECT_DELAY`.
- A configuration with `max_concurrent_connections <= 2` must now also lower
  `reserved_root_connections`.

## 7. Verification

- `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features --
  -D warnings`, `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`, and
  `./scripts/candid_review.sh` were run; see the branch report for results.
- Docs: `Docs/IPC_PROTOCOL.md` §11, `AI/DECISIONS.md` (two entries), `AI/VERIFICATION_MATRIX.md`
  component `daemon-peer-limits` (DPL1–DPL6).

## 8. Follow-ups

- **Evidence-store global cap (candid review suggestion 5, #175 / DMN-03)**: the review also
  recommended a global daily cap on intrusion snapshots and pruning of the `daily_counts` map in
  `crates/evidence-store`. Both are outside this diff. The new trust rule (only root peers may
  report for another UID) and the per-peer event quota remove the unprivileged exploitation path,
  so they are tracked as a follow-up issue rather than implemented here.
