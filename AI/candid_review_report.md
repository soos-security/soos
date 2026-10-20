# Candid Review Report

- **Date**: 2026-10-20
- **Target Branch**: `feat/gui-motion` (PR #449, GitHub #438, part of #358)
- **Base (merge-base)**: `6c4ecd40`
- **Reviewed-Diff-Fingerprint**: `4d1314d51f778ae4113103691d62d81734a500500ec92cf8c9a70e5e34092ca7`
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

Full branch diff reviewed, with focus on the latest commit `cd58559` (the two MINOR follow-ups of the previous
review of PR #449). `motion::fade_in_on_stable_change` correctly stops a camera-rate caption flicker from
restarting the stat tile fade: a fade starts only after the stable hold (2 x `T_SHORT`), a change during a running
fade keeps the running fade going (`elapsed` keeps growing, it is never reset), and a change after the fade but before
the hold is drawn at once. `render_status_banner` now forces opacity 1.0 and the severity's own title colour
for Warning/Error on their arrival frame. Docs (UI_MOTION G4/G7, GUI_APPLICATION), matrix rows GMO21–GMO22 and
walkthrough 234 match the code. The PAU17 change is only the `ALLOWED` list (owner-approved Planned
Contract Migration B.2a). No CRITICAL or MAJOR finding.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `crates/gui/tests/motion_flicker_tests.rs` (new, GMO21–GMO22), `crates/gui/tests/motion_tests.rs`
  (new in this branch, GMO1–GMO19), `tests/invariants/src/gui_motion_contract.rs` (new), `tests/invariants/src/lib.rs`
  (module registration), `tests/invariants/src/presence_unlock_contract.rs` (PAU17).
- Removed or changed assertion / `#[test]` lines (`^-` grep): **none**.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance): none. The only hit is a comment in `motion_tests.rs`
  ("the epsilon is relative to the span"), which describes production spring-settling behaviour, not a test tolerance.
- Inline `mod tests` added/removed: none.
- PAU17 (`test_pau_zbus_is_used_only_by_the_daemon`): `ALLOWED` grows from 2 to 3 entries (`crates/gui/Cargo.toml`).
  The loop body and its assertions are byte-identical. This is Planned Contract Migration B.2a of
  `AI/tester_contract_motion.md` (file exists), recorded in `AI/DECISIONS.md` (ADR 2026-10-18 "Owner-Approved Contract
  Migration of PAU17 for the `soos-gui` Portal Reader"). The new `gui_motion_contract.rs` invariant confines that
  zbus use to the read-only session-bus portal reader. Justified, no weakening.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Flicker every 1/2/3/6/12 frames after a stable period: the first flip sees `held == hold` and starts one fade.
  Later flips see `held < hold`, so `elapsed` is not reset, and it reaches `duration` within `T_SHORT`. After that
  the function returns 1.0 without `request_repaint`. → PASS.
- A change right after the fade ends (`held < hold`): drawn at once. After a hold of 2 x `T_SHORT` it fades again. → PASS.
- `duration` 0 / NaN / negative: clamped to 0, `hold = 0`, returns 1.0. → PASS.
- Reduced motion: `elapsed` is forced to `duration`, so every change is at rest. → PASS.
- Shared slot id between tiles: each tab draws exactly one `stat_tile` per `ui` (`app.rs:973–997`, `1418–1427`,
  `1705`), so no two tiles alternate on one slot within a pass. → PASS.
- Banner: Warning/Error take `fade = 1.0`, so `mix_color(prev, color, 1.0) == color`. Info after an Error still
  cross-fades from DANGER, and the previous text is never kept on screen. → PASS.
- First sight counts as stable (`held` initialised to `hold`): a change one frame after first sight fades once. This
  is harmless but not exactly what the doc says ("previous content on screen for 2 x `T_SHORT`"). → SUGGESTION 1.
- Warning/Error: `fade_in_on_change` still runs before the override and requests repaints for `T_SHORT`. These
  repaints are wasted but bounded. → SUGGESTION 2.

### PAM Concurrency & Deadlines
- `crates/pam` is untouched. The GUI portal reader runs Tokio only on its own named background thread, under
  `tokio::time::timeout(deadline)` and zbus `method_timeout(deadline)`. The UI thread polls the result with
  `try_recv` and enforces the deadline itself. → PASS (not a PAM path).

### Panic Safety & Fail-Closed
- The new code has no `unwrap`/`expect`/indexing. `ctx.data_mut` slot access is total. A portal spawn failure, a
  disconnect or the deadline all give `Unavailable` (the default motion preference applies). No authentication path is
  touched. → PASS.

### Test Integrity & Anti-Weakening
- The GMO21 flicker test fails against the previous `fade_in_on_change` wiring: period 1 restarts the fade, so the
  opacity drops and `o >= last` fails. The GMO21 tile wiring test fails if `stat_tile` keeps the old helper. GMO22
  fails against the old banner, because an Error had `opacity_factor < 1` on its arrival frame. All of these tests
  can catch a plausible wrong implementation. → PASS.

### Memory, Bounds & Secrets
- Each slot is a fixed-size `Copy` struct in egui temp data. The portal connection queue is bounded (8), and the
  value is validated and never logged. No frames, embeddings or credentials are involved. → PASS.

### Supply Chain & Automation
- `zbus` uses the workspace pin (`{ workspace = true }`). The `Cargo.lock` delta is that dependency only. No
  `.github/`, `deny.toml` or `scripts/` change. → PASS.

### English-Only Policy
- Code, comments, docs, walkthrough and commit messages are in English. → PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/gui/src/motion.rs:690` — `held` is initialised to `hold`, so first sight counts as
  stable and a change on the very next frame fades in. Either say so in the doc comment and in `Docs/UI_MOTION.md` G4
  ("first sight counts as stable"), or initialise `held` to 0 if the hold should also apply to first sight.
- **[SUGGESTION]** `crates/gui/src/camera_status.rs:169` — for Warning/Error, `fade_in_on_change` still requests
  repaints for `T_SHORT` even though its value is discarded. Only call it for Info, or accept the bounded cost.

## 5. Final Verdict

**VERDICT: APPROVED**
