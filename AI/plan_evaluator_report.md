# Plan Evaluation Report — Round 2
- **Date**: 2026-10-13
- **Issue**: GitHub #380 (PR 1 of #352) — read-only `LockState` request kind and daemon lock-state snapshot
- **Branch**: `feat/lockscreen-status`
- **Base commit**: `6f32ef8`
- **Spec evaluated**: `AI/architect_spec_lockscreen_status.md`
- **Draft contract tests checked for power**: `crates/protocol/tests/lock_state_tests.rs`,
  `crates/daemon/tests/lock_state_{board,dispatcher,presence}_tests.rs`,
  `tests/invariants/src/lock_state_contract.rs`, `crates/protocol/fuzz/fuzz_targets/decode_lock_state.rs`

## 1. Coverage Matrix

| Acceptance line (epic #352, PR 1) / TDD test | Spec element | Status |
|---|---|---|
| ADR | §8 "Read-Only Lock-Screen State Channel" (threat model, collapsed state, floor, authorization, quota) | Covered (ADR text must scope the floor claim, F4) |
| `RequestKind::LockState` wire 3 | §2.1, §3; `test_lks_request_kind_lock_state_is_wire_value_3` | Covered |
| Bounded `LockStateResponse` | §2.1 `LOCK_STATE_RESPONSE_MAX_PAYLOAD = 12`; `test_lks_response_payload_is_bounded`, fuzz bound assert | Covered |
| Docs wire table | §1 Docs row; existing `test_ipc_protocol_doc_wire_indices_match_code` + `test_lks_ipc_doc_lists_lock_state_wire_rows` | Covered |
| proptest | `prop_lks_round_trip`, `prop_lks_lock_state_is_never_another_response`, `prop_lks_decoder_never_panics`, `prop_lks_request_round_trip` | Covered |
| Fuzz target | §1 `decode_lock_state` + workflow matrix; `test_lks_fuzz_harness_covers_the_lock_state_response` | Covered |
| Unknown state byte decodes to error | §2.1 "3..=255, or a multi-byte varint ⇒ `CodecError::Deserialize`"; `test_lks_unknown_state_byte_is_a_decode_error` | **Defective**: the multi-byte half is false for the specified representation (F1) |
| UID mismatch ⇒ `ProtocolError`/`UidMismatch` | §2.4 reuses Step 6; `test_lks_other_uid_is_refused_uid_mismatch` | Covered |
| UID without local seat session gets no data | §2.4 step 1.3 `has_local_seat_session(uid_hint)` ⇒ `{Idle,0}`; `test_lks_uid_without_local_seat_session_gets_no_data` | Covered |
| Never records a rate-limit attempt | §2.4 step 2, separate `lock_state_limiter`; `test_lks_lock_state_has_no_side_effect` (`tracked_uids == 0`, 40 remaining), quota test | Covered |
| Never wakes the camera | §2.4 step 2; `camera.wakes() == 0`, `inferences() == 0` | Covered |
| Never changes presence timing | §2.3/§6; `test_lks_lock_state_polling_never_changes_presence_timing` | Covered |
| Every Deny reason incl. PAD ⇒ single `NotRecognized` | §2.3 mapping; PAM low score / PAD / no face / multiple faces / camera; presence no-match / spoof / camera | Covered (gaps: PAM `Aborted`/deadline and presence post-`Allow` failures untested, F7) |
| Errors/timeouts never map to a success | No success value on the wire (§2.1); `Allow`/`Unlocked` ⇒ `Idle`; drop ⇒ `Idle`; clock `None` ⇒ `Idle` | Covered |
| Invariant: no score/reason/frame/embedding/template/session-id fields | `test_lks_lock_state_response_has_only_coarse_fields`, `test_lks_daemon_snapshot_stores_nothing_biometric` | Covered |
| Invariant: forbid unsafe | `lock_state.rs` `#![forbid(unsafe_code)]` asserted | Covered |
| Invariant: no unwrap/expect | `lock_state.rs` textual check + workspace `unwrap_used`/`expect_used = "deny"` (`Cargo.toml:130-131`) | Covered |
| Invariant: nothing biometric logged | `lock_state.rs` has no log macro at all (asserted) | Partial: the dispatcher handler and the publication sites are not pinned (F6) |
| Owner Q2: visible states Scanning / Idle / Not recognized only | `LockWidgetState` 3 variants; `test_lks_widget_state_has_exactly_the_owner_approved_values` | Covered |
| Owner Q3: animate for PAM and presence | §2.3 two publication points; dispatcher + presence tests | Covered |
| PAM untouched | `test_lks_pam_module_is_untouched` | Covered |
| One shared board | `test_lks_daemon_shares_one_board` | Covered |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `RequestKind` = Auth 0 / Status 1 / PreviewFrame 2 | `crates/protocol/src/types.rs:59-66` | same; LockState appended at index 3 | Yes |
| `MAX_CAMERA_WAKE_WAIT_MS` | `crates/daemon/src/consensus.rs:28` | 1200 | Yes |
| `PRESENCE_WAKE_SETTLE_MS` | `crates/daemon/src/presence/mod.rs:88` | 1000 | Yes |
| `DECISION_BUDGET_MS` | `crates/daemon/src/pipeline.rs:19` | 900 | Yes |
| `RESPONSE_WRITE_MARGIN_MS` | `crates/daemon/src/inference.rs:30` | 50 | Yes |
| Floor = 1200 + 1000 + 900 + 50 | derived | 3150 | Yes |
| "Floor equals the longest regular non-matching presence scan" | `presence/worker.rs:633-690` (wake ≤ 1200, settle ≤ 1000, budget 900 + margin) | true for presence pre-`Allow` outcomes | Yes (presence only, F4) |
| Longest PAM scan | `config.rs:22-34` connection_timeout default 2500, bounds 100..=10 000; PAM `timeout_ms` ≤ 5000 | up to ~5000 ms, above the floor | Not covered by the floor (F4) |
| Post-`Allow` presence steps | `presence/mod.rs:45,65,80`: `DBUS_CALL_TIMEOUT_MS` 500, `MAX_ALLOW_TO_UNLOCK_MS` 1000, `ACCOUNT_CHECK_TIMEOUT_MS` 500 | can push `SessionChanged`/`AccountRefused`/`LidClosed`/`AllowExpired`/`UnlockFailed` past the floor | F4 |
| `RateLimiter::new_with_capacity(8, 1 s, 64)` | `crates/policy/src/rate_limit.rs:41,84` | constructor is `RateLimitConfig::new_with_capacity`; limiter is `RateLimiter::new(config)` | No (F2) |
| "limiter table full ⇒ RateLimited" | `rate_limit.rs:165-186` | a new UID evicts the LRU UID; refusal for capacity only when capacity is 0 | No (F2) |
| Step 6 answers `UidMismatch` for an unprivileged mismatching peer | `dispatcher.rs:756-783` | yes | Yes |
| Step 6b is `Auth` only | `dispatcher.rs:787-791` | yes | Yes |
| Auth-reserved connection admits only `Auth` | `dispatcher.rs:596-607` | yes | Yes |
| One-shot only for `Auth` | `dispatcher.rs:612` | yes; LockState stays persistent | Yes |
| No exhaustive `match` on `RequestKind` outside protocol | `dispatcher.rs` (`==`, `matches!`); `limits.rs`/`preview.rs` doc comments only; clients only construct | yes. Note: every kind other than `Status`/`PreviewFrame` currently falls through into the `Auth` pipeline, so the 6d branch must return before Step 7 (pinned by `test_lks_lock_state_has_no_side_effect`) | Yes |
| Publication point "after 8d, before 8d-1" | `dispatcher.rs:938-973` | 8c records the attempt, 8d answers missing template/store error, 8d-1 `notify_activity` | Yes |
| Presence publication "after `record_attempt_with_reserve` and `mark_scan_started`" | `presence/worker.rs:608-618` | yes | Yes |
| `ScanOutcome` list | `presence/worker.rs:119-150` | 14 variants as listed | Yes |
| `has_local_seat_session` | `session.rs:96` | exists; emits `warn!` on every call when the sessions dir is missing (`session.rs:110-115`) | Yes (F5) |
| Per-UID connection cap | `limits.rs:23,49` | 2 regular + 1 auth reserve | Yes (F8) |
| Codec strictness | `codec.rs:277-286` (`postcard::take_from_bytes` + trailing rejection) | trailing bytes rejected; `Response` payload ≥ 37 bytes vs ≤ 12 ⇒ mutually exclusive | Yes |
| Postcard varint canonicality | `postcard-1.1.3/src/de/deserializer.rs:85-101` | **overlong varints accepted** (`0x81 0x00` decodes to 1) | No (F1) |
| Old daemon rejects kind 3 | serde-derived enum index check | `Deserialize` error ⇒ frame rejected, connection closed | Yes |
| Existing invariants needing updates | `protocol_codec_contract.rs:255-268` (wire-index rows), `daemon_docs_contract.rs:125-140` | the IPC doc must gain the `RequestKind::LockState` / 3 row; the spec plans it | Yes |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario (cross-UID): unprivileged UID 1001 sends `LockState` with `uid_hint = 1000` ⇒ Step 6 answers `UidMismatch` before 6d. Root may query any UID (diagnostics, consistent with existing root semantics). PASS.
- Failure scenario (remote same-UID): an SSH process of the locked user reads `Scanning` and learns when a face scan runs at the machine. Accepted as the user's own coarse data and recorded in the ADR (no cgroup check). PASS with documentation.
- Failure scenario (wire exclusivity): a `Response` payload decoded as `LockStateResponse` leaves ≥ 25 trailing bytes ⇒ `TrailingBytes`; the reverse is too short. PASS. The "multi-byte varint is an error" claim is false under postcard (F1).
- Failure scenario (old daemon): kind 3 ⇒ decode error ⇒ connection closed with one `warn!` per attempt; a 4 Hz bridge would flood an old daemon's journal unless it backs off (PR 2 concern, F5/F8).
- No new socket, no permission change, UDS only.
- Result: FINDING (F1 MAJOR).

### Pillar 2 — PAM deadline & concurrency
- `crates/pam` is untouched and pinned by an invariant. The `Auth` path gains two `watch::Sender::send_modify` calls on a ≤ 32-entry map (no `.await`, no I/O): `timeout_ms` arithmetic unchanged.
- Failure scenario (supersession race): PAM registers interactive demand at Step 8; the presence consensus returns `Preempted` and its guard drop races with the PAM `begin_scan` at 8d. If the drop lands first, `Idle` (sequence + 1) is visible during the template lookup and attempt recording; if the presence scan had already ended hidden (`NotRecognized` before the floor), the PAM begin supersedes it directly (visible sequence jumps by 2). The two cases are distinguishable to a reader (F3).
- Failure scenario (stale guard): a PAM scan with `deadline_monotonic_ns == 0` is bounded by `connection_timeout` ≤ 10 000 ms; `MAX_SCANNING_MS` = 10 000 may make such a still-running scan read `Idle` at its very end. Benign (no success value).
- Result: FINDING (F3 MINOR).

### Pillar 3 — Panic safety & fail-closed
- No success value exists on the wire; `Allow`/`Unlocked`, a dropped guard and a clock failure read `Idle`; every rendered non-`Allow` verdict after the begin reads `NotRecognized`. An implementation mapping `Allow` to `NotRecognized`, or a failure to `Idle`, fails `test_lks_pam_allow_publishes_scanning_then_idle` / `assert_not_recognized`.
- Failure scenario: a clock error on the read path ⇒ `{Idle, 0}`, but §2.4 orders the quota (which needs `now`) before the clock read (F2).
- `lock_state.rs` arithmetic is saturating; `view` is pure; the workspace denies `unwrap`/`expect`.
- Result: PASS (ordering ambiguity tracked as F2).

### Pillar 4 — Dependencies
- No new crate (tokio `watch` is already a daemon dependency; postcard/serde unchanged). The fuzz crate gains one `[[bin]]`. PASS.

### Pillar 5 — Data confidentiality
- The board stores phase, sequence and timestamps only; no reason, score, frame, template, session id or PID (asserted textually). `since_monotonic_ns` dropped as a timing oracle.
- Failure scenario (PAD oracle): presence PAD veto at 300 ms vs non-match at ~3100 ms ⇒ both reveal at start + 3150 (`test_lks_fast_and_slow_failures_are_indistinguishable`). PASS for presence pre-`Allow` outcomes.
- Failure scenario (beyond the floor): (a) PAM scans with `timeout_ms`/`connection_timeout` above ~3150 ms end a non-match at up to ~5000 ms while a PAD veto reveals at 3150 ms; (b) presence post-`Allow` refusals (`LidClosed`, `AccountRefused`, `SessionChanged`, `AllowExpired`, `UnlockFailed`) run bounded D-Bus calls after the `Allow` and may reveal after the floor, i.e. "the face matched and passed PAD but the unlock was refused". (a) adds no channel beyond the PAM call duration the locker already exposes; (b) is narrow. The spec's §6 claim that the floor "hides their duration difference" is nonetheless absolute and must be scoped (F4).
- Logging: `lock_state.rs` has no log macro (asserted); the dispatcher handler's logging is specified in prose only (F6); a `warn!` per refused poll, and the session validator's `warn!` per call when `/run/systemd/sessions` is missing, can flood the journal at poll rate (F5).
- Result: FINDING (F4, F5, F6 MINOR).

### Pillar 6 — Test integrity
- Draft tests are new files; the only existing file modified is `tests/invariants/src/lib.rs` (module registration). Existing invariants `test_ipc_protocol_doc_wire_indices_match_code` and `test_ipc_protocol_doc_enum_references_resolve` will require the new doc row, which the spec plans. No existing test is weakened.
- Failure scenario (unsatisfiable contract): `test_lks_unknown_state_byte_is_a_decode_error` asserts `[CURRENT_VERSION, 0x81, 0x00, 0x01]` is an error. With the spec's serde-derived "varint discriminant", postcard 1.1.3 decodes `0x81 0x00` as index 1 (`Scanning`) and `0x01` as `sequence = 1` ⇒ `Ok`. The specified implementation cannot meet this contract; the developer would have to invent a representation or be tempted to weaken the test. The comment "varint 129" is also wrong (129 is `0x81 0x01`).
- Power gaps: the quota test only asserts `refused >= 1` (acceptable, because the side-effect test requires 8 accepted); no PAM `Aborted`/deadline case; no presence post-`Allow` failure case; no board test for a begin over a hidden `NotRecognized` (F7).
- Result: FINDING (F1 MAJOR, F7 MINOR).

## 4. Findings

- **[MAJOR] F1 — `LockWidgetState` wire representation contradicts the decoder.** §2.1 specifies a serde-derived enum ("varint discriminant 0..=2") and claims a multi-byte varint is a decode error. Postcard 1.1.3 accepts overlong varints (`try_take_varint_u32` stops at the first byte without the continuation bit and never checks canonicality), so `0x81 0x00` decodes as `Scanning`; the drafted contract `test_lks_unknown_state_byte_is_a_decode_error` cannot pass against the specified type. **Required plan change**: specify the representation explicitly, e.g. `#[serde(try_from = "u8", into = "u8")]` with `impl TryFrom<u8> for LockWidgetState` rejecting values above 2 (postcard encodes `u8` as one raw byte, so `0x81` is refused and the 12-byte bound still holds); state it in §2.1 and in the `Docs/IPC_PROTOCOL.md` row ("raw `u8`, not a varint"); have the tester fix the "varint 129" comment (`0x81 0x00` encodes 1). Alternatively drop the multi-byte claim and that assertion, but the explicit raw byte is preferred because it makes "unknown state byte ⇒ error" literally true for every byte.
- **[MINOR] F2 — Rate limiter citation and read-path order.** `RateLimiter::new_with_capacity` does not exist (use `RateLimiter::new(RateLimitConfig::new_with_capacity(8, 1_000_000_000, 64))`); the limiter never refuses because its table is full (it evicts the LRU UID), so remove "table full ⇒ RateLimited" or document the eviction (it resets that UID's quota; acceptable, peers are `soos` group members). §2.4 lists the quota (needs `now`) before the clock read: specify clock first, and whether a clock error answers `{Idle, 0}` without consuming quota.
- **[MINOR] F3 — Supersession and drop leak the hidden end through `sequence` / an `Idle` flicker.** A dropped (preempted) guard publishes `Idle` with `sequence + 1` immediately, while an ended-but-hidden `NotRecognized` keeps reading `Scanning`; a PAM begin over a hidden entry jumps the visible sequence by 2, over a running scan by 1. A user-triggered PAM attempt during a presence scan therefore reveals whether that presence scan had already ended (early end ≈ PAD veto). The PAM call already gives the attacker an equivalent timing channel, so no new capability, but the spec's own invariant ("the end of the scan is not observable before the floor") is violated. **Required**: either define a uniform rule (a begin over a hidden entry yields the same visible sequence step as a begin over a running scan; a preempted drop publishes nothing visible before the superseding begin or the floor) or state the residual leak in the ADR; add one board test pinning the chosen rule.
- **[MINOR] F4 — Scope of the reveal-floor guarantee.** The floor (3150 ms, verified) covers presence pre-`Allow` outcomes and default PAM scans (`connection_timeout` 2500 ms) only. PAM scans can last up to ~5000 ms (`timeout_ms` ≤ 5000, `connection_timeout` ≤ 10 000), and presence post-`Allow` refusals can end after the floor. **Required**: rephrase §6 and the ADR ("hides PAD vs non-match for presence scans; PAM scan duration is already observable through the PAM call"), and either cap the reveal of post-`Allow` presence refusals so it is indistinguishable from a regular failure, or record that residual oracle explicitly.
- **[MINOR] F5 — Journal flooding at poll rate.** §2.4 logs `warn!` on every quota refusal, and `SessionValidator::any_session_matches` emits `warn!` on every call when the sessions directory is missing; an old daemon also `warn!`s on every rejected kind-3 frame. **Required**: log the quota refusal at `debug!` or on transition only, and state that the bridge (PR 2) backs off after a refusal or a closed connection.
- **[MINOR] F6 — Logging/side-effect invariant only covers `lock_state.rs`.** "Nothing biometric logged" and "never wakes camera / never records an attempt" are tested behaviourally, but no static check pins `handle_lock_state_request`. **Required**: add an invariant on the body of `handle_lock_state_request` in `dispatcher.rs` (no `notify_activity`, `register_interactive`, `policy.write`, `pipeline`, `ReasonClass`; log macros carry no state value and no UID other than the peer's).
- **[MINOR] F7 — Test-plan gaps.** Add: PAM `ConsensusRun::Aborted` / deadline expiry ⇒ `NotRecognized`; one presence post-`Allow` failure (e.g. `SessionChanged` or `LidClosed`) ⇒ `NotRecognized`; the F3 board rule. Fix the misleading "varint 129" comment (F1).
- **[MINOR] F8 — Connection budget for the future bridge.** `DEFAULT_MAX_CONNECTIONS_PER_UID` = 2 (+1 auth reserve). A persistent `LockState` connection shares the user's regular slots with the GUI preview and `soos-remote`. The spec should state for PR 2 that the bridge holds at most one connection, reconnects within `DEFAULT_MAX_CONNECTION_LIFETIME_MS` (30 s), and backs off on an old daemon.

### Round 1 verdict

One MAJOR finding (F1): the specified wire representation of `LockWidgetState` could not satisfy the "unknown state byte decodes to error" contract, because postcard accepts overlong varints. Round 1 verdict: REVISION_REQUIRED.

## 5. Round 2 — Re-evaluation of spec §9 "Revision 1" (R1–R8) and the updated draft tests

### 5.1 Resolution of round-1 findings

| Finding | Revision | Verification | Status |
|---|---|---|---|
| F1 (MAJOR) state encoding | R1: `#[serde(try_from = "u8", into = "u8")]`, `TryFrom<u8>` rejects values above 2, raw byte documented | A postcard `u8` is one raw byte, so `0x81` is refused before any varint is read. The 12-byte bound still holds: 1 version byte + 1 state byte + 10 bytes for the `u64` sequence varint. `test_lks_unknown_state_byte_is_a_decode_error` now covers bytes 3..=255 and the overlong case, with a corrected comment ("0x81 0x00 = 1"). A serde-derived enum would fail it, as required. | Resolved |
| F2 limiter / order | R2: clock first (`Err` ⇒ `{Idle,0}`, nothing recorded); `RateLimiter::new(RateLimitConfig::new_with_capacity(..))`; LRU eviction documented | Matches `rate_limit.rs:41,84,165-186` | Resolved |
| F3 supersession leak | R3: per-UID `scans` counter. The sequence is `2*scans-1` while the end is hidden and `2*scans` once it is visible. Every end (Recognized, NotRecognized, abandoned, clock failure) is revealed at `max(end, start+FLOOR)`, and `begin_scan` always sets `scans += 1`. | A begin over an ended-but-hidden scan and a begin over a running scan now give the same sequence (2s-1 → 2s+1). A preempted drop stays hidden, so the `Idle` flicker is gone. Pinned by `test_lks_supersession_does_not_reveal_a_hidden_end`, `test_lks_abandoned_scan_reads_idle_after_the_floor`, `test_lks_presence_preempted_scan_reads_idle` ("not observable before the floor"), and by the PAM `Allow` and presence-unlock tests, which now read `(Scanning,1)` until the floor and then `(Idle,2)`. | Resolved |
| F4 floor scope | R4: claim scoped to presence pre-match ends and PAM at the shipped `timeout_ms=2500`; longer PAM scans and post-match refusals are accepted in the ADR | The shipped `timeout_ms=2500` appears in `Docs/DISTRIBUTION_DEPLOYMENT.md` and `Docs/DAEMON.md`, and `DEFAULT_CONNECTION_TIMEOUT_MS` = 2500. PAM scans begin after the request starts, so they end within 2500 ms of the request start, below 3150. The PAM code default is `DEFAULT_TIMEOUT_MS` = 1000 (`crates/pam/src/config.rs:9`), which is even shorter. The rationale holds: the locker already sees PAM durations, and a post-match refusal needs a face that already matched. | Resolved |
| F5 logging | R5: quota refusals at `debug!`, state never logged; PR 2 bridge backs off | The pre-existing `SessionValidator` warning stays (a missing `/run/systemd/sessions` is abnormal on systemd hosts). Accepted. | Resolved |
| F6 static check | R6: `test_lks_dispatcher_read_path_has_no_side_effect` | Forbids `notify_activity`, `register_interactive`, `policy`, `pipeline`, `biometric_store`, `camera`, `evidence`, `info!`/`warn!`/`error!` and `begin_scan`. `ReasonClass::` may only appear as `RateLimited`. | Resolved (residual R2-2 below) |
| F7 test gaps | R7 | Added `test_lks_pam_aborted_consensus_maps_to_not_recognized` (detector failure ⇒ `Unavailable` precondition), `test_lks_pam_short_deadline_maps_to_not_recognized` (400 ms deadline, no face), `test_lks_presence_post_match_refusal_maps_to_not_recognized` (`AccountRefused(PasswordLocked)`, zero unlock calls), and the board supersession test | Resolved |
| F8 client budget | R8: one connection, reconnect before the 30 s lifetime / 1024 requests, ≤ 4 Hz, back off | Matches `limits.rs:23,32,35,49` | Resolved (PR 2 contract) |

### 5.2 Round-2 attack on the revised design

- **Timing oracle (R3).** If every end is revealed at `max(end, start+FLOOR)`, can a reader still tell an early PAD veto from a non-match or an early `Allow`? Before the floor, all of them read `(Scanning, 2s-1)`. At the floor, Recognized and abandoned ends read `Idle` and NotRecognized ends read `NotRecognized`, as intended. `test_lks_fast_and_slow_failures_are_indistinguishable` still pins this. PASS.
- **Supersession (R3).** A begin at T0+500 ms over a hidden end and over a running scan gives identical views up to T0+FLOOR, and dropping the preempted guard afterwards changes nothing. PASS.
- **Stale guard.** A running scan older than `MAX_SCANNING_MS` reads `Idle` with sequence `2s-1`. A hidden end can only exist while `now < max(end, start+3150)`, which is far below 10 s, so the staleness rule cannot release a hidden end early. PASS.
- **Fail-closed.** There is still no success value. Aborted, deadline, camera and post-match refusals all map to `NotRecognized`, and the new tests check the `Unavailable` precondition, so a test cannot pass vacuously. PASS.
- **Cross-UID / DoS.** Unchanged from round 1: Step 6 `UidMismatch`, the local-seat predicate, an 8/s per-peer quota with the clock read first, and a bounded board (32) and limiter (64). PASS.

### 5.3 Round-2 findings

- **[MINOR] R2-1 — The sequence is not monotonic across board eviction.** When a UID's entry is evicted (more than 32 UIDs published) and later begins again, `scans` restarts and the sequence goes backwards. There is no security impact. The PR 2 bridge must treat a lower sequence as a new epoch and must not compare across it. Document this in `Docs/IPC_PROTOCOL.md` §14. No spec change is blocking.
- **[MINOR] R2-2 — The log check in the R6 invariant only reads the line that contains `debug!`.** A multi-line `debug!(` call with `state = ?view.state` on the next line, or `trace!`/`tracing::event!`, would pass. The developer must keep each log call in the handler on a single line, or the tester should extend the check to the whole macro invocation and also forbid `trace!`/`event!`. This does not block, because R5 states the rule and the candid review will check it.
- **[INFO]** The R6 forbidden substring `policy` is case-sensitive. The developer must import `RateLimitConfig`/`RateLimiter` at module level rather than writing `soos_policy::` paths inside `handle_lock_state_request`, otherwise the invariant fails on a legitimate implementation.

## 6. Verdict

All round-1 findings are resolved by Revision 1 (R1–R8) and the updated draft tests. Round 2 found no CRITICAL or MAJOR issue; R2-1 and R2-2 are MINOR and can be handled in the developer and tester phases without a further spec revision.

VALIDATION_VERDICT: APPROVED
