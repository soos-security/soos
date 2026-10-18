# Architect Spec — Companion Off-Switch and Opt-In Wording Aligned With the Setup Defaults (GitHub #423, part of #362)

- **Date**: 2026-10-17
- **Issue**: GitHub #423 (sub-issue of epic #362; no backlog id, so the branch is not registered in
  `scripts/sync_issue.py`; the commit carries `Closes #423` and `Refs #362`)
- **Branch**: `chore/companion-wording`
- **Base commit**: `74782b8` (`origin/main`)
- **Origin**: suggestions S-1, S-2 and S-4 of the #409 candid review
- **Lane**: help text and one summary line of `scripts/setup.sh`, `README.md`, `Docs/`, `AI/`,
  `tests/invariants` only. No runtime crate, packaging, workflow or asset change; no new
  dependency; no behaviour change of `setup.sh`.

## 0. Owner decisions

Relied on (recorded, none new):
- #400 and #362 OD-1: the one-command setup turns every phone companion feature on; a manual
  install (`scripts/install_remote.sh`) and the code defaults leave them all off.
- 2026-10-17: pull request only; the owner merges.

No open owner question: this change only makes existing text match the code.

## 1. Facts (verified on `74782b8`)

| Fact | Source |
|---|---|
| `soos-gui` (Features) switches: `allow_unlock`, `allow_funnel`, `push_notifications`, `camera_view`, `offline_alerts`; none for `password_alerts` or `camera_view_funnel` | `crates/gui/src/feature_control.rs` `RemoteSwitch::key` |
| Turning a switch off also clears its dependants: Funnel → `camera_view_funnel`; Push → `offline_alerts`; Camera view → `camera_view_funnel` | `RemoteSwitch::keys_to_clear` |
| `soos-remote` refuses `push_notifications` without `password_alerts` and `offline_alerts` without `push_notifications` (exit 78) | `crates/remote/src/config.rs` |
| `setup.sh --password-alerts off` turns push and offline alerts off with it ("needs password alerts"); only an explicit `--push on` with it is a usage error (exit 2) | `scripts/setup.sh` `resolve_features`, `FEATURE_PREREQS`; SET22 |
| Code defaults of `allow_funnel`, `allow_unlock`, `push_notifications` are `false`; Funnel requests are `403` while `allow_funnel` is `false` | `crates/remote/src/config.rs`, `Docs/REMOTE_COMPANION.md` §2b |

## 2. Changes

1. `scripts/setup.sh` `usage()` companion paragraph: replace "can be turned off later in soos-gui
   (Features)" for every feature with: Funnel, remote unlock, push, offline alerts and the camera
   view have a `soos-gui` (Features) switch, which also turns off the features that need them;
   the failed-password alerts have none and are turned off with `--password-alerts off`, or by
   setting `password_alerts`, `push_notifications` and `offline_alerts` to `false` in
   `remote.toml`. Every option line is unchanged (SET21).
2. `scripts/setup.sh` closing `info` line of the companion summary: same split, plus the
   unchanged `scripts/install_remote.sh --uninstall` hint.
3. `Docs/REMOTE_COMPANION.md` §2b example comment of `allow_funnel = true`: no "opt-in"; states the
   code default `false` (Funnel at `403`) and that `scripts/setup.sh` writes `true`.
4. `AI/ARCHITECTURE.md` companion table: "Push sender (opt-in)" and "Unlock (opt-in)" become
   "(off by default in code; on with `scripts/setup.sh`)"; cell text unchanged (the
   `default \`false\`` statement of Unlock stays).
5. `README.md` security model: the off-switch sentence also names push and offline alerts and
   says that turning push off also sets `offline_alerts` to `false`.

## 3. Contract (new module `tests/invariants/src/companion_wording_contract.rs`, rows CWD1–CWD5)

- **CWD1** `setup.sh` (non-comment lines, whitespace collapsed) never says "every feature" can be
  turned off in `soos-gui` (a sentence with "every feature" and "soos-gui" and no "except" or
  "no switch"); the `usage()` text names `--password-alerts off` and `password_alerts`,
  `push_notifications` and `offline_alerts` with `remote.toml`; the closing summary line names
  `--password-alerts off`.
- **CWD2** `RemoteSwitch::key` in `crates/gui/src/feature_control.rs` has no `password_alerts`
  arm (the premise of CWD1; a future switch must update the help and this row together).
- **CWD3** `Docs/REMOTE_COMPANION.md`: no `allow_funnel = true` example line calls it opt-in, and
  the commented one names `scripts/setup.sh`. `AI/ARCHITECTURE.md`: no table row headed
  "Push sender (opt-in)" or "Unlock (opt-in)"; the "Push sender" and "Unlock" row headers name
  `scripts/setup.sh`, and the Unlock row keeps `default \`false\``.
- **CWD4** `README.md` security section: the sentence with "off switch" names push and offline
  alerts.
- **CWD5** the CWD1 detector on synthetic input.

Out of scope (left alone by request): the Android app's own opt-in heartbeat wording, the daemon
`[preview]` `remote_view` opt-in, the network lock, and the other "opt-in" mentions of
`AI/ARCHITECTURE.md` §13 prose (lines 370 and 375 of the base).

## 4. Traceability

Matrix component `companion-wording` with rows CWD1–CWD5; walkthrough
`AI/walkthroughs/228_companion_wording.md`; no ADR (no decision, wording only).
