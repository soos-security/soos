# Walkthrough 242 — Lock-Widget Flag Reported Applied Only at the Read Path

- **Date**: 2026-10-18
- **Issue**: GitHub #451, part of epic #352 (GitHub-only, no backlog id), follow-up of PR #434 (#432) — **Branch**: `fix/lockwidget-flag-path`
- **Decision**: item (4) of ADR 2026-10-18 "Installer Opt-In of the Lock-Screen Adapters and the Shipped Widget Build" amended (same rule as the `daemon.toml` read path of ADR 2026-10-15 "Setup User Phase" item 7)
- **Matrix criteria**: LWF1–LWF6 (new, verified). LKI1–LKI12 unchanged.

## 1. Context & Objectives

The review of PR #434 found that `scripts/setup.sh` writes the lock-widget kill switch at
`<sysconfdir>/soos/lockwidget.disable`, while its only reader, the GNOME Shell extension
(`packaging/gnome-shell/soos-lockscreen@soos/extension.js`, `KILL_SWITCH_PATH`), hard-codes
`/etc/soos/lockwidget.disable`. With a non-default `--sysconfdir`, `--lock-widget off` created a file
nothing reads and setup printed `[DONE] lock widget: turned off`. This is the class fixed in #402 for
`daemon.toml` (`DAEMON_READ_CONFIG`, state `elsewhere`).

The same review listed minors, fixed here: the documentation said "nothing enabled or printed" while
setup prints an off notice; the GNOME success line promised the card at the next lock although GNOME
Shell may first need a log-out; the rollback paragraph of `Docs/PACKAGING_AND_PROVISIONING.md` left out
the lock-widget and camera-view failures; the documentation promised a `setup.sh --uninstall` that does
not exist; `uninstall.sh` checked `share/soos/lockscreen` but not `share/soos` for a symbolic link.

## 2. Architect Design

- **Readers.** The whole tree was searched: only `extension.js` reads `lockwidget.disable`.
  `soos-lockstatus`, `soos-lock-widget` and the hyprlock snippet do not (ADR 2026-10-15 item 8), so
  there is a single read path.
- `setup.sh`: `readonly LOCK_WIDGET_READ_FLAG="/etc/soos/lockwidget.disable"` next to
  `DAEMON_READ_CONFIG`; `lock_widget_flag_elsewhere` compares `LOCK_WIDGET_FLAG` with it;
  `lock_widget_not_applied off|on` gives "not applied: the GNOME extension reads
  /etc/soos/lockwidget.disable, not <flag> (--sysconfdir); as an administrator run: sudo touch|rm -f
  /etc/soos/lockwidget.disable".
- **Why the file is still written.** Unlike `daemon.toml` (never written elsewhere), the existing
  contract LKI6 (an immutable test) runs `--lock-widget off` with the harness's fake `--sysconfdir`
  and requires the 0644 flag, the `setup.json` keys and the plan text `already done - off`. The flag
  under `<sysconfdir>` therefore stays setup's journaled record of the request; what changes is
  every report:
  - phase line: `[SKIP] lock widget: <flag> created|removed|off in|on in ..., not applied: ...`
    instead of `[DONE]` (the no-option report too);
  - plan line: the existing status and detail, followed by `, not applied: ...`;
  - `lock_widget_report` (plan without option and summary): no "no GNOME card" claim, plus the note;
  - lock-screen plan with the widget off: no "no GNOME card" claim;
  - summary: a `[WARN] Lock screen (GNOME card): not applied: ...` line when the widget is off or
    `--lock-widget on` was given.
- GNOME success line: "...; the card shows from the next lock once GNOME Shell has loaded it (if it
  does not, log out and back in)".
- `uninstall.sh`: `SOOS_SHARE_DIR="${DESTDIR}${PREFIX}/share/soos"`; when it is a symbolic link, one
  warning and nothing below it is removed (the Arch PAM snippet and the hyprlock snippet blocks are
  both skipped, and so is `rmdir`). The `share/soos/pam` and `share/soos/lockscreen` link checks stay.
- Out of scope, recorded: `presence.disable` (the daemon reads `/etc/soos`, `DEFAULT_KILL_SWITCH_DIR`)
  has the same latent mismatch, already noted in ADR 2026-10-15 "Setup User Phase" item 7; its
  contract SET18 also writes under the fake `--sysconfdir`.

## 3. Tester Contract (Red)

Added to `tests/invariants/src/lockscreen_installer_contract.rs` (new functions only, no existing
assertion touched):

- LWF1 `test_lwf1_read_path_matches_the_gnome_extension` — the pinned constant equals the
  extension's `KILL_SWITCH_PATH`; no other shipped component reads the flag.
- LWF2 `test_lwf2_non_etc_sysconfdir_never_reports_the_flag_applied` — six runs with the fake
  sysconfdir: no `[DONE] lock widget`, one `[SKIP]` and one plan line with "not applied", the read path
  and the exact command, the summary repeats it, no "no GNOME card".
- LWF3 `test_lwf3_read_path_flag_is_reported_done` — with the read path pointed at the fake root in
  a copy of the script, `[DONE] lock widget: turned off|on` and no "not applied".
- LWF4 `test_lwf4_gnome_success_line_mentions_the_log_out`.
- LWF5 `test_lwf5_uninstall_refuses_a_linked_share_soos` — failed before the fix: the hyprlock
  snippet behind the link was removed.
- LWF6 `test_lwf6_documentation_and_traceability`.

All six failed before the implementation.

## 4. Auditor Constraints

1. No write outside `<sysconfdir>`: setup never touches `/etc/soos/lockwidget.disable` when the
   sysconfdir differs; it only prints the command (no `eval`, every path quoted in `printf '%s'`).
2. The symbolic-link refusals of `root_preflight` and `phase_lock_widget` stay before any write.
3. The rollback journal is unchanged: the flag created or removed under `<sysconfdir>` is still
   reversed on a later failure.
4. A default run still writes a byte-identical format-1 `setup.json` (SET10, LKI6).
5. `uninstall.sh` stays `set -euo pipefail` clean: every new test is in an `if` or an `||` chain.
6. No change to `pam_soos.so`, the daemon, the protocol, `soos-lockstatus`, `soos-lock-widget` or
   `packaging/gnome-shell/` (LKG7 guard).

## 5. Developer Implementation (Green)

`scripts/setup.sh` (constant, three helpers, `lock_widget_report`, the lock-screen plan and summary
lines, `phase_lock_widget`, the plan block, the GNOME success line, `--help`), `scripts/uninstall.sh`
(`SOOS_SHARE_DIR` guard), `Docs/LOCKSCREEN_WIDGET.md` §3, `Docs/PACKAGING_AND_PROVISIONING.md` §3.0
(phase table, state-file paragraph, rollback paragraph), ADR amendment, matrix LWF1–LWF6.

## 6. Verification

- `cargo test --locked -p soos-invariants` (scoped target directory, three threads): all green,
  LKI1–LKI10 unchanged and passing.
- `bash -n scripts/setup.sh scripts/uninstall.sh`.
