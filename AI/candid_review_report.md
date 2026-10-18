# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `fix/setup-start-bounds`
- **Base (merge-base)**: `1043be6`
- **Reviewed-Diff-Fingerprint**: `3a013c307298b84cef232d93175bf5996c5a9071c1cbc66acf5aa11f75506fc2`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/237_setup_start_bounds.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `scripts/setup.sh`, `tests/invariants/src/setup_contract.rs`

## 1. Executive Summary

Re-review after the rebase of `fix/setup-start-bounds` onto `origin/main` `1043be6`, which gained
#445 (`soos-daemon --check-config`, and `phase_preview` checking the `daemon.toml` it creates before
the restart). The branch moves the six `systemctl start|restart|stop|try-restart "${UNIT}"` calls
of `scripts/setup.sh` from `QUERY_TIMEOUT_S` (10 s) to a new `readonly UNIT_START_TIMEOUT_S=90`, adds
the SET40-SET42 invariant tests, a documentation paragraph and matrix rows SET40-SET43.

The rebase did not change the branch's own lines: for each of the five files, the added/removed
lines of `git diff 3f76090 faa60a3` and `git diff 1043be6 d22485e` are identical (`git range-diff`
shows a context-only difference in the `phase_preview` hunk).

In production, both changes compose correctly in `setup.sh` (check-config bounded by
`QUERY_TIMEOUT_S` first, then the restart bounded by `UNIT_START_TIMEOUT_S`). But the rebased branch
**fails `soos-invariants`**: #445's `daemon_check_config_contract` harness runs the real
`phase_preview` under `set -u` and only defines `QUERY_TIMEOUT_S`; the branch's
`${UNIT_START_TIMEOUT_S}` is unbound there, so the harness aborts right after the check and
`test_dcc8_accepted_file_is_checked_before_the_restart` fails ("no restart"). CI (Quality) will be
red. This is one MAJOR finding: **CHANGES_REQUESTED**.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/setup_contract.rs` only.
- Removed or changed `assert` / `#[test]` / `proptest!` / `#[should_panic]` lines: **none**. The only
  removed lines in the whole patch are the six `run_bounded "${QUERY_TIMEOUT_S}" systemctl ...` lines
  of `scripts/setup.sh`.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.
- Added: helpers `daemon_unit_seconds`, `setup_readonly_seconds`, `scan_shell`,
  `tailscale_in_substitution` and tests SET40 (`test_set_s6_daemon_unit_calls_bounded_by_unit_start_timeout`),
  SET41 (`test_set_s7_no_tailscale_command_inside_command_substitution`), SET42
  (`test_set_target_dir_foreign_owner_refused_before_build`), appended after the last existing test.
  SET41 tightens SET3 additively; SET3 itself is untouched.

No pre-existing test assertion changed. Note that the pre-existing #445 test
`daemon_check_config_contract.rs` is not touched by the branch, but now fails against it (finding 1).

## 3. Deep Reasoning Audit

### Logic & Architecture
- Ordering in `phase_preview` (`scripts/setup.sh:1663-1677`): create the file (`ln`), journal
  `daemon_toml_created`, `run_bounded "${QUERY_TIMEOUT_S}" "${DAEMON_BIN}" --check-config` (#445),
  then `run_bounded "${UNIT_START_TIMEOUT_S}" systemctl restart` (branch). The check runs before any
  restart. PASS.
- Timeouts: one constant per purpose. `--check-config` returns before the runtime, models, camera
  and socket (`crates/daemon/src/check_config.rs`, commit message of #445), so the 10 s query bound
  is the right class for it; the daemon unit calls use `UNIT_START_TIMEOUT_S`, tied to
  `TimeoutStartSec=60` by SET40. No duplicated or contradicting timeout constant. SET40's
  "`QUERY_TIMEOUT_S` never bounds a `systemctl start|...`" rule does not catch the check-config line
  (no `systemctl`), and #445's DCC9 pins that line to `QUERY_TIMEOUT_S`. PASS.
- Rollback on check refusal: the file is removed and the journal entry dropped before
  `return 1`, so `fail_and_rollback` never reaches the `daemon_toml_created` `try-restart`
  (now 90 s); the daemon is not restarted; exit 1. PASS.
- Rollback on restart failure: file removed, entry dropped, a second restart bounded by
  `UNIT_START_TIMEOUT_S`; when that also fails, `PREVIEW_DAEMON_DOWN=true` and the caller exits 70
  (`scripts/setup.sh:1818-1822`); otherwise exit 1 through the rollback. Exit codes 1/70 unchanged
  by the branch. PASS.
- Worst-case bound of the camera-view step grows only by the 10 s (+2 s kill grace) check:
  still finite. PASS.
- Test harness composition: FINDING 1 (below).

### PAM Concurrency & Deadlines
- No change in `crates/pam`. Every new bound is a finite `readonly`. PASS.

### Panic Safety & Fail-Closed
- No production Rust change. A refused or failing check never leads to a restart on the new file
  or to success. PASS.

### Test Integrity & Anti-Weakening
- Step 2 listing reviewed: purely additive. SET40/SET41/SET42 can fail against plausible wrong
  implementations (a call left on `QUERY_TIMEOUT_S`, `"$(tailscale serve)"`, a missing chown
  hint). PASS.
- The rebased tree fails a pre-existing test (DCC8 accepted path). FINDING 1.

### Memory, Bounds & Secrets
- No secret, frame or embedding involved; nothing new logged. PASS.

### Supply Chain & Automation
- Only `scripts/setup.sh` among automation files; no dependency, workflow or hook change. PASS.

### English-Only Policy
- All added text is English. PASS.

## 4. Detailed Findings & Action Items

- **[MAJOR]** `scripts/setup.sh:1670` and `scripts/setup.sh:1674` vs
  `tests/invariants/src/daemon_check_config_contract.rs:164` — the DCC8 harness of #445 runs the
  real `phase_preview` under `set -u` and defines `QUERY_TIMEOUT_S=10` but not
  `UNIT_START_TIMEOUT_S`. After the check succeeds, `${UNIT_START_TIMEOUT_S}` is an unbound
  variable and the non-interactive shell exits, so no `systemctl restart` is logged:
  `cargo test --locked -p soos-invariants` reports `794 passed; 1 failed`
  (`test_dcc8_accepted_file_is_checked_before_the_restart`, panic at line 310, "no restart"). CI
  will fail. Required correction: a setup-only migration of the harness so it defines the new
  bound, preferably from the script itself (e.g. `harness.push_str(&readonly_line(&script,
  "UNIT_START_TIMEOUT_S"))`, as it already does for `DAEMON_PREVIEW_MARKER`), with no assertion of
  DCC8/DCC9 touched; then re-run the full `soos-invariants` suite and a new review.
- **[MINOR]** `AI/walkthroughs/237_setup_start_bounds.md` §1-§2 — stale after the rebase: it says
  the follow-ups were open "on `main` (3f76090)", that "No other test names `QUERY_TIMEOUT_S`"
  (`daemon_check_config_contract.rs:32` and `:164` now do), and the worst-case camera-view bound
  omits the 10 s `--check-config` step. Update the wording with the fix above.
- **[MINOR]** `tests/invariants/src/setup_contract.rs` (`scan_shell`) — carried over from the
  previous review: `$(/usr/bin/tailscale serve ...)` or a variable-named command is not flagged by
  SET41; `setup.sh` contains neither today.
- **[MINOR]** `tests/invariants/src/setup_contract.rs` (SET40) — carried over: the cross checks are
  per line, a `\`-continued call would escape them; none exists (the `bounded >= 6` floor still
  catches a moved call).

## 5. Final Verdict

**VERDICT: CHANGES_REQUESTED**
