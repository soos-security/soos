# Architect Spec — GitHub #410: pin the refusal of `LockState` on the auth-reserved connection

- **Date**: 2026-10-17
- **Issue**: GitHub #410 (no backlog id), part of epic #352 (lock-screen face-scan widget)
- **Origin**: PR #403 follow-up "Daemon: do not admit LockState onto the auth-reserved permit"
  (`AI/walkthroughs/214_lockscreen_adapters.md` §9, candid review suggestion)
- **Branch**: `test/daemon-lockstate-reserve` (GitHub-only issue: not registered in
  `scripts/sync_issue.py`; the commit carries `Closes #410` and `Refs #352`)
- **Base**: `origin/main` 9e82795

## 1. Scope & Blast Radius

Test and documentation change only. No production code, no protocol wire change, no PAM change.

| Area | File | Change |
|---|---|---|
| Daemon tests | `crates/daemon/tests/peer_limits_tests.rs`, `mod auth_reserve` | New helpers and new `#[tokio::test]` functions only; no existing line of the module is edited |
| Matrix | `AI/VERIFICATION_MATRIX.md` | New component section `lockstate-reserve-refusal`, rows `LRR1`–`LRR3` (prefix unused on `origin/main`) |
| ADR register | `AI/DECISIONS.md` | One dated clarification entry (no new decision): where the refusal happens and why accept time is not feasible |
| User/protocol docs | `Docs/IPC_PROTOCOL.md` §14, `Docs/LOCKSCREEN_WIDGET.md` §7 | Wording: the refusal happens at first-frame classification, before any `LockState` processing |
| Walkthrough | `AI/walkthroughs/217_lockstate_reserve_refusal.md` | New; closes the #403 follow-up |

Consumers: none (no public item added or changed).

## 2. Current Behaviour (read from the code, `origin/main` 9e82795)

1. `ConnectionDispatcher::handle_connection` (`crates/daemon/src/dispatcher.rs`) reads
   `SO_PEERCRED`, then calls `PeerConnectionLimiter::try_acquire(peer_uid)`; only after a
   `PerUidCap` refusal does it try `try_acquire_auth_reserve(peer_uid)`. The admission class is
   therefore fixed **before any byte is read** and depends on the UID only.
2. `read_and_process`, for `AdmissionClass::AuthReserve`, reads the whole first frame under
   `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS` (200 ms), classifies it with
   `decode_client_message`, and returns `Err(DaemonError::Protocol(..))` for any message that is
   not `ClientMessage::Request(req)` with `req.kind == RequestKind::Auth`. This check runs
   before `handle_request`, so a `LockState` request on the reserve never reaches
   `handle_lock_state_request`: no clock read, no `lock_state_limiter.check_and_record`, no
   sessions scan, no board read, and no response bytes. The connection is dropped and the
   permit (RAII) released.
3. `LockStateBoard` is only written by `Auth` scans and the presence worker
   (`begin_scan`/`finish`); `LockState` only reads it.

## 3. Why Not "At Accept Time"

The #403 note asks to refuse `LockState` at accept time. At accept time only the kernel peer
credentials are known. The lock-status bridge, the widget and `pam_soos.so` inside the locker
run as the same UID, so the UID cannot separate them. Telling them apart before admission would
need the daemon to read (peek) the first frame of a connection that holds no permit yet, which
creates a new class of unadmitted pending connections with its own bound and timer. That
changes the ADR 2026-10-12 S-3 design (admission by credentials, then a bounded first-frame
window under a held permit) and needs the owner's approval; it is out of scope. The refusal at
first-frame classification already gives the security property that matters: the reserve is
held at most for the 200 ms window plus one frame decode, `LockState` never consumes the
`LockState` quota or reads the board through the reserve, and the reserve is free again for
the next `Auth`.

## 4. Types & Signatures

No production type changes. New test-only items inside `mod auth_reserve`:

```rust
/// A tagged `LockState` request for `uid` (the bridge's request shape).
fn lock_state_request(uid: u32, nonce: u8) -> Request;

/// Sends one `LockState` on `client` and returns `true` when the answer is a
/// `LockStateResponse` (the frame does not decode as a standard `Response`).
async fn lock_state_answered(client: &mut UnixStream, nonce: u8) -> bool;

/// Opens a connection on the reserve (the two regular permits are held), sends a
/// `LockState` for the caller's own UID and asserts it is closed without any payload,
/// then waits until all `expected` permits are free again.
async fn refuse_lock_state_on_reserve(
    sock_path: &Path, dispatcher: &ConnectionDispatcher, nonce: u8, expected: usize,
);
```

Bounds: every read uses `closed_promptly` (500 ms `CLOSE_DEADLINE`) or `exchange` on a live
connection; `permits_back` is bounded at 2 s. The request loop is bounded by the constant
`RESERVE_LOCK_STATE_ATTEMPTS` = 16 (twice `LOCK_STATE_MAX_REQUESTS_PER_SEC`).

## 5. Constants & Config

Used, never redefined: `LOCK_STATE_MAX_REQUESTS_PER_SEC` (8, `soos_daemon::lock_state`),
`AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS` (200), `CLOSE_DEADLINE` (500 ms, test file),
`DEFAULT_MAX_REQUESTS_PER_CONNECTION` (1024: the regular connection `c1` serves 1 Status +
8 `LockState` well inside it). New test constant: `RESERVE_LOCK_STATE_ATTEMPTS = 2 *
LOCK_STATE_MAX_REQUESTS_PER_SEC as usize`.

## 6. Error Taxonomy

Unchanged. A non-`Auth` message on the reserve maps to `DaemonError::Protocol` inside the
daemon, no answer on the wire; the bridge maps EOF to "unavailable" (renders like Idle,
ADR 2026-10-15 item 4) and backs off. PAM unaffected.

## 7. Latency Budget

Auth path not touched. The reserve's occupancy by a refused `LockState` is bounded by
`AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS` (frame arrival) plus one decode.

## 8. Contract (tests to add, all skipped as root like the siblings)

| Row | Test | Asserts |
|---|---|---|
| LRR1 | `test_lrr_reserve_closes_lock_state_without_payload` | With the two regular connections held (`two_held`), a `LockState` for the caller's own UID on the reserve is closed without any payload (`closed_promptly`), and the permits return to 6 (`permits_back`) |
| LRR2 | `test_lrr_reserve_lock_state_consumes_no_quota_and_leaves_the_board` | A board scan is published first (sequence 2 for the UID). After a reserve `LockState` refusal: `LOCK_STATE_MAX_REQUESTS_PER_SEC` (8) `LockState` requests on the held regular connection `c1` are all answered with a `LockStateResponse` (none `RateLimited`) within one window; the board view of the UID is unchanged (state and sequence) and `tracked_uids` is unchanged; an `Auth` is then admitted on the reserve and answered |
| LRR3 | `test_lrr_repeated_reserve_lock_state_cannot_hold_the_pam_reserve` | 16 sequential reserve `LockState` attempts are each closed without payload with the permit released; then 8 regular `LockState` requests are all answered (16 refusals consumed no quota) and an `Auth` on the reserve is still answered (`auth_response`) |

RED-phase evidence: the behaviour exists on `main`, so the tests pass on unmodified code. Their
discriminating power is shown by a temporary local mutation of the reserve guard in
`read_and_process` (admit `LockState` on the reserve): every new test must then fail on its
"no payload" assertion. The mutation is reverted and never committed.

## 9. Invariants Touched

- ADR 2026-10-12 S-3 (auth-reserved connection, rows RSH4, RSH5): pinned, unchanged.
- ADR 2026-10-13 item (5) (`LockState` quota, side-effect freedom, "the auth-reserved
  connection still refuses it"; rows LKS9): pinned for the reserve.
- ADR 2026-10-15 item (3) accepted residual: unchanged, now with test evidence.
- No sensitive data, no `unsafe`, no PAM change, no `PAM_SUCCESS` path.

## 10. Test Hooks

All existing: `ConnectionDispatcher::lock_state_board()` (public), `LockStateBoard::begin_scan`
/ `ScanGuard::finish` / `view` / `tracked_uids` (public), `current_monotonic_nanos`
(public), `available_permits()` (public), `soos_protocol::message::encode_request` (public).

## 11. Documentation Drift / ADR

- `AI/walkthroughs/214_lockscreen_adapters.md` §9 says "refuse ... at accept time"; walkthroughs
  are history and are not edited; walkthrough 217 closes that follow-up and explains §3.
- `Docs/IPC_PROTOCOL.md` §14 and `Docs/LOCKSCREEN_WIDGET.md` §7 say the reserve refuses
  `LockState` without saying where; clarify "at first-frame classification, before any
  `LockState` processing (no quota, no board read)".
- `AI/DECISIONS.md`: one dated clarification entry, no new decision.
