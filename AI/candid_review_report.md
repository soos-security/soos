# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `fix/gui-fps-and-egui-ids`
- **Base (merge-base)**: `304af03`
- **Reviewed-Diff-Fingerprint**: `755c11d286cae8e9689907a008a9af6700276121a1e1ed50df4582a9eb2a7b1d`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/184_gui_frame_pacing_and_egui_ids.md`, `Cargo.toml`, `Docs/GUI_APPLICATION.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `crates/gui/src/ipc_camera.rs`, `crates/gui/src/widgets.rs`, `crates/gui/src/worker.rs`, `crates/gui/tests/card_id_stability_tests.rs`, `crates/gui/tests/ipc_preview_cadence_tests.rs`, `crates/gui/tests/worker_pacing_tests.rs`

## 1. Executive Summary

The diff makes three changes to `soos-gui` pacing and layout:

- The vision worker (`worker.rs`) no longer sleeps after an analyzed frame. It waits
  `WORKER_IDLE_POLL` (5 ms) only when no new frame sequence was available.
- The daemon preview worker (`ipc_camera.rs`) now keeps a 33 ms cadence from request to request
  (`PollStep::Cadence`, `frame_cadence_delay`). Before, it slept 33 ms after each reply. All
  back-off paths keep their delays.
- `card_in_rect` and `card_in_rect_with_footer` (`widgets.rs`) give their content scope an
  explicit id (`card_scope_id`), so a stat tile shown or hidden above the card between two egui
  passes no longer shifts the ids of the widgets inside the card.

`Cargo.toml` adds four `[profile.dev.package.*] opt-level = 3` overrides. `[profile.release]` is
unchanged. No dependency, `unsafe`, logging or I/O was added.

Gates run by the reviewer: the fingerprint of `target/candid_diff.patch` matches
(`sha256sum` and `scripts/candid_subagent.sh --fingerprint`). `cargo clippy -p soos-gui
--all-targets -- -D warnings` is clean. `cargo test -p soos-gui` gives **156 passed, 0 failed**.
`cargo test -p soos-invariants` gives **448 passed, 0 failed**.

**Delta (re-review, fingerprint `755c11d2…7b1d`).** I checked that the fingerprint matches
(`sha256sum` and `scripts/candid_subagent.sh --fingerprint`). The patch grew from 564 to 565
lines. The delta changes only comments and docs:

- the module doc of `crates/gui/tests/worker_pacing_tests.rs` (M1);
- the profile-table row in `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` (M2);
- one added doc line on `card_scope_id` in `crates/gui/src/widgets.rs` (S2).

I checked this by filtering the diff of `worker.rs`, `ipc_camera.rs`, `Cargo.toml` and the three
test files down to non-comment lines. Every code line and every test assertion is the same as in
the first round. I re-ran the gates: clippy is clean, `cargo test -p soos-gui` gives 156 passed,
0 failed, and `cargo test -p soos-invariants` gives 448 passed, 0 failed.

I found no blocking issues. M1, M2 and S2 are resolved. S1 is kept on purpose, and I accept the
author's reason for keeping it.

## 2. Test Changes

I listed these mechanically from `target/candid_diff.patch`:

- Test files touched: `crates/gui/tests/card_id_stability_tests.rs` (new, 2 tests),
  `crates/gui/tests/ipc_preview_cadence_tests.rs` (new, 3 tests) and
  `crates/gui/tests/worker_pacing_tests.rs` (new, 2 tests). All three are new files. No existing
  test file is in the patch: `git diff --name-only 304af03 -- crates/gui/tests tests` lists only
  these three.
- Removed or changed assertions (`^-` lines in test files): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`): **none**. The file-level
  `#![allow(clippy::unwrap_used, clippy::expect_used, reason = ...)]` follows the existing GUI
  test style.
- Inline `mod tests` changes: **none**.
- Do the tests fail on the old code? I checked out a scratch worktree at `304af03` (the old
  `widgets.rs` and the old `Cargo.toml`), copied in `card_id_stability_tests.rs` and ran it. Both
  tests **FAIL** there (`left: Id::new(10978825285480028652)` vs `right: Id::new(8741797889808405595)`).
  Then I removed the worktree. `ipc_preview_cadence_tests` and `worker_pacing_tests` cannot
  compile against the old code (`frame_cadence_delay`, `worker_idle_delay` and
  `WORKER_IDLE_POLL` did not exist, and `PREVIEW_POLL_INTERVAL` was private).
- Matrix rows GFP1–GFP3 cite 7 test functions. All 7 exist under exactly those names, and all
  pass.

## 3. Deep Reasoning Audit

### Logic & Behavior Preservation

- *Vision worker spin risk* (`worker.rs` ~267–390): `analyzed_frame` becomes `true` only inside
  `if frame.sequence != last_seq`, right after `last_seq = frame.sequence`. So a zero-delay
  iteration always consumes a new sequence number, and the next iteration either finds another
  new frame or sleeps 5 ms. The worker cannot spin when frames stop, when `latest_frame()`
  returns `None`, or when it keeps returning the same frozen frame. The analysis-error path,
  including a failed `convert_to_rgb`, also consumes the sequence, so it cannot spin either.
  `running` is still checked at the top of every iteration, so shutdown is still prompt. PASS.
- *Frozen-frame withdrawal*: the `else if published && !camera.is_ready()` branch is unchanged
  and still runs every iteration where `latest_frame()` is `None`, followed by a 5 ms wait.
  PASS.
- *Idle cost*: the idle wait drops from 10 ms to 5 ms, so a camera that is absent or suspended
  now wakes the worker about 200 times per second instead of 100. Each wake calls
  `notify_activity` (one `RwLock` write in the V4L manager, a no-op for IPC). This is negligible
  but not free. See S1.
- *Preview request rate* (`ipc_camera.rs` ~470–477): `started` is taken before `poll_once`, and
  the delay is `33 ms - elapsed` (saturating). So the time from one request start to the next is
  always at least 33 ms, and `thread::sleep` never sleeps less than asked. That caps the rate at
  about 30.3 requests per second, even when a slow exchange polls again at once. This is below
  `DEFAULT_PREVIEW_MAX_REQUESTS_PER_SEC = 40` (`crates/daemon/src/preview.rs:26`). Before, the
  start-to-start time was 33 ms plus the exchange time, so the new cadence is strictly faster but
  still bounded. PASS.
- *Back-off paths*: `RateLimited` → `Continue(250 ms)`, `Unavailable` and post-frame empty preview
  → `Continue(UNAVAILABLE_BACKOFF)`, `Protocol` / `Io` → `Reconnect`, `Unauthorized` → `Stop`.
  All are unchanged. Only the two paths that used `Continue(PREVIEW_POLL_INTERVAL)` (frame
  received, and an empty preview before the first frame) moved to `Cadence`. PASS.
- *egui ids* (`widgets.rs` ~413–418, 842–859, 1225–1242): in egui 0.35 `Ui::new_child`
  (`ui.rs:251–256`), an `IdSource::Child` id is `stable_id.with(next_auto_id_salt)`, so it
  depends on how many widgets came before it. `IdSource::Explicit(id)` uses `id` for both the
  stable and the unique id. That confirms the walkthrough's claim and the fix.
  `card_scope_id = ui.id().with(("soos_card_scope", id_salt))`. In both call sites
  (`app.rs:844`, `app.rs:1262`), the parent `ui` is the same column `Ui` that draws the stat and
  star tiles, so its own id does not depend on those tiles.
- *Collision risk*: an explicit id has no auto-counter suffix, so two cards with the same
  `id_salt` under the same parent would share a scope id. Production has two salts
  (`live_telemetry_card`, `enrollment_card`). Each is rendered once per pass, on different tabs.
  The same card rendered on two passes of one frame should keep its id, and that is what this
  change does. The `ScrollArea` id now derives from the new scope id. That only resets the
  scroll offset once after the upgrade. `content_height_id` was already a global
  `Id::new((..., id_salt))`, so it has the same uniqueness requirement as before. No collision.
  See S2 for documenting it.

### Panic Safety

- No `unwrap`, `expect`, `panic!`, indexing or arithmetic that could overflow was added.
  `Duration::saturating_sub` cannot panic. `worker_idle_delay` is a `const fn` with two constant
  branches. `thread::sleep(Duration::ZERO)` is avoided explicitly, and would be harmless anyway.
  PASS.

### Security Invariants / Compiler Profiles

- `[profile.release]` is untouched: `opt-level = 3`, `lto`, `codegen-units = 1`,
  `panic = "unwind"`, `strip`, `overflow-checks`. `[profile.dev]` and `[profile.test]` keep
  `overflow-checks = true`. A per-package override only sets `opt-level`, and Cargo does not
  allow `panic` per package, so the unwind strategy and debug assertions are inherited.
  Invariant 7 (`test_workspace_cargo_toml_enforces_overflow_checks`) and
  `test_release_profile_unwinds_so_pam_catch_unwind_is_effective` pass. All four overridden
  packages are in `Cargo.lock`, and `cargo metadata` gives no "did not match any packages"
  warning. PASS.
- Because `[profile.test]` inherits from `[profile.dev]`, the overrides also apply to
  `cargo test`. That has no security impact, since overflow checks stay on, but see M2.

### Secrets & Logging

- No `log`, `tracing` or `eprintln` call was added. The walkthrough says the temporary timing
  logs were removed, and no such line is in the diff. PASS.

### English-Only Policy

- All code, comments, docs, matrix rows and walkthrough 184 are in English. PASS.

### Doc Accuracy

- `Docs/GUI_APPLICATION.md` §1, §4 and §5.4 match the code: 33 ms request-to-request,
  `WORKER_IDLE_POLL` of 5 ms, the explicit card scope id, and the four overridden packages. The
  matrix GFP1–GFP3 wording matches the tests. Two minor inaccuracies are listed below.

## 4. Detailed Findings & Action Items

- **[RESOLVED in delta] M1 / M2 / S2.**
  - *M1*: `worker_pacing_tests.rs:5-6` now says "about 27 instead of 30 analyzed frames per
    second in a release build". The conflicting debug figure is gone, and the assertions are
    unchanged.
  - *M2*: the row now says "Development and test builds only (`[profile.test]` inherits
    `[profile.dev]`)", which matches how Cargo applies the profiles.
  - *S2*: `card_scope_id` now documents that "`id_salt` must therefore be unique among the cards
    drawn in the same parent `Ui`".
  - The original text of each finding is kept below for traceability.
- **[ACCEPTED] S1.** The author keeps the 5 ms idle wait: it halves the delay before a frame
  that arrives during an idle spell is picked up, and the CPU cost is negligible. This is a
  reasonable trade-off and not a defect.
- **[MINOR] M1 — wrong debug-build rate in a test doc comment.**
  `crates/gui/tests/worker_pacing_tests.rs:6` says the old worker reached "about 20 in a debug
  build". The walkthrough (§2 table, about 6.5) and the matrix intro (about 6) both give a
  different number for the same old code. Align the comment (e.g. "about 6"). This is
  documentation only, and no assertion changes.
- **[MINOR] M2 — "Development builds only" is not quite right.** The new row in
  `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` (profile table) says the overrides apply to
  development builds only. `[profile.test]` inherits `[profile.dev]` and its package overrides,
  so `cargo test` and CI test builds also compile those four crates at `opt-level = 3`. That
  makes them a little slower to compile, and overflow checks are kept. Say "dev and test
  profiles". There is no security impact.
- **[SUGGESTION] S1 — idle wake rate.** `WORKER_IDLE_POLL` at 5 ms doubles the idle wake rate
  when no frame arrives (camera absent, suspended or failed). This is still cheap, but 10 ms
  would keep the old idle behavior. The frame-rate win comes from `Duration::ZERO` after an
  analyzed frame, not from the shorter idle wait (a 10 ms idle wait adds at most 10 ms of
  latency to the first frame after an idle spell). It is optional.
- **[SUGGESTION] S2 — document the uniqueness requirement for `id_salt`.** Since
  `card_scope_id` no longer uses egui's auto counter, two cards with the same `id_salt` under one
  parent `Ui` would share the scope and `ScrollArea` ids. That is true today only by convention
  (`live_telemetry_card` and `enrollment_card`). A one-line note on `card_in_rect` and
  `card_in_rect_with_footer` ("`id_salt` must be unique among cards drawn in the same parent")
  would prevent a future clash.
- **[OBSERVATION] Test depth.** GFP1 and GFP2 test the pure helpers (`frame_cadence_delay` and
  `worker_idle_delay`), not the loops that call them. I checked the wiring by reading the code
  (`ipc_camera.rs` ~471–477, `worker.rs` ~384–388), and it is correct. A loop-level test would
  need a fake socket and a clock, which is out of scope here.

## 5. Final Verdict

**VERDICT: APPROVED**
