# Candid Review Report

- **Date**: 2026-10-07
- **Target Branch**: `feat/remote-live-camera` (GitHub #345, PR #347), owner request "full screen, rotation, no cooldown", round 1
- **Base (merge-base)**: `fee8480` (review limited to the uncommitted diff, `CANDID_BASE_REF=HEAD`)
- **Reviewed-Diff-Fingerprint**: `a76be1ee29a222b834cd66e47bca79be07211b43c7cabcd3bdad4a7e3454702f`
- **Audited Files**: `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/tester_contract_remote_live_camera.md`, `AI/walkthroughs/191_remote_live_camera.md`,
  `Docs/REMOTE_COMPANION.md`, `crates/remote/assets/app.js`, `crates/remote/assets/style.css`,
  `crates/remote/src/camera.rs`, `crates/remote/src/camera_slot.rs`, `crates/remote/src/lib.rs`,
  `crates/remote/src/server.rs`, `crates/remote/tests/camera_config_tests.rs`,
  `crates/remote/tests/camera_server_tests.rs`, `crates/remote/tests/camera_slot_tests.rs`,
  `crates/remote/tests/common/camera.rs`, `tests/invariants/src/remote_camera_contract.rs`

## 1. Executive Summary

The diff implements the three owner-approved items: (A) a tap-to-toggle full-screen stage
(element fullscreen when the API exists, a fixed `inset: 0` overlay otherwise, exit by tap, ✕,
Escape, `fullscreenchange`/`webkitfullscreenchange`, and unconditionally in `endCameraView`);
(B) a Rotate button cycling 0/90/180/270 degrees through `classList` only, with the canvas box
swapped to container height x width for 90/270; (C) full removal of the cooldown
(`CAMERA_VIEW_COOLDOWN_MS`, `SlotState::Idle { cooldown_until }`, `SlotPhase::Cooldown`,
`SlotError::Cooldown`, the `429 camera_cooldown` answer, `cooldown_ms` in `GET /api/camera`, and
the page countdown). Face ID per view, the single global slot, the token TTL and the shared
lockout are untouched. Locally verified: `cargo fmt --all -- --check` clean, `cargo clippy -p
soos-remote -p soos-invariants --all-targets -- -D warnings` clean, `cargo test -p soos-remote`
all green, `cargo test -p soos-invariants` 531 passed. No CRITICAL or MAJOR finding; three MINOR
items below.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Test files touched: `camera_config_tests.rs`, `camera_server_tests.rs`, `camera_slot_tests.rs`,
`common/camera.rs`, `tests/invariants/src/remote_camera_contract.rs`. No `#[ignore]`,
`should_panic`, tolerance/epsilon or inline `mod tests` changes. 27 removed lines match the
assertion pattern; every one maps to migration M13 in `AI/tester_contract_remote_live_camera.md`
(owner approval 2026-10-07):

- `camera_slot_tests.rs` test 10 (M13a): `(Cooldown, Some(10_000))` and a reserve after the
  cooldown → `Idle`, `check == Ok`, reserve + begin at the same instant with a new view id.
- test 14 renamed `test_rlc_slot_no_cooldown_after_any_view` (M13b): the countdown asserts are
  replaced by immediate-reserve asserts after a shown, a stopped and an unshown view, plus
  exhaustive matches on `SlotPhase`/`SlotError` that stop compiling if a cooldown variant
  returns. The stale-id `end` no-op and double-`end` no-op checks are kept (the pending phase is
  now asserted explicitly, which is stronger).
- test 15 (M13c): the Streaming sub-case starts at `t0` instead of `t0 + COOLDOWN`, proving the
  immediate restart.
- test 12 and others (M13d): `phase()` / `end()` signature changes only; same assertions.
- `camera_config_tests.rs` (M13e): constant assertion removed; the absence is pinned by
  invariant test 60.
- `camera_server_tests.rs` `assert_ended_shown` (M13f), test 23 key list (M13g), test 28 (M13h),
  test 29 renamed `test_rlc_one_view_and_no_cooldown` (M13i: the `429` assert becomes a full
  second shown view right after the stop, then another reservation), test 30 (M13j: the stop and
  `max_view_s` sub-cases gain `assert_immediate_restart`). The `409 view_in_progress` and
  "failed first frame leaves the slot idle" checks are kept.
- `common/camera.rs` (M13k): comment only.

No assertion unrelated to the cooldown was removed or loosened. Each converted assertion fails
against a plausible wrong implementation (any cooldown left on `end` breaks tests 10/14/29/30; a
reintroduced variant breaks the exhaustive match or test 60).

## 3. Deep Reasoning Audit

### Logic & Architecture
- Every view end path leaves full screen: `stopCameraView` (Stop button, `visibilitychange`
  hidden, `pagehide`, sign-out via the login path) and the stream `then`/`catch` (max duration
  trailer, stall, error) all go through `endCameraView`, which calls `exitCameraFullscreen()` and
  hides the stage before clearing the canvas. `cameraController` is only reset in
  `endCameraView`, and `enterCameraFullscreen` refuses without a controller, so full screen cannot
  outlive a view through a missed path. → PASS.
- iOS-safe APIs: `requestFullscreen || webkitRequestFullscreen` and `exitFullscreen ||
  webkitExitFullscreen` are `typeof === "function"` guarded, called with `.call`, wrapped in
  try/catch with a guarded `.catch` on a returned promise; `fullscreenElement()` reads both the
  prefixed and unprefixed properties with a `null` default. iPhone Safari (no element fullscreen)
  falls back to the overlay. The request runs synchronously inside the click/keydown handler, so
  user activation holds. → PASS.
- A tap on Rotate/✕ does not toggle full screen (the listener is on the canvas; the buttons are
  siblings). → PASS.
- `[hidden] { display: none !important; }` outranks `.camera-stage { display: flex }`, so the
  hidden stage is really hidden. → PASS.
- Rotation: 90/270 set the canvas to `100cqh x 100cqw` inside a `container-type: size` stage
  before `rotate()`; with `flex: none` and centered alignment the turned box exactly fits the
  content box (cq units resolve against the content box, so the overlay's safe-area padding is
  respected). The `width/height: 100%` fallback precedes the cq values. Rotation is kept in
  memory for the page lifetime, documented in the app.js header, Docs §2f and walkthrough §11.
  → PASS.
- Cooldown removal: no dead code left (`remaining_ms` helper, `Instant` and `shown` parameters of
  `end` removed; `shown` remains used for the `ended` audit; `phase(now)` still needs `now` for
  pending expiry). `GET /api/camera` loses only `cooldown_ms`. Docs §2f, API table,
  troubleshooting, hardware checklist, ADR item (5) amendment note, ARCHITECTURE §13 and matrix
  RLC9/RLC16 are updated. → PASS (see MINOR 2 for historical documents).
- Scenario: Stop then immediate Start. The page aborts the stream and refetches state; the server
  frees the slot when the guard drops. Start requires `confirm()` + Face ID (seconds), so the slot
  is free; no `429 camera_cooldown` exists anywhere. → PASS (see MINOR 3 for a stale label).
- Scenario: element fullscreen request still pending when the view ends. → FINDING MINOR 1.

### PAM Concurrency & Deadlines
- `crates/pam` untouched. → N/A, PASS.

### Panic Safety & Fail-Closed
- No new `unwrap`/`expect`/indexing in production Rust; the removed `checked_add(..).unwrap_or`
  path is gone entirely. Removing the cooldown opens no authorization path: start still goes
  through the gate order, a fresh UV assertion of purpose `CameraView`, the single-use token, the
  single slot (`409`) and the shared lockout. The push alert stays one per started view; view
  frequency is now bounded by the human Face ID rate and the existing start rate limit instead of
  10 s spacing. → PASS.

### Test Integrity & Anti-Weakening
- See section 2: every removed assertion is an M13 entry replaced by a positive no-cooldown
  assertion. Invariant test 60 pins full screen, the rotation classes, classList-only styling
  (`.style.`, `cssText`, `"style"` forbidden), the exit in `endCameraView`, and the absence of
  `cooldown`/`retry_after_ms`/`CAMERA_VIEW_COOLDOWN_MS` in the page, camera sources and server.
  → PASS.

### Memory, Bounds & Secrets
- No new buffers, no storage API, no `blob:` or object URL, no `innerHTML`; text set through
  `setText`, elements via `createElement`; no new ids (`index.html` not in the diff); CSP untouched
  (no inline style attribute); all existing remote page invariants pass. Nothing logged. → PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. → N/A, PASS.

### English-Only Policy
- Code, comments, CSS, docs, contract and walkthrough are English. The ✕ glyph carries an English
  `aria-label`. → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/remote/assets/app.js:1276` (`cameraFullscreenChanged`) — if the view ends
  between `requestFullscreen()` and its asynchronous completion (desktop/iPad only),
  `exitCameraFullscreen` sees `fullscreenElement() !== stage` and skips `exitFullscreen`; the
  request then completes on a now-hidden stage and `cameraFullscreenChanged` ignores it because
  `cameraFull` is false, leaving the browser in element fullscreen of an invisible element until
  the user presses Escape. Suggested: in `cameraFullscreenChanged`, also run the guarded exit when
  `!cameraFull && fullscreenElement() === stage`. Narrow race, never on iPhone.
- **[MINOR]** `AI/architect_spec_remote_live_camera.md`, `AI/auditor_constraints_remote_live_camera.md`
  — the historical spec and auditor documents still describe the 10 s cooldown (spec §7.1/§7.2,
  the `GET /api/camera` example with `cooldown_ms`, "views are ≥ `CAMERA_VIEW_COOLDOWN_MS`
  apart"). Suggested: a one-line "superseded by M13 (2026-10-07)" note at the top of each, so a
  future agent does not reintroduce the cooldown from the spec.
- **[MINOR]** `crates/remote/assets/app.js:1487` (`endCameraView` → `renderCamera`) — the state is
  refetched immediately after a stop; if the server has not yet dropped the view guard the answer
  is `streaming`, and the card shows "A camera view is in progress" next to an enabled Start
  button until the next refresh. Pre-existing and cosmetic, but more visible now that a restart is
  immediate. Suggested: a short delayed second `fetchCamera()` after a stop.

## 5. Final Verdict

No CRITICAL or MAJOR finding. The cooldown is fully removed with positive replacement tests
limited to migration M13, full screen exits on every view end path, the fullscreen APIs are
guarded for iOS, and the page invariants (no storage, no new ids, classList only, CSP unchanged)
hold.

**VERDICT: APPROVED**
