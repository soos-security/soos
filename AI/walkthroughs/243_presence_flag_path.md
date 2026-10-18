# Walkthrough 243 — Presence Flag Reported Applied Only at the Read Path

- **Date**: 2026-10-18
- **Issue**: GitHub #453, part of epic #362 (GitHub-only, no backlog id), refs #323 (presence
  auto-unlock) — **Branch**: `fix/presence-flag-path`
- **Decision**: ADR 2026-10-13 "One-Command Setup `scripts/setup.sh` and Default-On Installer
  Profile" amended (presence step); the open note of ADR 2026-10-15 "Setup User Phase" item 7 marked
  resolved. Same rule as the `daemon.toml` read path (#402) and the lock-widget flag (#451).
- **Matrix criteria**: PFP1–PFP4 (new, verified). SET1–SET18 unchanged.

## 1. Context & Objectives

`scripts/setup.sh` writes the presence kill switch at `<sysconfdir>/soos/presence.disable`, but
soos-daemon only reads `/etc/soos/presence.disable`: `crates/daemon/src/main.rs` builds
`PresenceSwitch::new(PathBuf::from(DEFAULT_KILL_SWITCH_DIR))` with `DEFAULT_KILL_SWITCH_DIR =
"/etc/soos"` and `PRESENCE_DISABLE_FLAG = "presence.disable"` (`crates/daemon/src/presence/mod.rs`);
no option, configuration key or unit argument changes it. With a non-default `--sysconfdir`,
`--presence off` created a file nothing reads and setup printed `[DONE] presence: auto-unlock turned
off`. ADR 2026-10-15 item 7 recorded the mismatch and left it out of scope.

## 2. Architect Design

- **Readers.** Every shipped source was searched (`crates/*/src`, `packaging/`, `scripts/`):
  - soos-daemon: `presence/mod.rs` (constants), `presence/switch.rs` (the check),
    `presence/worker.rs` (doc comment); fixed directory `/etc/soos`.
  - soos-gui: `feature_control.rs` (`SOOS_ETC_DIR = "/etc/soos"`, `SystemFlag::Presence`, status
    read through `paths.etc_dir`) and `onboarding.rs` (text naming `/etc/soos/presence.disable`).
  - `scripts/uninstall.sh`: a comment only (the flag is configuration and is kept).
  - No PAM, packaging or companion reader. So there is a single read path,
    `/etc/soos/presence.disable`.
- `setup.sh`: `readonly PRESENCE_READ_FLAG="/etc/soos/presence.disable"` next to `PRESENCE_FLAG`;
  `presence_flag_elsewhere` compares `PRESENCE_FLAG` with it (string comparison, like
  `DAEMON_READ_CONFIG`: a sysconfdir spelled differently, for example `/etc/`, is reported not applied,
  which is the safe side); `presence_not_applied off|on` gives "not applied: soos-daemon reads
  /etc/soos/presence.disable, not <flag> (--sysconfdir); as an administrator run: sudo touch|rm -f
  /etc/soos/presence.disable"; `presence_requested` gives the requested state (the option, or the
  recorded flag without it).
- **Why the file is still written.** SET18 (`test_set_presence_flag_toggles`, immutable) runs
  `--presence off|on` with the harness's fake `--sysconfdir` and requires the 0644 flag, the
  `setup.json` keys `"presence"` and `"presence_flag_created_by_setup"`, the plan text
  `[PLAN] presence: already done` and `--presence on` in a plain run; SET16 requires the rollback when
  the flag cannot be written. The flag under `<sysconfdir>` therefore stays setup's journaled record
  of the request. What changes is every report:
  - phase line: `[SKIP] presence: <flag> created|removed|off in|on in ..., not applied: ...` instead of
    `[DONE]` (the no-option report too); the exit code is unchanged;
  - plan line: `will run - create|remove <flag>, not applied: ...`, or `already done - off recorded in
    <flag>, not applied: ...` / `already done - on recorded (no <flag>), not applied: ...`;
  - `presence_report` (plan without option, phase without option and summary): with a non-default
    sysconfdir it names only the recorded state ("off recorded in <flag> (--presence on records on)"
    or "on recorded (no <flag>; --presence off records off)") followed by the note; it no longer says
    "auto-unlock off/on" or "everything off" from files no component reads. The `/etc` text is
    unchanged (`presence_state`).
- The status word `already done` that SET18 pins is kept; it describes setup's own step (the record
  is already as requested), and the same line says `not applied`, so no line claims that the daemon
  sees the state.
- Out of scope, recorded for a follow-up: the global kill switch `KILL_SWITCH="${SYSCONFDIR}/soos/disabled"`
  has the same class. With a non-default `--sysconfdir` the summary prints `Rescue: sudo touch
  <sysconfdir>/soos/disabled`, while `pam_soos.so` (`DEFAULT_FLAG_DIR`) and soos-daemon read only
  `/etc/soos/disabled`. Setup never writes that file; only the printed rescue command is affected.

## 3. Tester Contract (Red)

Added to `tests/invariants/src/setup_contract.rs` (new functions only, no existing assertion
touched):

- PFP1 `test_pfp1_presence_read_path_matches_the_daemon` — the pinned constant equals the daemon's
  `DEFAULT_KILL_SWITCH_DIR`/`PRESENCE_DISABLE_FLAG` and the GUI's `SOOS_ETC_DIR`/
  `PRESENCE_DISABLE_FLAG`; `main.rs` watches the constant directory; the decision is the comparison;
  every shipped source naming the flag is a known reader.
- PFP2 `test_pfp2_non_etc_sysconfdir_never_reports_the_presence_flag_applied` — six runs with the
  fake sysconfdir: no `[DONE] presence`, one `[SKIP]` and one plan line with "not applied", the read
  path and the exact command, the summary repeats it, no "auto-unlock turned off|on".
- PFP3 `test_pfp3_read_path_presence_flag_is_reported_done` — with the read path pointed at the fake
  root in the harness copy of the script, `[DONE] presence: auto-unlock turned off|on` and no
  "not applied".
- PFP4 `test_pfp4_documentation_and_traceability`.

All four failed before the implementation (assertion failures).

## 4. Auditor Constraints

1. No write outside `<sysconfdir>`: setup never touches `/etc/soos/presence.disable` when the
   sysconfdir differs; it only prints the command (no `eval`, paths passed through `printf '%s'`).
2. The symbolic-link refusals of `root_preflight` and `phase_presence` stay before any write.
3. The rollback journal is unchanged: a flag created or removed under `<sysconfdir>` is still reversed
   on a later failure (SET16).
4. `setup.json` is unchanged in grammar and content (SET10, SET18).
5. Exit codes unchanged: a not-applied step is `[SKIP]`, never a failure.
6. No change to `pam_soos.so`, soos-daemon, soos-gui or the protocol; no password, frame or embedding
   handled.

## 5. Developer Implementation (Green)

`scripts/setup.sh` (constant, three helpers, `presence_report` split into `presence_report` and
`presence_state`, `phase_presence`, the presence plan block, `--help`),
`Docs/PACKAGING_AND_PROVISIONING.md` §3.0 (presence row), ADR amendments, matrix PFP1–PFP4.

## 6. Verification

- `cargo test --locked -p soos-invariants -- --test-threads=3` (scoped target directory): all green,
  SET1–SET18 unchanged and passing.
- `bash -n scripts/setup.sh`.
