# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `fix/setup-start-bounds`
- **Base (merge-base)**: `1043be6`
- **Reviewed-Diff-Fingerprint**: `07976bebc0e960c19888c023df20bca4b8a61ace406ebcdd20c10247db92701a`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/237_setup_start_bounds.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `scripts/setup.sh`, `tests/invariants/src/daemon_check_config_contract.rs`, `tests/invariants/src/setup_contract.rs`

## 1. Executive Summary

Independent re-review of `fix/setup-start-bounds` (GitHub #437) after the fix of the previous MAJOR
finding. The branch, rebased onto `origin/main` `1043be6` (#445, `soos-daemon --check-config`),
moves the six `systemctl start|restart|stop|try-restart "${UNIT}"` calls of `scripts/setup.sh`
from `QUERY_TIMEOUT_S` (10 s) to a new `readonly UNIT_START_TIMEOUT_S=90`, adds SET40-SET42,
documentation and matrix rows SET40-SET43.

Since the last review, two commits landed:

- `9f85a78` adds three lines to the DCC8 harness of
  `tests/invariants/src/daemon_check_config_contract.rs` (`run_phase_preview`): a comment and
  `harness.push_str(&readonly_line(&script, "UNIT_START_TIMEOUT_S")); harness.push('\n');`, the
  same pattern already used for `DAEMON_PREVIEW_MARKER` and `MAX_DAEMON_TOML_SCAN_BYTES`. It is
  setup-only (owner-approved on 2026-10-18): no line removed, no assertion added or changed.
  `readonly_line` panics when the line is absent, so the harness fails loudly rather than
  silently defining nothing.
- `ccc800a` updates walkthrough 237 (rebase note, worst case including the 10 s check, the DCC8/DCC9
  mention of `QUERY_TIMEOUT_S`), closing the previous walkthrough MINOR.

`cargo test --locked -p soos-invariants -- --test-threads=3` (with the requested environment):
**795 passed, 0 failed**, including DCC8 accepted/refused, DCC9, SET40, SET41 and SET42. The previous
MAJOR is resolved. Two MINORs are carried forward. **APPROVED**.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/setup_contract.rs` (additive, SET40-SET42) and
  `tests/invariants/src/daemon_check_config_contract.rs` (3 added lines, setup-only, `9f85a78`).
- Removed or changed `assert` / `#[test]` / `proptest!` / `#[should_panic]` lines: **none**. The only
  removed lines of the patch are the six `run_bounded "${QUERY_TIMEOUT_S}" systemctl ...` lines of
  `scripts/setup.sh` and three walkthrough lines.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.
- Justification of the `daemon_check_config_contract.rs` change: the harness runs the real
  `phase_preview` under `set -u`; the branch's restart line references `UNIT_START_TIMEOUT_S`, so the
  harness must define it. Reading it from the script (not hard-coding 90) keeps the harness bound to
  production. DCC8/DCC9 assertions are byte-identical to `1043be6`.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Composition with #445 in `phase_preview` (`scripts/setup.sh:1660-1680`): the file is created by
  hard link, journaled `daemon_toml_created`, then checked with
  `run_bounded "${QUERY_TIMEOUT_S}" "${DAEMON_BIN}" --check-config --config "${DAEMON_TOML}"` (10 s),
  and only then restarted with `run_bounded "${UNIT_START_TIMEOUT_S}" systemctl restart` (90 s).
  Check before restart; correct bound for each. PASS.
- Check refused: file removed, journal entry dropped, `return 1`; the caller
  (`scripts/setup.sh:1818-1824`) goes to `fail_and_rollback` (exit 1) and never reaches the
  `daemon_toml_created` `try-restart`; the daemon is untouched. PASS.
- Restart or readiness failed: file removed, entry dropped, second restart under
  `UNIT_START_TIMEOUT_S`; if it also fails, `PREVIEW_DAEMON_DOWN=true` and the caller exits 70
  (`EXIT_NOT_READY`) without further rollback; otherwise exit 1 through rollback. PASS.
- Rollback `try-restart` (`:1500`), EXIT trap start (`:1537`), enrollment stop/start (`:1734`,
  `:1746`) all use `UNIT_START_TIMEOUT_S`; the read-only `is-enabled`/`is-active` helper (`:506`)
  and every `systemctl --user` call stay on `QUERY_TIMEOUT_S`. PASS.
- Worst case of the camera-view step: 10 s check + 2 x (90 + 2) s restarts + 2 readiness waits,
  about 270 s, finite and as stated in the walkthrough. PASS.
- Scenario tried: an attacker-free regression where someone renames the constant: SET40
  (`setup_readonly_seconds`) and the DCC8 harness (`readonly_line`) both panic. PASS.

### PAM Concurrency & Deadlines
- No change in `crates/pam`. All new bounds are finite `readonly` integers. PASS.

### Panic Safety & Fail-Closed
- No production Rust change. No path from a refused check or a failed restart to a reported
  success. PASS.

### Test Integrity & Anti-Weakening
- `9f85a78` verified as exactly the `readonly_line` setup line (plus comment and newline). PASS.
- SET40/SET41/SET42 fail against plausible wrong implementations (a call left on
  `QUERY_TIMEOUT_S`, `"$(tailscale serve)"`, a missing chown hint). PASS, with the two MINOR gaps
  below.

### Memory, Bounds & Secrets
- No secret, frame or embedding involved; nothing new logged. PASS.

### Supply Chain & Automation
- Only `scripts/setup.sh` among automation files; no dependency, workflow or hook change. PASS.

### English-Only Policy
- All added text and both new commit messages are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `tests/invariants/src/setup_contract.rs` (`scan_shell`, SET41) — carried forward:
  `$(/usr/bin/tailscale serve ...)` (the word is preceded by `/`, a word character for the scanner)
  or a command held in a variable (`"$("${ts}" serve)"`) is not flagged. `setup.sh` contains
  neither today.
- **[MINOR]** `tests/invariants/src/setup_contract.rs` (SET40) — carried forward: the cross checks
  are per line, so a `\`-continued `run_bounded "${QUERY_TIMEOUT_S}" \` / `systemctl restart
  "${UNIT}"` would escape the "bounded by `UNIT_START_TIMEOUT_S`" rule; none exists, and the
  `bounded >= 6` floor still catches a moved call only if its `systemctl ... "${UNIT}"` line keeps
  the `run_bounded` prefix.
- **[SUGGESTION]** `Docs/PACKAGING_AND_PROVISIONING.md` "Time bounds" paragraph — could mention
  that the #445 `soos-daemon --check-config` run is bounded by `QUERY_TIMEOUT_S` = 10 s before the
  restart.

## 5. Final Verdict

**VERDICT: APPROVED**
