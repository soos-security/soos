# Walkthrough 244 — Setup Read Paths: the Rescue Kill Switch and the Normalised Path Options

- **Date**: 2026-10-18
- **Issue**: GitHub #455, part of epic #362 (GitHub-only, no backlog id), follow-up of #402, #452 and #454 — **Branch**: `fix/setup-read-paths`
- **Decision**: ADR 2026-10-18 "Setup Read Paths: the Rescue Kill Switch and One Lexical Normalisation of the Path Options" (amends ADR 2026-10-13 "One-Command Setup"; completes the read-path rule of ADR 2026-10-15 "Setup User Phase" item 7 and ADR 2026-10-18 "Installer Opt-In of the Lock-Screen Adapters" item 4)
- **Matrix criteria**: SRP1–SRP6 (new, verified). SET*, LKI*, LWF*, PFP* and DCC* unchanged.

## 1. Context & Objectives

PRs #402 and #452 introduced the read-path rule of `scripts/setup.sh`: a step that writes a file
below `<sysconfdir>` is reported done only when that file is the one its reader opens
(`DAEMON_READ_CONFIG`, `LOCK_WIDGET_READ_FLAG`); PR #454 (still open) adds `PRESENCE_READ_FLAG`.
Two gaps were left:

1. **Rescue command.** The summary printed `Rescue: sudo touch <sysconfdir>/soos/disabled`
   (`KILL_SWITCH="${SYSCONFDIR}/soos/disabled"`). `pam_soos.so` reads the global kill switch only at
   `/etc/soos/disabled` (`crates/pam/src/config.rs`: `DEFAULT_FLAG_DIR = "/etc/soos"`,
   `self.flag_dir.join("disabled")`, `flag_dir` set only by `Default`, no module argument) and so does
   soos-daemon (`crates/daemon/src/presence/mod.rs`: `DEFAULT_KILL_SWITCH_DIR = "/etc/soos"`,
   `GLOBAL_DISABLE_FLAG = "disabled"`; `main.rs` builds `PresenceSwitch` from the constant). Setup never
   writes that file, but under a non-/etc `--sysconfdir` the printed rescue command named a file no
   component reads: an administrator following it would not stop face PAM.
2. **Spelling of `--sysconfdir`.** The read-path comparisons compare strings. `absolute_dir` already
   stripped trailing slashes (`/etc/` and `/etc//` were `/etc`), but kept repeated inner slashes and
   `.` components (`//etc/./` was reported "not applied") and accepted `..`.

## 2. Architect Design

- `setup.sh`: `readonly KILL_SWITCH_READ_FLAG="/etc/soos/disabled"` next to the other read-path
  constants; the summary prints `Rescue: sudo touch ${KILL_SWITCH_READ_FLAG}`. `KILL_SWITCH` stays: the
  presence state report still inspects `<sysconfdir>/soos/disabled` (with #454 that report is only
  reached when `<sysconfdir>` is `/etc`, where both paths are equal).
- `absolute_dir` (every path option, `--sysconfdir` included, and the internal `--artifact-dir`):
  after the existing validation (absolute, no newline) the value is split on `/`; empty and `.`
  components are dropped, a `..` component calls `usage_error` (exit 2, nothing changed), the rest is
  joined back (`/` when nothing is left). No `realpath`, `readlink` or `cd`: the normalisation never
  touches the file system, so a symbolic link to `/etc` is not `/etc`.
- Why refuse `..` instead of resolving it: `/a/link/..` is `/a` by text but the kernel resolves it
  through the link's target; resolving it would need to follow links, which the read-path comparison
  must never do.
- Since `DAEMON_TOML`, `LOCK_WIDGET_FLAG`, `PRESENCE_FLAG` and `KILL_SWITCH` all derive from the one
  normalised `SYSCONFDIR`, every comparison (including the `presence.disable` rule of #454 after its
  merge) sees the normalised value. The value passed to the root half (`--system-phase`) and to
  `install.sh` is the normalised one, and normalising it again changes nothing.
- Help text and `Docs/PACKAGING_AND_PROVISIONING.md` ("Read paths (GitHub #455)") document both.

## 3. Tester Contract (Red)

Added at the end of `tests/invariants/src/setup_user_phase_contract.rs` (new functions only; the file
is not touched by #454, so the two PRs do not conflict on the test files):

- SRP1 `test_srp1_rescue_kill_switch_matches_pam_and_daemon` — the pinned constant equals the PAM and
  daemon constants; `flag_dir` is set only from `DEFAULT_FLAG_DIR`; the single `Rescue:` line prints it.
- SRP2 `test_srp2_summary_rescue_names_the_read_kill_switch` — a full run with the fake `--sysconfdir`
  prints `Rescue: sudo touch /etc/soos/disabled`.
- SRP3 `test_srp3_slash_spellings_of_the_read_directory_are_applied` — with the read paths pointed at
  the fake root (as LWF3 does), `<etc>/`, `<etc>//` and `<parent>//etc/./` give a dry-run plan that
  creates `daemon.toml` and `lockwidget.disable` there and never says "not applied".
- SRP4 `test_srp4_double_slash_sysconfdir_run_reports_the_flag_done` — a real run with `<etc>//`
  reports the lock-widget flag `[DONE]` at the normalised path.
- SRP5 `test_srp5_links_are_not_followed_and_dot_dot_is_refused` — a link to the read directory stays
  "not applied"; `..` in `--sysconfdir` or `--prefix` exits 2 with no change.
- SRP6 `test_srp6_documentation_and_traceability`.

Red run: SRP1, SRP2, SRP3 (third spelling), SRP5 and SRP6 failed; SRP4 and the first two spellings of
SRP3 already passed (the trailing-slash stripping existed) and pin that behaviour.

## 4. Auditor Constraints

1. No `eval`, no `realpath`/`readlink`/`cd` in the normalisation; pure parameter expansion.
2. The loop consumes at least one character per iteration (bounded by the argument length).
3. A refused value exits 2 through `usage_error` before any detection, plan or change.
4. The rescue command names a constant, never a value derived from an option.
5. No existing test is modified; new helpers carry an `srp_` prefix so no name collides with #454's.

## 5. Developer Implementation

- `scripts/setup.sh`: `KILL_SWITCH_READ_FLAG`, the rescue line, the new `absolute_dir`, help text.
- `Docs/PACKAGING_AND_PROVISIONING.md`, `AI/DECISIONS.md` (new ADR entry), `AI/VERIFICATION_MATRIX.md`
  (component `setup-read-paths`, SRP1–SRP6).

## 6. Verification

- `bash -n scripts/setup.sh`.
- `cargo test --locked -p soos-invariants -- --test-threads=3`: all green, SRP1–SRP6 included.

## 7. Follow-ups

- After #454 merges, a rebase keeps both sets of tests; the presence comparison then uses the
  normalised `SYSCONFDIR` without further change.
