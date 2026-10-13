# Architect Spec — GitHub #380 (PR 1 of #352): read-only `LockState` request kind and daemon lock-state snapshot

- **Date**: 2026-10-13
- **Epic**: #352 (lock-screen face-scan widget) — **Sub-issue**: #380
- **Branch**: `feat/lockscreen-status` (GitHub-only issue, not registered in `scripts/sync_issue.py`)
- **Owner decisions applied** (2026-10-13 comment on #352): Q2 the locked screen may show
  *Scanning / Idle / Not recognized*, "Not recognized" collapses every failure reason; Q3 animate
  for PAM face requests and presence scans alike; Q1 adapters later, swaylock-plugin + hyprlock first.

## 1. Scope & Blast Radius

| Crate / file | Change |
|---|---|
| `crates/protocol/src/types.rs` | `RequestKind::LockState = 3`; new `LockWidgetState` (3 values); new `LockStateResponse`; `LOCK_STATE_RESPONSE_MAX_PAYLOAD` |
| `crates/protocol/src/lib.rs`, `message.rs` | re-exports; doc comments listing the request kinds |
| `crates/protocol/fuzz/` | new target `decode_lock_state` (Cargo.toml `[[bin]]`, README) |
| `.github/workflows/fuzz.yml` | add `decode_lock_state` to the target matrix (string only) |
| `crates/daemon/src/lock_state.rs` (new, `#![forbid(unsafe_code)]`) | `LockStateBoard`, `ScanGuard`, `ScanEnd`, `LockStateView`, constants, pure projection |
| `crates/daemon/src/dispatcher.rs` | `with_lock_state_board`, `lock_state_board()`, `handle_lock_state_request` (Step 6d), PAM scan publication (after 8d, before the camera wake) |
| `crates/daemon/src/presence/worker.rs` | `with_lock_state_board`, scan publication (after the attempt is recorded) |
| `crates/daemon/src/main.rs` | one board shared by the dispatcher and the presence worker |
| `crates/daemon/src/lib.rs` | `pub mod lock_state;` |
| `tests/invariants/src/lock_state_contract.rs` (new) | static contract (fields, variants, docs rows, no biometric identifiers, wiring, PAM untouched) |
| Docs | `Docs/IPC_PROTOCOL.md` (wire rows, request table, new §14), `Docs/DAEMON.md` §2/§3 and new constants |
| AI | ADR "Read-Only Lock-Screen State Channel", matrix component `lockscreen-status` (prefix `LKS`), walkthrough 205 |

Consumers of `RequestKind` (grep `RequestKind::` across `crates/` and `tests/`): no exhaustive
`match` exists outside the protocol crate (the daemon uses `==` comparisons; `pam`, `admin-cli`,
`gui`, `remote` only construct `Auth`, `Status` or `PreviewFrame`). Adding a variant therefore
changes no consumer. `crates/pam` is **untouched** (it never sends or decodes `LockState`).

Out of scope: the `soos-lockstatus` bridge, renderers, adapters, installer (later PRs of #352).

## 2. Types & Signatures

### 2.1 Protocol (`soos-protocol`, `#![forbid(unsafe_code)]`, zero I/O)

```rust
pub enum RequestKind { Auth = 0, Status = 1, PreviewFrame = 2, LockState = 3 }

/// Collapsed scan state shown at a locked screen (ADR 2026-10-13 "Read-Only Lock-Screen
/// State Channel"). Exactly three wire values; no success value, no failure reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum LockWidgetState { Idle = 0, Scanning = 1, NotRecognized = 2 }

/// Answer to `RequestKind::LockState`. Bounded: at most `LOCK_STATE_RESPONSE_MAX_PAYLOAD`
/// (12) payload bytes, far below `MAX_MESSAGE_SIZE`. No zeroization needed: no secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockStateResponse {
    pub version: u8,            // CURRENT_VERSION
    pub state: LockWidgetState, // varint discriminant 0..=2; any other value is a decode error
    pub sequence: u64,          // per-UID visible transition counter (0 = nothing visible)
}

/// 1 (version) + 1 (state varint) + 10 (u64 varint max).
pub const LOCK_STATE_RESPONSE_MAX_PAYLOAD: usize = 12;

impl LockStateResponse {
    /// `version == CURRENT_VERSION`.
    #[must_use] pub fn is_current(&self) -> bool;
}
```

- Request: unchanged `Request` struct, `kind = LockState`. `uid_hint` = the UID whose state is
  asked; `service` and `deadline_monotonic_ns` are validated by `Request::validate` (bounds) but not
  otherwise used. The client message tag (`MESSAGE_TAG_REQUEST`) is unchanged.
- Response: on success a `LockStateResponse` frame (`codec::encode`, bounded by
  `MAX_MESSAGE_SIZE`); on refusal the standard `Response` with `ProtocolError` (no
  `LockStateResponse`). A client tells them apart by strict decoding: a `LockStateResponse` payload
  is 3..=12 bytes, a `Response` payload is at least 37 bytes (32-byte `request_id`), so neither
  decodes as the other (strict codec, `TrailingBytes`/`Deserialize`). Pinned by tests.
- Unknown `state` byte (3..=255, or a multi-byte varint) ⇒ `CodecError::Deserialize`; there is
  no `Recognized` wire value at all.
- Compatibility: a pre-#380 daemon fails to decode `kind = 3` (unknown discriminant) and closes the
  connection without an answer; a client must map that to its "unavailable" rendering (PR 2).

### 2.2 Daemon (`crates/daemon/src/lock_state.rs`)

```rust
#![forbid(unsafe_code)]

pub const MAX_LOCK_STATE_UIDS: usize = 32;
pub const NOT_RECOGNIZED_REVEAL_FLOOR_MS: u64 =
    MAX_CAMERA_WAKE_WAIT_MS + PRESENCE_WAKE_SETTLE_MS + DECISION_BUDGET_MS + RESPONSE_WRITE_MARGIN_MS; // 3150
pub const NOT_RECOGNIZED_HOLD_MS: u64 = 3000;
pub const MAX_SCANNING_MS: u64 = 10_000;
pub const LOCK_STATE_MAX_REQUESTS_PER_SEC: u32 = 8;
pub const LOCK_STATE_RATE_WINDOW_NS: u64 = 1_000_000_000;
pub const LOCK_STATE_MAX_TRACKED_UIDS: usize = 64;

/// How a published scan ended (daemon-internal; never on the wire).
pub enum ScanEnd { Recognized, NotRecognized }

/// What a reader sees for one UID at `now`.
pub struct LockStateView { pub state: LockWidgetState, pub sequence: u64 }

#[derive(Clone, Default)]
pub struct LockStateBoard { /* Arc<tokio::sync::watch::Sender<Board>> */ }

impl LockStateBoard {
    pub fn new() -> Self;
    /// Publishes `Scanning` for `uid` (new scan id; `sequence += 1`).
    pub fn begin_scan(&self, uid: u32, now_ns: u64) -> ScanGuard;
    /// Pure read: never mutates the board.
    pub fn view(&self, uid: u32, now_ns: u64) -> LockStateView;
    /// Number of UIDs currently held (`<= MAX_LOCK_STATE_UIDS`; test and diagnostics hook).
    pub fn tracked_uids(&self) -> usize;
}

#[must_use]
pub struct ScanGuard { /* board, uid, scan_id, settled: bool */ }
impl ScanGuard {
    /// `Recognized` ⇒ `Idle`; `NotRecognized` with `Some(now)` ⇒ `NotRecognized`;
    /// `NotRecognized` with `None` (clock failure) ⇒ `Idle`. `sequence += 1`.
    pub fn finish(self, end: ScanEnd, now_ns: Option<u64>);
}
impl Drop for ScanGuard { /* not finished ⇒ Idle (abandoned scan never claims anything) */ }
```

Internal board (never exposed): `BTreeMap<u32, Entry>` bounded by `MAX_LOCK_STATE_UIDS`, plus
`next_scan_id: u64`. `Entry { phase: LockWidgetState, sequence: u64, scan_started_ns: u64,
changed_at_ns: u64, scan_id: u64 }`. **No** reason class, score, frame, embedding, template,
session id or PID is stored.

Rules (all arithmetic saturating):

1. `begin_scan(uid, now)`: `next_scan_id += 1`; if `uid` is absent and the map holds
   `MAX_LOCK_STATE_UIDS` entries, evict the entry with the smallest `changed_at_ns` (ties: smallest
   UID); then `phase = Scanning`, `sequence += 1`, `scan_started_ns = changed_at_ns = now`,
   `scan_id = next_scan_id`.
2. `finish` / drop: applies only when the entry still carries this guard's `scan_id` **and** is
   `Scanning` (a later `begin_scan` of the same UID supersedes the earlier guard, whose settle is a
   no-op). Then `sequence += 1`, `changed_at_ns = now` (or unchanged when `None`), `phase` per the
   table above. Settling twice is impossible (`finish` consumes the guard; `Drop` checks `settled`).
3. `view(uid, now)` (pure projection over a copy of the entry):
   - no entry ⇒ `(Idle, 0)`;
   - `Idle` ⇒ `(Idle, sequence)`;
   - `Scanning` and `now - changed_at_ns > MAX_SCANNING_MS` ⇒ `(Idle, sequence)` (stale guard,
     defence in depth); otherwise `(Scanning, sequence)`;
   - `NotRecognized`: `reveal_at = max(changed_at_ns, scan_started_ns + FLOOR)`;
     `now < reveal_at` ⇒ `(Scanning, sequence - 1)` (the sequence of the begin, so the end of the
     scan is not observable before the floor); `now >= reveal_at + HOLD` ⇒ `(Idle, sequence)`;
     otherwise `(NotRecognized, sequence)`.
   - `now` earlier than a stored stamp is treated as elapsed 0 (`saturating_sub`).

Why the reveal floor: a presentation-attack veto ends the consensus at the first spoof capture,
a non-match only at the decision budget. Without the floor the moment `NotRecognized` appears
would distinguish the two at the locked screen (a PAD oracle the PAM text never gives for presence
scans). The floor equals the longest regular non-matching presence scan (camera wake + settle +
decision budget + write margin), built from the imported constants (single source of truth).

### 2.3 Publication points

- **PAM `Auth`** (`ConnectionDispatcher::handle_request`): after Step 8d (template found, attempt
  recorded) and before 8d-1 `notify_activity`: `let scan = self.lock_state.begin_scan(uid_hint, now_ns)`.
  At the final verdict: `Allow` ⇒ `finish(Recognized, clock)`; every other rendered verdict after
  the begin (camera not ready, consensus aborted, preempted, `Deny`, stale) ⇒
  `finish(NotRecognized, clock)`. Early `?` returns drop the guard ⇒ `Idle`. Requests refused
  before 8d (validation, UID mismatch, session policy, deadline, foreign template, rate limit,
  missing template, store error, no pipeline) publish nothing.
- **Presence** (`PresenceWorker::tick`): after `record_attempt_with_reserve` succeeded and
  `mark_scan_started`, `begin_scan(candidate uid, attempt_ns)`; outcome mapping:
  `Unlocked` ⇒ `Recognized`; `Preempted`, `ShuttingDown` ⇒ guard dropped (`Idle`, and a no-op when
  the PAM request already superseded it); every other `ScanOutcome` ⇒ `NotRecognized`
  (`NoMatch`, `SpoofVetoed`, `CameraUnavailable`, `Aborted(_)`, `SessionChanged`, `AllowExpired`,
  `UnlockFailed`, `AccountRefused(_)`, `KillSwitchEngaged`, `LidClosed`). Clock for `finish`: the
  worker's `clock_fn` (`None` on error).

### 2.4 Read path — `RequestKind::LockState` in the dispatcher

Order (after Step 5a wire validation and Step 6 `verify_peer_credentials`, which already answers
`ProtocolError`/`UidMismatch` for an unprivileged peer whose `SO_PEERCRED` UID differs from
`uid_hint`; Step 6b is `Auth` only):

1. Step 6d `handle_lock_state_request(peer_uid, uid_hint)`:
   1. per-peer-UID quota `lock_state_limiter` (`RateLimiter::new_with_capacity(8, 1 s, 64)`,
      separate from the authentication limiter of `PipelineComponents::policy`); over quota or
      table full ⇒ `Response` `ProtocolError`/`RateLimited`;
   2. clock: `Err` ⇒ `LockStateResponse { Idle, 0 }` (fail neutral, connection kept);
   3. `session_validator.has_local_seat_session(uid_hint)` false ⇒ `{ Idle, 0 }` (no data, no hint
      about any other session; same predicate as the preview path, ADR 2026-10-07 LC-3a);
   4. otherwise `board.view(uid_hint, now)`.
2. Never: `pipeline` access, `notify_activity`, `register_interactive`, `policy.write()`,
   evidence, logind D-Bus, `/proc` reads. Not one-shot (persistent connections allowed). The
   auth-reserved connection still refuses it (only `Auth` admitted there, unchanged).
3. Logging: `debug!` only on success (no state value, no UID-other-than-peer); `warn!` on quota.

Authorization rationale (define and justify): the requesting principal is the UID itself (the
widget runs as the locked user, who already belongs to the `soos` group for PAM) or root
(diagnostics). The session predicate reuses the existing local-seat binding so that a UID with no
local seat session (e.g. only SSH) learns nothing. A cgroup placement check (preview S-5) is not
added: the data is the caller's own coarse state, so a same-UID remote process gains nothing it
could not already infer from its own PAM requests; this is recorded in the ADR.

## 3. Constants & Config

| Name | Location | Value | Sentinel semantics |
|---|---|---|---|
| `RequestKind::LockState` | `crates/protocol/src/types.rs` | wire 3 | — |
| `LockWidgetState::{Idle,Scanning,NotRecognized}` | same | 0 / 1 / 2 | other values: decode error |
| `LOCK_STATE_RESPONSE_MAX_PAYLOAD` | same | 12 | encoded payload never larger |
| `MAX_LOCK_STATE_UIDS` | `crates/daemon/src/lock_state.rs` | 32 | full ⇒ evict oldest `changed_at_ns` |
| `NOT_RECOGNIZED_REVEAL_FLOOR_MS` | same | 3150 (derived) | — |
| `NOT_RECOGNIZED_HOLD_MS` | same | 3000 | after it `Idle` |
| `MAX_SCANNING_MS` | same | 10 000 | after it a `Scanning` entry reads `Idle` |
| `LOCK_STATE_MAX_REQUESTS_PER_SEC` / `LOCK_STATE_RATE_WINDOW_NS` / `LOCK_STATE_MAX_TRACKED_UIDS` | same | 8 / 1 s / 64 | over ⇒ `ProtocolError`/`RateLimited` |

Nothing is configurable (no new `daemon.toml` key). The 8/s quota leaves room for two consumers at
the bridge's 4 Hz cap (PR 2).

## 4. Error Taxonomy

| Situation | Answer |
|---|---|
| malformed request / bad version / oversize service | `Response` `ProtocolError`/`MalformedRequest` (existing Step 5a) |
| unprivileged peer, `uid_hint != SO_PEERCRED` UID | `Response` `ProtocolError`/`UidMismatch` (existing Step 6) |
| quota exceeded / limiter table full | `Response` `ProtocolError`/`RateLimited` |
| clock error | `LockStateResponse { Idle, 0 }` |
| no local seat session of `uid_hint` | `LockStateResponse { Idle, 0 }` |
| unknown UID / nothing published | `LockStateResponse { Idle, 0 }` |
| on an auth-reserved connection | connection closed, no answer (existing) |

No path produces anything but the three states; there is no success state on the wire. The PAM
module, its verdicts and `PAM_IGNORE` fallbacks are unchanged.

## 5. Latency Budget

The `Auth` path gains one in-memory `watch::Sender::send_modify` before the camera wake and one at
the verdict (bounded map of ≤ 32 entries, no I/O, no `.await`): negligible against
`DECISION_BUDGET_MS` 900 ms. PAM `timeout_ms` arithmetic unchanged. `LockState` requests never
take the inference slot, the policy lock or the camera; the per-UID connection cap (#157) and the
8/s quota bound their cost (one bounded `/run/systemd/sessions` scan each, as for preview).

## 6. Invariants Touched

- ARCHITECTURE §2 / AGENTS: UDS only, no new socket, no permission change, never 0666; PAM `.so`
  untouched (no Tokio, blocking, `catch_unwind`, `PAM_IGNORE`); no biometric data or reason class on
  the wire or in logs; `ReasonClass` never exposed (review PAM-03 / #174 stays true: PAD reject and
  non-match are the same `NotRecognized`, and the reveal floor hides their duration difference).
- Peer limits (#157) and the auth-reserved connection (ADR 2026-10-12 S-3) unchanged.
- Rate limiter (#200, #323): `LockState` never records an authentication attempt.
- Presence (#323): timing, tracker and unlock path unchanged; publication is a side effect only.

## 7. Test Hooks for Tester

- `LockStateBoard::{begin_scan, view}`, `ScanGuard::finish` with explicit `now_ns` (clockless).
- `ConnectionDispatcher::with_lock_state_board(board)` and `lock_state_board()`; existing
  `with_session_validator`, `with_inference_gate`, `with_clock_fn`.
- `PresenceWorker::with_lock_state_board(board)`; existing `SpyCamera.on_wake` hook to observe
  `Scanning` mid-scan; `PipelineParts::{tracked_uids, inferences}`; `SpyCamera::wakes`.
- Protocol: `codec::{encode, decode}` on `LockStateResponse`, `decode_client_message`.

## 8. Documentation Drift / ADR

- The epic text proposed eight states (`Unavailable`, `NotLocked`, `Grace`, `Recognized`,
  `Paused`, `Disabled`, …) and a `since_monotonic_ns` field. The owner decision (Q2) limits the
  visible states to three; `since_monotonic_ns` is dropped because an exact transition timestamp is a
  timing oracle (§2.2). "Disabled"/"no session" collapse to `Idle`; "Unavailable" is a client-side
  rendering of transport errors (bridge, PR 2). **Assumed — owner to confirm**: no `Recognized`
  state (the session unlocking is the success signal); errors after a scan started show
  `NotRecognized`; the reveal floor of 3150 ms and the 3 s hold.
- ADR to add: "Read-Only Lock-Screen State Channel" (threat model, collapsed state, reveal floor,
  authorization, quota, no zbus, per-locker adapters rationale, rendering dependencies deferred to
  PR 3 with their own supply-chain review).

## 9. Revision 1 (plan evaluation of 2026-10-13, findings F1–F8)

This section **supersedes** the conflicting statements above.

- **R1 (F1, MAJOR) — state is a raw byte.** `LockWidgetState` is serialized as one raw `u8`
  (`#[serde(try_from = "u8", into = "u8")]`, `impl From<LockWidgetState> for u8`,
  `impl TryFrom<u8> for LockWidgetState` with error `UnknownLockWidgetState(u8)` for every value
  above 2). Postcard writes a `u8` as one raw byte, so no overlong varint can alias a state: any
  byte 3..=255 is a decode error. The payload stays `version (u8) | state (u8) | sequence (varint)`,
  at most 12 bytes. `Docs/IPC_PROTOCOL.md` documents the state as a raw byte.
- **R2 (F2) — quota and clock order.** The read path is: clock first (`Err` ⇒
  `LockStateResponse { Idle, 0 }`, nothing recorded); then the quota
  `RateLimiter::new(RateLimitConfig::new_with_capacity(LOCK_STATE_MAX_REQUESTS_PER_SEC,
  LOCK_STATE_RATE_WINDOW_NS, LOCK_STATE_MAX_TRACKED_UIDS))`, `check_and_record(peer_uid, now)`;
  over quota ⇒ `ProtocolError`/`RateLimited`. A full table evicts the least recently used UID
  (existing limiter behaviour); it never refuses.
- **R3 (F3) — one uniform reveal rule for every scan end.** The board keeps per UID
  `scans: u64` (begins) and the current scan `{ started_ns, end: Option<(LockWidgetState, at_ns)> }`.
  `sequence` is derived: `2 * scans - 1` while the current scan's end is not visible,
  `2 * scans` once it is. **Every** end (`Recognized` ⇒ `Idle`, `NotRecognized`, abandoned ⇒ `Idle`,
  clock failure ⇒ `Idle`) becomes visible only at `reveal_at = max(end_at, started_ns + FLOOR)`;
  before that the view is `(Scanning, 2 * scans - 1)` (or `Idle` with the same sequence once the
  running scan is older than `MAX_SCANNING_MS`). A `begin_scan` always replaces the current scan
  and sets `scans += 1`, so the sequence advances identically whether the superseded scan had
  already ended (hidden) or was still running: supersession and preemption reveal nothing. After
  the reveal, `NotRecognized` decays to `Idle` at `reveal_at + HOLD` (sequence unchanged). A
  superseded guard's settle is a no-op (scan id check). Recognized/abandoned ends also wait for the
  floor; the session unlock is the visible success signal, so the delay costs nothing.
- **R4 (F4) — scope of the floor.** The floor (3150 ms) covers every presence scan that ends
  before a match and PAM scans at the shipped default `timeout_ms=2500`. A PAM scan with a larger
  `timeout_ms` (up to 5000) and a presence refusal *after a match* (`LidClosed`, `AccountRefused`,
  `SessionChanged`, `AllowExpired`, `UnlockFailed`, `KillSwitchEngaged`) may end after the floor and
  are revealed at their end. Accepted and recorded in the ADR: the PAM duration is already visible to
  the locker itself (its PAM conversation returns), and a post-match refusal requires a face that
  already matches, so it is no presentation-attack oracle.
- **R5 (F5) — logging.** Quota refusals log at `debug!` (never `warn!` per request); success logs
  nothing above `debug!` and never the state. The `SessionValidator` warning when
  `/run/systemd/sessions` is missing is pre-existing (the directory exists on every systemd host).
  The PR 2 bridge must back off on refusal and on a closed connection (old daemon).
- **R6 (F6) — static check of the read path.** `tests/invariants/src/lock_state_contract.rs` also
  asserts that the body of `ConnectionDispatcher::handle_lock_state_request` contains no
  `notify_activity`, `register_interactive`, `policy`, `pipeline`, `ReasonClass::` other than
  `RateLimited`, `info!`/`warn!`, and no state value in a log field.
- **R7 (F7) — added tests.** PAM vision failure (`ConsensusRun::Aborted`) and a short client
  deadline map to `NotRecognized`; a presence post-match refusal (`AccountRefused`) maps to
  `NotRecognized`; board tests for the uniform rule (supersession of a hidden end and of a running
  scan give the same sequence; early `Recognized`/abandoned ends stay hidden until the floor).
- **R8 (F8) — client budget (PR 2 contract).** Each UID has 2 regular connections plus 1
  auth-reserved one. The bridge uses at most one connection, reconnects before the 30 s
  connection lifetime and the 1024-request cap, polls at ≤ 4 Hz (≤ 8/s quota), and backs off on
  refusal or EOF.

## 10. Revision 2 (candid review of 2026-10-13, CHANGES_REQUESTED)

This section **supersedes** the conflicting statements above.

- **C1 (MAJOR) — presence cadence anchored to the scan start.** The presence worker scheduled its
  next scan from the end of the previous one (`run()` sleeps `LOCK_POLL_INTERVAL_MS` after each tick,
  then the `scan_interval` gate), so an early end (spoof veto at the first capture) started the next
  scan earlier and superseded the hidden end before the floor: no visible `NotRecognized` and an
  earlier sequence jump, unlike a full non-match. Fix: `PresenceWorker::post_scan_hold(&PresenceTick)
  -> Duration` returns, after a `Scanned` tick whose outcome is not `Unlocked`, the time left until
  `last_scan_started_ns + REVEAL_FLOOR_NS` (bounded by the floor; the whole floor on a clock error);
  `ZERO` after an unlock or a skipped tick. `run()` waits `LOCK_POLL_INTERVAL_MS + post_scan_hold`
  (shutdown still interrupts it). The next scan therefore starts about floor + 1 s after the previous
  start whatever the end. Unlock latency is unchanged; the failing-scan retry cadence becomes about
  4.24 s start to start (documented in the ADR). Tests: `test_lks_post_scan_hold_is_anchored_to_the_scan_start`,
  `test_lks_fast_and_slow_presence_failures_schedule_the_next_scan_identically` (drives `run()`).
- **C2 — floor margin.** `REVEAL_FLOOR_MARGIN_MS = DEFAULT_INFERENCE_ESTIMATE_MS + FRAME_POLL_INTERVAL_MS`
  (90 ms): an inference admitted inside the decision budget may finish up to one nominal inference
  after its estimate, and the loop notices the end within one frame poll. The floor becomes 3240 ms.
- **C3 — sessions scan off the runtime.** `handle_lock_state_request` runs
  `SessionValidator::has_local_seat_session` through `spawn_blocking` under
  `LOCK_STATE_SESSION_CHECK_TIMEOUT_MS` = 250 ms; expiry or a failed job answers `{ Idle, 0 }`.
