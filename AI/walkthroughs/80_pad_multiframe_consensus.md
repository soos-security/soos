# Walkthrough 80 — Multi-Frame PAD Consensus and Spoof Veto

- **Date**: 2026-09-29
- **Issue**: Review finding PAD-02 (GitHub #147) — **Branch**: `fix/pad-multiframe-aggregation`
- **Matrix criteria**: PMC1, PMC2, PMC3, PMC4, PMC5

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, finding PAD-02, severity
CRITICAL, verifier CONFIRMED) showed that the dispatcher's multi-frame loop introduced by Issue #48.2
was "first passing frame wins": every fresh camera capture was evaluated independently, a
`PadFailed` capture only updated the last context and the loop continued, and the first capture that
satisfied `AuthorizationEngine::evaluate` returned `Allow` immediately. With a budget of up to
`connection_timeout - 50 ms` (2450 ms for GDM) one request therefore gave an attacker roughly 5–15
independent PAD trials, so the per-request attack presentation acceptance rate was
`1 - (1 - p)^N` for a per-capture false-live probability `p`. The rate limiter counted requests, not
captures, and did not bound this.

Re-verified in code before the change: `crates/daemon/src/dispatcher.rs` step 8e (`while
auth_start.elapsed() < total_budget`), per-capture `engine.evaluate(&ctx)` and `if verdict ==
Verdict::Allow { ... return }`, `PadFailed` arm reduced to `(0.0, 1u8, false)` with no counter,
consensus or veto.

Objectives (task scope: daemon/policy):

1. Replace the rule with zero-I/O aggregation: `Allow` only when `k` consecutive evaluated captures
   are live with PAD score `>= pad_threshold` and match `>= match_threshold` (`k = 3` within a
   window `n = 5`).
2. Any spoof-classified capture in the request vetoes `Allow` (fail closed).
3. Keep latency within `DECISION_BUDGET_MS`.
4. Tests: alternating spoof/live mock never allows; `k` consecutive live captures allow; property
   test that any spoof capture prevents `Allow`.
5. Do not change the MiniFASNet class index (separate issue #146).

## 2. Architect Design

### Scope & blast radius
- `crates/policy/src/pad_consensus.rs` (new), `crates/policy/src/lib.rs` (re-exports),
  `crates/policy/src/error.rs` (`PolicyError::InvalidConsensus`),
  `crates/policy/src/decision.rs` (`AuthorizationEngine::record_attempt`).
- `crates/daemon/src/dispatcher.rs` step 8e/8f, `crates/daemon/src/pipeline.rs`
  (`FRAME_POLL_INTERVAL_MS`).
- `crates/inference-ort/src/mock.rs`: `MockPadDetector::set_result_sequence` (per-call round-robin
  results) and `call_count`, needed to simulate flickering PAD output deterministically.
- `AuthContext` and its `new` signature are unchanged; no consumer outside daemon/policy changed.

### Types and constants (single source of truth in `soos-policy`)

| Item | Value / semantics |
|---|---|
| `DEFAULT_PAD_CONSENSUS_REQUIRED` | 3 consecutive passing captures (`k`) |
| `DEFAULT_PAD_CONSENSUS_WINDOW` | 5 retained classifications (`n`) |
| `MAX_PAD_CONSENSUS_WINDOW` | 32, memory bound of the ring buffer |
| `PadConsensusConfig::new(window, required)` | rejects `required == 0`, `required > window`, `window > MAX` |
| `FrameEvaluation { face_count, pad_live, pad_score, match_score }` | one evaluated capture |
| `FrameClass` | `Passing`, `Spoof`, `NoFace`, `MultipleFaces`, `NoMatch` |
| `ConsensusDecision` | `Allow`, `SpoofVetoed`, `Pending(Option<FrameClass>)` with `verdict()` |
| `PadAggregator` | bounded `VecDeque<FrameClass>`, sticky `spoof_seen`, saturating counters; no reset method |
| `FRAME_POLL_INTERVAL_MS` (daemon) | 10 ms poll between capture checks, never sleeping past the budget |

Classification order: `face_count == 0` → `NoFace`; `> 1` → `MultipleFaces`;
`!pad_live || !pad_score.is_finite() || pad_score < pad_threshold` → `Spoof`;
`!match_score.is_finite() || match_score < match_threshold` → `NoMatch`; otherwise `Passing`.

Decision: spoof ever seen → `(Deny, PadFailed)`; trailing run of `Passing` `>= k` → `(Allow,
FaceMatch)`; nothing evaluated or run shorter than `k` → `(Unavailable, Timeout)`; otherwise the
last class's `Deny` reason. `Deny` and `Unavailable` remain indistinguishable to the PAM caller.

### Latency budget
`k = 3` distinct captures at 30 fps are available about 67 ms after the first fresh capture; with
three inference passes (each within the 150 ms p95 target of `AI/ARCHITECTURE.md` §7) consensus is
reached in roughly 170–450 ms, inside `DECISION_BUDGET_MS = 900` (sudo, PAM `timeout_ms=1000`) and
far inside the 2450 ms GDM budget. The loop bound, the `is_new` sequence check and the
`MAX_FRAME_AGE_NS` freshness check are unchanged; a veto exits the loop immediately.

### Invariants touched
Fail-closed decision (ARCHITECTURE §2, §3 Request State Matrix), zero-I/O policy crate, rate limiting
(PO-series), PAD7 (single spoof capture still `Deny`/`PadFailed`), Issue #48.2 multi-frame recovery.

### Documentation drift / ADR
New ADR line in `AI/DECISIONS.md` ("Multi-Frame PAD Consensus"). `AI/ARCHITECTURE.md` §3 states
that the `Allow` row is never satisfied by a single capture. `AI/BACKLOG.md` #48.2 still describes
the original "break immediately on Allow" loop; it is historical and superseded by this walkthrough.

## 3. Plan Evaluation

Performed inline (review-issue workflow, no `AI/plan_evaluator_report.md`). Findings:
- Placing the aggregator in `soos-policy` keeps it zero-I/O and testable with `proptest`; the daemon
  only maps `VisionError` variants to `FrameEvaluation`.
- The policy `pad_threshold` and the vision `pad_threshold` are kept identical by
  `crates/daemon/src/config.rs` (both set from the same `[pipeline.thresholds]` value), so the
  policy-side score gate cannot contradict the vision-side gate.
- Recording the rate-limit attempt once after the loop (instead of inside the `Allow` branch)
  removes a pre-existing weakness where the recording result was ignored on `Allow`.
- Verdict: APPROVED.

## 4. Tester Contract

| Test (path::name) | Acceptance line / matrix ID | Red evidence |
|---|---|---|
| `crates/policy/tests/pad_consensus_tests.rs::test_pad_consensus_config_defaults_k3_within_n5` | constants k=3, n=5 / PMC1 | passed on stub (constants only) |
| `…::test_pad_consensus_config_rejects_invalid_bounds` | bounds / PMC1 | `assertion failed: matches!(PadConsensusConfig::new(5, 0), Err(...))` |
| `…::test_pad_aggregator_three_consecutive_live_frames_allow` | k consecutive live allow / PMC2 | `left: NoFace, right: Passing` |
| `…::test_pad_aggregator_fewer_than_k_live_frames_is_pending_timeout` | k-1 never allows / PMC2 | passed on stub (fail-closed guard) |
| `…::test_pad_aggregator_no_frames_is_unavailable_timeout` | empty request / PMC2 | passed on stub (fail-closed guard) |
| `…::test_pad_aggregator_first_passing_frame_does_not_allow` | PAD-02 regression guard / PMC2 | passed on stub (fail-closed guard) |
| `…::test_pad_aggregator_custom_required_count_is_honored` | configurable k / PMC2 | `left: Pending(None), right: Allow` |
| `…::test_pad_aggregator_alternating_spoof_live_never_allows` | alternating never allows / PMC3 | `left: NoFace, right: Spoof` (decision never `SpoofVetoed`) |
| `…::test_pad_aggregator_spoof_veto_is_sticky_for_the_request` | veto sticky / PMC3 | `left: NoFace, right: Spoof` |
| `…::test_pad_aggregator_spoof_after_consensus_reached_revokes_allow` | veto after Allow / PMC3 | `left: Pending(None), right: Allow` |
| `…::test_pad_aggregator_pad_score_below_threshold_is_spoof_even_if_classified_live` | score gate / PMC3 | `left: NoFace, right: Spoof` |
| `…::test_pad_aggregator_non_finite_scores_fail_closed` | NaN/inf / PMC3 | `left: NoFace, right: Spoof` |
| `…::test_pad_aggregator_no_face_frame_resets_consecutive_run` | run reset / PMC4 | `left: (Unavailable, Timeout), right: (Deny, NoFace)` |
| `…::test_pad_aggregator_multiple_faces_frame_resets_consecutive_run` | run reset / PMC4 | `left: NoFace, right: MultipleFaces` |
| `…::test_pad_aggregator_low_match_score_breaks_run_without_veto` | run reset, no veto / PMC4 | `left: NoFace, right: NoMatch` |
| `…::test_pad_aggregator_history_is_bounded_by_window` | bounded memory / PMC4 | `left: 0, right: 4` |
| `…::prop_any_spoof_frame_prevents_allow` (proptest) | any spoof prevents Allow / PMC3 | failed on stub (decision `Pending(None)` instead of `SpoofVetoed`) |
| `…::prop_allow_iff_trailing_k_frames_pass_without_spoof` (proptest) | Allow iff trailing k pass / PMC2 | passed on stub (non-authorizing stub never allows) |
| `crates/policy/tests/decision_tests.rs::test_record_attempt_enforces_rate_limit_without_evaluation` | one attempt per request / PMC5 | added with the implementation (compile-time API) |
| `crates/daemon/tests/pipeline_integration_tests.rs::test_147_alternating_live_spoof_pad_never_allows` | alternating mock never allows / PMC5 | `assertion left != right failed: alternating live/spoof frames must never authorize; left: Allow` |
| `…::test_147_spoof_frame_vetoes_subsequent_live_frames_for_whole_request` | spoof veto / PMC5 | `left: Allow, right: Deny` |
| `…::test_147_live_frame_below_pad_threshold_vetoes_request` | score gate via vision `PadFailed` / PMC5 | `left: Allow, right: Deny` |
| `…::test_147_k_consecutive_live_frames_allow_within_budget` | k consecutive allow, budget / PMC5 | `Allow requires at least k=3 evaluated frames, observed 1` |

Red runs: `cargo test --locked -p soos-policy --all-features --test pad_consensus_tests` against a
signature-only stub (13 failed on assertions, 5 fail-closed guards passed);
`cargo test --locked -p soos-daemon --all-features --test pipeline_integration_tests test_147`
against the unchanged dispatcher (4 failed, `0 passed; 4 failed`).

### Migrated existing tests
none — `test_15_*`, `test_12_5_*`, `test_12_7_*` and `test_48_*` pass unchanged under the new rule.

### Flakiness check
`pipeline_integration_tests` (13 tests, including the four `test_147_*`) run 10 times in a row:
10/10 `ok`, 0.65–0.71 s each. Two isolated early-EOF failures observed earlier occurred while the
host load average was ~25 on 16 cores (other sessions compiling); the dispatcher poll is now
deadline-aware so the last iteration can no longer sleep past the budget.

## 5. Auditor Constraints

| # | Constraint | How it was met |
|---|---|---|
| 1 | `soos-policy` keeps `#![forbid(unsafe_code)]`, zero I/O, no new dependencies | `pad_consensus.rs` uses only `std::collections::VecDeque`; candid audit 1 OK; `proptest` was already a dev-dependency |
| 2 | No `unwrap`/`expect`/indexing/panicking arithmetic in production paths | ring buffer via `pop_front`/`push_back`, `saturating_add` counters, `NonZeroUsize` guard for the mock modulo; workspace clippy `-D warnings` green |
| 3 | Non-finite PAD or match scores never classify as `Passing` | `classify()` checks `is_finite()` before every comparison; `test_pad_aggregator_non_finite_scores_fail_closed` |
| 4 | Spoof veto is request-scoped and sticky; no reset API | `PadAggregator` exposes no reset; one aggregator is created per request in the dispatcher; `test_pad_aggregator_spoof_veto_is_sticky_for_the_request` |
| 5 | `Allow` requires `k` distinct fresh captures | `is_new` (sequence) and `MAX_FRAME_AGE_NS` checks kept; `test_147_k_consecutive_live_frames_allow_within_budget` asserts `>= k` PAD evaluations |
| 6 | Rate limiter recorded exactly once per request; a rejection downgrades `Allow` | step 8f `record_attempt` after the loop; `ConsensusDecision::Allow if rate_limited => (ProtocolError, RateLimited)`; `test_record_attempt_enforces_rate_limit_without_evaluation`, `test_12_5_*` |
| 7 | No new sensitive data in logs | new log lines use `captures_evaluated` / `consecutive_live` (no scores, no frames); `logging_audit_test` green |
| 8 | Loop bounded by `total_budget`; veto short-circuits | `while auth_start.elapsed() < total_budget`, deadline-aware sleep `min(FRAME_POLL_INTERVAL_MS, remaining)`, `break` on `Allow`/`SpoofVetoed` |
| 9 | Mock extension must not alter existing mock semantics | `set_result` clears any sequence; `new_live`/`new_spoof` delegate to `new_with_result`; all existing `pad_tests` green |
| 10 | Class index untouched (#146) | `crates/inference-ort/src/pad.rs` and the `new_with_class_index(..., 2)` call in `pipeline.rs` unchanged |

Pre-existing violations found (not introduced by this change): none in the touched files.
Clearance: CLEARED.

## 6. Implementation

Files changed:
- `crates/policy/src/pad_consensus.rs` (new): constants, `PadConsensusConfig`, `FrameEvaluation`,
  `FrameClass`, `ConsensusDecision::verdict`, `PadAggregator`.
- `crates/policy/src/lib.rs`, `crates/policy/src/error.rs`, `crates/policy/src/decision.rs`
  (`record_attempt`).
- `crates/daemon/src/dispatcher.rs`: step 8e builds one `FrameEvaluation` per fresh capture, records
  it, breaks on `Allow`/`SpoofVetoed`, polls with a deadline-aware sleep; step 8f records one
  rate-limit attempt and renders the aggregate verdict (stale capture → `Unavailable`/`StaleFrame`).
  The `AuthContext`-based per-capture `evaluate` call and the early `return` inside the loop were
  removed; there is now a single exit path after the loop.
- `crates/daemon/src/pipeline.rs`: `FRAME_POLL_INTERVAL_MS = 10`.
- `crates/inference-ort/src/mock.rs`: `MockPadDetector::set_result_sequence`, `call_count`.
- Tests: `crates/policy/tests/pad_consensus_tests.rs` (new), `crates/policy/tests/decision_tests.rs`,
  `crates/daemon/tests/pipeline_integration_tests.rs`.
- Docs: `Docs/POLICY_CRATE.md` (new section 5), `AI/ARCHITECTURE.md` §3, `AI/DECISIONS.md`,
  `AI/VERIFICATION_MATRIX.md` (PMC1–PMC5), `.agents/skills/dev-workflow/references/project-facts.md`.

Notable decisions:
- A run shorter than `k` at budget expiry maps to `Unavailable`/`Timeout` rather than `Deny`: the
  biometric did not fail, the daemon ran out of evidence; both are `PAM_IGNORE`.
- The poll interval dropped from 25 ms to 10 ms so every 30 fps capture is picked up promptly; the
  Issue #48.2 text ("25–30 ms") is superseded.
- `PadAggregator` exposes `history()` and counters for tests and observability only; the dispatcher
  never logs scores from it.

## 7. Candid Review

pending (layer 1 `./scripts/candid_review.sh`: PASSED; layer 2 fingerprint-bound review to be run by
the release workflow before push).

## 8. Verification Results

```bash
cargo fmt --all                                                                 # OK
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings   # Finished, 0 warnings
cargo test   --locked --workspace --all-targets --all-features                  # 104 test binaries ok, 502 passed, 0 failed
./scripts/candid_review.sh                                                      # Candid Review PASSED (audits 1–7)
for i in $(seq 10); do cargo test --locked -p soos-daemon --all-features --test pipeline_integration_tests -q; done   # 10/10 ok
```

`./run_tests.sh` (Docker PAM matrix) was not run: it exercises `pam_soos.so` against
`tests/docker/mock_daemon.py`, and neither the PAM crate nor the mock daemon changed.

## 9. Known Limitations / Follow-ups

- `k` and `n` are compile-time constants; a `daemon.toml` knob was deliberately not added (the task
  asked for constants). If field data shows too many `Unavailable`/`Timeout` outcomes on slow CPUs,
  expose `PadConsensusConfig` through `[pipeline]` with the same validation.
- The veto is per request; an attacker can still open a new request after a veto, bounded by the
  per-UID rate limiter (5 attempts / 60 s by default).
- `crates/daemon/src/pipeline.rs` still instantiates `OrtPadDetector::new_with_class_index(..., 2)`
  while `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` is 1 — tracked separately as GitHub #146, untouched here.
- Backlog item #48.2 describes the superseded single-capture rule; update its prose when the backlog
  is next revised.
