# Candid Review Report

- **Date**: 2026-10-20
- **Target Branch**: `feat/gui-motion` (PR #449, GitHub #438, part of #358), rebased onto `origin/main`
- **Base (merge-base)**: `587c73a` (equal to `origin/main` at review time; main gained #436, #463 and #457 since the previous review on `6c4ecd4`)
- **Reviewed-Diff-Fingerprint**: `9ea832072e90daec58c30a7ab7e606d86d03442437fccd39e213b3655c6d2d51`
- **Reviewer**: independent context-free sub-agent (not the author of the branch, of the rebase or of the earlier reviews)
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

Re-review after the rebase of `feat/gui-motion` onto `origin/main` `587c73a` (#436 web motion system,
#463 webpki lockfile, #457 actions bump). Scope: the full raw diff against the new merge base (29 files,
5185 patch lines), with a focus on the hand-resolved conflicts and on semantic interaction with #436.

Rebase content proof (independent of the author's): `git merge-tree --write-tree origin/main
origin/feat/gui-motion` (the pre-rebase reviewed head `9f4ec87`) auto-merges every file except four.
The rebased HEAD `b81d642` is byte-identical to that automatic merge for every other file, and for the
four conflicted files:

- `AI/candid_review_report.md`, `AI/plan_evaluator_report.md`: identical to the branch's pre-rebase
  version (branch-scoped reports; this file is overwritten here).
- `AI/VERIFICATION_MATRIX.md`: the conflict is resolved exactly as described: main's UIM7 (verified,
  owner approval 2026-10-20) and UIM8 (verified, RMO rows), the branch's UIM9 (GMO1–GMO19, GMO21–GMO22);
  then main's `remote-motion` section, `---`, the branch's `gui-motion` section. No marker remains.
- `tests/invariants/src/lib.rs`: main's `mod remote_motion_contract;` kept, the branch's
  `mod gui_motion_contract;` added after it; 97 `mod` lines = 96 on main + 1.

Every UIM (12), RMO (13) and GMO (22) row appears exactly once (47 rows, no duplicate id); each of the
two motion component headings appears once. The test command and clippy are green, and RAC1 confirms
the README GUI captures are unchanged. No CRITICAL, MAJOR or MINOR finding; two optional wording
suggestions.

## 2. Test Changes (mechanical listing from the frozen patch)

- Test files touched: `crates/gui/tests/motion_flicker_tests.rs` (new), `crates/gui/tests/motion_tests.rs`
  (new), `tests/invariants/src/gui_motion_contract.rs` (new), `tests/invariants/src/lib.rs` (one `mod`
  added, nothing removed relative to main), `tests/invariants/src/presence_unlock_contract.rs`.
- Removed or changed assertions (`^-` with `assert`/`#[test]`/`proptest!`/`should_panic`): **none**.
- New escape hatches: none (`#[ignore]`, `#[cfg(any())]`, `should_panic`: none; the only hit is a comment
  stating that a spring settle epsilon is relative to the span, a property under test, not a tolerance
  loosening).
- Inline `mod tests` added or removed: none.
- The only change to a pre-existing test is the PAU17 allowlist in
  `presence_unlock_contract::test_pau_zbus_is_used_only_by_the_daemon`: `ALLOWED` grows from 2 to 3 with
  `crates/gui/Cargo.toml`. This is the owner-approved Planned Contract Migration B.2a (ADR
  "[2026-10-18] Owner-Approved Contract Migration of PAU17"). The assertion body is unchanged; the
  companion `gui_motion_contract` invariants confine the new use.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Tried: a lost or duplicated matrix row or section from the hand resolution → counted rows by id and
  section headings: each exactly once; UIM9 keeps GMO21–GMO22.
- Tried: a semantic clash with #436 in shared docs. `Docs/UI_MOTION.md`: main added a web
  "Implementation" paragraph in §6 and an "As implemented" bullet under reduced motion (web); the branch
  edits only the intro sentence and the G4 and G7 rows of §7. No overlapping or contradictory text; the
  token values are unchanged on both sides (#436 does not touch `theme.rs`), and
  `ui_motion_spec_contract::test_uim_s2_motion_tokens_are_documented_and_consistent` (live GUI cross-check)
  passes against the merged document. The ADR line in `AI/DECISIONS.md` that #436 edited is a different
  entry from the branch's appended PAU17 ADR. `AI/tester_contract_motion.md` gained a "Delivered" note for
  B.1 only; B.2/B.2a are untouched. README: #436 changed only the web capture wording; the branch does not
  touch `README.md`, and no README sentence claims GUI motion exists. → PASS.
- Tried: `Cargo.lock` interaction with #463 → the branch adds only `tokio` and `zbus` to the
  `soos-gui` dependency list; no new package enters the lockfile; the `--locked` build succeeds. → PASS.

### PAM Concurrency & Deadlines
- The PAM crate is not touched. The GUI portal reader: one named thread, its own current-thread runtime,
  `zbus::connection::Builder::session()` only (no system bus), `method_timeout(deadline)`, the whole read
  wrapped in `tokio::time::timeout(deadline, …)`, and the UI thread enforces the deadline itself in
  `PortalReader::poll_at` (`try_recv` only, `Unavailable` from `deadline_at` on).
  `theme::PORTAL_READ_DEADLINE_MS = 200`. No `object_server`, `serve_at`, `request_name` or match rule in
  `crates/gui/src`. → PASS.

### Panic Safety & Fail-Closed
- Tried: a reader thread that panics or never answers → the channel disconnects or the deadline passes,
  both map to `Unavailable` (full motion), never block, never retry. Spawn failure → `Unavailable`. No
  security decision depends on motion; camera Warning/Error states are drawn at full opacity on arrival
  (GMO22). → PASS.

### Test Integrity & Anti-Weakening
- See §2: no pre-existing assertion changed except PAU17 (approved). The rebase did not alter any test
  file of the branch (content proof above). → PASS.

### Memory, Bounds & Secrets
- Connection queue bounded (`max_queued` 8), sync channel of 1, variant unwrapping depth-bounded, the
  portal value never logged (pinned by `gui_motion_contract`). No frame or embedding is touched by motion
  (the face overlay is explicitly not animated, G9). → PASS.

### Supply Chain & Automation
- `zbus`/`tokio` via `workspace = true`, already locked; no new crate, no `.github/` or script change in
  this diff (#457 is main's). The security guideline sentence names the read-only portal exception.
  → PASS.

### English-Only Policy
- All added text in the diff is English. → PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md` `gui-motion` section heading — it still reads "not merged
  before the owner approves the prototype, UIM7", while UIM7 is now verified on main; main's
  `remote-motion` heading has the same wording, so this is consistent and harmless. Optional refresh in a
  later documentation pass.
- **[SUGGESTION]** `Docs/GUI_APPLICATION.md` §1 tour bullet "Motion: no animation yet … the hook the
  motion system of #358 / #377 will use" — pre-existing text, unchanged by the rebase; the tour is not in
  the G1–G10 table, so it is not wrong, but the future tense could be clarified once the motion system is
  on main.

README GUI captures (`Docs/assets/screenshots/gui-*.png`, last changed in `a3cae16`, #392): still current.
`test_rac_capture_scenes_match_committed_assets` (RAC1) passes in the test command below.

Validation run on the reviewed tree:
- `LIBONNXRUNTIME_NO_PKG_CONFIG=1 … cargo test --locked -p soos-gui -p soos-invariants --all-features --
  --test-threads=3`: 40 test binaries, 1200 passed, 0 failed, 0 ignored.
- `cargo clippy --locked -p soos-gui -p soos-invariants --all-features --all-targets -- -D warnings`: clean.
- `cargo fmt --all -- --check`: clean.

## 5. Final Verdict

**VERDICT: APPROVED**
