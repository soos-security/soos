# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `fix/presence-flag-path`
- **Base (merge-base)**: `8f03eb7`
- **Reviewed-Diff-Fingerprint**: `1509ecae0acd25be7b5f36c7b92a5804f24f5813f290103f316488a8e7928c84`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/243_presence_flag_path.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `scripts/setup.sh`, `tests/invariants/src/setup_contract.rs`

## 1. Executive Summary

GitHub #453: `scripts/setup.sh` wrote the presence kill switch at `<sysconfdir>/soos/presence.disable`
and reported `[DONE]`, while soos-daemon (`crates/daemon/src/presence/mod.rs`
`DEFAULT_KILL_SWITCH_DIR = "/etc/soos"`, `PRESENCE_DISABLE_FLAG`, watched by `main.rs:222` with no
path option) and soos-gui (`feature_control.rs` `SOOS_ETC_DIR`) only read
`/etc/soos/presence.disable`. The diff pins `readonly PRESENCE_READ_FLAG`, adds
`presence_flag_elsewhere` / `presence_not_applied` / `presence_requested`, and turns every plan,
phase and summary line into a "not applied" line with the exact `sudo touch` / `sudo rm -f`
command when the two paths differ. The state file, journal, rollback and symlink refusal are
unchanged. The change mirrors the merged lock-widget fix (#452) line for line. No daemon, PAM,
GUI or protocol code is touched. `cargo test -p soos-invariants`: 832 passed, 0 failed (PFP1-PFP4
included); `cargo fmt --check` clean; `bash -n scripts/setup.sh` clean (shellcheck not installed
locally; CI runs it).

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/setup_contract.rs` only (additions only, appended
  after the last existing test).
- Removed/changed assertions (`^-` with `assert|#[test]|...`): **none**.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.

New tests PFP1-PFP4 are additive. Each fails against the base code: PFP1 and PFP3 require the
`readonly PRESENCE_READ_FLAG=...` line; PFP2 requires "not applied" and the command, which base
code never prints; PFP4 requires the new docs/matrix/walkthrough. PFP3 patches the harness copy
of the script only (`h.repo`), not the repository file. No existing test was weakened.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: `--sysconfdir <fakeroot>/etc --presence off` run twice, then plain, then `on`, then
  plain (PFP2). Each branch of `phase_presence` (create, already present, remove, already absent,
  report-only) has an `elsewhere` arm printing `[SKIP] ... not applied ...`; no `[DONE]` path is
  reachable when `PRESENCE_FLAG != PRESENCE_READ_FLAG`. PASS.
- Scenario: dry run with each `--presence` value: `PRESENCE_PLAN_NOTE` is built from
  `presence_requested`, which mirrors the requested or recorded state, so the printed command
  (`touch` for off, `rm -f` for on) matches the request. The `*` plan arm uses `presence_report`,
  which carries its own note. PASS.
- Scenario: default `/etc`: `presence_report` falls through to the unchanged `presence_state`
  (kill-switch, flag, on) and every phase arm keeps the original `[DONE]` text (PFP3). PASS.
- Scenario: `--sysconfdir /etc/` or `//etc`: string comparison reports "not applied" although
  the daemon would read the file. Fails safe (under-reports, never over-reports), identical to
  `DAEMON_READ_CONFIG` and `LOCK_WIDGET_READ_FLAG`, and documented in walkthrough 243. SUGGESTION
  only.
- In the elsewhere case `presence_report` no longer mentions `<sysconfdir>/soos/disabled`; that
  file is also not read by the daemon in that case, so dropping it is accurate. PASS.
- Single source of truth: PFP1 parses the daemon and GUI constants and checks `main.rs` uses
  the constant directory, and scans `crates/*/src`, `packaging`, `scripts` for any new reader.
  `git grep` confirms the only non-test, non-doc sources naming the flag are in the known list.
  PASS.

### PAM Concurrency & Deadlines
- No change under `crates/pam`. N/A, PASS.

### Panic Safety & Fail-Closed
- No Rust production code changes. The script change only alters reporting; no path turns an
  unapplied step into a success claim; the opposite direction (false "not applied") is the safe
  one. PASS.

### Test Integrity & Anti-Weakening
- See section 2. `unwrap`/`expect`/`panic!` in the new test helpers are test-only. The PFP1 scan
  ignores unreadable directories with `continue` but asserts that the daemon file is seen, so an
  empty scan cannot pass vacuously. PASS.

### Memory, Bounds & Secrets
- Messages print only file paths and fixed commands; no credentials, frames or embeddings.
  Flag creation still uses `set -C` (no-clobber) and the symlink refusal precedes the new code;
  mode `0644` unchanged. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or hook change. `scripts/setup.sh` gains no new external
  command or `eval`; all expansions are quoted. PASS.

### English-Only Policy
- Code, comments, messages, ADR amendment, matrix rows and walkthrough 243 are in English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `scripts/setup.sh:1006-1008` — `presence_flag_elsewhere` is a plain string
  comparison, so `--sysconfdir /etc/` (trailing slash) or `//etc` is reported "not applied" even
  though the daemon reads that file. Fail-safe and consistent with `DAEMON_READ_CONFIG`
  (`setup.sh:1336`) and `LOCK_WIDGET_READ_FLAG` (`setup.sh:1061`); a shared normalisation of
  `SYSCONFDIR` (strip trailing and duplicate slashes) could be a follow-up for all three.
- **[SUGGESTION]** `scripts/setup.sh:2389`, `2400`, `2406` — in the elsewhere case the plan status
  column still says "already done" while the detail says "not applied". The detail is correct and
  tested, and it matches the merged lock-widget precedent (`setup.sh:2415`, `2422`, `2426`), but a
  status such as "manual (not applied)" (as used for the camera view at `setup.sh:1673`) would be
  clearer. Changing it later must keep PFP2's "one `[PLAN] presence:` line" contract.

No CRITICAL, MAJOR or MINOR findings.

## 5. Final Verdict

**VERDICT: APPROVED**
