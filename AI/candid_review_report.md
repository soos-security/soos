# Candid Review Report

- **Date**: 2026-10-15
- **Target Branch**: `feat/one-command-install`
- **Base (merge-base)**: `cae9d7c`
- **Reviewed-Diff-Fingerprint**: `7b8a52256e14d4c803b49ebdb407cf0ab11f7ffe4bfb59545dfbe604b47d089a`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_one_command_install.md`, `AI/auditor_constraints_one_command_install.md`, `AI/tester_contract_one_command_install.md`, `AI/walkthroughs/200_one_command_install.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `README.md`, `scripts/setup.sh`, `tests/invariants/src/lib.rs`, `tests/invariants/src/setup_contract.rs`

## 1. Executive Summary

Re-review of PR #385 (`scripts/setup.sh`, GitHub #372, PR 1 of #362) after the branch was
rebased from merge base `94a7cb3` onto `origin/main` `cae9d7c`. Main gained #383 (push-test
deflake), #394 (actions bump), #384 (motion spec), #388 (read-only lock-screen state request)
and #386 (offline intervals and back-online push, with an `offline_alerts` key in the
`soos-remote` configuration). The previous report (fingerprint `72984545...`) is stale.

The rebase is mechanically faithful: for every one of the 13 changed files, the sorted set of
added and removed lines vs the respective merge base is identical before and after the rebase,
except `AI/VERIFICATION_MATRIX.md`, which gains only a `---` separator between main's last
section and the `one-command-setup` section. The three conflicted files keep main's blocks
first and this branch's block unchanged (`AI/DECISIONS.md`: ADR appended after main's three
new ADRs; `tests/invariants/src/lib.rs`: `mod setup_contract;` after main's three new modules).
`AI/candid_review_report.md` and `AI/plan_evaluator_report.md` have the same content as on
`origin/feat/one-command-install`; their diff stat differs only because main changed the base
versions. `scripts/setup.sh` and `tests/invariants/src/setup_contract.rs` are byte-identical in
content to the previously approved branch.

No semantic interaction with main was found that this PR must handle (see Logic). No CRITICAL
or MAJOR finding. Verdict: APPROVED.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (adds `mod setup_contract;`) and
  `tests/invariants/src/setup_contract.rs` (new in this PR, 28 `#[test]` functions).
- Removed or changed assertions (`^-` lines with assert, `#[test]`, proptest, should_panic):
  **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, should_panic, tolerance, epsilon): **none**.
- Inline test modules added or removed: none.
- No test of main (`lock_state_contract`, `remote_offline_contract`, `ui_motion_spec_contract`
  or any other) is touched by this branch.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Scenario: did the rebase silently change behavior? `git range-diff
  origin/feat/one-command-install...HEAD` shows only context drift; the per-file comparison of
  sorted `+`/`-` lines (old diff vs `94a7cb3`, new diff vs `cae9d7c`) is identical for 12 of 13
  files, and the 13th differs by one added `---` line. PASS.
- Scenario: did an install script that `setup.sh` orchestrates change on main? Between
  `94a7cb3` and `cae9d7c`, the only change under `scripts/` is a commented
  `# offline_alerts = false` block in the `remote.toml` template of `scripts/install_remote.sh`.
  `install.sh`, `check_build_deps.sh`, `install_rustup.sh`, `wait_daemon_ready.sh`,
  `download_models.sh`, `uninstall.sh` and `packaging/` are unchanged, so the
  "already done" byte-equality checks, package lists and exit-code handling of `setup.sh` still
  match their targets. `setup.sh` never calls `install_remote.sh` nor writes `remote.toml`
  (static row SET3 still passes). PASS.
- Scenario: does the new `[offline]` / `offline_alerts` key need handling in this PR? No: this
  PR applies the face profile only; the remote user phase is deferred (ADR item (9)). Main's ADR
  "Offline History and Back-Online Push Notifications" item (4) states that the #362 setup will
  write `offline_alerts = true` together with `push_notifications = true`, and the service exits
  78 (`OfflineRequiresPush`) if `offline_alerts = true` without push. This branch's ADR item (7)
  lists the default-on features of the later user phase (presence, companion, Web Push, Funnel,
  camera view) without `offline_alerts`. Not a defect of this PR (no code writes the key), but
  the later user-phase PR must honour that coupling; recorded as MINOR below.
- Scenario: lock-state request (#388) or motion spec (#384) interacting with setup? Setup does
  not touch the IPC protocol, the GUI or `zbus`; the daemon stop/start around enrollment is
  unaffected by the new read-only `LockState` request. PASS.
- Module count in `tests/invariants/src/lib.rs`: 77 module declarations (73 base + 3 from main +
  `setup_contract`), matching the lead's count. PASS.

### PAM Concurrency & Deadlines
- No `crates/pam` change; `setup.sh` performs no PAM activation (SET3). PASS.

### Panic Safety & Fail-Closed
- No production Rust change. `setup.sh` unchanged since the approved review: `set -euo
  pipefail`, no `eval`, no `pkexec` call (mentioned only in comments and help text as a way
  the root shell may have been reached), EXIT and HUP/INT/QUIT/TERM traps restart the daemon
  after enrollment, the state file is refused when symlinked, foreign-owned or malformed. PASS.

### Test Integrity & Anti-Weakening
- `CARGO_TARGET_DIR=~/.cache/soos-targets/one-command-install ... cargo test --locked -p
  soos-invariants -- --test-threads=4`: **653 passed, 0 failed, 0 ignored**, including all 28
  `setup_contract` tests and main's new `lock_state_contract`, `remote_offline_contract` and
  `ui_motion_spec_contract` (so main's repository-wide scans accept `setup.sh`).
- `bash -n scripts/setup.sh`: OK (the only shell script changed by the branch).
- Previous findings carried forward: the uid harness fix (`STUB_UID` from the real `id -u`)
  is unchanged and still passes. PASS.

### Memory, Bounds & Secrets
- Unchanged: state file bounded to 4096 bytes with a strict grammar, no secret or timestamp;
  no frames, embeddings or credentials handled. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or `.githooks/` change on the branch; the only script
  is `scripts/setup.sh`, which downloads nothing (`curl`/`wget` absent) and pins no new tool. PASS.

### English-Only Policy
- All branch content (code, comments, docs) is in English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `AI/DECISIONS.md` (this branch's ADR "One-Command Setup", items (7) and (9)) —
  the default-on list and the deferred user phase do not mention `offline_alerts`, while main's
  ADR "Offline History…" item (4) commits the #362 setup to writing `offline_alerts = true`
  together with `push_notifications = true` (and `soos-remote` refuses `offline_alerts = true`
  without push, exit 78). No code in this PR writes `remote.toml`, so nothing is broken now;
  the later user-phase PR of #362 must write both keys together and keep them coupled on any
  toggle. Optional for this PR: add one sentence to item (9).
- **[SUGGESTION]** `tests/invariants/src/setup_contract.rs` — the ownership preflight of
  `scripts/setup.sh` (`find -H "${TARGET_DIR}" ! -uid "${BUILD_UID}"`) still has no
  negative-path test (carried forward, pre-existing, optional).

No CRITICAL or MAJOR findings.

## 5. Final Verdict

**VERDICT: APPROVED**
