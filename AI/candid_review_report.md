# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `fix/lockwidget-flag-path`
- **Base (merge-base)**: `ed0bb84`
- **Reviewed-Diff-Fingerprint**: `e1f60bda296c34093e1fda9551c3303ffd43ecf293083c6a13c5c6375c46decc`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/242_lockwidget_flag_path.md`, `Docs/LOCKSCREEN_WIDGET.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `scripts/setup.sh`, `scripts/uninstall.sh`, `tests/invariants/src/lockscreen_installer_contract.rs`

## 1. Executive Summary

The diff implements GitHub #451. `scripts/setup.sh` pins `LOCK_WIDGET_READ_FLAG="/etc/soos/lockwidget.disable"` (the GNOME
extension's `KILL_SWITCH_PATH`). When `<sysconfdir>/soos/lockwidget.disable` is a different path, the flag is still written,
journaled and recorded, so LKI6 keeps passing unchanged. In that case every lock-widget plan, phase and summary line is
downgraded from `[DONE]` to `[SKIP]` and says "not applied", with the exact `sudo touch` or `sudo rm -f` command for
`/etc/soos/lockwidget.disable`. The GNOME success line now mentions the log-out. `scripts/uninstall.sh` no longer follows a
symbolic link in place of `<prefix>/share/soos`. The docs, ADR amendment, matrix rows LWF1–LWF6 and walkthrough 242 are
included. `bash -n` passes on both scripts. `cargo test --locked -p soos-invariants` reports 828 passed and 0 failed.
There are no CRITICAL or MAJOR findings.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:
- Test files touched: `tests/invariants/src/lockscreen_installer_contract.rs`, additions only (+277, 0 removed lines).
- Removed or changed assertions (`^-` with assert/#[test]/proptest/should_panic): **none**.
- New escape hatches (`#[ignore]`, `cfg(any())`, `should_panic`, tolerance/epsilon): **none**.
- Inline `mod tests` changes: **none**.

The six new tests are LWF1–LWF6, each mapped to a scope line of #451. LKI6 is untouched. Its needles
`"[PLAN] lock widget: already done - on (default"` and `"already done - off"` remain prefixes of the new plan text, and LKI6
asserts no `[DONE] lock widget` line, so it still holds under the fake `--sysconfdir`.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: run `--lock-widget off` with a fake sysconfdir twice, then a plain run, `on`, and a plain run. Every path in
  `phase_lock_widget` (off/new, off/existing, on/existing, on/absent, keep) branches on `elsewhere`, and none prints
  `[DONE]`. The plan block appends `LW_PLAN_NOTE` on the explicit branches and goes through `lock_widget_report` on the keep
  branch. The summary line (`Lock widget: $(lock_widget_report)`) inherits the note. **PASS**.
- Scenario: `/etc` is the real path. `lock_widget_flag_elsewhere` is false, so the output is byte-identical to before.
  LWF3 checks this by pointing the read path at the fake root. **PASS**.
- Scenario: a non-canonical spelling of `/etc` (a symlinked sysconfdir, a doubled slash). The plain string comparison then
  under-reports as "not applied". This is the safe direction, and it is the same rule as `DAEMON_READ_CONFIG`. **PASS**.
- Scenario: the GNOME enable and adapter steps when the flag is off but elsewhere. The plan line says no adapter is enabled
  and that the GNOME card was "not applied", which is truthful. **PASS**.
- Scope matches the issue: no PAM, daemon, protocol, lockstatus, lock-widget or extension change, and `presence.disable` is
  left out as stated.

### PAM Concurrency & Deadlines
- No `crates/pam` change. **N/A / PASS**.

### Panic Safety & Fail-Closed
- No Rust production change. In the shell, `cond && var=...` is exempt from `set -e`. Return codes of `rm`/`chmod` still
  feed `fail_and_rollback`, and the journal entries (`lock_widget_flag_created` / `_removed`) are unchanged, so rollback
  still reverses the flag. **PASS**.

### Test Integrity & Anti-Weakening
- No assertion removed. The new tests would fail against plausible wrong implementations:
  - a `[DONE]` line kept in any branch;
  - a missing command;
  - a missing plan note;
  - uninstall following the link (LWF5 checks that the content behind the link and the link itself survive).
  **PASS**.

### Memory, Bounds & Secrets
- No secrets or biometric data are printed. Uninstall now refuses a linked `share/soos` and a linked `share/soos/pam` before
  any `rm`, and the `rmdir` calls are skipped when the directory is a link. **PASS**.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or hook change. **PASS**.

### English-Only Policy
- All added text is English. **PASS**.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `scripts/setup.sh:1140` — `print_lockscreen_lines` warns "not applied" only for explicit `on` or the
  off state. A plain run with no flag elsewhere relies on the summary `Lock widget:` line (line 2102), which does carry the
  note. This is acceptable as is. An optional follow-up is to also warn when `LOCK_WIDGET` is empty, so the lock-screen
  block is self-contained.

## 5. Final Verdict

**VERDICT: APPROVED**
