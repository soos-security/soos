# Candid Review Report

- **Date**: 2026-10-02
- **Target Branch**: `test/pam-hermetic-and-screensaver`
- **Base (merge-base)**: `4d56a01`
- **Reviewed-Diff-Fingerprint**: `c8ccbb0f9317d4782134f3a35dcc2d2af1cb8f5891c39ecb6e577f4178422873`
- **Audited Files**:
  - `AI/walkthroughs/173_hermetic_pam_test_and_screensaver_procedure.md` (new)
  - `crates/pam/tests/config_tests.rs`
  - `tests/physical/screensaver_test.md`

## 1. Executive Summary

The diff makes one PAM integration test hermetic (setup only), rewrites the `swaylock` physical
validation procedure (§3.1) and one behaviour-matrix row, and adds walkthrough 173. No
production code changes. The test assertion is byte-for-byte unchanged and still rejects a
fail-open implementation. The swaylock procedure was walked step by step against this host's
PAM chain, `packaging/pam/arch/system-auth`, `/etc/security/faillock.conf`, the swaylock man
page and the daemon log sites; every expected result is reachable from the state left by the
previous step, and both journal strings exist in `crates/daemon/src/dispatcher.rs` (and appear
verbatim in this host's `soos-daemon` journal). Only SUGGESTION-level findings remain, all in
unchanged surrounding lines.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `crates/pam/tests/config_tests.rs`, `tests/physical/screensaver_test.md`
  (a document, not code; matched by path filter).
- Removed/changed checks (`^-...assert|#[test]|...`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.
- `test_authenticate_with_none_handle_returns_ignore_cleanly`: only the configuration line
  changed, from `PamConfig::default()` (socket `/run/soos/daemon.sock`) to
  `PamConfig { socket_path: <tempdir>/absent.sock, ..Default::default() }`. The `assert_eq!`
  on `PAM_IGNORE` and its message are untouched. Justification: setup-only migration (the
  test was non-hermetic: on a host with a running daemon and an enrolled user it received a
  genuine `Allow`). Contract strength: `authenticate_with_config(None, ..)` goes through
  `authenticate_detached` → `authenticate_flow` with `detached_uid_resolver` (`getuid`), so the
  daemon connection is attempted and fails on the absent socket; an implementation that
  returns `PAM_SUCCESS` unconditionally, or maps a connect error to `PAM_SUCCESS`, still fails
  the assertion. The `TempDir` guard lives until the end of the function. Not a weakening.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Swaylock chain: host `/etc/pam.d/swaylock` = `auth include login`; `login` →
  `system-local-login` → `system-login` → `system-auth`, matching §3.1 and
  `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.3. Host `system-auth` equals
  `packaging/pam/arch/system-auth`. PASS.
- Step 2: `swaylock -C /dev/null -c 000000`. Man page: `-C, --config <path>` replaces the
  default search of `$HOME/.swaylock/config`, `$XDG_CONFIG_HOME/swaylock/config`,
  `SYSCONFDIR/swaylock/config`; `-e` is `--ignore-empty-password`. The three listed paths match. PASS.
- Test Case 1 (locked by step 2, face present, empty Enter): `pam_soos.so` `[success=4]`
  skips `pam_systemd_home`, `pam_unix`, the event line and `pam_faillock authfail`, landing on
  `pam_permit`; then `pam_faillock authsucc`. Unlock within the 1000 ms default
  (`DEFAULT_TIMEOUT_MS`, `crates/pam/src/config.rs:9`). Journal substring
  `verdict=Allow reason=FaceMatch` is produced by `info!(verdict = ?.., reason = ?..,
  "Rendered authentication response")` (`dispatcher.rs:1431-1435`) with the `fmt` layer
  (`logging.rs:74`); seen on this host at 16:38:12. PASS.
- Test Case 2 (re-locked explicitly since TC1 unlocked; lens covered; empty Enter): main line
  Deny → `PAM_IGNORE`; `pam_unix nullok` fails the empty input against a set hash →
  `default=bad`; event line sends `PasswordFailed`; daemon logs `Processing telemetry auth
  failure event` (`dispatcher.rs:587-591`) — swaylock runs as the user, so peer UID equals the
  event UID and the per-peer quota admits one event; `pam_faillock authfail` records one
  failure. `faillock --user <user>` is readable by the user (`/run/faillock/<user>` is
  `rw-rw---- <user>:root` here). `deny = 3` / `unlock_time = 600` are the documented defaults
  (commented in `/etc/security/faillock.conf`). Lockout claim: `pam_faillock preauth` is
  `required` before `pam_soos.so`, so a locked account fails even after a face success. The
  following correct password unlocks and `authsucc` resets the tally, so TC3/TC4 start clean. PASS.
- Test Case 3 (daemon stopped, password typed): no `.socket` unit exists in `packaging/`
  (only `soos-daemon.service`), so no socket activation restarts the daemon; connect fails at
  once → `PAM_IGNORE`; `pam_unix` success skips the event line. Daemon restarted for TC4. PASS.
- Test Case 4 (`-e`, daemon active): empty Enter never reaches PAM, so no `Rendered
  authentication response` line (INFO level, default visible). A typed password runs the main
  `pam_soos.so` line first. PASS.
- Walkthrough 173 matches the final files (§2.1 and every §2.2 bullet checked against the
  diff; test counts reproduced: invariants 378 passed, `config_tests` 25 passed). PASS.

### PAM Concurrency & Deadlines
- No PAM production code changed. The procedure's latency statements are tied to the clamped
  `timeout_ms` budget (10–5000 ms), consistent with the ADR "PAM Deadline Derived From Clamped
  `timeout_ms`". PASS.

### Panic Safety & Fail-Closed
- No production change. The new test `unwrap()` on `tempdir()` is in a test file with
  `#![allow(clippy::unwrap_used, ..)]`. `cargo clippy -p soos-pam --tests -- -D warnings` clean. PASS.

### Test Integrity & Anti-Weakening
- See §2. Tried: could the absent socket make the test vacuous? No — the flow still runs UID
  resolution and the IPC client, and any fail-open mapping would surface as `PAM_SUCCESS`. PASS.

### Memory, Bounds & Secrets
- No code paths changed. The procedure asks the operator to read only verdict/reason and the
  telemetry line, never frames or embeddings. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or hook changes. `tempfile` is an existing
  dev-dependency of `crates/pam`. PASS.

### English-Only Policy
- All added text, comments and file names are English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `tests/physical/screensaver_test.md:203-207` — the unchanged matrix rows
  ("Unknown Person", "Presentation Attack", "Multiple Faces", "Lens Covered") still say
  "Prompts for password", while the new §3.1 text states that `swaylock` shows no prompt and
  only a failure state after a submit. Correction: reword to "Stays locked; the password still
  unlocks" (or similar) in a follow-up.
- **[SUGGESTION]** `tests/physical/screensaver_test.md:208-210` — unchanged rows still promise
  "<= 5ms" / "<= 2ms" and "Evaluation > 250ms" for the deadline, inconsistent with the 1000 ms
  default `timeout_ms`. Correction: express them relative to `timeout_ms` as was done for the
  "Nominal Unlock" row.
- **[SUGGESTION]** `tests/physical/screensaver_test.md:68` — name where to read the journal
  line, e.g. "`journalctl -u soos-daemon -f` shows `Rendered authentication response
  verdict=Allow reason=FaceMatch`", to match the precision of Test Cases 2 and 4.
- **[SUGGESTION]** `tests/physical/screensaver_test.md:48-50` — the shown block
  `auth include login` is Arch's file; label it "Arch:" in the code-block lead-in so operators
  on Debian/Fedora do not copy it.

## 5. Final Verdict

**VERDICT: APPROVED**
