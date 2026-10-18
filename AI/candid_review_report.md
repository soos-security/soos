# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `feat/gui-motion` (GitHub #438, PR 3 of #358)
- **Base (merge-base)**: `1043be6`
- **Reviewed-Diff-Fingerprint**: `059e624df30a6aaf77163717b5901d042018816b78241aa7208b23ea24ef2a0f`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_gui_motion.md`,
  `AI/auditor_constraints_gui_motion.md`, `AI/tester_contract_gui_motion.md`, `AI/walkthroughs/234_gui_motion.md`,
  `Cargo.lock`, `Docs/GUI_APPLICATION.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `Docs/UI_MOTION.md`,
  `crates/gui/Cargo.toml`, `crates/gui/src/app.rs`, `crates/gui/src/app/capture.rs`, `crates/gui/src/args.rs`,
  `crates/gui/src/brand.rs`, `crates/gui/src/camera_status.rs`, `crates/gui/src/header.rs`, `crates/gui/src/lib.rs`,
  `crates/gui/src/main.rs`, `crates/gui/src/motion.rs`, `crates/gui/src/motion/portal.rs`, `crates/gui/src/overlay.rs`,
  `crates/gui/src/theme.rs`, `crates/gui/src/widgets.rs`, `crates/gui/tests/motion_tests.rs`,
  `tests/invariants/src/gui_motion_contract.rs`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/presence_unlock_contract.rs`

## 1. Executive Summary

The diff adds the `soos-gui` motion system (`motion.rs` pure helpers and egui glue, `motion/portal.rs`
one-time XDG portal reader), animates the header switch, tabs, column entrances, banners, progress bar,
camera status card and star tiles, extracts the face overlay into `overlay.rs` unchanged in behaviour, and
replaces both `egui::Spinner` uses with the pending star. zbus usage is a single read-only session-bus
connection on one named background thread with its own current-thread Tokio runtime, calling only
`org.freedesktop.portal.Settings.ReadOne` (fallback `Read`) under a 200 ms overall deadline enforced both on
the reader (`tokio::time::timeout` plus `method_timeout`) and on the UI side (`poll_at` deadline). No object
server, no system bus, no name request, no match rule. The only pre-existing test assertion change is the
owner-approved PAU17 `ALLOWED` migration. Tests, clippy and fmt are green. No CRITICAL or MAJOR finding; two
MINOR findings and one merge-precondition note.

Gate results (from the worktree, the mandated env):
- `cargo fmt --all -- --check`: exit 0.
- `cargo test --locked -p soos-gui -p soos-invariants --all-features -- --test-threads=3`: exit 0, 1123 passed,
  0 failed, 0 ignored.
- `cargo clippy --locked -p soos-gui -p soos-invariants --all-features --all-targets -- -D warnings`: exit 0.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `crates/gui/tests/motion_tests.rs` (new), `tests/invariants/src/gui_motion_contract.rs`
  (new), `tests/invariants/src/lib.rs` (adds `mod gui_motion_contract;` only),
  `tests/invariants/src/presence_unlock_contract.rs` (modified).
- Removed or changed `assert` / `#[test]` / `proptest!` / `#[should_panic]` lines: **none** (the grep for
  removed check lines is empty).
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance): none; the only match is a
  comment in `motion_tests.rs` explaining that the spring settle epsilon is relative to the span.
- Inline `mod tests` changes: none.
- `presence_unlock_contract.rs` (`test_pau_zbus_is_used_only_by_the_daemon`, PAU17): the `ALLOWED` constant
  goes from `[&str; 2]` to `[&str; 3]`, gaining `"crates/gui/Cargo.toml"`; test name and every assertion are
  unchanged, and the loop still asserts each listed manifest contains exactly the workspace zbus line. This is
  Planned Contract Migration B.2a of `AI/tester_contract_motion.md`, approved by the owner on 2026-10-18 and
  recorded in `AI/DECISIONS.md` (ADR 2026-10-18 "Owner-Approved Contract Migration of PAU17..."). Approved,
  not a finding.
- `responsiveness_tests.rs`, `theme_tests.rs`, `brand_layout_tests.rs`: not touched (B.2.1, B.2.7).

## 3. Deep Reasoning Audit

### Logic & Architecture

- Preference resolution (`motion.rs:459`): CLI > `SOOS_REDUCE_MOTION` (exact `1`/`0`, > 8 bytes ignored) >
  portal (`Reduce`/`Pending` reduced, `NoPreference`/`Unavailable` full). Tried: `SOOS_REDUCE_MOTION=0` with the
  portal saying reduce → full (spec §8 item 2); `"1 "`, `"true"`, 9-byte value → ignored. PASS.
- `MotionControl::from_sources` spawns the portal reader only when neither override decides (`motion.rs:731`);
  the app installs `REDUCED` before the first frame (`app.rs:251`) so a reduced-motion user never sees a first
  animation. PASS.
- Portal interpretation (`portal.rs:77`): `reduced-motion` u32 1/0 decides; u32 2, a bool, a string or an
  error fall back to GNOME `enable-animations`; anything else is `Unavailable` (full motion). `Read` wraps one
  extra variant, unwrapped at depth 1 only. A v1 portal answering `UnknownMethod` on `ReadOne` falls back to
  `Read` (both the `FDO` and the raw `MethodError` forms are classified). PASS.
- `PortalReader::poll_at` (`portal.rs:157`): a late answer after the deadline never overrides (`result`
  cached, `rx` dropped); a panicking reader drops `tx` → `Disconnected` → `Unavailable`; a `Pending` sent by
  the thread is mapped to `Unavailable`. PASS.
- Spring (`motion.rs:135`): closed-form damped oscillator (k=400, c=30, m=1, zeta 0.75, underdamped), relative
  settle epsilon, hard snap after `4 * T_SPRING`, non-finite intermediates snap. Tried dt = NaN, ±inf, 10 s,
  target NaN, zero span. PASS (GMO2 tests).
- `cubic_bezier`: Newton with bisection fallback, exact endpoints, NaN → 0. PASS.
- Tab pill (`header.rs:567`): slides only when the active index changes; on a resize it follows at once.
  Entrance (`motion.rs:589`): replays only when the element was not drawn in the previous pass; ids are per
  tab, so a tab change replays the new tab's column. Opacity push/restore pairs in `app.rs` (962/1000,
  1002/1004, 1006/1175, 1404/1430, 1432/1434, 1436/1683, 1704/1713, 1719/1725, 1730/1843) have no early
  `return` between them. PASS.
- Camera status card (`camera_status.rs:150`): the new state's text is drawn on the frame it arrives (G7 as
  amended, the amendment justified in the plan evaluation P-1 and walkthrough 234); opacity starts at 0 and
  reaches 1 within `T_SHORT`. Face overlay (`overlay.rs`) is a verbatim extraction with no interpolation
  (G9). PASS.
- Stat tile value fade (`widgets.rs:307`): see MINOR finding 1.

### PAM Concurrency & Deadlines

- `crates/pam` is not touched. The GUI's only new blocking work happens on the `soos-motion-portal` thread;
  the UI thread only calls `try_recv`. The reader is bounded by `tokio::time::timeout(deadline, …)` plus
  zbus `method_timeout(deadline)`; the UI deadline is independent of the reader. Tried: a bus that never
  answers → `Unavailable` at the deadline (GMO5). PASS.

### Panic Safety & Fail-Closed

- `motion.rs` denies `indexing_slicing`, `unwrap_used`, `expect_used`, `panic`; `portal.rs` uses `let … else`
  and `unwrap_or_default` only; the `MAX_QUEUED_MESSAGES` const avoids `unwrap`. No `unwrap`/`expect` in the
  new production paths of `widgets.rs`, `header.rs`, `camera_status.rs`, `overlay.rs`, `brand.rs`. Thread spawn
  failure → `Unavailable`. A panic on the reader thread is contained (disconnected channel). Motion is purely
  cosmetic and has no path to any authorization decision. PASS.

### Test Integrity & Anti-Weakening

- See section 2. New tests can fail against plausible wrong implementations (overshoot bounds 1–4 %, late
  answer override, spinner presence, overlay colour on the same frame, idle repaint budget). GMO15–GMO19 pin
  the zbus/Tokio confinement statically. PASS.

### Memory, Bounds & Secrets

- `SOOS_REDUCE_MOTION` length bounded (8 bytes); zbus queue bounded (`max_queued(8)`); banner motion state
  hashed into at most 1024 slots; `banner_slot` copies the text only when it changes. No frame, embedding or
  credential is involved; the portal value is never logged (GMO18). No `unsafe` (`#![forbid(unsafe_code)]` in
  both new modules). PASS.

### Supply Chain & Automation

- `soos-gui` adds `tokio` and `zbus` as `workspace = true` dependencies; `Cargo.lock` gains only two
  dependency-edge lines, no new package. The session bus address is environment-derived (documented in the
  security guideline as the exception, unlike the pinned system-bus address of the daemon and remote). No CI,
  script or hook change. PASS.

### English-Only Policy

- Code, comments, docs and walkthrough are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/gui/src/widgets.rs:307` — the stat tile fade key is `(caption, value.is_some(),
  sub.is_some())`. On the Live tab the caption switches between "Match score" and "Liveness (PAD)" whenever
  `live_score` appears or disappears (`app.rs:964`), which can happen at camera rate when the face is lost
  intermittently; each switch restarts the fade at opacity 0, so the value can stay faint and keep requesting
  repaints while detection flickers. Suggested follow-up: skip or shorten the fade when the previous change is
  younger than `T_SHORT`. Cosmetic only.
- **[MINOR]** `crates/gui/src/camera_status.rs:448` — a new camera status (for example an Error) is drawn at
  opacity 0 on its first frame and reaches full opacity after `T_SHORT` (180 ms); the text is never stale, as
  the amended G7 requires, but it is briefly low-contrast. Acceptable under the spec; consider an opacity floor
  for Error severity.
- **[SUGGESTION / merge precondition]** `AI/VERIFICATION_MATRIX.md` UIM7 is still "Pending (owner approval of
  the prototype)", and `AI/tester_contract_motion.md` B.3 says `feat/gui-motion` must not merge before the
  owner approves `Docs/prototypes/ui_motion_prototype.html`; no approval is recorded on #358. This is not a
  defect of the diff, but the PR should not be auto-merged until UIM7 is recorded.

## 5. Final Verdict

No CRITICAL or MAJOR finding. The single pre-existing test change is the owner-approved PAU17 migration.

**VERDICT: APPROVED**
