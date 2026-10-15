# Candid Review Report

- **Date**: 2026-10-16
- **Target Branch**: `feat/setup-user-phase`
- **Base (merge-base)**: `2025738`
- **Reviewed-Diff-Fingerprint**: `2770ee0bc045b78752dd42db81b2316341473cf3e8aa4ee48c442c24e7f42cff`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_setup_user_phase.md`, `AI/auditor_constraints_setup_user_phase.md`, `AI/tester_contract_setup_user_phase.md`, `AI/walkthroughs/213_setup_user_phase.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/REMOTE_COMPANION.md`, `README.md`, `crates/remote/tests/setup_render_tests.rs`, `scripts/install_remote.sh`, `scripts/setup.sh`, `tests/fixtures/setup/remote.toml`, `tests/invariants/src/lib.rs`, `tests/invariants/src/setup_contract.rs`, `tests/invariants/src/setup_user_phase_contract.rs`

## 1. Executive Summary

The diff adds the user phase of `scripts/setup.sh` (GitHub #400, PR 2 of #362): a setup-managed
`remote.toml` with the owner's default-on features and prerequisite cascade, orchestration of
`scripts/install_remote.sh` as the session owner (never root), validation through the real
`soos-remote` parser, user-unit enablement with a journaled rollback (exit 5), the daemon-side
`[preview]` file created only when `daemon.toml` is absent (hard link, never replacing), and the
`install_remote.sh` `CARGO_TARGET_DIR` fix. No Rust production code changes; no PAM, IPC or daemon
code is touched. The only modified pre-existing test is SET3, an owner-approved contract migration
recorded in ADR 2026-10-15 item (1). Contract tests (22 new invariants, 3 parser tests) and the
existing setup suite were run and pass. No CRITICAL or MAJOR defect was found; two MINOR and two
SUGGESTION items are listed.

## 2. Test Changes

Mechanical listing (step 3 of the skill) on `target/candid_diff.patch`:

- Test files touched: `crates/remote/tests/setup_render_tests.rs` (new), `tests/fixtures/setup/remote.toml` (new),
  `tests/invariants/src/lib.rs` (module registration only), `tests/invariants/src/setup_contract.rs` (modified),
  `tests/invariants/src/setup_user_phase_contract.rs` (new).
- Removed/changed assertion lines matched by the grep: none matched the `^-...assert` pattern, but a manual read of
  the `setup_contract.rs` hunk shows SET3 (`test_set_s3_no_pam_tailscale_remote_or_network_lock_action`) was
  relaxed: the banned strings `install_remote.sh`, `[preview]` and `remote.toml` were removed, and the Tailscale
  static rule now accepts the word `tailscale ` inside a quoted string in addition to `tailscale version`.
  **Justification**: ADR 2026-10-15 "Setup User Phase…" item (1) records this as an owner-approved contract
  migration (2026-10-15, commit 40b0fb1); the scope of #400 is exactly to run `install_remote.sh`, write
  `remote.toml` and the `[preview]` section, so these bans are superseded by the backlog acceptance lines. All other
  bans and the dynamic executed-call check `assert_no_forbidden_calls` (only `tailscale version` may execute; no
  `pam-auth-update`, `authselect`, `curl`, `usermod`) are unchanged. Accepted as a justified migration, not weakening.
- New escape hatches (`#[ignore]`, `cfg(any())`, `should_panic`, tolerance): none.
- Inline `mod tests` changes: none.

## 3. Deep Reasoning Audit

### Logic & Architecture

Scenarios attempted:
- **Prerequisite cascade ordering**: `resolve_features` reads `FEATURE_VALUE[prereq]` under `set -u`; `COMPANION_KEYS`
  orders every prerequisite before its dependant (password_alerts → push → offline; camera_view, allow_funnel →
  camera_view_funnel), so there is no unset read. PASS.
- **Writing a combination `soos-remote` refuses (exit 78)**: the cascade forces dependants off; an explicit `on` with an
  off prerequisite is refused at parse time (no file) and again in `resolve_companion` (with the managed file) before
  any change. Verified against the real parser by `setup_render_tests.rs` (accepted and refused variants). PASS.
- **GUI-disabled feature kept off on re-run**: soos-gui's `disable_remote_keys` rewrites only `key = true` lines in place
  and keeps the marker comment, so the file stays "managed" and `FILE_BOOL[...] == false` is honoured. PASS.
- **User-written file**: anything without the marker and strict grammar (and not the exact untouched template) is
  classified `user` and never edited; with companion options it is refused (exit 2). Template detection requires
  exactly one `allowed_logins = []` and no other non-comment line, matching `install_remote.sh`'s template. PASS.
- **Validation step**: `soos-remote --config <file> passkeys list` runs `load_config`, `load_config_with_android` and
  `resolve_credentials_path` (EXIT_CONFIG 78) before the subcommand; an absent passkey store yields `Ok(None)` →
  "No passkey registered." exit 0, so a first install is not falsely refused. PASS.
- **Password alerts without journal access**: `soos-remote` logs "password alerts unavailable" and still starts, so the
  default-on profile does not fail with exit 5 on hosts where the user is not in `systemd-journal`. PASS.
- **Daemon `[preview]` file**: field names `enabled`, `allowed_uids`, `remote_view` match `PreviewConfigFile`;
  `from_toml_str` starts from `runtime_default()`, so a file holding only `[preview]` changes nothing else. PASS.
- **`--sysconfdir` other than `/etc`**: `DAEMON_TOML` follows `SYSCONFDIR`, while `soos-daemon` reads the hard-coded
  `DEFAULT_CONFIG_PATH = /etc/soos/daemon.toml`; see MINOR finding 1.
- **Exit-code composition**: root half 0 → companion failure becomes 5; root half 3 stays 3 with the companion failure
  printed; any other root exit skips the companion. Matches ADR item (2) and the docs table. PASS.

### PAM Concurrency & Deadlines

No change under `crates/pam` and no IPC or daemon code change. Every `systemctl` and `soos-remote` call in the new
phase is wrapped in `run_bounded` (`QUERY_TIMEOUT_S`, `timeout --kill-after=2`); the settle sleep is a fixed 2 s.
`install_remote.sh` (cargo build) is unbounded by design, as is the existing workspace build. PASS (PAM not affected).

### Panic Safety & Fail-Closed

No Rust production code. Shell fail-closed review: `run_companion_phase` runs in an `||` context, so every step checks
its status explicitly (`|| { companion_failed …; return; }`), and `return` propagates `EXIT_COMPANION` from
`companion_failed`. The config fingerprint is rechecked before writing; symbolic links are re-checked immediately
before the write; rollback restores a replaced file through a fresh 0600 temporary file and rename. A signal during the
phase triggers `rollback_companion`. No path turns a refusal into success. PASS.

### Test Integrity & Anti-Weakening

- SET3 migration reviewed in §2 (owner-approved, recorded in the ADR).
- The new tests can fail against plausible wrong implementations: the render is asserted byte-equal to a fixture that
  the real `soos-remote` parser must accept, and four never-written variants must be refused for their prerequisite
  (not for syntax); behavioural tests drive the real script with fake root/HOME and stub commands (exit 5 rollback,
  unit kept disabled, explicit `false` kept, daemon.toml never edited, root start skipped).
- Runs: `cargo test -p soos-remote --test setup_render_tests` → 3 passed; `cargo test -p soos-invariants -- setup` →
  50 passed, 0 failed, 0 ignored.
- SET3's new quote heuristic would accept a command substitution inside double quotes (`"$(tailscale funnel …)"`); see
  SUGGESTION 3. No such construct exists in the current `setup.sh` (manual check of every `tailscale` occurrence).

### Memory, Bounds & Secrets

- Reads are bounded: `remote.toml` at 16384 bytes (oversized → `user`, never edited), `daemon.toml` scan at 65536 bytes.
- `remote.toml` is written 0600 via `umask 077` + `mktemp` + rename in a directory checked to be owned and not group- or
  world-writable; the backup is 0600. `daemon.toml` is written 0644 by root via a temporary file + `ln` (never replaces
  an existing file).
- The login is validated by `^[A-Za-z0-9._%+@-]+$` (≤254) and rp_id by strict DNS label rules ending in `.ts.net`;
  neither can break out of a TOML string. The login is never printed in the plan or summary (only "login given"); the
  `passkeys list` output (credential fingerprints) is discarded. No password, frame or embedding is involved. PASS.

### Supply Chain & Automation

`scripts/` changed: no download, no `curl`/`wget`, no new tool, no `eval`. `install_remote.sh` now honours
`CARGO_TARGET_DIR` (relative values resolved against the checkout, as cargo does). No `Cargo.*`, `deny.toml` or
`.github/` change. `shellcheck` is not installed locally, so the CI lint result was not reproduced; `bash -n` passes. PASS.

### English-Only Policy

All added code, comments, docs, ADR, walkthrough and test text are English (non-ASCII grep of added lines: no hit). PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `scripts/setup.sh:931` (`DAEMON_TOML="${SYSCONFDIR}/soos/daemon.toml"`) — with a non-default
  `--sysconfdir`, setup creates `[preview]` in a file `soos-daemon` never reads (it reads only
  `/etc/soos/daemon.toml` unless `--config` is passed, and the installed unit passes none); the plan then reports
  "camera view (daemon): already done" although the view is not allowed. This follows the pre-existing convention of
  the other `SYSCONFDIR` paths, so it is not blocking. Required change (follow-up): skip the daemon side with a printed
  hint when `SYSCONFDIR` is not `/etc`, or document the limitation next to `--sysconfdir`.
- **[MINOR]** `scripts/setup.sh:2084` (`write_remote_config`) — the backup is moved with
  `mv -f -- "${tmp}" "${REMOTE_BACKUP}"` after only a symlink check; if `remote.toml.soos-setup.bak` is a directory,
  `mv` moves the temporary file into it and a later rollback cannot restore from it (an error is printed; the
  original content is still inside that directory). Required change (follow-up): in `resolve_companion`, refuse a
  `REMOTE_BACKUP` that exists and is not a regular file, alongside the symlink check at line 1198.
- **[SUGGESTION]** `tests/invariants/src/setup_contract.rs:165` (SET3) — the quote-parity heuristic also accepts
  `"$(tailscale serve …)"`, which would execute. The dynamic `assert_no_forbidden_calls` covers exercised paths only.
  A future contract could also reject `$(` or a backtick between the opening quote and `tailscale `.
- **[SUGGESTION]** `scripts/setup.sh:1624` (`phase_preview`) — a signal between `mktemp` and `rm` leaves a
  `.daemon.toml.XXXXXX` file in `<sysconfdir>/soos`; journaling the temporary path would let `rollback_journal`
  remove it.

## 5. Final Verdict

No CRITICAL or MAJOR finding. The single modified pre-existing test is an owner-approved contract migration recorded
in the ADR, and the contract tests pass.

**VERDICT: APPROVED**
