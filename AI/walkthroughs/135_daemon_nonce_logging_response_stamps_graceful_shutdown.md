# Walkthrough 135 — Daemon Nonce Logging, Response Timestamps and Graceful Shutdown

- **Date**: 2026-09-30
- **Issues**: GitHub #257 (DMN-11), #258 (DMN-18), #259 (DMN-19)
- **Branch**: `fix/p3-daemon-logging-shutdown`
- **Matrix criteria**: DLX1–DLX5 (new component `daemon-logging-shutdown`)
- **ADR**: 2026-09-30 "Daemon Logging Anonymization, Response Stamping and Bounded Shutdown Drain"

---

## 1. Findings

| Issue | Finding | Location (before) |
|---|---|---|
| #257 | The no-pipeline fallback logged `request_id = ?req.request_id` (all 32 nonce bytes), contrary to logging policy D5; the audit test scanned single lines for a keyword list without `request_id` | `dispatcher.rs` (fallback `warn!`), `tests/logging_audit_test.rs` |
| #258 | `build_response` trusted a caller-supplied `now_ns`; several paths passed `0`, producing `issued = 0, expires = 2 s after boot` | `dispatcher.rs` (`build_response` and its callers) |
| #259 | Handlers were detached `tokio::spawn` tasks: SIGINT/SIGTERM returned from `main` and dropped them without a log; a handler panic reached only the default stderr hook | `main.rs` accept loop |

## 2. Specification

- `soos_daemon::logging::short_request_id(&RequestId) -> ShortRequestId`: the first 4 bytes of
  `SHA-256("soos.request-id.log.v1" || request_id)`, rendered as 8 lowercase hex digits by both
  `Display` and `Debug`. `SHORT_REQUEST_ID_HEX_LEN = 8`. A digest rather than a prefix so that no
  nonce bit is revealed. `sha2` (already a workspace dependency) is added to `soos-daemon`.
- `soos_daemon::dispatcher::RESPONSE_VALIDITY_NS = 2_000_000_000`. `build_response(request_id,
  verdict, reason_class)` samples the dispatcher clock: `Ok(now > 0)` gives `issued = now`,
  `expires = now + RESPONSE_VALIDITY_NS`; a failed or zero clock gives `issued = expires = 0` and
  downgrades `Allow` to `Unavailable` / `InternalError`.
- New module `soos_daemon::shutdown`:
  - `ConnectionTasks` (wraps `JoinSet<()>`): `new`, `spawn`, `len`, `is_empty`,
    `drain(budget) -> DrainReport { completed, panicked, aborted }` (`is_clean()`).
  - `accept_until_shutdown(&listener, dispatcher, &mut tasks, shutdown)`: accepts into the set,
    reaps finished handlers, returns when `shutdown` resolves.
  - `install_panic_hook()`: `tracing::error!` with file, line and thread only.
- `main.rs`: installs the hook after logging init; after the signal it drops the listener, clears
  `socket_ready`, drops the `SocketGuard` (unlink) and drains for `connection_timeout`.

## 3. Tests First (Red Evidence)

New test files (no pre-existing test changed):
`crates/daemon/tests/request_id_logging_tests.rs`, `response_timestamp_tests.rs`,
`graceful_shutdown_tests.rs`, `panic_hook_tests.rs`.

1. Against `origin/main`, all four files failed to compile (`short_request_id`,
   `RESPONSE_VALIDITY_NS` and `soos_daemon::shutdown` did not exist; `sha2` not a dependency).
2. With only the helper, the constant and the new module in place (dispatcher unchanged):
   - `test_daemon_logs_never_carry_the_raw_request_nonce` FAILED on
     `dispatcher.rs: raw nonce reference in request_id = ?req.request_id, "Rejecting authentication request: ..."`;
   - `test_wire_validation_rejection_is_stamped`, `test_uid_mismatch_rejection_is_stamped`,
     `test_preview_authorization_refusal_is_stamped` FAILED (`left: 0, right: 7000000000`);
   - `test_clock_failure_yields_expired_non_allow_response` FAILED (`left: 2000000000, right: 0`).
3. After the dispatcher and `main.rs` changes, all new tests pass, and so does every pre-existing
   daemon test, including `policy_concurrency_tests::test_monotonic_clock_failure_returns_unavailable`.

## 4. Audit Notes

- No `unwrap`/`expect` in production code; counters use `saturating_add`.
- The `JoinError` of a panicked handler is never formatted (its `Display` carries the payload).
- The drain is bounded twice: `connection_timeout` for normal completion, then 100 ms
  (`ABORT_REAP_BUDGET`) for aborted handlers to acknowledge cancellation; anything left is aborted
  when the set is dropped. The daemon therefore never hangs at shutdown.
- `JoinSet` reaping during the accept loop keeps memory bounded by the number of live connections.
- No new `unsafe`; `main.rs` keeps `#![forbid(unsafe_code)]`. No PAM change: the PAM client still
  treats the timestamps as informational (ADR "Response Timestamps Are Informational").

## 5. Remaining Work

- Client-side expiry enforcement in `pam_soos.so` stays an open option, gated on the fixture
  migration described in the 2026-09-30 "Response Timestamps Are Informational" ADR.
- The camera health monitor task is not part of the drain (it holds no client connection).
