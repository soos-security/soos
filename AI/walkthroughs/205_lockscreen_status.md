# Walkthrough 205 — Read-Only Lock-Screen State Channel (`RequestKind::LockState`)

- **Date**: 2026-10-13
- **Issue**: GitHub #380, PR 1 of the epic #352 (lock-screen face-scan widget). GitHub-only: no `AI/BACKLOG.md`
  entry, not registered in `scripts/sync_issue.py`; the commit carries `Closes #380` and `Refs #352`.
  **Branch**: `feat/lockscreen-status`.
- **ADR**: "[2026-10-13] Read-Only Lock-Screen State Channel" (`AI/DECISIONS.md`).
- **Documents**: spec `AI/architect_spec_lockscreen_status.md` (§9 Revision 1 and §10 Revision 2 supersede the conflicting earlier
  text); plan evaluation `AI/plan_evaluator_report.md` (round 2 APPROVED); auditor constraints
  `AI/auditor_constraints_lockscreen_status.md` (1 to 23, CLEARED); `Docs/IPC_PROTOCOL.md` §14, `Docs/DAEMON.md`
  §3 and §7.
- **Matrix criteria**: LKS1–LKS20 (LKS17 and LKS18 pending).

## 1. Context & Objectives

The owner wants an animated soos widget on every lock screen (#352). Wayland `ext-session-lock-v1` gives the
locker exclusive ownership of the locked outputs, so the design is one daemon-side status source plus thin
per-locker adapters. This first PR adds only the status source: a read-only IPC request and a daemon snapshot of
the current scan state. Owner decisions of 2026-10-13: the locked screen may show Scanning / Idle / Not recognized,
"Not recognized" collapses every failure reason; animate for PAM face requests and presence scans alike;
swaylock-plugin + hyprlock adapters first. The `soos-lockstatus` bridge, renderers, adapters and installer are later
PRs.

## 2. Architect Design

- **Protocol** (`crates/protocol/src/types.rs`): `RequestKind::LockState = 3`; `LockWidgetState { Idle = 0,
  Scanning = 1, NotRecognized = 2 }` serialized as one raw byte (`#[serde(try_from = "u8", into = "u8")]`,
  `UnknownLockWidgetState` for every other byte); `LockStateResponse { version, state, sequence }`;
  `LOCK_STATE_RESPONSE_MAX_PAYLOAD` = 12. A refusal stays a standard `Response`; the two never decode as each other.
- **Daemon snapshot** (`crates/daemon/src/lock_state.rs`, `#![forbid(unsafe_code)]`): `LockStateBoard`
  (`tokio::sync::watch`, at most `MAX_LOCK_STATE_UIDS` = 32 UIDs), `ScanGuard` (`finish` or drop), `ScanEnd`,
  `LockStateView`. Uniform reveal rule: every scan end becomes visible only at
  `max(end, start + NOT_RECOGNIZED_REVEAL_FLOOR_MS)`, the floor being 1200 + 1000 + 900 + 50 + 90 = 3240 ms derived
  from `MAX_CAMERA_WAKE_WAIT_MS`, `PRESENCE_WAKE_SETTLE_MS`, `DECISION_BUDGET_MS`, `RESPONSE_WRITE_MARGIN_MS` and
  `REVEAL_FLOOR_MARGIN_MS` (`DEFAULT_INFERENCE_ESTIMATE_MS + FRAME_POLL_INTERVAL_MS`, added after the candid review);
  `sequence` is `2 × scans − 1` while hidden and `2 × scans` once visible; `NOT_RECOGNIZED_HOLD_MS` 3000;
  `MAX_SCANNING_MS` 10 000.
- **Publication**: the `Auth` path publishes after the attempt reservation and the template lookup, right before the
  camera wake (`Allow` ⇒ recognized, read as `Idle`; every other rendered verdict ⇒ `NotRecognized`); the presence
  worker publishes after recording its attempt (`Unlocked` ⇒ recognized; `Preempted`/`ShuttingDown` ⇒ abandoned;
  every other `ScanOutcome`, listed exhaustively, ⇒ `NotRecognized`).
- **Read path** (`ConnectionDispatcher::handle_lock_state_request`, Step 6d, after wire validation and the
  `SO_PEERCRED` check): clock (error ⇒ `{ Idle, 0 }`), own quota (8/s per peer UID, 64 UIDs) ⇒
  `ProtocolError`/`RateLimited`, local seat session of `uid_hint` (none ⇒ `{ Idle, 0 }`), then the view.
- **Wiring** (`crates/daemon/src/main.rs`): one `LockStateBoard::new()` handed to the dispatcher and the presence
  worker.
- Invariants touched: UDS only, no permission change, PAM untouched, no biometric data or reason class on the wire
  or in logs, PAM-03 / #174 indistinguishability, peer limits #157, auth-reserved connection, rate limiter #200/#323,
  presence timing #323.

## 3. Plan Evaluation

Round 1: `REVISION_REQUIRED` with one MAJOR finding — postcard accepts overlong varints, so a varint-encoded state
`0x81 0x00` would alias `Scanning` — and seven MINOR ones (limiter API and clock order, a supersession leak of
hidden ends, the floor scope, journal flooding, a missing static check of the read path, missing tests, the client
connection budget). Revision 1 (spec §9, R1–R8) made the state a raw byte, read the clock before the quota, adopted
one uniform reveal rule for every end, narrowed the floor claim, logged refusals at `debug` only, and added the
static check and the tests. Round 2: `APPROVED`, with R2-1 (sequence restarts after an eviction: documented as an
epoch rule in `Docs/IPC_PROTOCOL.md` §14) and R2-2 (the log check now scans whole `debug!` calls and forbids
`trace!`/`event!`) folded in.

## 4. Tester Contract

64 new tests, all written before the production code (61 in the first pass, 3 after the candid review):

| File | Tests | Matrix |
|---|---|---|
| `crates/protocol/tests/lock_state_tests.rs` | 9 unit + 4 proptest | LKS1–LKS3 |
| `crates/daemon/tests/lock_state_board_tests.rs` | 17 | LKS4–LKS6 |
| `crates/daemon/tests/lock_state_dispatcher_tests.rs` | 15 | LKS7–LKS11 |
| `crates/daemon/tests/lock_state_presence_tests.rs` | 10 | LKS12–LKS13, LKS19 |
| `tests/invariants/src/lock_state_contract.rs` | 9 | LKS14–LKS16, LKS20 |

Red evidence: `soos-protocol` failed to compile on exactly the specified API (`unresolved imports
soos_protocol::types::LockStateResponse, LockWidgetState, LOCK_STATE_RESPONSE_MAX_PAYLOAD`; `no variant ...
LockState found for enum RequestKind`); the daemon test files failed on the missing `soos_daemon::lock_state`
module; `soos-invariants` ran with 6 of the 8 contract tests failing on their assertions (fields, variants, docs rows,
snapshot module, read path, shared board), the PAM-untouched and fuzz-coverage checks passing as expected.
Migrated existing tests: none (no pre-existing repository test was edited). Contract amendment within this
branch's own new tests: `test_lks_constants_are_derived_and_bounded` was made stricter after the candid review (§7).

## 5. Auditor Constraints

23 constraints (`AI/auditor_constraints_lockscreen_status.md`), `Clearance: CLEARED`. Highlights and how they are
met: PAM untouched (invariant); `forbid(unsafe_code)` and no panic path in `lock_state.rs`, saturating arithmetic
everywhere (clippy `-D warnings`, board tests); no biometric identifier and no logging in the snapshot module, even
in doc comments (invariant); board writes through `send_modify`/`send_if_modified` only, no borrow held across a
write or an `.await`; exhaustive presence outcome mapping without a wildcard; the read path imports its limiter types
at module level and contains no `policy`, `pipeline`, `camera`, `notify_activity` or `register_interactive`
(invariant); one board shared in `main.rs` (invariant). Pre-existing, not introduced here: `SessionValidator` scans
`/run/systemd/sessions` with blocking calls (the `LockState` path now runs it on the blocking pool, §7), and a full
`RateLimiter` table evicts the least recently used UID.

## 6. Implementation

- `crates/protocol/src/types.rs`, `lib.rs`, `message.rs`: new kind, state enum, response, bound, re-exports.
- `crates/protocol/fuzz/`: target `decode_lock_state` (no panic, three states, bound, round-trip, never also a
  `Response`), `Cargo.toml` bin, README; `.github/workflows/fuzz.yml` matrix entry.
- `crates/daemon/src/lock_state.rs` (new), `lib.rs`: the board.
- `crates/daemon/src/dispatcher.rs`: board and quota fields, `with_lock_state_board`, `lock_state_board`, Step 6d
  `handle_lock_state_request`, `encode_lock_state`, the `Auth` publication (8d-0, camera-not-ready, aborted,
  preempted, 8f-1).
- `crates/daemon/src/presence/worker.rs`: `with_lock_state_board`, publication around `scan()`, and (candid
  review) `post_scan_hold` with the `run()` pause anchored to the scan start.
- `crates/daemon/src/main.rs`: one shared board.
- Docs: `Docs/IPC_PROTOCOL.md` (raw-byte note, wire rows, request table, `LockStateResponse`, tag table, §14),
  `Docs/DAEMON.md` (§2, §3 row, §7). AI: ADR, matrix component `lockscreen-status`, this walkthrough, spec, plan
  report, auditor constraints.
- Deviation from the epic draft (recorded in the ADR): three states instead of eight, no `since_monotonic_ns`,
  "disabled"/"no session" collapse into `Idle`. Assumed — owner to confirm: no `Recognized` state, errors after a
  scan started read `NotRecognized`, the 3240 ms floor, the 3 s hold, and the longer presence retry cadence after a failed scan.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) passes. The independent layer-2 review (run by the orchestrator,
`AI/candid_review_report.md`) returned `CHANGES_REQUESTED` with one MAJOR finding and two suggestions;
all three were fixed tester-first (spec §10 Revision 2):

1. **MAJOR — presence cadence oracle.** The worker scheduled its next scan from the end of the previous
   one (1 s pause after each tick, then `scan_interval` from the start), so an early spoof veto started the
   next scan about 1 s after the start and superseded the hidden end: no visible `NotRecognized`, an earlier
   sequence jump. Red evidence: `test_lks_fast_and_slow_presence_failures_schedule_the_next_scan_identically`
   failed with "fast: next scan 1022148713 ns after the previous start, expected about 4150000000", and
   `test_lks_post_scan_hold_is_anchored_to_the_scan_start` failed on its hold bound. Fix:
   `PresenceWorker::post_scan_hold` and `run()` wait until the scan start plus the floor after every scan that
   did not unlock; unlock latency unchanged; the failing-scan cadence becomes about 4.24 s (ADR). The new
   `run()` test passed 8 consecutive runs.
2. **Floor margin.** `REVEAL_FLOOR_MARGIN_MS` (90 ms) added; floor 3240 ms. Contract amendment: the
   constant test `test_lks_constants_are_derived_and_bounded` now also requires the margin and the exact
   3240 ms value (stricter, mandated by the finding); no assertion was relaxed.
3. **Sessions scan off the runtime.** `spawn_blocking` under `LOCK_STATE_SESSION_CHECK_TIMEOUT_MS` (250 ms),
   neutral answer on expiry; invariant `test_lks_session_check_runs_off_the_runtime_with_a_bound`.

A fresh layer-2 review is needed for the new diff fingerprint (not run in this pass by orchestrator
instruction).

## 8. Verification Results

Private build directory (`CARGO_TARGET_DIR` per worktree, `CARGO_INCREMENTAL=0`):

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test   --locked --workspace --all-targets --all-features
./scripts/candid_review.sh
```

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | pass (one `assertions_on_constants` lint in the new board test fixed by a `const { assert!(..) }` block, same assertion) |
| `cargo test --locked --workspace --all-targets --all-features -- --test-threads=4` | pass: 3613 passed, 0 failed, 6 ignored (pre-existing hardware-gated tests), after the candid-review fixes |
| `./scripts/candid_review.sh` (layer 1) | pass |

`cargo deny` was not needed: no dependency changed (the fuzz harness `Cargo.lock` is unchanged).

## 9. Known Limitations / Follow-ups

- LKS17 (owner hardware hand check) needs the `soos-lockstatus` bridge of PR 2; LKS18 (Docker PAM matrix, nightly
  fuzz) runs in CI on the pull request.
- PR 2 contract: one connection, reconnect before the 30 s lifetime and the 1024-request cap, poll at most 4 Hz,
  back off on refusal or EOF, treat a lower `sequence` as a new epoch, render an old daemon (connection closed on
  `kind = 3`) as unavailable.
- Accepted residuals (ADR): a PAM `timeout_ms` above 2500 and presence refusals after a match may be revealed after
  the floor.
- Rendering dependencies (proposal `smithay-client-toolkit` + `tiny-skia`) get their own supply-chain review in
  PR 3.
