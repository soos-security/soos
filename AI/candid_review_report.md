# Candid Review Report

- **Date**: 2026-10-16
- **Target Branch**: `feat/setup-user-phase`
- **Base (merge-base)**: `2025738`
- **Reviewed-Diff-Fingerprint**: `da92335806c6df8bfb4207f66c38c3eb159e3b239a6015672a2b9752d2296e0d`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_setup_user_phase.md`, `AI/auditor_constraints_setup_user_phase.md`, `AI/tester_contract_setup_user_phase.md`, `AI/walkthroughs/213_setup_user_phase.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/REMOTE_COMPANION.md`, `README.md`, `crates/remote/tests/setup_render_tests.rs`, `scripts/install_remote.sh`, `scripts/setup.sh`, `tests/fixtures/setup/remote.toml`, `tests/invariants/src/lib.rs`, `tests/invariants/src/setup_contract.rs`, `tests/invariants/src/setup_user_phase_contract.rs`

## 1. Executive Summary

The diff adds the phone-companion user phase to `scripts/setup.sh` (GitHub #400, PR 2 of #362): new options, a
strict classifier for `remote.toml` (absent / untouched template / setup-managed / user), the owner's default-on
feature set with prerequisite cascade and preserved explicit `false`, an atomic 0600 write with backup, a call to
`install_remote.sh` as the user, validation with `soos-remote --config … passkeys list`, user-unit enable/start with
a journaled rollback (exit 5), a root-half creation of `/etc/soos/daemon.toml` `[preview]` only when absent and only
when `--sysconfdir` is `/etc`, and printed (never executed) Tailscale steps. `install_remote.sh` now honours
`CARGO_TARGET_DIR`. No Rust production code changes; `crates/pam` untouched.

Verified locally: `cargo test --locked -p soos-invariants --all-features setup_` (51 passed, including all 23
`setup_user_phase_contract` tests and the migrated SET3), `cargo test --locked -p soos-remote --test
setup_render_tests` (3 passed), `cargo test -p soos-invariants --all-features remote` (147 passed), `bash -n
scripts/setup.sh`. `shellcheck` is not installed on this host (CI runs it).

No CRITICAL or MAJOR finding. Three MINOR items and one SUGGESTION are listed in section 4.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Test files touched (`grep '^diff --git' | grep tests/fixtures`):
- `crates/remote/tests/setup_render_tests.rs` (new)
- `tests/fixtures/setup/remote.toml` (new)
- `tests/invariants/src/lib.rs` (module registration only)
- `tests/invariants/src/setup_contract.rs` (**existing test modified**)
- `tests/invariants/src/setup_user_phase_contract.rs` (new)

Removed/changed checks (`^-.*assert|#[test]…`): none matched by the regex, but manual reading of the
`setup_contract.rs` hunk shows a real contract migration of `test_set_s3_no_pam_tailscale_remote_or_network_lock_action`:
- the banned strings `install_remote.sh`, `[preview]`, `remote.toml` were removed from the forbidden list;
- the Tailscale rule changed from "every `tailscale ` must be `tailscale version`" to "`tailscale version`, or inside a
  quoted string" (every occurrence on a line is now checked, not only the first, which is stricter in that respect).
- Justification: owner decision recorded on GitHub #400 (comment of 2026-10-15T21:25Z, account `Willi363363`):
  "SET3 contract migration approved … All other bans and the executed-call check stay." The other bans
  (`$EUID`/`$UID`, PAM, network lock, `curl`/`wget`, `chmod 666`, `pkexec`, …) and the behavioural
  `assert_no_forbidden_calls` are unchanged; matrix row SET3 text updated accordingly; the tester contract lists the
  migration. Provenance note: the approval comment was posted ~20 s after commit 40b0fb1 from the owner's account,
  so it was most likely transcribed by the lead agent; the lead should make sure the owner really gave this approval
  in chat (not a code defect, recorded for traceability).

New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): none.
Inline `mod tests` changes: none.

`setup_user_phase_contract.rs` is new on this branch (no test from `main` weakened). Within the branch,
`test_set_u16` was rewritten after the lead review (R3-1) because the `/etc` creation path became unreachable with a
fake `--sysconfdir`; this is a spec revision, not a weakening of a merged contract (see MINOR-2 on the resulting
coverage gap).

## 3. Deep Reasoning Audit

### Logic & Architecture
- Prerequisite ordering in `resolve_features`: `COMPANION_KEYS` lists each prerequisite before its dependent, so
  `FEATURE_VALUE[${prereq}]` is always set under `set -u` → PASS.
- Explicit `on` vs prerequisite `off` from flags (parse time, exit 2) and from the managed file (preflight, exit 2
  with "add --x on") → PASS (SET22, SET26).
- `classify_remote_config`: multi-login arrays, tables, indented keys, duplicate keys, unknown keys, missing
  `rp_id`, oversize (>16 KiB) all fall to `user` and are never edited → PASS. Compatibility with the GUI tour writer
  (`crates/gui/src/feature_control.rs::disable_remote_keys` rewrites only `key = true` → `key = false`, keeps every
  other line) keeps the marker and grammar → PASS.
- Template detection matches the real `install_remote.sh` heredoc (only `allowed_logins = []` uncommented) → PASS.
- Daemon side: `from_toml_str` starts from `DaemonConfig::runtime_default()`, so a `daemon.toml` holding only
  `[preview]` changes nothing else of the daemon behaviour → PASS. `allowed_uids` comes from the root half's own
  `TARGET_UID` → PASS. Gate on `DAEMON_READ_CONFIG == /etc/soos/daemon.toml` in both halves → PASS.
- Exit-code composition: root rc 0 → companion 0/5; rc 3 kept with a companion failure; any other rc skips the
  companion → PASS.
- Validation command: `soos-remote --config <file> passkeys list` loads and validates the whole config (exit 78) and
  only reads the passkey store (no side effect) → PASS.
- `install_remote.sh` `TARGET_DIR` resolution mirrors `resolve_target_dir` (relative → checkout) → PASS.

### PAM Concurrency & Deadlines
- `crates/pam` not touched. All new `systemctl --user` and `soos-remote` calls go through `run_bounded` (10 s);
  `wait_daemon_ready.sh` 30 s; settle 2 s → PASS. `install_remote.sh` (cargo build) is unbounded by design, as the
  PR 1 build step.

### Panic Safety & Fail-Closed
- No Rust production code. Shell: `run_companion_phase` runs in a `||` context; every step checks its status and
  returns through `companion_failed` (verified each branch returns 5) → PASS.
- No path turns a companion failure into success: validation exit ≠ 0, unit not active after settle, failed
  install → exit 5 with rollback → PASS.
- Signals during the user phase replay the journal (HUP/INT/QUIT/TERM) → PASS.

### Test Integrity & Anti-Weakening
- SET3 migration justified by the owner decision (section 2). Behavioural tests run the real script against a fake
  root/HOME with stubs; render fixture byte-equality plus real `parse_config` acceptance/refusal of variants would
  catch a wrong key, wrong cascade or wrong quoting → PASS, with MINOR-1 and MINOR-2.

### Memory, Bounds & Secrets
- Reads bounded with `head -c` (16 KiB / 64 KiB). Login never printed (plan says "login given"; usage errors do not
  echo it); validation stdout discarded; no enroll code, VAPID key or passkey data handled → PASS.
- `remote.toml` 0600 via `mktemp` under `umask 077` and same-directory `mv -f`; directory 0700; symlinks refused for
  config home, directory, file and backup; foreign or group/world-writable directory refused; backup must be a
  regular owned file → PASS.
- `daemon.toml` created 0644 via hard link (no clobber), never through a symlink, never edited when present → PASS.
- Login/node-name character sets cannot break out of a TOML string (no quote, backslash, newline) → PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or hook changes. `scripts/` changes contain no download (`curl`/`wget`),
  no `eval`, no `sudo` in the user phase, no Tailscale execution other than `tailscale version` → PASS.

### English-Only Policy
- Code, comments, docs, matrix, ADR and walkthrough are English → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `tests/invariants/src/setup_contract.rs:165` (and `tests/invariants/src/setup_user_phase_contract.rs:68`
  `quoted_at`) — the "inside a quoted string" heuristic counts quotes before the match, so an executed call wrapped
  in a command substitution inside double quotes (`x="$(tailscale serve …)"`) passes the static check. The current
  script has no such use (only `info "… tailscale funnel/serve …"` lines) and the behavioural
  `assert_no_forbidden_calls` still covers the tested paths. Required change (follow-up, test-side hardening only):
  also reject a match preceded by an unclosed `$(` or a backtick on the same line.
- **[MINOR]** `scripts/setup.sh:1613` (`phase_preview` create/restart/readiness-failure path, lines 1641–1662 and the
  caller at 1800–1806) — the `/etc` creation path (hard link, restart, removal + second restart, `PREVIEW_DAEMON_DOWN`
  → exit 70, otherwise `fail_and_rollback` → exit 1, `unset 'JOURNAL[...]'`) has no automated coverage: the fake-root
  harness can never reach it because `DAEMON_READ_CONFIG` is a hard-coded `/etc` path. Only the owner hardware row
  SET39 exercises it. Required change (follow-up): add it to the Dockerized matrix (`tests/docker/`, real `/etc`
  in a container) so the rollback branches are executed at least once in CI.
- **[MINOR]** `scripts/setup.sh:1805` — a failure of the optional camera-view daemon side while the daemon is healthy
  without it rolls back the whole root half, i.e. the face install (spec R-U15), whereas every other companion failure
  keeps the face install (exit 5). This is spec-conformant but inconsistent for the user; recommended: record it
  explicitly under "Owner to confirm" in the ADR/walkthrough, or downgrade it to "file removed, `[SKIP]`, face install
  kept" in a later PR.
- **[SUGGESTION]** `scripts/setup.sh` `resolve_companion` — when `remote.toml` is user-written (`kept`), a plain
  `--yes` run still rebuilds, reinstalls and `try-restart`s the companion. It is in the plan line and spec, but a
  one-line note in `Docs/PACKAGING_AND_PROVISIONING.md` §3.0 ("a hand-written configuration still gets the companion
  binaries updated; use `--no-remote` to avoid it") would avoid surprise.

## 5. Final Verdict

**VERDICT: APPROVED**
