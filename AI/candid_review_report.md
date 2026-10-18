# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `chore/companion-wording` (PR #424, after the rebase onto `origin/main`)
- **Base (merge-base)**: `144d409` (`test(gui): deflake the held-store-lock enroll task test (#426)`)
- **Reviewed-Diff-Fingerprint**: `a5e3984f9959db07e6f4d364a8e913da576411fc30b804dfe6126f29132850c4`
- **Audited Files**: `AI/ARCHITECTURE.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/architect_spec_companion_wording.md`, `AI/auditor_constraints_companion_wording.md`,
  `AI/walkthroughs/228_companion_wording.md`, `Docs/PACKAGING_AND_PROVISIONING.md`,
  `Docs/REMOTE_COMPANION.md`, `README.md`, `scripts/setup.sh`,
  `tests/invariants/src/companion_wording_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

Independent re-review after the branch was rebased from `74782b8` onto `144d409`. The change is
wording only: the `setup.sh` help paragraph and closing summary line, two doc sentences, two
`AI/ARCHITECTURE.md` row headers and the README security sentence now match the setup defaults and
the real `soos-gui` switches; a new std-only invariant module (CWD1–CWD5) pins that wording.

Rebase integrity: `git range-diff 74782b8..cd6a34b 144d409..HEAD` shows the content commit
`8d26a97` differing from `90b4cce` only in the matrix hunk context (the new tail rows DGS1/DGS2 of
#426 instead of PTR9–PTR11); the review-record commits differ only in
`AI/candid_review_report.md`. A line-by-line comparison of every added and removed line of
`git diff 74782b8 cd6a34b` and `git diff 144d409 HEAD` (report excluded) is identical (empty
`diff`). The branch's own changes are therefore unchanged by the rebase.

Interaction with what main gained since the old base (#426 only: `crates/gui/tests/store_task_tests.rs`,
`Docs/GUI_APPLICATION.md`, walkthrough 226, matrix DGS1/DGS2): no file of #426 is touched by this
branch except the matrix, where the new `companion-wording` component is appended after the DGS
rows; the CWD2 premise (`RemoteSwitch::key` in `crates/gui/src/feature_control.rs`) is unaffected
by #426, which changes only a test helper. The other open PRs of the train (#422 walkthrough 227,
#428) use different walkthrough numbers; no collision with 228.

No CRITICAL, MAJOR or MINOR finding.

## 2. Test Changes

Mechanical listing on the frozen patch:

- Test files touched: `tests/invariants/src/companion_wording_contract.rs` (new file) and
  `tests/invariants/src/lib.rs` (one new `#[cfg(all(test, unix))] mod companion_wording_contract;`
  with its doc comment, pure addition).
- Removed/changed checks (`^-` with assert/test markers): one hit, line 17, the removed
  `| Unlock (opt-in) |` row of `AI/ARCHITECTURE.md`, matched only because its text contains the
  word "assertion". It is documentation, not a test. No test assertion of `origin/main` is removed
  or changed.
- New escape hatches (`#[ignore]`, `cfg(any())`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Claim check against code at `144d409`: `RemoteSwitch::key` maps exactly `allow_unlock`,
  `allow_funnel`, `push_notifications`, `camera_view`, `offline_alerts` (no `password_alerts`), so
  "the failed-password alerts have no switch there" is true. `keys_to_clear` clears
  `camera_view_funnel` with Funnel or camera view and `offline_alerts` with Push, matching the new
  README sentence and the help's "also turns off the features that need them". `FEATURE_PREREQS`
  in `setup.sh` (`push_notifications` needs `password_alerts`, `offline_alerts` needs
  `push_notifications`) backs "set password_alerts, push_notifications and offline_alerts to false"
  and `--password-alerts off` cascading. PASS.
- The ARCHITECTURE Unlock cell keeps `default \`false\``; only the two header cells change. PASS.
- Tried to break CWD1: a future sentence such as "every feature can be switched off in soos-gui"
  is flagged; one with "except" or "no switch" is accepted by design (CWD5 covers both). The unit
  splitter splits on ". " and ";", so the old wrapped heredoc sentence is still flagged (CWD5 last
  assertion). PASS.
- Tried to break CWD3: a second `| Unlock` row would fail the exactly-one assertion rather than
  silently pass. PASS.

### PAM Concurrency & Deadlines
- No file under `crates/pam` or any runtime crate changes. Not applicable. PASS.

### Panic Safety & Fail-Closed
- No production Rust changes; the only Rust is a test module with an explicit, reasoned
  `#![allow(clippy::unwrap_used, ...)]` like the sibling contract modules. PASS.

### Test Integrity & Anti-Weakening
- New tests are red against the old wording (CWD1, CWD3, CWD4 red on `74782b8` per the
  walkthrough; consistent with the needles, e.g. the old summary line lacks `--password-alerts off`).
  CWD2 fails on purpose if a `password_alerts` switch is ever added. No existing assertion changed.
- Re-run: `LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_TARGET_DIR=~/.cache/soos-targets/companion-wording
  CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=3
  cargo test --locked -p soos-invariants -- --test-threads=3` on the rebased tree: 776 passed,
  0 failed, 0 ignored; all five `companion_wording_contract` tests pass; the setup (SET*) and README
  (RDM*) contracts stay green. PASS.
- `AI/VERIFICATION_MATRIX.md`: one `companion-wording` component heading, CWD1–CWD5 each appear
  once; no duplicated row id in the matrix. `AI/DECISIONS.md` is not touched by this branch (no
  ADR, wording only) and has no duplicated heading. PASS.

### Memory, Bounds & Secrets
- No allocation, I/O or secret-handling code changes; the new text names no password, identity or
  key. PASS.

### Supply Chain & Automation
- `scripts/setup.sh` changes only text: the unquoted `usage()` heredoc gains no `$`, backtick or
  `$(...)`; the closing `info` string stays inside double quotes with no unescaped `"`.
  `bash -n scripts/setup.sh` passes and `--help` renders the new paragraph. No `Cargo.*`,
  `deny.toml`, `.github/` or `.githooks/` change. PASS.

### English-Only Policy
- All new text, identifiers and commit subjects are English. PASS.

## 4. Detailed Findings & Action Items

- None.

## 5. Final Verdict

**VERDICT: APPROVED**
