# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `chore/companion-wording` (PR #424, after the rebase onto `origin/main`)
- **Base (merge-base)**: `3a5ab6a` (`feat(android): add the first-run feature tour to the native app (#428)`)
- **Reviewed-Diff-Fingerprint**: `65faef9c79fac98d17f62d26879ebdd67562c05e6b628f57a0018ca904966cb1`
- **Audited Files**: `AI/ARCHITECTURE.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/architect_spec_companion_wording.md`, `AI/auditor_constraints_companion_wording.md`,
  `AI/walkthroughs/228_companion_wording.md`, `Docs/PACKAGING_AND_PROVISIONING.md`,
  `Docs/REMOTE_COMPANION.md`, `README.md`, `scripts/setup.sh`,
  `tests/invariants/src/companion_wording_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

Independent re-review after the branch was rebased from `74782b8` onto `3a5ab6a`, which adds
#426 (GUI test deflake), #422 (Android alerts acknowledgement feedback) and #428 (native Android
feature tour). The change stays wording only: the `setup.sh` help paragraph and closing summary
line, the packaging defaults sentence, the `allow_funnel` example comment, two `AI/ARCHITECTURE.md`
row headers and the README security sentence now match the setup defaults and the real `soos-gui`
switches; a new std-only invariant module (CWD1–CWD5) pins that wording.

Rebase integrity, checked mechanically:

- `git range-diff 74782b8..origin/chore/companion-wording 3a5ab6a..HEAD`: the content commit
  differs only in context lines (the matrix tail now ends with main's ATR rows, and the
  `plan_evaluator_report.md` it replaces is now #428's instead of #409's).
- For every file the branch touches, the sequence of added and removed lines of
  `git diff -U0 74782b8 <old tip>` equals that of `git diff -U0 3a5ab6a 5417839` (all files
  except the excluded plan evaluator report, whose content is byte-identical to the old tip).
- The branch files not touched by main (`ARCHITECTURE.md`, spec, constraints, walkthrough 228,
  packaging doc, `README.md`, `setup.sh`, the contract module, `plan_evaluator_report.md`) are
  byte-identical between the old and new tips. The only conflict was the matrix tail.

No claim of this branch is contradicted by #428 or #422 (see Logic & Architecture). No finding at
MAJOR or above.

## 2. Test Changes

Mechanical listing on `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/companion_wording_contract.rs` (new, 5 tests) and
  `tests/invariants/src/lib.rs` (one `mod companion_wording_contract;` declaration added, nothing
  removed).
- Removed or changed checks (`^-.*assert|#[test]...`): one hit, patch line 17, the removed
  `AI/ARCHITECTURE.md` "Unlock (opt-in)" row, which matches only the word "assertion" in prose.
  It is not a test. No pre-existing assertion, test or test module changed.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): none.
- Inline test modules (`mod tests`): none touched.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Claims checked against the code: `RemoteSwitch::key` (`crates/gui/src/feature_control.rs`)
  maps Unlock, Funnel, Push, CameraView and OfflineAlerts only, with no `password_alerts` arm, so
  the setup text "the failed-password alerts have no switch there" is true. `keys_to_clear` turns
  off `camera_view_funnel` with Funnel or the camera view and `offline_alerts` with Push, matching
  the README "also sets ... to `false`" sentence and the setup "also turns off the features that
  need them".
- Conflict with #428 (native Android tour): no branch-added line says any app has no tour. The
  only branch line naming a tour is the README "feature switches of the first-run tour" in
  `soos-gui`, which is pre-existing text rewrapped and names the `soos-gui` tour. #428's
  `strings.xml` and `app.js` failed-password step say to turn the alerts off in `remote.toml`, not
  in `soos-gui`, which agrees with this branch. `android_tour_contract` ATR8 (§2k no longer says the
  app has no tour) passes on the rebased tree.
- Conflict with #422 (alerts acknowledgement feedback): the branch says nothing about the
  acknowledgement flow; no overlap.
- The spec's out-of-scope note ("the Android app's own opt-in heartbeat wording") stays accurate:
  #428 still calls the PC offline alerts the app's own opt-in check.
- Matrix: one `companion-wording` component header, CWD1–CWD5 once each, no duplicate row id in
  the whole file, main's ATR1–ATR9 and AAF1–AAF4 present once and unchanged, the new component
  appended after ATR9.

### PAM Concurrency & Deadlines

No `crates/pam` change. PASS (not applicable).

### Panic Safety & Fail-Closed

No production Rust change. The only shell change is text inside the `usage()` heredoc and one
double-quoted `info` string: no `$`, backtick or unescaped quote added, so `--help` does no
expansion. PASS.

### Test Integrity & Anti-Weakening

The CWD tests read the real files and would fail on the old wording (walkthrough 228 records each
red run; CWD5 checks the detector on synthetic old and new text). CWD2 ties the setup claim to
the GUI code, so a future `password_alerts` switch breaks the test instead of leaving the help
wrong. No existing test weakened. PASS.

### Memory, Bounds & Secrets

No runtime code, no new log line, no credential or biometric data. PASS.

### Supply Chain & Automation

`scripts/setup.sh` changes text only; no new command, dependency, workflow or action. PASS.

### English-Only Policy

All added text is English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `README.md:297` — with three tours now (`soos-gui`, web app, native Android
  app after #428), "the feature switches of the first-run tour" could say "the `soos-gui`
  first-run tour" for clarity. The sentence already says "in `soos-gui`", so it is not wrong.
- **[SUGGESTION]** `README.md:296` — "offline alerts" here are the server `offline_alerts` key;
  the Android app's own "PC offline alerts" switch (in the app's Settings) is a different feature.
  An optional follow-up could name both to avoid confusion.

## 5. Final Verdict

Tests: `cargo test --locked -p soos-invariants -- --test-threads=4` on the rebased tree, 787
passed, 0 failed, 0 ignored.

**VERDICT: APPROVED**
