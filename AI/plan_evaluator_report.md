# Plan Evaluation Report
- **Date**: 2026-10-07
- **Issue**: GitHub #345 — Live camera view in `soos-remote` through the daemon preview channel (GitHub-only issue, no `AI/BACKLOG.md` entry, like #339)
- **Branch**: `feat/remote-live-camera`
- **Base commit**: `05001c9` (`origin/main`)
- **Spec evaluated**: `AI/architect_spec_remote_live_camera.md`, **round 2** (§19 maps the round-1 findings F1–F12), against the ADR "[2026-10-07] Live Camera View in `soos-remote` Through the Daemon Preview Channel" as amended (D-1, D-2), owner decisions LC-1..LC-4, `AI/research_live_camera.md` and the current code.
- **Previous round**: round 1, REVISION_REQUIRED (F1–F3 MAJOR; F5, F6, F8–F12 and one wording fix MINOR).

## 1. Coverage Matrix

| Acceptance line (ADR item / owner decision / orchestrator default) | Spec element (round 2) | Status |
|---|---|---|
| (1) Single camera owner; remote consumes only `PreviewFrame` | §1.1, §5, RLC-S1, tests 50, 59 | Covered |
| (1) Face unlock keeps working while a view is open | §11, R-3, test 63 `test_rlc_view_connection_leaves_room_for_auth`, RLC16 | Covered (round-1 F11 resolved) |
| (2)/LC-1 Only `soos-protocol`; RMC-S4 migrated | §1.1, §12, tests 50, 60 | Covered |
| (2) ≤ 1 daemon connection, reconnect before lifetime, back off on `RateLimited` | §5.2 steps 1, 3a, 6; tests 20 (F5 extension), 33, 62 | Covered |
| (3)/LC-2 `jpeg-encoder`, crate-scoped IJG, no decoder, Grey/YUYV/RGB24 | §6, §16 D-6, tests 1–9, 50, 51, 57, 59 | Covered (IJG attribution added) |
| (4a)/LC-3a Local seat session `CLASS=user`, `SEAT`, `REMOTE=0`, active | S-7, §9.1 (`check_local_seat_session_of(uid).is_ok()`), RLC3, test 44 (+ `REMOTE` absent / `REMOTE=yes` fixtures) | Covered (F1 resolved) |
| (4b)/LC-3b `remote_view`, cgroup recognition fail closed, one `info` line | §4.2, §9.2, §9.3, tests 41 (+ uid-mismatch case), 42, 43, 45–47 | Covered |
| (5)/LC-4 `camera_view` / `camera_view_funnel`, tailnet-only default | §4.1, §8.4–8.6, tests 23, 26, 36 | Covered |
| (5) Fresh UV `CameraView` assertion, `X-Soos-Action: camera-view`, same origin | §8.4, §8.5, tests 24, 25, 38, 40 | Covered |
| (5) Single-use token, one slot, max duration, fps, resolution, cooldown | §7, §8.6, §8.7 tick rule, tests 10–16, 28, 29, 31 (+ no burst catch-up), 4 | Covered |
| (5) Ends on stop, close, write timeout, session expiry | §7.2 durable flag, §8.7 rules 1–3, §8.8, tests 30 (a)(b)(c), 61 | Covered (F2, F3 resolved) |
| (6) multipart JPEG, `fetch` + `ReadableStream` into a canvas, CSP unchanged | §8.6, §10.3, tests 27, 55 | Covered |
| (7) No recording, no pixel logging, soos-remote buffers zeroized | §6.4, RLC-S7, tests 52–54, 39 | Covered |
| (8) Push at start (best effort), fixed-text audit lines | §10.1, §10.2, tests 34, 35, 39 | Covered |
| (9) No view after full logout | test 44, RLC16 | Covered |
| Orchestrator defaults (120/300 s, 5/1..10 fps, 640/320, q70, one slot, cooldown, push best effort, no recording) | §3.2, §4.1, tests 29, 35, 36, 53 | Covered |
| Q-1 → S-3 runtime DOM; D-1 amended residual | S-3, §6.4, §16 | Covered |
| Contract migrations listed and justified | §12 (unchanged, grep-verified complete in round 1) | Covered |
| Docs, walkthrough 191, matrix RLC1–RLC16 after RMC88 | §14, §16 D-4..D-6 | Covered (F8 resolved) |

## 2. Facts Verified Against Code

Round-1 facts still hold. Round-2 checks:

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Root `Auth` predicate reused for preview | `crates/daemon/src/session_policy.rs:106-123` | `check_local_seat_session_of` refuses `remote != Some(false)` (absent/malformed `REMOTE` refused) | Yes |
| `REMOTE=yes` parses as `remote = None` | `session_policy.rs:87-93` | only `"0"`/`"1"` set it | Yes — test 44's malformed fixture has power |
| Daemon releases the per-UID permit when `handle_connection` returns on EOF | `crates/daemon/src/dispatcher.rs:381-470` (`UnexpectedEof` → `break` → `_permit` dropped) | yes, so `shutdown(Write)` + wait for EOF orders the release before the next connect | Yes |
| Daemon never reads a request after its lifetime/request cap or idle timeout fires | `dispatcher.rs:403-452` | caps checked before the read; idle close happens while waiting for a request | Yes — the one immediate retry cannot duplicate a processed request in the normal case |
| SSE drain pattern | `crates/remote/src/server.rs:118,2136,2231-2235` | `STREAM_SINK_BYTES = 64` (private const of `server.rs`), drain, end on `Ok(0)`/`Err` | Yes (camera module needs its own or a `pub(crate)` constant, observation O4) |
| Per-UID admission default | `crates/daemon/src/limits.rs:23` | 2, root exempt | Yes — test 63's third-connection refusal is correct |
| `jpeg-encoder` 0.7.1 | local cargo cache (per coordinator), crates.io | `(MIT OR Apache-2.0) AND IJG`, zero normal deps | Yes |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: a logind record without `REMOTE=` or with a malformed value. Round 2 refuses it (`check_local_seat_session_of`), identical to the root `Auth` path; test 44 fails against a `REMOTE != 1` implementation. Result: PASS (F1 resolved).
- Failure scenario considered: a process placed outside or inside `soos-remote.service`. Unchanged administrative semantics (R-1, RLC-S13). Result: PASS.

### Pillar 2 — PAM deadline & concurrency
- Failure scenario considered (2-connection limit): a proactive reconnect while swaylock-plugin starts a face request. Round 2 half-closes, waits ≤ 200 ms (`CAMERA_DAEMON_CLOSE_WAIT_MS` < connect timeout, compile-asserted) for the daemon's EOF, then connects; the daemon drops the permit before closing its side, so in the normal case at most one permit is held at the next connect; the residual (daemon slower than 200 ms) is fail-safe and documented in R-3. Test 20 pins the ordering; test 63 pins that a held view connection leaves room for a second connection. Result: PASS (F5 resolved).
- Failure scenario considered (single immediate retry): the daemon idle-closes a reused connection; the client's write gets `EPIPE` or the read gets EOF before any reply byte → one retry on a fresh connection with a new nonce, no back-off; a fresh-connection failure or a second failure is `Io` (bounded, test 62). A reply to the first request can never be read on the retry (different connection, new nonce, refusals checked with `matches_request`); a duplicate daemon-side processing would only cost one preview rate-limit token of a read-only request. Worst-case duration of one `next_frame` becomes ≈ IO timeout + connect + IO timeout (≈ 6.5 s) when the EOF arrives late, above the `CAMERA_DAEMON_IO_TIMEOUT_MS < CAMERA_FIRST_FRAME_TIMEOUT_MS` intuition, but still bounded because the first-frame and stall timers are terminal arms that drop the step. Result: PASS with observation O2.
- PAM code and the `Auth` path untouched. Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered (F2): a Funnel session-check tick or stray read-half bytes while an exchange is in flight or a part write is blocked. Round 2 pins the frame step outside `select!`, polls it by `&mut`, recreates it only after completion, writes the part in the arm body (no other arm runs during it), and drains the read half into a 64-byte sink. Only terminal arms drop the step, and the stream ends right after. Test 61 asserts aligned parts and zero cancelled requests. Result: PASS (F2 resolved); implementation note O1.
- Failure scenario considered (F3): stop issued while a part write is blocked, or during the first-frame phase. Round 2 combines the durable slot flag (never cleared until `end`; `stop_requested` also true once the slot no longer holds the view) with a `watch` epoch (versions are never lost) and a flag check at the top of every iteration and after every arm body. Test 30(a) fails against a `notify_waiters`-only implementation. Result: PASS (F3 resolved); observation O3.
- `JoinError` → `Internal`, `ViewGuard` drop on every path, poisoned mutex fail closed. Result: PASS.

### Pillar 4 — Dependencies
- `jpeg-encoder =0.7.1` (cached), crate-scoped IJG exception (cargo-deny 0.20.2 syntax), attribution sentence pinned by test 57, licence policy line of the guidelines updated (D-5). Result: PASS.

### Pillar 5 — Data confidentiality
- The retry adds no new buffer: the dropped connection's partial reply buffer is the `Zeroizing` exchange buffer. The close-wait sink holds no pixel data (it is drained after the last full reply). Token, audit and push unchanged from round 1. Result: PASS.

### Pillar 6 — Test integrity
- New tests 61–63 and extended tests 4, 5, 20, 30, 31, 41, 44, 57 each fail against the plausible wrong implementation named in round 1 (`REMOTE != 1`, `notify_waiters`, frame write inside the polled future, burst catch-up, missing uid-mismatch rule, missing retry). §12 migrations unchanged and complete. Result: PASS.

## 4. Findings

Every round-1 required revision is resolved in the spec text, not only claimed in §19 (verified in S-7, §5.2 steps 1/3a, §6.2–6.3, §7.2, §8.5 step 5, §8.6 (step 5 and "Permits held by a stream"), §8.7 rules 1–3 and the tick rule, §8.8, §9.1, §14, §16 D-2/D-5/D-6, R-3, and tests 4, 5, 20, 30, 31, 41, 44, 57, 61–63). No `REMOTE != 1`, dangling "§7.4" or "body present" text remains.

No CRITICAL or MAJOR finding. Non-blocking observations for the tester and the developer (no spec round needed):

- **[MINOR] O1 — Borrowing in the pinned frame step.** `frame_step.set(make_step(&mut source, ..))` does not compile when the step future borrows `source`: the replacement is built while the old future still holds the `&mut` borrow. The step future must **own** the `Box<dyn PreviewSource>` and hand it back with its outcome (`async move { let r = source.next_frame().await; …; (source, outcome) }`), or an equivalent `Option`-based state machine. The semantics of §8.7 rule 1 are unchanged.
- **[MINOR] O2 — Retry bound.** Make the immediately retried exchange use the **remaining** `CAMERA_DAEMON_IO_TIMEOUT_MS` budget of the original attempt (or document the ≈ IO + connect + IO worst case next to the `CAMERA_DAEMON_IO_TIMEOUT_MS < CAMERA_FIRST_FRAME_TIMEOUT_MS` assertion). Only EOF/`EPIPE`/reset before the first reply byte may trigger it, never a timeout, as §5.2 step 3a already says.
- **[MINOR] O3 — `watch` receiver error.** Treat `stop_rx.changed()` returning `Err` (sender dropped) as terminal (`Shutdown`); otherwise a biased `select!` would spin on an always-ready arm. Unreachable while the view task holds the `CameraRuntime` `Arc`, but cheap to make explicit.
- **[MINOR] O4 — Editorial.** In the §8.7 arm table the read-half row says "terminal" with "loop continues" for bytes; rule 2 is the normative text (ends only on `Ok(0)`/`Err`). `STREAM_SINK_BYTES` is a private constant of `server.rs`: either make it `pub(crate)` or give `camera.rs` its own 64-byte constant.

## 5. Verdict
VALIDATION_VERDICT: APPROVED
