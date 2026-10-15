# Candid Review Report

- **Date**: 2026-10-15
- **Target Branch**: `fix/post-merge-followups` (GitHub #398)
- **Base (merge-base)**: `640e1db`
- **Reviewed-Diff-Fingerprint**: `427709d9bf0f04164ad50116e4196c04e1fe3800b0f3a770990a5e8de1df1b00`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_post_merge_followups.md`, `AI/auditor_constraints_post_merge_followups.md`, `AI/walkthroughs/212_post_merge_followups.md`, `Docs/GUI_APPLICATION.md`, `Docs/README_ASSETS.md`, `Docs/REMOTE_COMPANION.md`, `crates/gui/src/app.rs`, `crates/gui/src/feature_control.rs`, `crates/gui/tests/feature_restart_check_tests.rs`, `scripts/android_build.sh`, `tests/fixtures/mod.rs`, `tests/fixtures/tests/readme_fixture_tests.rs`, `tests/invariants/src/android_native_contract.rs`

## 1. Executive Summary

The diff closes the three follow-ups of #398: (1) `soos-gui` confirms that `soos-remote` survives the
`try-restart` after a `remote.toml` edit through a bounded state probe plus polls (`RestartCheck`,
`SystemFeatureExecutor::for_desktop`); (2) `scripts/android_build.sh` `check_permissions` now parses
both `uses-permission:` and `uses-permission-sdk-23:` lines against the unchanged allowlist and fails
closed on any other `uses-permission*` line; (3) the README web fixture serves `GET /api/offline`.
No PAM, daemon, IPC or biometric code is touched. No test or assertion was removed or weakened.
Targeted runs were green: `feature_restart_check_tests` (18), `feature_control_tests` (43),
`readme_fixture_tests::test_rac_web_fixture_serves_the_offline_history`,
`soos-invariants test_ana_s13`. The cited CI run ids were checked with `gh run view`
(38446125267 failure at 8f629a0, 38772439975 success at d54e4bd, 38773360400 success on `main` at 3f10f61).
Only non-blocking suggestions remain.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `crates/gui/tests/feature_restart_check_tests.rs` (new),
  `tests/fixtures/tests/readme_fixture_tests.rs` (additive: helpers + one new test),
  `tests/fixtures/mod.rs` (fixture route, additive), `tests/invariants/src/android_native_contract.rs`
  (additive: helpers + one new test).
- Removed/changed assertions (`^-.*assert|#[test]|...`): **none**.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.
- Removed lines in the whole patch (12): two `AI/VERIFICATION_MATRIX.md` rows (ANA11, ANA13) updated
  to record CI evidence (justified by #398 item 2, run ids verified), two Docs sentences extended, and
  production lines replaced in `app.rs`, `feature_control.rs` (`new` body, restart `map_err` gaining `?`)
  and `android_build.sh` (`sed` pipeline replaced). None is a test.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Probe `inactive`/`failed`/`deactivating` → no poll: matches systemd `try-restart` collapsing to a
  no-op for inactive-or-deactivating units. PASS.
- Probe unknown (failed call, non-zero exit even with `inactive` stdout) → treated as maybe running and
  polled; a stopped unit is then reported as stopped. Conservative and tested. PASS.
- Unit failing with exit 78 after `Type=exec` start (`RestartPreventExitStatus=78` → `failed`) is caught
  at the first failing poll and polling stops; runtime exit (`SubState=auto-restart…`, any
  `ActiveState`) reported as `restarting`. PASS.
- Last poll not `active` (activating, deactivating, garbage, call error) → "not confirmed running";
  late `active` after `activating` confirmed. PASS.
- `polls = 0` clamps to 1, huge values to 10, interval to 1 s, call timeout to 3 s
  (`USER_SYSTEMCTL_TIMEOUT`). The default after-restart cost is 0.5 + 3 × 1 s, matching the issue's
  "about 3 s"; total worst case 6.5 s as documented. PASS.
- No-op edit runs no command; restart failure skips the polls and keeps the original message. PASS.
- `SystemFeatureExecutor::new` keeps its exact call sequence (existing `feature_control_tests` green);
  the only production construction is `for_desktop` in `app.rs`, guarded by a source-scan test. PASS.
- Permission parser: regex anchored at line start, exact-name allowlist comparison, trailing attributes
  (`maxSdkVersion`) tolerated, empty name / unknown `uses-permission-sdk-m` / indented variants fail
  closed, `fail` exits from the main shell (here-string loop, no subshell). `uses-implied-permission`
  stays out of scope (justified in walkthrough 212: `targetSdk` 36 implies none). PASS.
- Fixture `/api/offline` keys and enum values are checked against `crates/remote/src/offline.rs`
  sources (`OfflineView`, `OfflineInterval`, snake_case variants). GET only, like every fixture route
  (the real server also answers HEAD; the fixture already refuses HEAD for all routes). PASS.

### PAM Concurrency & Deadlines
- `crates/pam` untouched. The new GUI sleeps and calls run on the existing `soos-gui-features` worker
  thread (`FeatureRunner::submit`), never on the UI thread; the runner never joins it on drop, so
  closing the GUI is not stalled. Every `systemctl` call carries a deadline. PASS.

### Panic Safety & Fail-Closed
- No `unwrap/expect/panic!`/indexing in the new production code; parse errors map to `Unknown`.
  A confirmation failure is reported as an error, never as success. The permission check fails closed
  on unparseable lines. PASS.

### Test Integrity & Anti-Weakening
- New executor tests would fail against plausible wrong implementations (no probe, polling a stopped
  unit, parsing failed-call output, unclamped interval — timed test, missing sleep — timed test,
  leaking `systemctl` output). The ANA17 test executes the script's own `check_permissions` block
  against a fake `aapt2`, so a regex-only change cannot pass without real behaviour. PASS.

### Memory, Bounds & Secrets
- State reads are capped at `MAX_UNIT_SHOW_BYTES`; messages carry only fixed state words (marker test).
  Fixture bodies are synthetic and identity-free (forbidden-token and IPv4 scan). PASS.

### Supply Chain & Automation
- `scripts/android_build.sh`: no new dependency, download or permission change; allowlist unchanged.
  No `Cargo.*`, `deny.toml` or `.github/` change. PASS.

### English-Only Policy
- All added code, comments, docs and AI files are English (scanned for non-ASCII French and common
  French words: none). PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `tests/invariants/src/android_native_contract.rs:830` — the ANA17 harness uses a
  predictable directory `$TMPDIR/soos-ana17-<pid>-<case>` with `remove_dir_all` + `create_dir_all`
  instead of `tempfile::tempdir()`, and leaves it behind when an assertion panics before cleanup. Test
  only, no production impact; a `tempfile` dev-dependency (if acceptable for `soos-invariants`) would
  make it race-free and self-cleaning.
- **[SUGGESTION]** `tests/fixtures/tests/readme_fixture_tests.rs` (helpers `interval_objects`,
  `string_field`) — ad-hoc JSON slicing (`split("},{")`) is tied to the current flat fixture shape;
  parsing with `serde_json` (if already available to the crate) would be more robust to future fields.
- **[SUGGESTION]** `crates/gui/src/feature_control.rs:873` — a unit observed `deactivating` during
  polling is only reported at the last poll as "not confirmed running"; acceptable, but it could be
  reported as stopping at once. Optional.

## 5. Final Verdict

No CRITICAL, MAJOR or MINOR finding. The three acceptance items of #398 are implemented, tested and
documented; no test was weakened.

**VERDICT: APPROVED**
