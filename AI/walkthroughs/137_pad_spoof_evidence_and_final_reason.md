# Walkthrough 137 — Opt-In Spoof Evidence and Spoof-First Final Reason

- **Date**: 2026-09-30
- **Issues**: GitHub #261 (PAD-14), #262 (PAD-16)
- **Branch**: `fix/p3-pad-evidence-reason`
- **Matrix criteria**: PEV1–PEV5 (new component `pad-spoof-evidence-and-final-reason`)
- **ADR**: 2026-09-30 "Opt-In Spoof Evidence and Spoof-First Final Reason"

---

## 1. Findings

| Issue | Finding | Location (review base) |
|---|---|---|
| #261 | A presentation attack only produced a `debug!` log; evidence snapshots were stored for `PasswordFailed` events only, so the strongest intrusion signal left no evidence | `crates/daemon/src/dispatcher.rs` (`handle_event`, PAD arm of the consensus loop) |
| #262 | The final reason was the last evaluated frame's reason, so a spoof followed by no-face frames was reported `NoFace` | `crates/daemon/src/dispatcher.rs` (`last_reason` loop, since replaced) |

## 2. Specification

- `soos_daemon::dispatcher::PAD_FAILED_EVIDENCE_REASON` = `"PadFailed"` and
  `PASSWORD_FAILED_EVIDENCE_REASON` = `"PasswordFailed"`: the evidence reasons.
- `store_evidence_capture(store, uid, reason, frame)`: one private helper that builds the
  self-describing `EvidenceFrame` (GitHub #181), calls `store_frame_snapshot` and rotates
  retention. Both the `PasswordFailed` event path and the new spoof path use it. Failures are
  logged at `warn` with the UID and reason only.
- Consensus loop: the first capture classified `FrameClass::Spoof` is retained as an
  `Arc<Frame>` (no pixel copy).
- Step 8g, after the verdict is rendered: when the decision is `SpoofVetoed` and
  `[pipeline.evidence] enabled` is set, `capture_spoof_evidence` moves the retained capture and
  an `Arc<EvidenceStore>` into a detached `spawn_blocking` job. The response is never delayed
  and a failed write never changes the verdict.
- Final reason (#262): no production change. The sticky `PadAggregator::spoof_seen` flag of
  GitHub #147 makes `SpoofVetoed` (`Deny` / `PadFailed`) outrank every later capture, and the
  loop stops at the first spoof. The issue described the pre-#147 `last_reason` loop.

## 3. Tests (written first)

- `crates/daemon/tests/pad_evidence_tests.rs` (own fixture, mock camera, evidence store in a
  temporary directory):
  - `test_261_spoof_veto_stores_one_pad_failed_evidence_snapshot`: exactly one snapshot, reason
    `PadFailed`, target UID, 320×240 frame metadata.
  - `test_261_spoof_veto_without_opt_in_stores_no_evidence`: evidence directory stays empty.
  - `test_261_spoof_evidence_respects_daily_cap_per_uid`: three spoofs, cap 1, one snapshot.
  - `test_261_allowed_request_stores_no_evidence`, `test_261_pad_failed_evidence_reason_is_stable`.
  - `test_262_spoof_then_no_face_captures_reports_pad_failed`: the detector is emptied as soon
    as the first request capture reaches the PAD mock.
- `crates/policy/tests/pad_final_reason_tests.rs`: spoof then 16 no-face frames stays
  `PadFailed`; no-face without a spoof stays `NoFace`; a proptest puts any mix of no-face,
  multi-face, non-matching and passing captures before and after one spoof.

Red evidence (with only the reason constant added): `test_261_spoof_veto_stores_one_pad_failed_evidence_snapshot`
and `test_261_spoof_evidence_respects_daily_cap_per_uid` failed with `left: 0, right: 1`
(no snapshot written). The #262 tests passed on the current code: they are regression guards
for the #147 aggregator. A first draft of the #262 integration test failed with `NoFace`
because of the test itself: the fixture's enrollment capture had already called the PAD mock,
so the face was withdrawn before the request started. The test now waits for a PAD call
beyond that baseline.

## 4. Audit

- Opt-in: the dispatcher checks `evidence_store.config().enabled` before it spawns anything,
  and the store enforces the flag again (`EvidenceStoreError::Disabled`).
- Bounded: one snapshot per request at most (the loop stops at the first spoof). Writes are
  bounded by the per-UID attempt reservation, the connection limiter, `daily_cap_per_uid` and
  `daily_cap_total`. The retained frame is an `Arc` clone of a capture that is already in memory.
- No secrets in logs: the new `info!` and `warn!` lines carry `uid` and `reason` only. The
  existing `debug!` PAD score line is unchanged.
- No `unwrap` or `expect` in production code, no new `unsafe`, and no change to the PAM module
  or the wire protocol. The verdict path is unchanged, so no error can become `Allow`.

## 5. Follow-ups

- PEV5: capture a real printed-photo or screen-replay attempt on physical hardware and check
  that the `PadFailed` snapshot decodes (`EvidenceStore::load_snapshot`, `to_rgb24`).
