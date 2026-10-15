# Walkthrough 209 — First-Run Onboarding Tour and Feature Switches in `soos-gui`

- **Date**: 2026-10-13
- **Issue**: GitHub-only #379 (PR 1 of epic #363), no backlog id. **Branch**: `feat/onboarding-tour`. It is not registered in `scripts/sync_issue.py`; the commit message carries `Closes #379` and `Refs #363`.
- **Matrix criteria**: OBT1–OBT22 (OBT16–OBT18 are owner hardware checks, pending)
- **Numbering**: 200–208 are reserved by other open branches.

## 1. Context & Objectives

The owner decided on 2026-10-12 (#362, #358) that the install enables every feature by default (presence auto-unlock, phone companion, push alerts, Internet access through Funnel, live camera view), and that each feature must be explained by a first-run tour and stay toggleable in the GUI at any time, presence auto-unlock included. The tour must not be childish. #363 asks for the tour on the PC and on the phone; this PR delivers the PC part in `soos-gui`. The phone tour, and the motion of #358 / #377, are later PRs.

## 2. Architect Design

Spec: `AI/architect_spec_onboarding_tour.md`, with §10 Revision 1 after the first plan evaluation.

- **Tour** (`onboarding.rs`, pure): `Tour` pages are the introduction, nine feature pages (face unlock, presence auto-unlock, phone companion, remote lock and unlock, Internet access, push alerts, live camera view, Wi-Fi lock, offline alerts) and an overview. `Tour::generation` changes on every page or visibility change; the page content is keyed by it, so #377 can animate page changes without touching the logic.
- **Per-user state**: `TourStore` keeps `{"version":1,"seen":true}` in `$XDG_CONFIG_HOME/soos/gui-tour.json` (else `~/.config/soos/gui-tour.json`). Reads are bounded (4096 bytes), non-blocking and never follow a symbolic link. Writes are atomic (0600 temporary file, `fsync`, rename) and happen only when the path belongs to the effective uid.
- **State and routing**: `FeatureSnapshot` holds probed facts only: flag files, `[presence]` and `[preview]` of `daemon.toml`, the companion user unit, `remote.toml` keys, and the Wi-Fi lock rule files. `toggle_route` maps an on/off request to one of three routes: an in-GUI `FeatureAction`, an exact command to show, or "unavailable" with a reason.
- **Control** (`feature_control.rs`): fixed argument vectors and absolute programs. `pkexec /usr/bin/touch` / `/usr/bin/rm -f --` act on the closed set `/etc/soos/disabled` and `/etc/soos/presence.disable`; the Wi-Fi lock uses `pkexec soos-admin network-lock install|uninstall`; the companion uses `systemctl --user`. The `remote.toml` edit is off-only and verified by a TOML parse. `FeatureRunner` runs one action at a time on the `soos-gui-features` thread.
- **View** (`onboarding_view.rs`): an `egui::Modal` in the brand frame (pale fill, 2 px blue border, 20 px radius). Each feature page has a state chip, a switch, a shown command with a Copy button, or a reason. It does no I/O.
- **App**: on the first launch the tour opens before the first frame is drawn. The header **Features** button opens the overview. Every `TaskRunner` submission goes through `submit_task`, so only one Polkit dialog is ever open.

Owner decisions applied: OD-1 to OD-4 (spec header). Assumed, owner to confirm:

- **AS-1**: `soos-remote` keys are off-only in the GUI.
- **AS-2**: `touch` / `rm` under `pkexec` for the flags.
- **AS-3** (superseded 2026-10-15, spec §11): offline alerts shipped with #376 first; the tour now
  reads `offline_alerts`, offers its off switch, and turning push alerts off clears it too.
- **AS-4**: the location of the state file.

## 3. Plan Evaluation

- **Round 1: REVISION_REQUIRED.** There were three MAJOR findings:
  - The Wi-Fi lock "on" switch could silently delete an installed rfkill rule. `install` without `--rfkill` removes that rule, and on Arch the polkit rules directory is unreadable.
  - Presence "on" reported success while the global flag kept presence off.
  - The daemon configuration was never read.

  There were also eight MINOR findings: the Polkit exclusion did not cover every submit site, a FIFO could hang a read, the turn-on commands lacked their prerequisites, the runner's lifecycle, the coverage map, the per-service flags and masked units.
- **Round 2: APPROVED.** Revision 1 (R-1 to R-11) resolved all eleven findings.

## 4. Tester Contract

`AI/tester_contract_onboarding_tour.md`. Four new files, then one additive file after the audit:

- `onboarding_state_tests.rs`
- `onboarding_feature_tests.rs`
- `feature_control_tests.rs`
- `onboarding_view_tests.rs` (headless `egui::Context::run_ui`)
- `onboarding_audit_tests.rs` (added after the audit)

Red evidence: before the modules existed, `cargo test -p soos-gui --test onboarding_state_tests --no-run` failed with `error[E0432]: unresolved import soos_gui::onboarding`. No existing test was migrated. The exhaustive `PrivilegedAction` matches in `responsiveness_tests.rs` and `camera_source_tests.rs` are why feature changes use a separate runner.

## 5. Audit

`AI/auditor_constraints_onboarding_tour.md`: **Clearance: CLEARED**. The audit found three defects in the draft, all fixed:

1. The camera-view "on" command now names `camera_view = true` and the prerequisites, unless the daemon side alone keeps the view off.
2. `mark_seen` and the app refuse a state path in another user's tree. Without this, `sudo -E soos-gui` would leave a root-owned `~/.config/soos` behind, and that breaks `soos-remote`.
3. A dead worker reports a failed change.

Hardening also applied:

- the temporary file is removed only when this call created it;
- reads use `O_NOCTTY`;
- a source test pins the single `submit_task` path;
- a test kills a child at the deadline.

## 6. Implementation Notes

- `toml` (workspace 0.8, already in `Cargo.lock`) is a new `soos-gui` dependency, with no new crate. `daemon.toml` is read through the daemon's own `soos_camera_v4l::daemon_config::read_daemon_config_text`.
- `SOOS_ADMIN_PROGRAM` follows `SOOS_BINDIR`, as `SOOS_ENROLL_PROGRAM` does (GitHub #318).
- Wording rules are enforced by a test: ASCII English, no exclamation mark, no childish vocabulary.

## 7. Verification

Gates were run with a private target directory: `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`, `cargo test --locked --workspace --all-targets --all-features`, `cargo deny --locked check` and `./scripts/candid_review.sh` (layer 1). The final report of the run lists the results.

## 8. Owner Checks (OBT16–OBT18)

1. On the Arch laptop, as the desktop user, start `soos-gui`. The tour opens.
   - Skip it, then relaunch: it does not reopen, and `~/.config/soos/gui-tour.json` has mode 0600.
   - **Features** reopens the overview, and "Replay the tour" restarts the tour.
2. Turn presence auto-unlock off. Polkit asks for the administrator password, `/etc/soos/presence.disable` appears, the state shows Off, and a locked session stays locked in front of the camera.
   - Turn it back on.
   - Repeat with face unlock (`/etc/soos/disabled`) and the Wi-Fi lock.
   - A cancelled dialog leaves the state unchanged and shows an error.
3. With the phone companion installed, turn off remote unlock, Internet access, push alerts and camera view.
   - Only those `remote.toml` lines change, comments are kept and the mode stays 0600.
   - `soos-remote` restarts and the phone page reflects the change.
   - The companion switch stops and starts `soos-remote.service`.
   - Pasting each shown "on" command works.

## 9. Follow-ups

- The phone tour (web app, native app #357), the next PR of #363.
- Motion on page changes once `feat/gui-motion` lands (#377).
- A GUI "on" switch for `soos-remote` keys once #362 ships its checked TOML editor.
- A dedicated `soos-admin` flag subcommand, to replace `touch` / `rm`.
- Check `ActiveState` after `systemctl --user try-restart soos-remote.service`, so a refused
  configuration is never reported as success (candid review of the rebase, 2026-10-15).
