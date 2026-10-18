# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `fix/lockwidget-flag-path`
- **Base (merge-base)**: `cb071cd`
- **Reviewed-Diff-Fingerprint**: `0639e197520a061c9b53ed338dc81234e835594d6c32ce2b44addcddfb956edc`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/242_lockwidget_flag_path.md`, `Docs/LOCKSCREEN_WIDGET.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `scripts/setup.sh`, `scripts/uninstall.sh`, `tests/invariants/src/lockscreen_installer_contract.rs`

## 1. Executive Summary

This review was rerun after the rebase onto `origin/main` `cb071cd`. That base adds #446 (doc follow-ups) on top of the
previous base `ed0bb84` (#447, release dry run). The review covers the raw diff from `cb071cd` to the working tree, frozen
in `target/candid_diff.patch`.

The rebase did not change the branch's own work. `git range-diff ed0bb84..origin/fix/lockwidget-flag-path cb071cd..HEAD`
reports the fix commit as identical (`ac4d7b4 = 3884c12`). A diff of the two per-commit patches differs only in their
`index` lines. Before this rewrite, the review-report commit `f504156` had the same report blob as the pushed `5520cc3`.
Every other difference between `5520cc3` and `HEAD` is #446's content (`AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`,
walkthrough 238, `Docs/REMOTE_COMPANION.md`, `README.md`).

The diff implements GitHub #451:
- `scripts/setup.sh` pins `LOCK_WIDGET_READ_FLAG="/etc/soos/lockwidget.disable"`, which is the GNOME extension's
  `KILL_SWITCH_PATH`. It reports the lock-widget step `[DONE]` only when `<sysconfdir>/soos/lockwidget.disable` is that
  path. In every other case it reports `[SKIP]`, with "not applied" and the exact `sudo touch` or `sudo rm -f` command.
- The GNOME success line now mentions the log-out.
- `scripts/uninstall.sh` never follows a symbolic link in place of `<prefix>/share/soos`.
- Docs, matrix rows LWF1–LWF6 and walkthrough 242 are added.

Results: `bash -n` passes on both scripts, and `cargo test --locked -p soos-invariants` reports 828 passed and 0 failed.
There are no CRITICAL or MAJOR findings.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:
- **Test files touched**: only `tests/invariants/src/lockscreen_installer_contract.rs`, with 277 added lines and no
  removed lines. The patch appends a new `#451` section after the last existing test (`test_lki10_...`).
- **Removed or changed assertions** (`^-` with `assert`, `#[test]` or `proptest!`): none.
- **New escape hatches** (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): none.
- **Inline `mod tests` changes**: none.

No pre-existing assertion changed. The new tests LWF1–LWF6 can fail against plausible wrong implementations:
- LWF1 cross-checks `setup.sh` against `extension.js` `KILL_SWITCH_PATH`, and checks that no other component reads the
  flag.
- LWF2 rejects any `[DONE] lock widget` line, and any "no GNOME card" claim, under a non-`/etc` sysconfdir.
- LWF3 proves the `[DONE]` path still works when the read path matches.
- LWF5 uses a real symbolic link and checks that the targets stay byte-for-byte unchanged.

## 3. Deep Reasoning Audit

### Logic & Architecture
- **Rebase interaction with #446.** Both PRs edit `AI/DECISIONS.md` and `AI/VERIFICATION_MATRIX.md`, on different lines.
  #446 changed ADR 2026-10-02 and matrix row OBT9. This branch changes only the 2026-10-18 lock-screen ADR line and
  appends a new LWF section. The diff against `cb071cd` touches neither #446 hunk, and `README.md` and
  `Docs/REMOTE_COMPANION.md` are not in the diff, so no #446 change is reverted. The walkthrough numbers do not collide:
  238 belongs to #446 and 242 to this branch. → PASS.
- **Rebase interaction with #447.** #447 was already in the old base. `scripts/release_dry_run.sh` builds the source
  tarball with `git archive` of `HEAD`, so the changed `setup.sh` and `uninstall.sh` are shipped as tracked files, with no
  pinned file list or checksum to update. Both PRs edit `Docs/PACKAGING_AND_PROVISIONING.md`, but the
  `release_dry_run_contract` needles in that file still pass (828/828). → PASS.
- **Scenario: non-`/etc` sysconfdir.** Tried `off` twice, a plain run, `on`, a plain run again, and a dry run. The flag is
  still created or removed and journaled under `<sysconfdir>`, but every phase, plan and summary line says "not applied"
  with the read path and the command. The command matches the requested state: `lock_widget_requested` uses
  `lock_widget_will_be_off`, and the plan note is computed once, before the `case`. → PASS.
- **Scenario: `--sysconfdir /etc/` (trailing slash).** The comparison is a string comparison, so this path is reported
  "not applied" although it is the read path. That is a false negative in the conservative direction: setup never falsely
  claims `[DONE]`. → SUGGESTION.
- The rollback paragraph says a lock-widget or camera-view failure triggers rollback. This matches `fail_and_rollback`,
  which is called for "lock widget" and "camera view (daemon)" (setup.sh, around lines 2154 and 2160). → PASS.

### PAM Concurrency & Deadlines
No `crates/pam` change. The only external call touched is `gnome-extensions enable`, whose message text changed. The call
stays wrapped in `run_bounded "${QUERY_TIMEOUT_S}"`. → PASS (not applicable).

### Panic Safety & Fail-Closed
No Rust production code changes. In the scripts, the new branches only downgrade `[DONE]` to `[SKIP]`, and no path claims
a protection or state that was not applied. The symbolic-link refusal on the flag itself is kept, before the `case`. →
PASS.

### Test Integrity & Anti-Weakening
See section 2. The diff only adds tests, and the LKI rows are unchanged. → PASS.

### Memory, Bounds & Secrets
- **`uninstall.sh` with `<prefix>/share/soos` as a symbolic link.** `SOOS_SHARE_LINKED=true` skips both the Arch snippet
  removal and the hyprlock snippet removal, along with every `rmdir`, and prints a warning. A link at the `pam/` or
  `lockscreen/` subdirectory is also refused. `share/soos` is referenced nowhere else in the script. → PASS.
- No secrets, frames or embeddings appear in the new output. Only fixed paths and commands are printed. → PASS.

### Supply Chain & Automation
The diff touches no `Cargo.*`, `deny.toml`, `.github/` or hook files. The script changes use only coreutils and add no
downloads. → PASS.

### English-Only Policy
All code, comments, docs and test messages in the diff are English. → PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `scripts/setup.sh:519` — `LOCK_WIDGET_FLAG` is built from the raw `--sysconfdir` value, so `/etc/` or
  `/etc/.` is reported "not applied" even though it is the read path. This is fail-safe (never a false `[DONE]`) and
  follows the same rule as `DAEMON_READ_CONFIG`. A possible follow-up is to normalize trailing slashes in `--sysconfdir`.
  No action is required for this PR.

## 5. Final Verdict

**VERDICT: APPROVED**
