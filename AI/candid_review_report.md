# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `feat/lockscreen-installer`
- **Base (merge-base)**: `d543bbd`
- **Reviewed-Diff-Fingerprint**: `7fecdff83626311a99153303e54ff4ebb58e6c2e1e30b1286e528114c19dcd6d`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_lockscreen_installer.md`, `AI/walkthroughs/233_lockscreen_installer.md`, `Docs/LOCKSCREEN_WIDGET.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `packaging/arch/PKGBUILD`, `packaging/rpm/soos.spec`, `scripts/install.sh`, `scripts/setup.sh`, `scripts/uninstall.sh`, `tests/invariants/src/lib.rs`, `tests/invariants/src/lockscreen_installer_contract.rs`

## 1. Executive Summary

Re-review after the rebase of `feat/lockscreen-installer` (GitHub #432, PR 6 of #352) from merge-base `3f76090` onto `origin/main` `d543bbd`, which added #445 (`soos-daemon --check-config`, setup validation of the `daemon.toml` it creates, DCC1–DCC11) and #440 (setup daemon start bounds of 90 s, SET40–SET42). The branch's own change is unchanged by the rebase: for every file except `AI/VERIFICATION_MATRIX.md`, the sorted set of added and removed lines of `git diff 3f76090 7baf609` (pre-rebase) equals that of `git diff d543bbd b375e31` (post-rebase); the matrix differs only by a blank line and a `---` line that now separate the appended `lockscreen-installer` section from the `daemon-check-config` section (cosmetic, consistent with the rest of the file). No semantic conflict with #440 or #445 was found. No CRITICAL or MAJOR finding; the three MINOR items and two suggestions of the previous review still apply unchanged, plus one new documentation SUGGESTION.

Re-run evidence (this review, mandated environment, `CARGO_TARGET_DIR=~/.cache/soos-targets/lockscreen-installer`): `cargo test --locked -p soos-invariants -- --test-threads=3` → 805 passed, 0 failed, 0 ignored (includes LKI1–LKI10, SET40–SET42, DCC1–DCC10, LKG7); `bash -n` on `scripts/install.sh`, `scripts/setup.sh`, `scripts/uninstall.sh` → ok (shellcheck is not installed on this host).

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `tests/invariants/src/lib.rs` (12 added lines: doc comment plus `#[cfg(all(test, unix))] mod lockscreen_installer_contract;`), `tests/invariants/src/lockscreen_installer_contract.rs` (new file). Both byte-identical in content to the pre-rebase branch.
- Removed/changed checks (`^-` lines with `assert`, `#[test]`, `proptest!`, `should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): **none**.
- Inline `mod tests` changes: **none**.
- The 22 removed lines of the patch are all in `AI/DECISIONS.md` (three ADR amendments), `Docs/LOCKSCREEN_WIDGET.md`, `Docs/PACKAGING_AND_PROVISIONING.md` and `scripts/setup.sh` (`root_preflight` path list, `write_state` printf); each is the same line the pre-rebase branch replaced, so no main-side text of #440 or #445 is lost. No pre-existing assertion removed or changed; `setup_contract.rs` and `daemon_check_config_contract.rs` (main) are untouched by the branch.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Phase order after the rebase (`setup.sh:2083-2092`): access → presence → lock widget → camera view (daemon). The #445 check (`setup.sh:1925`) sits between `JOURNAL+=(daemon_toml_created)` and the restart, unchanged (DCC9 green). Scenario: `--lock-widget off` plus camera view on, `--check-config` refuses → `phase_preview` returns 1 → `fail_and_rollback "camera view (daemon)"` replays the journal in reverse, removing `lockwidget.disable` (`lock_widget_flag_created`) and the created `/etc/soos` like the presence flag; exit 1 skips the user half (`run_lockscreen_step` only for 0 or 3). PASS.
- Scenario: the daemon stays down even without `daemon.toml` (`PREVIEW_DAEMON_DOWN`, exit 70) → `finish_with_state` keeps the lock-widget flag and records it in the state like presence; the user half is skipped. Consistent with the documented "nothing else rolled back". PASS.
- Timeouts (#440): the branch adds no `systemctl` call on `soos-daemon.service`; its only new bounded calls are `gnome-extensions list --enabled` / `enable` under `QUERY_TIMEOUT_S` = 10 s in the user half, which is a read-mostly user call, not a unit start, so SET40's `UNIT_START_TIMEOUT_S` rule does not apply and stays green. The rollback entry `daemon_toml_created` keeps its 90 s `try-restart`. PASS.
- Default state file: with the widget on and no flag, `lock_keys` is empty and the format-1 content is byte-identical, so the #445 Docker harness F1/F2 (`tests/docker/setup_root_preview_test.sh`) sees the same state; its fake checkout has no `packaging/` tree, so the new `system_up_to_date` comparisons are skipped and the stub `install.sh` behaviour is unchanged; no locker exists in the container, so `run_lockscreen_step` returns at once. PASS.
- LKG7 change-set guard (`lockscreen_gnome_contract.rs:453`): diffed against the new merge-base, the branch touches no `packaging/gnome-shell/` file, so the guard returns early; it also touches no `crates/`, so it would pass even if it ran. #445's `crates/daemon/` change is on main and outside the branch diff. PASS.
- `install.sh` / `uninstall.sh` / packaging: main did not modify them between `3f76090` and `d543bbd`; branch hunks unchanged. PASS.
- The previous MINOR finding on the `--sysconfdir` kill-switch mismatch is still present (see §4).

### PAM Concurrency & Deadlines
- No file under `crates/pam` changed; no PAM stack edit. PASS (not applicable).

### Panic Safety & Fail-Closed
- No Rust production code changed. New shell functions are called in `if`/`||` contexts; `phase_lock_widget` failure → `fail_and_rollback "lock widget"`; the user step never changes `rc`. No path turns an error into authentication success. PASS.

### Test Integrity & Anti-Weakening
- §2 listing reviewed: additions only, identical to the pre-rebase branch. LKI tests run the real scripts against stages and stubs and would fail for root enable, locker execution, config writes or a missing journal entry. PASS.

### Memory, Bounds & Secrets
- Unchanged from the previous review: bounded `gnome-extensions` calls, 64 KiB match window, `set -C` flag creation, symlink refusals in `root_preflight` and `phase_lock_widget`, no secret, frame or embedding printed. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or hook change. PASS.

### English-Only Policy
- All added text is English. PASS.

### Traceability (DECISIONS and matrix)
- `AI/DECISIONS.md`: no duplicated ADR heading; the 2026-10-18 entries appear once each, in order: Android tour (#427), `--check-config` (#441), lock-screen installer (#432).
- `AI/VERIFICATION_MATRIX.md`: no duplicated row id; one `lockscreen-installer` component section (LKI1–LKI12) after `daemon-check-config`; SET40–SET42 present once.

## 4. Detailed Findings & Action Items

- **[MINOR]** `scripts/setup.sh:510` — `LOCK_WIDGET_FLAG` follows `--sysconfdir`, but `packaging/gnome-shell/soos-lockscreen@soos/extension.js` hard-codes `/etc/soos/lockwidget.disable`; with `--sysconfdir` other than `/etc`, `--lock-widget off` creates a file the extension never reads. Warn when `SYSCONFDIR != /etc` (as the camera-view step already reports `manual (not applied)`), or document the restriction. (Carried over.)
- **[MINOR]** `Docs/LOCKSCREEN_WIDGET.md` and `Docs/PACKAGING_AND_PROVISIONING.md:105` — "nothing enabled or printed" with the widget off, while `setup.sh:1095` still prints the off notice; align with the ADR wording ("prints no locker line"). (Carried over.)
- **[MINOR]** `scripts/setup.sh:1130` — the DONE line promises "the card shows from the next lock" even when GNOME Shell may still need a log-out; consider appending the log-out hint. (Carried over; owner row LKI12.)
- **[SUGGESTION]** `Docs/PACKAGING_AND_PROVISIONING.md:89` — the **Rollback** paragraph lists the lock-widget flag but still says the rollback happens "when access, presence or an interrupt fails"; it also runs on a lock-widget failure and on a camera-view failure (including the #445 `--check-config` refusal). Name those phases.
- **[SUGGESTION]** `Docs/PACKAGING_AND_PROVISIONING.md` §3.0 — "a later `--uninstall` can reverse exactly what setup did" refers to a `setup.sh --uninstall` that does not exist yet. (Carried over.)
- **[SUGGESTION]** `scripts/uninstall.sh` — the snippet removal checks only that `share/soos/lockscreen` is not a link, not `share/soos` itself (single named `rm -f`, low impact). (Carried over.)

## 5. Final Verdict

The rebase preserved the branch's own changes, composes correctly with #440 and #445 (phases, journal, rollback, timeouts, SET/DCC/LKG7 contracts), changed no pre-existing assertion and introduced no duplicate in DECISIONS or the matrix; the invariant suite is green (805 passed).

**VERDICT: APPROVED**
