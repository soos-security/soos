# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `fix/android-alerts-feedback` (PR #422, GitHub #420)
- **Base (merge-base)**: `144d409`
- **Reviewed-Diff-Fingerprint**: `8f8c8e29be60be62c4f4b273c2cdce12bab9508a52a0453bcc2aa792947a2de8`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/227_android_alerts_feedback.md`, `Docs/REMOTE_COMPANION.md`, `android/app/src/main/java/org/soos/remote/alerts/AlertsFeedback.kt`, `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt`, `android/app/src/test/java/org/soos/remote/AlertsFeedbackTest.kt`, `tests/invariants/src/android_alerts_feedback_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

Re-review after the rebase of the branch from `74782b8` onto `origin/main` `144d409`. Main gained a
single commit since the previous base: #426 (`test(gui): deflake the held-store-lock enroll task
test`), which touches only `crates/gui/tests/store_task_tests.rs`, `Docs/GUI_APPLICATION.md`,
walkthrough 226 and its own matrix component. `git range-diff 74782b8..origin/fix/android-alerts-feedback
origin/main..HEAD` shows the fix commit and the independent review commit as identical (`=`); only
the earlier report commit differs, and only because `AI/candid_review_report.md` is diffed against
another base. A line-by-line comparison of the branch's added/removed lines (old base → old tip vs
new base → new tip, report excluded) is identical. The change is an Android-only client fix with
no Rust production code, no PAM, IPC, daemon or dependency change. No finding.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `android/app/src/test/java/org/soos/remote/AlertsFeedbackTest.kt` (new),
  `tests/invariants/src/android_alerts_feedback_contract.rs` (new), `tests/invariants/src/lib.rs`
  (+8 lines: module registration only).
- Removed or changed assertions / `#[test]` / `proptest!` / `#[should_panic]`: none (every removed
  line of the patch is in `SoosApp.kt` production code).
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, tolerance): none.
- Inline `mod tests` changes: none.

No pre-existing test assertion was modified. The #426 test helper change in
`crates/gui/tests/store_task_tests.rs` belongs to main and is not part of this diff.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Interaction with main: #426 is GUI test-only; no shared file, symbol or matrix id with this
  branch. Walkthrough numbering is sequential (226 on main, 227 here), no collision. → PASS
- Matrix: one `android-alerts-feedback` component heading, AAF1–AAF4 each present once; the
  only duplicated table lines across the file are generic header rows. `AI/DECISIONS.md` is not
  touched by the branch. → PASS
- Scenario: successful ack then a user refresh. `refresh()` sets `ackFeedback = null` first and
  `afterRefresh(null, …)` yields no card text, so "Acknowledged." cannot outlive its view. → PASS
- Scenario: `login_required` refusal then signed-out refresh. The carried feedback is promoted to
  the screen message as a fixed `strings.xml` text; other refusals are dropped while signed out;
  unreachable keeps the screen's own message. → PASS
- Scenario: unknown/missing refusal token → `REFUSED` (generic text); the `when` over
  `AckFeedback` in `ackFeedbackText` is exhaustive, so a new enum value fails compilation. → PASS
- The Acknowledge visibility rule (`pending && view.epoch != null`) is unchanged; only a comment
  was added. → PASS

### PAM Concurrency & Deadlines
- No change under `crates/pam`, daemon or IPC. → PASS (not applicable)

### Panic Safety & Fail-Closed
- No Rust production change; the Kotlin helper has no `!!`, no indexing, no throw. An
  acknowledgement refusal never marks alerts acknowledged (the view is replaced only on `Ok`). → PASS

### Test Integrity & Anti-Weakening
- New JVM tests distinguish a plausible wrong implementation (e.g. keeping DONE after a refresh, or
  promoting every refusal while signed out). Invariant tests pin the purity of the helper and the
  `HomeScreen` wiring. Re-run on the rebased tree:
  `cargo test --locked -p soos-invariants -- --test-threads=3` → 774 passed, 0 failed. → PASS

### Memory, Bounds & Secrets
- No new logging, no new data stored; server tokens only select fixed texts (no server string is
  rendered). → PASS

### Supply Chain & Automation
- No `Cargo.*`, Gradle, `deny.toml`, `.github/`, `scripts/` or hook change. → PASS

### English-Only Policy
- All added code, comments, docs and walkthrough text are in English. → PASS

## 4. Detailed Findings & Action Items

None.

## 5. Final Verdict

**VERDICT: APPROVED**
