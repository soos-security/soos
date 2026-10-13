# Candid Review Report

- **Date**: 2026-10-13
- **Target Branch**: `feat/one-command-install`
- **Base (merge-base)**: `6f32ef8`
- **Reviewed-Diff-Fingerprint**: `3eff69a541a8c64679233e78f82d6b4ddb5f4345efa03200c0fdd707c6933aec`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_one_command_install.md`, `AI/auditor_constraints_one_command_install.md`, `AI/tester_contract_one_command_install.md`, `AI/walkthroughs/200_one_command_install.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `README.md`, `scripts/setup.sh`, `tests/invariants/src/lib.rs`, `tests/invariants/src/setup_contract.rs`

## 1. Executive Summary

Re-review of `scripts/setup.sh` (GitHub #372, PR 1 of #362) after the previous
CHANGES_REQUESTED verdict (fingerprint `62a7a749...`). All three earlier findings are fixed in
production code, and new contract tests would fail against the old behavior:

1. MAJOR (SSH session could enable enrollment): `session_is_local` now returns three states:
   0 local, 1 positively not local, 2 unknown. `detect_local_seat` falls back to the
   `Display` session only on state 2, only in the user half, and only when a target user is
   known (`scripts/setup.sh:441-487`).
2. MINOR (`DAEMON_STOPPED` cleared too early): the marker is now set before
   `systemctl stop` and cleared only after a successful `systemctl start`
   (`scripts/setup.sh:1036-1058`).
3. MINOR (root environment leaking into the build): `as_build_user` now runs
   `runuser -u <user> -- env -i` with an explicit allow-list (`scripts/setup.sh:1312-1326`).

No new CRITICAL or MAJOR defect was found. Verdict: APPROVED.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (only adds `mod setup_contract;`) and
  `tests/invariants/src/setup_contract.rs` (new in this PR; every test in it is new).
- Removed or changed assertions (`^-` lines with assert, `#[test]`, proptest or
  should_panic): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, should_panic, tolerance, epsilon):
  **none**.
- Inline test modules added or removed: none.

No pre-existing test from `main` is modified, so no test is weakened. The new tests and stub
changes since the last review:

- `STUB_LOGINCTL` now answers per session id: the desk `Display` session (`c2`) is local by
  default, and the shell's own session is controlled by `STUB_REMOTE`, `STUB_SEAT` and
  `STUB_SELF_FAIL`. This makes the mixed local/remote scenario testable, as the earlier
  SUGGESTION asked.
- `test_set_remote_own_session_never_borrows_the_desk_session`: the own session is
  `Remote=yes` or has no seat while the desk session is local. The test asserts the session is
  reported as remote, `loginctl show-user` is never called, `soos-enroll` never runs and the
  command is printed. With the old code (fallback on any non-local answer), the desk session
  would be found local and enrollment would run, so this test would fail. Verified by reading
  both versions.
- `test_set_seat_falls_back_to_the_display_session_in_the_user_half`: the fallback is now
  triggered by `STUB_SELF_FAIL` (own session unknown) instead of the old shared remote stub.
  Started as root, there is still no fallback.
- `test_set_full_run_as_root_builds_as_invoking_user`: sets root `CARGO_HOME`, `RUSTUP_HOME`
  and `RUSTFLAGS`, then asserts the stub cargo sees all three empty and sees `HOME`/`USER` of
  the build user. Without `env -i`, the test would fail.
- `test_set_daemon_stop_marker_cleared_only_after_restart`: a static check that every
  `DAEMON_STOPPED=false` inside `phase_enrollment` comes after `systemctl start "${UNIT}"`.
  It is a structural check, not a behavioral one, but it is enough for a signal-race ordering
  that cannot be reproduced deterministically.
- The stubs now set default values for `STUB_LOG`, `STUB_STATE`, `STUB_HOME` and
  `STUB_FAKES`, so they keep logging under the script's `env -i`. This is a harness setup
  change only and relaxes no assertion.

`cargo test -p soos-invariants -- --test-threads=4`: 632 passed, 0 failed.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Owner decision on #362 (enrollment from a local seat only):
  - `su - alice` or `sudo -u alice -i` over SSH: the SSH variables are dropped, but
    `loginctl show-session self` resolves to the SSH session, which answers `Remote=yes`.
    That gives state 1, which is final: not local. PASS.
  - Root shell over SSH: `detect_local_seat false` never falls back. PASS.
  - Any `SSH_*` variable set: not local immediately. PASS.
  - `Remote=no` with an empty or invalid `Seat`: state 1, final. PASS.
  - The own session is unknown to logind (a terminal under the user service manager): the
    fallback uses `Display` of the target user, and that session must itself be
    `Remote=no` with a seat. This is the documented rule. PASS.
  - The session id is validated (`^[A-Za-z0-9]{1,32}$`) before it is passed to `loginctl`;
    every query is bounded to 5 s. PASS.
- The root half takes the seat verdict only from `--local-seat yes|no` (a closed enum). The
  internal options are refused outside `--system-phase`, and `--system-phase` requires uid 0.
  PASS.
- Parsing:
  - os-release is read line by line with `case`, quotes and non-printable characters are
    stripped, values are bounded to 80 characters, and tokens are split with `read -a`. It is
    never sourced or evaluated.
  - The state file uses a strict regex grammar with fixed keys.
  - There is no `eval` and no `source` anywhere in the script (also enforced by
    `test_set_s1`). PASS.
- `./scripts/setup.sh --dry-run` on the reviewer host printed the detection and the plan,
  exited 0, and left the working tree unchanged. PASS.

### PAM Concurrency & Deadlines

No `crates/pam` change. Setup never activates PAM: there is no `pam-auth-update` and no
`authselect select`, which `test_set_s3` enforces statically. The only Tailscale command is
`tailscale version`, which is read-only and bounded. External queries are bounded by
`run_bounded`, `wait_daemon_ready.sh --timeout 30` and a 15 s limit on the camera list. PASS.

### Panic Safety & Fail-Closed

No Rust production code changed. The script runs under `set -euo pipefail`.

Daemon hand-off:

- `DAEMON_STOPPED=true` is set before the stop.
- Scenario: a signal arrives during `soos-enroll` or during `systemctl start`. Bash runs the
  trap after the child returns, `on_signal` exits, and `on_exit_system` sees the marker still
  true and starts the unit (an idempotent start). PASS.
- Scenario: `systemctl start` fails. The marker stays true, the EXIT trap retries, and the
  exit code is mapped to 70. PASS.
- Scenario: the stop fails. Enrollment is skipped, the unit is started, and the run reports a
  failure. PASS.

Any doubt in the local-seat detection resolves to "not local". PASS.

### Test Integrity & Anti-Weakening

See section 2. No pre-existing assertion was removed or relaxed. The new tests discriminate
between the old and the fixed behavior. PASS.

### Memory, Bounds & Secrets

- **State file**:
  - It lives at `<localstatedir>/lib/soos/state/setup.json`, written by root in a `0700`
    directory, created with `umask 077` and `mktemp`, set to `chmod 0600`, and moved
    atomically.
  - Symbolic links are refused for both the directory and the file.
  - On read, the file must be owned by the effective uid (`-O`), at most 4096 bytes, not
    group- or world-writable, and must record the same user.
  - It holds no secret. PASS.
- **Rollback scope**: only the actions journaled during this run are undone
  (`group_added`, `presence_flag_created`, `presence_flag_removed`,
  `soos_conf_dir_created`). Packages, the toolchain and the `install.sh` install are never
  removed by setup. PASS.
- **Privilege flow**:
  - The build never runs as root: as root it runs under `runuser` with `env -i` and the
    allow-list; build user `root` or uid 0 is refused.
  - The root half is re-executed once through `sudo --` or `doas --`, with every input passed
    as an argument.
  - Root's `PATH` is kept, prefixed with `~/.cargo/bin`. It is controlled by root, so this is
    acceptable. PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/` or hook change. Packages come only from
`check_build_deps.sh --print-packages`, and the toolchain only from the verified
`install_rustup.sh`. The build uses `--locked`. PASS.

### English-Only Policy

The patch has no non-ASCII accented characters. The code, comments, ADR, docs and walkthrough
200 are all in English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `scripts/setup.sh:471`: a terminal multiplexer started at the desk and
  later attached over SSH keeps the desk `XDG_SESSION_ID` and has no `SSH_*` variables in old
  windows, so its own session is the local desk session. This is an inherent logind limit, not
  a regression. A sentence about it in `Docs/PACKAGING_AND_PROVISIONING.md` would be enough.
- **[SUGGESTION]** `tests/invariants/src/setup_contract.rs:1674`: the stop-marker ordering is
  checked structurally. A behavioral test (a `systemctl start` stub that signals the parent)
  could be added later.
- shellcheck is not installed on the reviewer host, so it was not run here. CI quality jobs
  still apply.

## 5. Final Verdict

**VERDICT: APPROVED**
