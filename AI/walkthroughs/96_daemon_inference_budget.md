# Walkthrough 96 — Daemon Inference Off the Async Runtime, Within One Request Deadline

- **Date**: 2026-09-30
- **Issues**: Review findings DMN-05 (GitHub #158) and DMN-06 (GitHub #159) —
  **Branch**: `fix/daemon-inference-budget`
- **Matrix criteria**: DIB1–DIB6 (new)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`) confirmed two defects in
the daemon authentication path:

1. **DMN-05** — `handle_request` called the synchronous `VisionPipeline::process_frame`
   (SCRFD + MiniFASNet + ArcFace on CPU) inline on a Tokio worker. Two concurrent Auth requests
   on a 2–4 vCPU machine stalled the accept loop, Status requests, timers and the preview stream,
   and the outer `timeout` could not cancel a worker blocked in inference. `AI/ARCHITECTURE.md`
   already described "dedicated worker threads with semaphore limit 1", which did not exist.
2. **DMN-06** — the consensus loop measured its budget from `auth_start` (after reads, peer checks
   and camera wake) instead of the start of the `connection_timeout` window, only checked the
   deadline *before* an iteration, and reserved no time for writing the response. An iteration
   started a few milliseconds before the deadline ran a full inference and overran it, so PAM
   read a late response (read timeout → `PAM_IGNORE`) and the lock-screen feedback was lost.

## 2. Architect Design

New module `crates/daemon/src/inference.rs` (no new dependency, `spawn_blocking` is part of the
existing Tokio `rt` feature):

| Item | Role |
|---|---|
| `MAX_CONCURRENT_INFERENCES = 1` | Inference slots; the ORT sessions are behind a mutex, so a second concurrent inference would only block another thread |
| `RESPONSE_WRITE_MARGIN_MS = 50` | Reserved before the client and outer deadlines for the write and the PAM read |
| `DEFAULT_INFERENCE_ESTIMATE_MS = 80`, `MAX_INFERENCE_ESTIMATE_MS = 1000` | Initial and upper bound of the admission estimate |
| `RequestDeadline` | `compute(now_ns, client_deadline_ns, request_started, connection_timeout)`; `remaining_at` / `can_start_at` / `is_expired_at` take explicit instants for deterministic tests |
| `InferenceEstimator` | Bounded EMA (weight 1/4, rounded up so it converges to the clamp) of measured job durations |
| `InferenceGate` | Semaphore + estimator; `acquire_within(max_wait)` and `run(permit, job)` (`spawn_blocking`, permit moved into the job) |
| `InferenceJobError` | `Panicked` / `Cancelled`, no payload (nothing from the vision stage is logged) |

`ConnectionDispatcher` owns an `InferenceGate` (`InferenceGate::default()`), overridable with
`with_inference_gate` and observable with `inference_gate()`. `PipelineComponents` and
`DispatcherConfig` are unchanged, so every existing fixture compiles untouched.

Dispatcher flow (Step 8):
1. `read_and_process` records `request_started` before any socket read; `handle_request` receives it.
2. After the Step 7 deadline check, one `RequestDeadline` is computed and threaded through the
   camera wake wait (`max_wake` is also bounded by the remaining budget) and the consensus loop.
3. For every new, fresh capture: `can_start(now, estimate)` or finalize; wait for the slot at most
   `remaining − estimate`, else finalize; re-check the budget and the frame age after the wait;
   run `process_frame` on the blocking pool; a `JoinError` fails closed with
   `Unavailable`/`InternalError`.
4. The loop exits on `Allow`, `SpoofVetoed` or an exhausted/insufficient budget; Step 8f still
   records exactly one rate-limit attempt and renders the aggregate verdict (`Pending(None)` →
   `Unavailable`/`Timeout`, i.e. password fallback).

## 3. Tester Contract (red first)

`crates/daemon/tests/inference_budget_tests.rs` (13 tests). Timing strategy: pure deadline and
estimator logic uses explicit `Instant` arithmetic; dispatcher tests use a `GatedPad` mock blocked
on a condition variable that only the test releases (5 s safety timeout), so assertions depend on
ordering, not machine speed. No new wall-clock sleep assertion was added.

Red evidence (API skeleton wired, dispatcher still inline — `cargo test -p soos-daemon --test
inference_budget_tests`: 7 passed, 6 failed):
- `test_158_status_request_served_while_inference_is_blocked`: "Status was only served after the
  blocked inference gave up: inference blocks the runtime".
- `test_158_second_auth_never_queues_behind_busy_inference_gate`: concurrent inferences `2 != 1`.
- `test_158_panicking_inference_returns_internal_error_not_eof`: EOF instead of a response.
- `test_159_client_deadline_shorter_than_estimate_skips_inference`: PAD calls `2 != 0`.
- `test_159_estimate_exceeding_budget_times_out_and_records_attempt`: `Allow != Unavailable`.
- `test_159_estimator_tracks_measured_latency_with_bounded_ema`: `999.997ms != 1s` (floor-rounded
  EMA stalled below the clamp; fixed by rounding up in the implementation).

Before the skeleton existed the file failed to compile on the specified API
(`soos_daemon::inference`, `with_inference_gate`, `inference_gate`).

## 4. Auditor Constraints

1. No `unwrap`/`expect`/`panic` in production code; clock failures after admission map to
   `u64::MAX` (budget exhausted → finalize), never to `Allow`.
2. The permit is moved into the blocking job: at most `MAX_CONCURRENT_INFERENCES` jobs exist, and a
   request cancelled by the outer timeout leaves only the job it had already started (which was
   admitted because it fit in the budget).
3. Waiting for a slot is bounded by `remaining − estimate`; a request never queues work it cannot
   finish.
4. A frame that aged past `MAX_FRAME_AGE_NS` while waiting for the slot is never evaluated.
5. The enrolled template and cosine similarity stay on the async side; only the `Arc<Frame>` and
   `Arc<VisionPipeline>` move into the job. Job errors carry no payload; nothing biometric is logged.
6. Exactly one `record_attempt` per request on every loop exit (unchanged early returns for
   inference/internal errors, as before).

## 5. Developer Implementation

- `crates/daemon/src/inference.rs` (new), `crates/daemon/src/lib.rs` (`pub mod inference`).
- `crates/daemon/src/dispatcher.rs`: `request_started`, single `RequestDeadline`, admission and
  blocking-pool execution in step 8e, `is_frame_fresh` helper; the `8d` budget block and the
  `auth_start` wall-clock loop condition were removed.

## 6. Verification

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`: green.
- `./scripts/candid_review.sh`: PASSED.

Load note: the debug-build mock pipeline takes 20–110 ms per inference on this machine. In one
full-suite run while the host was at load average ~44 on 16 cores (parallel agents compiling), four
existing Allow-path tests in `pipeline_integration_tests` (500 ms `connection_timeout`) received the
new fail-closed `Unavailable`/`Timeout` instead of an overrunning `Allow`, together with the known
load-sensitive `soos-vision` latency and `soos-gui` worker tests. Standalone (6 consecutive runs)
and in a full rerun they are green. Under starvation, a timely password fallback is the intended
behavior of #159; the tests were not modified.

## 7. Drift and Follow-ups

- `DECISION_BUDGET_MS` (900 ms) is documented as "within the 1000 ms PAM deadline" while every
  packaged PAM stack passes `timeout_ms=250`; with the 50 ms write margin a packaged request leaves
  ~200 ms, i.e. about two inferences at the default estimate, fewer than the 3 captures the PAD
  consensus needs on a slow CPU. This deployment drift (noted by the #159 verifier) is out of scope
  here and should be settled by an ADR on the packaged `timeout_ms`.
- The per-inference estimate is shared by all requests of one dispatcher; a per-sensor estimate
  could be considered if IR and RGB pipelines diverge in latency.
