# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `fix/setup-start-bounds`
- **Base (merge-base)**: `3f76090`
- **Reviewed-Diff-Fingerprint**: `eb76aaf87ce1f316ae4468042f3e0e09b3779a6537c4ec2043a42daf45eefbe8`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/237_setup_start_bounds.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `scripts/setup.sh`, `tests/invariants/src/setup_contract.rs`

## 1. Executive Summary

The change moves the six `systemctl start|restart|stop|try-restart "${UNIT}"` calls of
`scripts/setup.sh` from the 10 s `QUERY_TIMEOUT_S` bound to a new `readonly UNIT_START_TIMEOUT_S=90`
(the unit's `TimeoutStartSec=60` plus 30 s). It adds three invariant tests (SET40 bound contract,
SET41 Tailscale-in-command-substitution scanner, SET42 target-directory foreign-owner preflight
refusal), a documentation paragraph and four matrix rows (SET43 is an owner hardware row, pending).
No production Rust code, no PAM code and no pre-existing test changed. No CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/setup_contract.rs` only (hunk `@@ -1727,3 +1727,432 @@`, pure append after the last existing test).
- Removed/changed checks (`^-` lines with `assert`/`#[test]`/`proptest!`/`should_panic`): **none**.
  `git diff 3f76090 HEAD -- tests 'crates/*/tests' android/app/src/test` contains no `-` line.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.
- New tests: `test_set_s6_daemon_unit_calls_bounded_by_unit_start_timeout`,
  `test_set_s7_no_tailscale_command_inside_command_substitution`,
  `test_set_target_dir_foreign_owner_refused_before_build`, plus helpers `daemon_unit_seconds`,
  `setup_readonly_seconds`, `scan_shell`, `tailscale_in_substitution`, `ShellCtx`.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Enumerated every `systemctl` line of `setup.sh`: the only daemon-unit mutating calls are lines
  1499 (rollback `try-restart`), 1536 (EXIT trap `start`), 1661/1665 (camera-view `restart`),
  1725/1737 (enrollment `stop`/`start`); all use `UNIT_START_TIMEOUT_S`. `unit_query` (505),
  `getent` (735/744/901), `tailscale version` (721) and every `systemctl --user` call stay on
  `QUERY_TIMEOUT_S`; `loginctl` on 5 s, camera report on 15 s, readiness on `--timeout 30`, as the
  doc paragraph states. PASS.
- `packaging/soos-daemon.service`: `Type=notify`, `TimeoutStartSec=60`, no `TimeoutStopSec`, and its
  own comment confirms model verification and warm-up run before `READY=1`. The walkthrough and doc
  claims match. The "~37.5 s" readiness worst case matches `scripts/wait_daemon_ready.sh` (timeout + 7.5 s). PASS.
- Tried to break SET40: reverting any one call to `QUERY_TIMEOUT_S` fails both the per-verb check and
  the `QUERY_TIMEOUT_S` + non-`--user` verb check; a duplicated or non-numeric constant fails the
  `readonly` parser; a margin outside 10..60 s fails. PASS.
- SET42 strings match `check_target_dir_owner` (line 834) exactly; the harness log format
  `uid=N <cmd> <args>` makes `split_whitespace().nth(1)` the command, as in existing tests. PASS.

### PAM Concurrency & Deadlines
- No change in `crates/pam`. Every new bound in the script is finite and `readonly`; no call became
  unbounded (fallback without coreutils `timeout` is pre-existing and documented). PASS.

### Panic Safety & Fail-Closed
- Only shell bounds changed. A hung stop still ends the client at 92 s and reports failure (same
  fail-closed path, later). Test-only `expect`/`panic!`/indexing are in test code. PASS.

### Test Integrity & Anti-Weakening
- No pre-existing assertion removed or changed (Section 2). The 17 scanner snippets (9 bad, 8 good)
  make SET41 fail against a scanner that ignores nested quotes, here-documents or backticks.
  Scoped re-run: `cargo test -p soos-invariants --lib -- --test-threads=3` → 790 passed, 0 failed;
  `cargo clippy -p soos-invariants --all-targets -- -D warnings` clean; `cargo fmt --all -- --check`
  clean; `bash -n scripts/setup.sh` clean. PASS.

### Memory, Bounds & Secrets
- No secret, frame or embedding involved; nothing new logged. Scanner looks ahead at most 64 chars. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or hook change. PASS.

### English-Only Policy
- All added text is English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `tests/invariants/src/setup_contract.rs:316` — `scan_shell` only detects `tailscale`
  as a bare word: `/` is a word character, so `$(/usr/bin/tailscale serve ...)` or a command named
  through a variable (`$("${TS}" serve)`) is not flagged by SET41. `setup.sh` contains neither
  today; a follow-up could also match a path suffix `/tailscale`.
- **[MINOR]** `tests/invariants/src/setup_contract.rs:264` — the SET40 cross checks are per line; a
  call split with `\` continuation between `run_bounded` and `systemctl` would escape them. No such
  split exists today (the `bounded >= 6` floor still catches a moved call).
- **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md` SET40 lists "system phase" next to rollback/EXIT
  trap/camera-view/enrollment; those four are the complete set, the extra label is redundant.

## 5. Final Verdict

**VERDICT: APPROVED**
