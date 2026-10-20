# Candid Review Report

- **Date**: 2026-10-20
- **Target Branch**: `feat/gui-motion` (PR #449, GitHub #438, part of #358)
- **Base (merge-base)**: `6c4ecd40` (equal to `origin/main` at review time, no rebase needed)
- **Reviewed-Diff-Fingerprint**: `4d1314d51f778ae4113103691d62d81734a500500ec92cf8c9a70e5e34092ca7`
- **Reviewer**: independent context-free sub-agent (not the author of `cd58559` or of the review in `6b90e98`)
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_gui_motion.md`,
  `AI/auditor_constraints_gui_motion.md`, `AI/tester_contract_gui_motion.md`, `AI/walkthroughs/234_gui_motion.md`,
  `Cargo.lock`, `Docs/GUI_APPLICATION.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `Docs/UI_MOTION.md`,
  `crates/gui/Cargo.toml`, `crates/gui/src/app.rs`, `crates/gui/src/app/capture.rs`, `crates/gui/src/args.rs`,
  `crates/gui/src/brand.rs`, `crates/gui/src/camera_status.rs`, `crates/gui/src/header.rs`, `crates/gui/src/lib.rs`,
  `crates/gui/src/main.rs`, `crates/gui/src/motion.rs`, `crates/gui/src/motion/portal.rs`, `crates/gui/src/overlay.rs`,
  `crates/gui/src/theme.rs`, `crates/gui/src/widgets.rs`, `crates/gui/tests/motion_flicker_tests.rs`,
  `crates/gui/tests/motion_tests.rs`, `tests/invariants/src/gui_motion_contract.rs`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/presence_unlock_contract.rs`

## 1. Executive Summary

The full raw diff (merge-base `6c4ecd40` to the working tree, 5185 lines) was reviewed, with focus on
`cd58559` ("keep flickering stat tiles and camera errors fully visible") against `Docs/UI_MOTION.md` G4, G7 and §8.

- `motion::fade_in_on_stable_change` (G4): a key change restarts the fade only when the previous key was held for
  at least `2 x duration`; otherwise `elapsed` is left running (never reset) and `held` restarts from 0. A flicker
  therefore yields at most one fade, opacity is monotonic, it reaches 1 within `T_SHORT` (0.18 s) of the first
  change, and no repaint is requested once `elapsed >= duration`. Non-finite or non-positive durations collapse to 0
  and return 1.0. Reduced motion pins `elapsed = duration` on every pass (at rest, no repaint).
- `render_status_banner` (G7): Warning and Error states force opacity 1.0, and since `mix_color(prev, color, 1.0)`
  is `color`, the title is in its own colour (`WARN_TEXT` / `DANGER`) on the arrival frame. Info keeps the fade.
  The previous text is never kept on screen (the banner always draws the current `banner`).
- Reduced motion is not regressed: both paths return at-rest values with reduced prefs (GMO21 reduced test,
  GMO22 reduced test, and the unchanged GMO suite).
- The zbus portal reader (`crates/gui/src/motion/portal.rs`) is byte-identical to `d6b8420`: session bus only
  (`zbus::connection::Builder::session()`), `PORTAL_READ_DEADLINE_MS = 200`, no object server, no well-known name,
  no match rule, no signal stream, value never logged.
- No pre-existing assertion was removed or changed; the only edit to a pre-existing test is the owner-approved PAU17
  `ALLOWED` list widening (not flagged, per owner approval).

No CRITICAL or MAJOR finding. One SUGGESTION.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `crates/gui/tests/motion_flicker_tests.rs` (new, GMO21–GMO22),
  `crates/gui/tests/motion_tests.rs` (new, GMO suite), `tests/invariants/src/gui_motion_contract.rs` (new),
  `tests/invariants/src/lib.rs` (module registration), `tests/invariants/src/presence_unlock_contract.rs`
  (PAU17 `ALLOWED` gains `crates/gui/Cargo.toml`; owner-approved Planned Contract Migration B.2a; the assertion
  body is unchanged).
- Removed/changed `assert`/`#[test]`/`proptest!`/`should_panic` lines: **none** (grep empty).
- New escape hatches: none; the single `epsilon` hit is a comment in a new test (relative settle epsilon of the
  new spring tests), not a loosened tolerance.
- Inline `mod tests` changes: none.

Fail-against-wrong-implementation check: with the previous `fade_in_on_change` wiring the stat tile restarts the
fade at every flip, so `test_gmo21_flicker_every_frame_never_restarts_the_fade` (monotonic opacity) and
`test_gmo21_flickering_stat_tile_value_reaches_full_opacity` fail; with the previous banner the arrival frame has
opacity < 1 and a cross-faded title colour, so both GMO22 arrival tests fail.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Flicker at periods 1, 2, 3, 6, 12 frames: period 12 (0.2 s) is below the hold (0.36 s), so only the first change
  fades; traced the slot state frame by frame: PASS.
- Flicker slower than the hold (e.g. every 0.5 s): each change fades again. This is the specified G4 behaviour
  (stable content), not a defect: PASS.
- First sight: the slot is created with `key = key`, `held = hold`, `elapsed = duration` → 1.0, no repaint: PASS.
- Multiple passes per frame: `pass` guard prevents double `dt` accumulation, same as `fade_in_on_change`: PASS.
- Widget not drawn for a while then redrawn with a new key: `held` only grew by one clamped `dt`, so the change is
  drawn at once (fully visible, fail-safe direction): PASS.
- Error → Info: title cross-fades from `DANGER` to `INK` while Info fades in — matches G7 ("Info fades"): PASS.
- Warning → Error, Error kind A → kind B: both drawn at 1.0 in their own colour: PASS.
- Docs (`UI_MOTION.md` G4/G7, `GUI_APPLICATION.md`), matrix GMO21–GMO22, walkthrough 234 match the code: PASS.

### PAM Concurrency & Deadlines
- No change to `crates/pam`. The portal reader is a GUI-side thread with its own current-thread runtime, a
  200 ms deadline enforced by the UI poll, unchanged since `d6b8420`: PASS.

### Panic Safety & Fail-Closed
- New production code has no `unwrap`/`expect`/`panic!`/indexing; `motion.rs` keeps its module-level denies and
  GMO10 (no allocation) still passes. No authorization path is touched (GUI display only): PASS.

### Test Integrity & Anti-Weakening
- See §2: no assertion removed or relaxed; new tests are discriminating: PASS.

### Memory, Bounds & Secrets
- `StableFadeSlot` is a fixed-size `Copy` value in egui temp storage keyed per widget; no frame, embedding or
  portal value logged: PASS.

### Supply Chain & Automation
- `cd58559` touches no manifest. The branch's `zbus` use in `soos-gui` is the workspace dependency (owner-approved
  PAU17 scope); `Cargo.lock` adds no new crate source: PASS.

### English-Only Policy
- Code, comments, docs and commit message of `cd58559` are English: PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/gui/src/camera_status.rs:169` — `fade_in_on_change` is still called for Warning/Error
  states and its result discarded, so it requests about `T_SHORT` worth of repaints (~11 frames) that draw nothing
  different. Harmless; it could be skipped for non-Info severities (keeping the slot in sync) if repaint economy
  matters.

## 5. Final Verdict

Tests: `cargo test --locked -p soos-gui -p soos-invariants --all-features -- --test-threads=3` all green
(0 failed, including GMO21–GMO22); `cargo clippy --locked -p soos-gui -p soos-invariants --all-targets
--all-features -- -D warnings` clean; `cargo fmt --check -p soos-gui` clean. Merge remains gated on owner UIM7
prototype approval (not a review finding).

**VERDICT: APPROVED**
