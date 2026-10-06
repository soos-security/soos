# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/gui-brand-redesign`
- **Base (merge-base)**: `222665f`
- **Reviewed-Diff-Fingerprint**: `46e66a4733da251986ef9ac8a9d879fd568b638de1ba3cdb3b6c46a1dc10ce24`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/183_gui_brand_redesign.md`, `Docs/CAMERA_V4L_CRATE.md`, `Docs/GUI_APPLICATION.md`, `Docs/README.md`, `crates/camera-v4l/src/mock.rs`, `crates/camera-v4l/tests/mock_warmup_activity_tests.rs`, `crates/gui/src/app.rs`, `crates/gui/src/brand.rs`, `crates/gui/src/camera_status.rs`, `crates/gui/src/header.rs`, `crates/gui/src/lib.rs`, `crates/gui/src/main.rs`, `crates/gui/src/theme.rs`, `crates/gui/src/widgets.rs`, `crates/gui/tests/brand_layout_tests.rs`, `crates/gui/tests/theme_tests.rs`

## 1. Executive Summary

The diff restyles `soos-gui` to match the soos brand. It adds four presentation modules:
`theme` (palette, metrics, faux bold, `content_rect`, `fit_video`), `brand` (SVG path data, a
small scanline rasterizer, cached wordmark and star textures, a procedural window icon),
`widgets` (cards, tiles, banners, `BrandButton`, progress bar, chips, checklist, toggle switch)
and `header` (band, tabs, `DaemonSwitch`, daemon panel). `app.rs` re-lays out the three pages on
these modules, and `main.rs` only adds the window icon. The header toggle switch replaces the
Pause/Resume buttons. It submits the same `PrivilegedAction::PauseDaemon` / `ResumeDaemon` through
the unchanged `submit_privileged` path, is disabled while `TaskRunner` is busy and does nothing
while the state is `Unknown`. The enrollment, Save/import, store-task, profile-listing and
two-step delete flows keep the same calls, messages and threading. The deletion dialog moves from
`egui::Window` to `egui::Modal`, and Escape or a backdrop click only cancels. There are two small
behavior additions, both fail-safe: a selected match profile that has left the list clears the
match reference, and the live score is shown only while its profile is listed. No dependency
changed. No logging was added, and no frame, embedding or credential reaches a new sink.

Gates run by the reviewer: `cargo clippy -p soos-gui --all-targets -- -D warnings` clean;
`cargo test -p soos-gui` **149 passed, 0 failed**; `cargo test -p soos-invariants` **448 passed**;
`cargo fmt --all -- --check` clean.

**Delta (re-review, fingerprint `7d70ab4e…ea87`).** The diff now also changes
`MockCameraManager`. A new `stabilized` flag is set by the capture thread when it publishes its
first frame. `notify_activity` publishes a forced frame only after that, so the startup warmup no
longer flaps between `Ready` and `Starting`. A forced wake frame now also stores
`warmup_remaining = 0`. The delta adds `mock_warmup_activity_tests.rs` (2 tests), updates
`Docs/CAMERA_V4L_CRATE.md` (mock note and C5 row), fixes the docs side of M1 and the walkthrough
side of M3. For the delta I ran `cargo clippy -p soos-camera-v4l --all-targets -- -D warnings`
(clean) and `cargo test -p soos-camera-v4l -p soos-daemon -p soos-gui`: **1053 passed, 0 failed,
5 ignored** (the ignored tests already existed; the diff adds no `#[ignore]`), across 138 test
binaries. The tests that call `notify_activity` (`mock_camera_tests`, `warmup_tests`, the daemon
suites) still pass.

**Second delta (fingerprint `46e66a47…ce24`).** This round fixes M4, M5 and M6.
`cargo clippy -p soos-camera-v4l --all-targets -- -D warnings` is clean, and
`cargo test -p soos-camera-v4l` gives **224 passed, 0 failed, 3 ignored** (all three already
ignored before this branch).

I found no blocking issues. M4, M5 and M6 are resolved, and the MINOR items M2 and the
suggestions S1/S2 remain.

## 2. Test Changes

I listed these mechanically from `target/candid_diff.patch`:

- Test files touched: `crates/gui/tests/theme_tests.rs` (new, 15 tests),
  `crates/gui/tests/brand_layout_tests.rs` (new, 9 tests) and
  `crates/camera-v4l/tests/mock_warmup_activity_tests.rs` (new, 2 tests). All three are new files
  only, and no existing test file appears in the patch.
- Removed or changed assertions (`^-` lines in test files): **none**. `git diff 222665f --
  crates/gui/tests/layout_tests.rs` is empty.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`): **none**. The new files use
  file-level `#![allow(...)]` with a `reason`, in the same style as the existing GUI tests.
- Inline `mod tests` changes: **none**.
- The C5 row in `Docs/CAMERA_V4L_CRATE.md` now also cites
  `mock_warmup_activity_tests::test_notify_activity_during_warmup_never_publishes_ready`. That
  test exists and passes. It would fail on the old code, where `notify_activity` during warmup set
  `is_ready` and stored a frame.
- Matrix rows GUX1–GUX13 cite 24 test functions. All 24 exist under exactly those names (checked
  by grep), and all pass.

## 3. Deep Reasoning Audit

### Logic & Behavior Preservation
- *Daemon Pause/Resume*: `DaemonSwitch::new` / `click_action` (header.rs) maps `Active`→
  `PauseDaemon` and `Inactive`→`ResumeDaemon`. It returns `None` when `busy` or `Unknown`.
  `daemon_panel` shows a spinner and returns `None` while busy, and calls `click_action` only
  on `clicked()`. `toggle_switch` senses only hover when it is not interactive. `app.rs` passes the
  action to `submit_privileged`, which is unchanged and still calls `request_refresh()`. GUX4
  pins the four cases. The daemon result message moved from the header to `render_notices` with
  the same "only when not busy" condition. PASS.
- *Privileged / store flows*: the Save branch (fusion error surfaced, `ImportTemplate` in Polkit
  mode, `StoreTask::Enroll` otherwise, `pending_store_username`, messages) is a re-indented copy.
  The `DeleteTemplate` / `StoreTask::Delete` paths and their error messages are the same.
  `refresh_profiles` is now deferred through a `refresh` flag until the scope ends, which
  satisfies the borrow checker and keeps the same semantics. PASS.
- *Delete confirmation*: the modal is still reachable only from a non-empty table, as before.
  `confirmed` is checked first, and `should_close()` (Escape or a backdrop click) can only set
  `cancelled`. PASS.
- *Match reference*: the combo selection still goes through `select_match_reference` with
  `MODEL_ID_EMBEDDING` and `embedding_dimension()`. The new block that clears a profile that left
  the list calls `select_match_reference(None, ..)`, which wipes the reference (Zeroizing drop),
  the score and the note. That is fail-safe. `live_score` is gated on the selected uid still being
  listed. The threshold still comes from `pipeline.config().match_threshold`. The liveness color
  still comes only from `frame.pad_live` (#215). PASS.
- *Enrollment*: Start/Cancel still create or clear `enrollment_session`. Save is still shown only
  when `current_step == Completed`. The step-status logic is the same as before. A `Waiting` state
  is added when there is no session, which is display only. PASS.
- *Layout*: `fit_video` falls back to 4:3 for a non-finite or non-positive aspect. The old code
  produced NaN sizes when `frame.height == 0`, so this is an improvement. GUX12 re-checks the old
  CLP4/GARP3 canvas minimums (500x350 at 1120x780, 450x300 at 900x600, 16:9 at least 450x250) on
  the real formulas with the band and an 80-point developer-banner allowance. They match the
  thresholds in `layout_tests.rs`. PASS.

### Panic Safety
- No `unwrap`/`expect`/`panic!`/raw indexing was added to production code. The scan of added
  `src/` lines found none, and slices use `get`/`get_mut`, `zip` and `windows`.
- `f32::clamp`: every call has constant bounds with `min <= max`. In `column_stack`, the bounds
  `140.min(tile)` and `tile.max(140)` are always ordered, even for NaN. A NaN receiver returns NaN
  and does not panic. `map_shapes` replaces `clamp` with `max().min()` on a normalized,
  finite-checked rect (GUX13).
- Division: `rasterize` returns early when a dimension is zero or the view is non-positive, before
  `chunks_exact_mut(out_w)`, which would panic on 0. `fit_aspect` guards the view. `to_screen`
  divides floats only.
- Loops are bounded. The `stat_tile` font-shrink loop is monotone down to 40. `faux_bold_offsets`
  runs at most 255 passes, and callers pass 1–6.
- `window_icon(128)` is computed once at startup, with `n*n*4` capacity and a fixed size.

### Memory & UI-Thread Blocking
- The brand textures are rasterized only on a cache miss, and the size is clamped to 4096 px per
  axis. While a window is being resized, the star is re-rasterized on frames where its pixel size
  changes. At a column width of about 250–300 points the cost is small. No new lock, I/O,
  `Command::new` or thread was added. The session and score locks are short, non-nested reads,
  as before. `store_content_height` requests one extra repaint only when the measured height
  changes by more than 0.5 point, so the layout settles.
- Cache growth: see SUGGESTION S1.

### Secrets & Logging
- No `log`/`tracing`/`eprintln` call was added. The UI shows only scores, percentages, uid,
  username and model metadata, which it already showed before. PASS.

### English-Only Policy
- Code, comments, docs, matrix rows and walkthrough 183 are in English. The non-ASCII characters
  are typographic symbols (`×`, `·`, `°`, `—`) and the emoji button glyphs that already existed.
  PASS.

### Delta — `MockCameraManager` warmup / `notify_activity`
- *Startup warmup (C5)*: `stabilized` starts `false` and is set only after the capture thread
  publishes its first frame (mock.rs:171, Release). `notify_activity` reads it with Acquire before
  it forces a frame (mock.rs:317). During the initial warmup, activity therefore only refreshes
  the idle clock: no frame and no `is_ready`. The slot stays empty until the warmup ends. This
  fixes the flapping, and it also fixes a real C5 gap in the old code, where a forced frame stayed
  in the slot during warmup because the warmup branch never cleared it. PASS.
- *Wake from suspension*: `stabilized` stays true, so a wake still serves a frame at once
  (`mock_camera_tests::test_mock_camera_idle_throttling_and_wake` passes). Storing 0 in
  `warmup_remaining` stops the thread from withdrawing readiness on its next re-warmup tick. The
  thread then re-checks idle, finds recent activity and resumes streaming. PASS, apart from the
  race described in M4.
- *Error / starve recovery*: the thread resets `warmup_remaining` to `warmup_frames` on an injected
  error or starvation (mock.rs:98), but `stabilized` is never reset. After recovery, one
  `notify_activity` during the re-warmup now ends it at once and serves an un-warmed frame. The old
  code also served that frame, but readiness was withdrawn and the warmup kept counting. The docs
  only describe the suspension case. See M5.
- *Other hooks*: `set_warmup_remaining` is used only inside `mock.rs` (no test calls it). After
  stabilization, a following `notify_activity` would cancel a warmup set through it. That matches
  the new wake semantics and affects no test. `stop()` stops the thread, `notify_activity` checks
  `running`, and `stabilized` does not matter after stop. Restart works by building a new
  instance, which starts with a fresh `stabilized = false`. PASS.
- *Daemon impact*: production uses `V4lCameraManager`, so the mock change affects only
  `--mock` / test runs. The daemon suites pass. Under load, a daemon test that called
  `notify_activity` right after construction and expected an immediate frame would now wait for
  warmup. No such test fails.
- *Panic / unsafe*: no new `unwrap`, indexing or `unsafe`. The tests use the shared
  `wait_until(SETTLE_TIMEOUT, ..)` helper, which is bounded. PASS.

### Doc Accuracy
- §5 of `Docs/GUI_APPLICATION.md` matches the code: 44-point band, `(259*k).clamp(250,300)`
  column, `pad` of 40 points scaling down to a 20-point floor, card-first column stacking with the
  140-point stat floor, modal semantics, palette hex values. See M1 and M3 for the small
  mismatches.

## 4. Detailed Findings & Action Items

- **[RESOLVED in delta] M1 / M3 (docs side).** `Docs/GUI_APPLICATION.md` now lists separate rows
  for `Active`/`Inactive` + pending and `Unknown` + pending ("Daemon status unknown"), and §1
  matches. Walkthrough 183 now names `feat/gui-brand-redesign` and records the Layer 2 review.
  The original text of both findings is kept below for traceability.
- **[MINOR] M1 — busy + `Unknown` label differs from the docs.** `crates/gui/src/header.rs:66`
  shows "Waiting for authorization..." only when `busy && position.is_some()`. With
  `DaemonState::Unknown` and a pending action, the panel shows a spinner labeled "Daemon status
  unknown". `Docs/GUI_APPLICATION.md:288` ("any, privileged action pending | spinner, 'Waiting for
  authorization...'") and line 52 say otherwise. The old header showed the authorization text in
  that case too. This is cosmetic (no action can be submitted either way). Fix either the docs or
  the label condition.
- **[MINOR] M2 — stale canvas contracts.** The canvas tests in
  `crates/gui/tests/layout_tests.rs` (`test_live_inspection_layout_allocates_large_canvas`,
  `test_windowed_mode_live_inspection_layout_with_checkboxes`,
  `test_guided_enrollment_layout_allocates_large_canvas`) emulate the former 0.32/0.35 sidebar
  formulas, which production no longer uses. They still pass, but they no longer exercise the
  production layout. GUX12 carries the real contract with the same thresholds, and the docs say so.
  As a follow-up, consider repointing the GARP3 matrix row
  (`layout_tests::test_windowed_mode_live_inspection_layout_with_checkboxes`) to GUX12 as well.
  Keep the old tests; do not delete them.
- **[MINOR] M3 — walkthrough 183 is stale.** `AI/walkthroughs/183_gui_brand_redesign.md:5` and
  `:111` still say the branch is `willi363/feat/ui` and must be renamed. It is already
  `feat/gui-brand-redesign`. Line 94 says Layer 2 has not been run. Update both during
  traceability.
- **[RESOLVED in second delta] M4 / M5 / M6.**
  - *M4*: the warmup decrement is now
    `warmup_clone.fetch_update(AcqRel, Acquire, |r| r.checked_sub(1))` (`mock.rs` ~line 109).
    When `notify_activity` has already zeroed the counter, the closure returns `None`, the update
    fails and the counter stays at 0. It can no longer wrap. The following
    `warmup_clone.load(..) == 0` check still marks completion correctly. In the normal case the
    last decrement takes 1 to 0, so the activity clock is refreshed and the next iteration
    streams. When a wake zeroed the counter, the check also sees 0 and refreshes the clock, which
    is harmless because `notify_activity` has just done the same. When an error or starvation
    reset happens during the sleep, the counter is non-zero and the warmup continues, as intended.
    One small window remains: the thread's `ready.store(false)` (line 106) can land just after a
    wake's `ready.store(true)`. That gives at most one frame interval of `Starting`, which clears
    itself on the next iteration once the counter is 0. It is cosmetic and mock-only.
  - *M5*: the code comment (`mock.rs` ~line 343) and `Docs/CAMERA_V4L_CRATE.md` (lines 89–92) now
    say that a stabilized camera skips any re-warmup, including after a cleared injected error or
    starvation. That matches the behavior.
  - *M6*: walkthrough 183 §8 now lists `cargo test -p soos-camera-v4l -p soos-daemon -p soos-gui`
    (1053 passed) and the camera clippy run.
  - The original text of M4–M6 is kept below for traceability.
- **[MINOR] M4 — `warmup_remaining` can wrap to `usize::MAX` (delta, mock only).**
  `crates/camera-v4l/src/mock.rs:340` stores 0 from the caller's thread. The capture thread does
  `load` (line 104), sees `remaining > 0`, and then calls `fetch_sub(1)` (line 107). If the store
  lands between those two calls, `fetch_sub` turns 0 into `usize::MAX`. The thread then stays in
  the warmup branch: it reports `Starting`, publishes no frames and never reaches the
  idle/suspension check. Only the next `notify_activity` gets it out (it stores 0 again). The
  window is a few instructions wide and the state self-heals on the next activity, so this is not
  blocking, but it can make tests flaky. The thread already had the same race with the test hook
  `set_warmup_remaining(0)`. Fix: decrement with saturation, e.g.
  `warmup_clone.fetch_update(AcqRel, Acquire, |r| r.checked_sub(1))`.
- **[MINOR] M5 — the wake path also ends re-warmup after an error or starvation.** Because
  `stabilized` is never reset (mock.rs:171), `notify_activity` (mock.rs:340) also cancels the
  re-warmup the thread starts after an injected error or starvation clears (mock.rs:98), not only
  the one a suspension starts. `Docs/CAMERA_V4L_CRATE.md` and the code comment mention only
  suspension. Either document this (it is still an improvement on the old flapping) or limit the
  `store(0)` to the post-suspension case.
- **[MINOR] M6 — walkthrough §8 omits the camera and daemon test runs.**
  `AI/walkthroughs/183_gui_brand_redesign.md` §8 lists only `soos-gui` and `soos-invariants`
  results, although the delta changes `soos-camera-v4l`. Add the
  `cargo test -p soos-camera-v4l -p soos-daemon` results (all green in this review).
- **[SUGGESTION] S1 — cache wording and growth.** `crates/gui/src/brand.rs:419` keys the
  texture cache by `(mark, color, w/64, h/64)`, so a resize that crosses 64-px buckets keeps one
  texture per bucket visited, for the life of the context. This is bounded and small (around
  300x300 RGBA per entry in practice). The doc comment "a resized window replaces the texture
  instead of accumulating stale ones" is true only within a bucket. Consider one slot per
  `(mark, color)`.
- **[SUGGESTION] S2 — match reference cleared only on the Live tab.**
  `crates/gui/src/app.rs:770`: the clearing runs only while the Live tab is rendered with a frame.
  After a deletion on the Profiles tab, the worker keeps the in-memory (Zeroizing) reference
  until the Live tab is shown again. The score is gated and never displayed meanwhile, and the old
  code never cleared it at all, so this is not a regression. Clearing it in
  `handle_store_task_outcomes` / `handle_task_outcomes` on a successful delete would be tighter.

## 5. Final Verdict

**VERDICT: APPROVED**
