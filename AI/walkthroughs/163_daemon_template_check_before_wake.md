# Walkthrough 163 — Template Model Binding Before the Camera Wake

- **Date**: 2026-10-01
- **Issue**: GitHub #298 (follow-ups of the SFace switch review, PR #297), first and fifth items.
  GitHub-only follow-up: the branch is not registered in `BRANCH_TO_ISSUE`; commits carry
  `Refs #298`.
- **Branch**: `fix/daemon-template-check-order`
- **Base commit**: `9347449`
- **Matrix criteria**: TCO1–TCO8 (new section `daemon-template-check-order` right after the SFC
  section of walkthrough 162)
- **ADR**: 2026-10-01 "Template Model Binding Before the Camera Wake; Foreign Templates Consume
  No Attempt" (refines item (2) of ADR 2026-09-30 "Client Message Tag Trailer and Atomic Auth
  Attempt Reservation")

---

## 1. Context and Objectives

The approved candid review of the SFace switch (walkthrough 162) left two optional findings on
`crates/daemon/src/dispatcher.rs`:

1. Steps 8c/8d (template fetch and `classify_template`) ran **after** the camera wake (8a/8b). A
   user whose template predates SFace waited for the camera (up to the 1000–1200 ms wake budget on
   a cold sensor) and turned its LED on for a request that was certain to be refused with
   `Unavailable` / `ModelUnavailable`.
2. The same refused request consumed one rate-limit attempt, because the attempt was reserved
   (8-pre) before the template check.

Objectives: classify the template before the wake and before any capture or inference, keep every
earlier gate and every verdict unchanged, and decide (security first) whether a foreign template
may be answered without charging an attempt.

## 2. Architect Design

No type, trait, constant or public API changes. Only the order of step 8 of
`ConnectionDispatcher::handle_request` changes:

| Before | After |
|---|---|
| deadline (`RequestDeadline::compute`) | deadline (unchanged) |
| 8-pre attempt reservation | **8a** `biometric_store.get(uid)` (result kept, not answered yet) |
| 8a/8b camera wake + wait | **8b** `classify_template` on `Ok(Some(t))`; `Foreign` → `Unavailable` / `ModelUnavailable`, no reservation, no wake |
| 8c template fetch (`None`/error answered) | **8c** atomic attempt reservation (unchanged code) |
| 8d `classify_template` | **8d** missing template / store error → `Unavailable` / `InternalError`; then 8d-1/8d-2 camera wake + wait |
| 8e consensus loop | 8e consensus loop (unchanged) |

Earlier gates stay in front of the template read: wire validation (5a), peer UID versus
`uid_hint` (6), local-session policy (6b), preview routing (6c) and the deadline check (7).
The template is read exactly once and the request keeps that snapshot for the match, as before.

Error taxonomy (each condition keeps its verdict and reason; all → `PAM_IGNORE`). Precedence changed: the template outcome now wins over camera availability, so a missing template or a store error with a camera that cannot wake answers `Unavailable` / `InternalError` (was `CameraUnavailable`), and a foreign template with a cold camera answers `ModelUnavailable` (was `CameraUnavailable`):

| Condition | Verdict / reason | Attempt charged | Camera woken |
|---|---|---|---|
| `Foreign` template | `Unavailable` / `ModelUnavailable` | no (was yes) | no (was yes) |
| no template | `Unavailable` / `InternalError` | yes | no (was yes) |
| store error | `Unavailable` / `InternalError` | yes | no (was yes) — pinned by TCO9 |
| current template, limit exhausted | `ProtocolError` / `RateLimited` | rejected reservation | no |
| current template | consensus verdict | yes, before wake and capture | yes |

Latency budget: unchanged for a current template (one template read moved a few microseconds
earlier); a foreign or missing template now ends without the wake wait, so its latency drops from
up to the wake budget to the store read.

Documentation drift: ADR 2026-09-30 item (2) and `Docs/POLICY_CRATE.md` said the reservation
happens "before any camera, enrollment or vision work"; the template read now precedes it. The new
ADR records the refinement.

## 3. Plan Evaluation

Condensed (no separate plan-evaluator report for this two-item follow-up). Checks made against the
code before writing tests: every existing test that pins attempt accounting or ordering was read
(`rate_limit_reservation_tests`, `pipeline_integration_tests` rate-limit case,
`sface_template_binding_tests`, `template_model_binding_tests`, `preview_authorization_tests`), and
no daemon test asserts `CameraUnavailable`. One pinned contract shaped the design:
`rate_limit_reservation_tests::test_200_attempt_ending_before_vision_work_is_recorded` requires a
not-enrolled request to consume the attempt and the next one to be `RateLimited`. The design
therefore charges every path except `Foreign`. Verdict: APPROVED.

### Security analysis (item 2: may a foreign template skip the attempt?)

The rate limiter exists to bound biometric brute force (presenting faces, photos or replays until
one matches). The questions are whether skipping the charge for a foreign template can create a
bypass, a faster probe, or an oracle.

1. **No brute force through the uncharged path.** A `Foreign` template returns before the
   reservation, the camera wake, any capture and any inference. No embedding is ever compared, so
   the uncharged path can never produce `Allow` or even a score.
2. **No time-of-check / time-of-use bypass.** The request classifies the snapshot it read and
   matches against that same snapshot (one `get`, kept in `template_lookup`). If an administrator
   re-enrolls the user between the read and the reservation, the in-flight request still answers
   from its foreign snapshot and evaluates nothing; any request that read a current template is
   charged before its first capture. Turning a template into a current one needs root
   (`soos-enroll`, store `0700 root:root`).
3. **No oracle beyond the existing verdicts.** Only root may target another UID; an unprivileged
   peer may only target its own UID (step 6) from an active local session (step 6b). The distinct
   `ModelUnavailable` verdict was already returned on the first attempt. The single new observable
   is that, once the limit is exhausted, a foreign-template requester keeps receiving
   `ModelUnavailable` instead of `RateLimited`: it learns again what it learned on its first
   request, about itself. A missing template and a store error still consume the attempt and are
   still `RateLimited` once the limit is exhausted, so enumerating "enrolled or not" is exactly as
   limited as before.
4. **Current templates are untouched.** The reservation code is unchanged (one write-lock
   acquisition, `record_attempt`), still before the camera wake, the first `latest_frame` and the
   first inference; concurrent requests still cannot overshoot `max_attempts` (TCO6–TCO8 plus the
   unchanged #200 tests).
5. **Accepted cost.** A rate-limited request now performs one template read (size-bounded file,
   AES-GCM decrypt, no lock, no camera) before its refusal, instead of returning after the lock
   alone. A foreign-template requester can repeat uncharged requests, each costing that read and
   no camera or inference. Volume is bounded by the peer limits (`max_connections_per_uid` 2,
   one-shot `Auth`, global permits). This is a small, bounded CPU cost and no security weakening.

Conclusion: safe; implemented.

## 4. Tester Contract

New file `crates/daemon/tests/template_check_order_tests.rs`: a `SpyCamera` wraps the
`MockCameraManager`, counts `notify_activity` (wake) and `latest_frame` (capture) calls and samples
the limiter (`try_read`, `remaining_attempts`) at the first of each; a `CountingExtractor` counts
inferences. The dispatcher is bound to `EMBEDDING_MODEL_ID`.

| Test (`template_check_order_tests::`) | Matrix | Red evidence on `9347449` |
|---|---|---|
| `test_foreign_template_never_wakes_the_camera` | TCO1 | FAILED: wakes `left: 1, right: 0` |
| `test_wrong_dimension_template_never_wakes_the_camera` | TCO1 | FAILED: wakes `1 != 0` |
| `test_foreign_template_does_not_wait_for_a_cold_camera` | TCO2 | FAILED: `(Unavailable, CameraUnavailable)` instead of `(Unavailable, ModelUnavailable)` |
| `test_current_template_still_wakes_the_camera_and_authenticates` | TCO3 | passes (preservation) |
| `test_missing_template_is_refused_before_the_camera_wake_and_still_counts` | TCO4 | FAILED: wakes `1 != 0` |
| `test_foreign_template_consumes_no_rate_limit_attempt` | TCO5 | FAILED: request 11 `(ProtocolError, RateLimited)` |
| `test_concurrent_foreign_template_requests_reserve_nothing` | TCO5 | FAILED |
| `test_current_template_reserves_the_attempt_before_wake_and_capture` | TCO6 | passes (preservation) |
| `test_rate_limited_current_template_never_wakes_the_camera` | TCO7 | passes (preservation) |
| `test_concurrent_current_template_reservations_stay_atomic` | TCO8 | passes (preservation) |

Migrated existing tests: none. No existing test or assertion was modified.

Flakiness: the new file was run 10 times in a row after the fix, 10/10 green (10 passed each).

## 5. Auditor Constraints

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| 1 | No `unwrap`/`expect`/`panic`/indexing added in production code | `dispatcher.rs` step 8 | clippy `-D warnings`, candid layer 1 |
| 2 | Every gate before step 8 (wire, peer UID, session, deadline) stays in front of the template read | `handle_request` | code order; existing `session_policy_tests`, `peercred_tests` green |
| 3 | A current template, a missing template and a store error reserve exactly one attempt atomically before any camera or vision work | 8c | TCO4, TCO6, TCO8, `rate_limit_reservation_tests` |
| 4 | A foreign template answers the unchanged `Unavailable` / `ModelUnavailable` and never reaches the wake, a capture or inference | 8b | TCO1, TCO2, `sface_template_binding_tests` |
| 5 | One template read per request; the matched template is the classified snapshot | 8a–8e | code review (single `get`, moved value) |
| 6 | No template, embedding, frame or nonce in logs (only model id, dimension, UID as before) | 8b, 8d | unchanged log fields; `logging_audit_test`, `request_id_logging_tests` |
| 7 | Fail closed: no new path to `Allow` | step 8 | all daemon tests green |

Pre-existing violations found: none. Clearance: CLEARED.

## 6. Implementation

- `crates/daemon/src/dispatcher.rs`: step 8 reordered as in section 2. The template lookup result
  is stored in `template_lookup`; the foreign check matches `(expected_model, Ok(Some(t)))`; the
  reservation block is unchanged; the `Ok(None)` / `Err` arms moved after it unchanged; the wake
  code moved after them unchanged. The 8f comment now points to the reservation as 8c.
- `crates/daemon/tests/template_check_order_tests.rs`: new contract tests (section 4).
- Docs: `Docs/DAEMON.md` §3 (Auth check order), `Docs/POLICY_CRATE.md` (daemon integration),
  `AI/DECISIONS.md` (new ADR), `AI/VERIFICATION_MATRIX.md` (TCO1–TCO8).

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) run on the branch (section 8). The layer 2 sub-agent review
is run by the orchestrator before the release, not on this branch.

## 8. Verification Results

Commands run with `CARGO_BUILD_JOBS=6` and `SOOS_MODELS_DIR` pointing at an SFace + SCRFD +
MiniFASNet deployment. Results: tests 794 passed, 0 failed, 2 ignored (pre-existing) across
`soos-daemon`, `soos-policy` and `soos-invariants`, then `soos-invariants` again after the
documentation edits (344 passed); fmt clean; clippy clean; candid layer 1 PASSED.

```bash
cargo test --locked --all-features -p soos-daemon -p soos-policy -p soos-invariants
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -p soos-daemon -p soos-invariants -- -D warnings
./scripts/candid_review.sh
```

## 9. Known Limitations / Follow-ups

- The other three items of GitHub #298 (GUI stale score, NCHW-only `with_spec`, the 512-D example
  in `Docs/BIOMETRIC_STORE_CRATE.md`) are handled by other batches.
- The template read stays a synchronous call inside the async handler, as before (size-bounded,
  lock-free); moving store reads to the blocking pool is out of scope.
