# Walkthrough 121 — PAM Deadline Before Connect, UID Resolution Budget, Response Freshness

- **Date**: 2026-09-30
- **Issues**: GitHub #219 (PAM-06), #222 (PAM-10), #223 (PAM-11)
- **Branch**: `fix/p2-pam-deadline-replay-uid`
- **Matrix criteria**: PDR1–PDR4 (new, verified), PDR5 (new, pending)
- **ADRs**: 2026-09-30 "Response Timestamps Are Informational"; 2026-09-30 "PAM Budget Starts
  Before UID Resolution; Daemon Deadline Fixed Before Connect"
- **Scope**: `crates/pam/src/ipc.rs`, `crates/pam/src/lib.rs`, doc comments of
  `crates/protocol/src/types.rs`, `Docs/IPC_PROTOCOL.md`, `Docs/PAM_MODULE.md`, new tests. No
  wire-format change, no pre-existing test changed.

---

## 1. Findings

| Issue | Defect |
|---|---|
| #222 | `authenticate` computed `deadline_monotonic_ns` after `connect()` and `getrandom`, so the daemon's deadline was later than the client's by the connect latency. `connect_with_timeout` truncated `remaining.as_millis()`, so a 0.9 ms remainder became `poll(.., 0)` and a spurious `IpcError::Timeout`. |
| #223 | `pam_get_user` + `getpwnam_r` ran before any deadline existed. NSS (LDAP, SSSD, NIS) can block for seconds; neither `timeout_ms` nor the 20 ms event budget covered it. |
| #219 | `Docs/IPC_PROTOCOL.md` and `types.rs` described `expires_monotonic_ns` as replay protection, but the client never checks it. |

## 2. Design

- `pam_soos::ipc::ExchangeDeadline` (public, `Copy`): `start(timeout_ms)` reads `Instant::now()`
  and CLOCK_MONOTONIC once; `remaining()` returns the budget left or `IpcError::Timeout`;
  `monotonic_deadline_ns()` is the fixed value the request carries.
- `ipc::authenticate_before(config, uid, deadline)` runs connect, write and read under that
  deadline and sends exactly `deadline.monotonic_deadline_ns()`. `ipc::authenticate` is a thin
  wrapper that starts the deadline itself. `notify_event` uses the same type (20 ms).
- `ipc::poll_timeout_ms(Duration) -> c_int`: ceiling division to milliseconds, `ZERO -> 0`,
  saturating at `c_int::MAX`.
- `SoosPam::authenticate_with_uid_resolver(pamh, config, resolver)`: `authenticate_with_config`
  now delegates to it with the production resolver (`pam_get_user` + `getpwnam_r`, `getuid`
  fallback, behaviour unchanged). The authentication deadline starts before the resolver runs.
  If the resolver spent the whole budget the module logs the duration at `LOG_INFO` and returns
  `PAM_IGNORE` without connecting. On the password-failed path the event is still sent (bounded
  by `EVENT_TIMEOUT_MS` after the lookup) and a lookup above 20 ms is logged. `uid=` skips the
  resolver. Everything stays inside the existing `catch_unwind` region.
- #219 takes the documentation route offered by the review: replay protection is the single-use
  `request_id` plus the client deadline; the timestamps are informational. Client-side
  enforcement would break pre-existing fixtures (see §5).

Why NSS is not moved to a helper thread: the lookup cannot be cancelled, and a thread left
blocked inside NSS would outlive the PAM call and possibly the `dlclose` of `pam_soos.so`. The
module therefore bounds what it controls and reports the rest.

## 3. Red evidence

Before the implementation:

```text
cargo test --locked -p soos-pam --all-features --test deadline_uid_tests
error[E0432]: unresolved imports `pam_soos::ipc::authenticate_before`, `pam_soos::ipc::poll_timeout_ms`, `pam_soos::ipc::ExchangeDeadline`
error[E0599]: no associated function or constant named `authenticate_with_uid_resolver` found for struct `SoosPam`

cargo test --locked -p soos-invariants pam_response_freshness
test pam_response_freshness_contract::test_ipc_protocol_documents_the_real_replay_protection ... FAILED
test pam_response_freshness_contract::test_response_timestamps_are_not_documented_as_replay_protection ... FAILED
```

`deadline_uid_tests::test_authenticate_request_deadline_never_exceeds_the_client_budget` and
`deadline_uid_tests::test_replayed_allow_response_is_rejected_by_request_id_binding` are
regression guards and characterization tests; they pass on both versions.

## 4. Green evidence

`cargo test --locked -p soos-pam -p soos-invariants --all-targets --all-features`: all suites
pass, including the 9 tests of `crates/pam/tests/deadline_uid_tests.rs` and the 2 tests of
`tests/invariants/src/pam_response_freshness_contract.rs`. The full gate is listed in the
branch report.

## 5. Open decision: enforcing the response timestamps

Enforcement (`now < expires`, `issued <= now + skew`, `expires == 0` invalid) is not implemented.
It would reject the fixtures of these pre-existing tests, which the test-integrity rule forbids
changing without approval:

- `crates/pam/tests/ipc_tests.rs` (several `Allow` fixtures with `issued 1000 / expires 2000`),
- `crates/pam/tests/pam_handle_tests.rs` (same values),
- `tests/docker/mock_daemon.py` (`0 / 0`, used by the Docker matrix `Allow` cases).

If the user approves migrating those fixtures to real CLOCK_MONOTONIC values, enforcement becomes
a small change in `ipc::authenticate_before` (matrix row PDR5).

## 6. Follow-ups

- Real LDAP / SSSD latency with an unreachable directory was not measured (no directory in the
  hermetic suite); the resolver injection simulates it.
- Sending the username instead of the UID on the event path would remove the lookup from that
  path but changes the v1 `Event` schema; not adopted.
