# Plan Evaluation Report
- **Date**: 2026-10-17 (round 1)
- **Issue**: GitHub-only (no backlog id) — GNOME Shell unlock-dialog extension `soos-lockscreen@soos` (GitHub #405, PR 4 of epic #352)
- **Branch**: `feat/lockscreen-gnome`
- **Base commit**: `9e82795`
- **Plan evaluated**: `AI/architect_spec_lockscreen_gnome.md` §1–§7
- **Mode**: self-check by the implementing agent against `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/BACKLOG.md`, `AI/VERIFICATION_MATRIX.md` and the GNOME Shell sources; the independent candid review follows separately.

## 1. Coverage Matrix

| Requirement (issue #405 / epic #352 / owner) | Spec element | Status |
|---|---|---|
| System extension, `unlock-dialog` session mode (owner 2026-10-12) | §3 `session-modes` `["user", "unlock-dialog"]`, uuid `soos-lockscreen@soos`, never `gdm` | Covered |
| Declared `shell-version` range starting at the first ESM release | §3: 45–51, APIs checked in 45.0 and 51.1 sources; owner to confirm | Covered |
| `soos-lockstatus --watch` through `Gio.Subprocess`, absolute path, bounded lines | §4.2 candidates, fixed argv, 256-byte chunks and lines | Covered |
| Only Scanning / Idle / Not recognized; `available:false` like Idle (Q2) | §4.2 parse and `present`, §4.3 labels | Covered |
| Animate for PAM and presence scans alike (Q3) | §4.4 driven by the bridge's state only (the daemon already publishes both) | Covered |
| Every callback in `try`/`catch`; no keyboard or input signal | §4.6, §5 S2 | Covered |
| `disable()` comment and full cleanup | §4.1 | Covered |
| `enable-animations` reduced motion | §4.4 via `St.Settings` | Covered |
| Kill switch `/etc/soos/lockwidget.disable` (ADR 2026-10-15 item 8) | §4.5 | Covered |
| Cannot unlock, scan, wake the camera or send input | §5 S1–S3, S9 | Covered |
| Static contract tests (no gnome-shell in CI) | §6 LKG1–LKG8 | Covered |
| PAM, daemon, protocol unchanged | §5 S8, LKG7, LKG11 | Covered |
| ADR, `Docs/LOCKSCREEN_WIDGET.md` (row, install, troubleshooting, kill switch), `Docs/DISTRIBUTION_DEPLOYMENT.md` §5, matrix rows with pending owner hardware rows, walkthrough 215 | §2 files, §6 | Covered |

## 2. Facts Verified

| Fact cited by the plan | Check | Actual | Match |
|---|---|---|---|
| ESM extensions start at 45 | `js/extensions/extension.js` 45.0 exports `class Extension`; 44 used `imports` | ESM since 45 | Yes |
| `Main.layoutManager.screenShieldGroup` is a `BinLayout` `St.Widget` | `js/ui/layout.js` 45.0 line 267, 51.1 line 264 | identical | Yes |
| `ScreenShield.actor` is that group | 51.1 `screenShield.js` line 54 | `this.actor = Main.layoutManager.screenShieldGroup` | Yes |
| `Main.sessionMode.isLocked` and `updated` | `js/ui/sessionMode.js` (`unlock-dialog` has `isLocked: true`; `emit('updated')`) | present in 45 and 51 | Yes |
| `enable_animations` drives `ease` | `js/misc/animationUtils.js` `adjustAnimationTime` returns 0 when `!St.Settings.get().enable_animations` | 45 and 51 | Yes |
| A thrown `enable()`/`disable()` is contained | `js/ui/extensionSystem.js` 45.0: `try { … stateObj.disable(); } catch (e) { this.logExtensionError(uuid, e); }`, same for enable | contained | Yes |
| Extensions stay enabled across a lock when their modes include `unlock-dialog` | 51.1 `_extensionSupportsSessionMode` checks `currentMode` and `parentMode` | Yes | Yes |
| `Clutter.BoxLayout` has `orientation` and `spacing`; `EASE_OUT_CUBIC`, `rotation-angle-z`, `translation-x`, `set_pivot_point` | `Clutter-51.gir` | present | Yes |
| `St.DrawingArea` `repaint` + `get_context()` + `$dispose()`; theme foreground colour has `red/green/blue/alpha` | 45.0 and 51.1 `switcherPopup.js` `drawArrow`; 51.1 `environment.js` `setSourceColor` reads those fields | present | Yes |
| `Layout.MonitorConstraint({primary: true})` exported and used by the unlock dialog | `js/ui/layout.js` 45.0 line 41 and 51.1 line 30 (`export const MonitorConstraint`), 51.1 `unlockDialog.js` line 772 | present | Yes |
| `--watch` contract: keys `state`, `available`, `sequence`, ≤ 96 bytes | `crates/lockstatus/src/format.rs` `json`, matrix LKA5 | Yes | Yes |
| `PENDING_LOOP_MAX_MS` = 4800 | `crates/lockstatus/src/motion.rs` line 23 | 4800 | Yes |
| `gnome-extensions enable` on an unknown extension | 45.0 `command-enable.c`: prints "Extension … does not exist" | Yes | Yes |
| `gnome-extensions info` state names | 45.0 `STATE_ENABLED`, 46+ `STATE_ACTIVE` | documented both | Yes |

## 3. Pillars

1. **Security invariants.** No path to unlock or input; no privileged component; no new socket; the extension only reads three coarse states the locked user's own processes can already read. Executing only root-owned binaries closes the "system extension executes a user-writable file" gap. No logging of state or bridge output. Pass.
2. **Fail-safe.** Every failure renders Idle or removes the card; the shell contains exceptions; the stock unlock dialog is never touched (no `Main.screenShield` call, card added as a non-reactive child). Pass.
3. **Bounds.** One process, one restart timer, one turn timer, 256-byte lines, pending buffer ≤ 512 bytes; back-off 1/2/4/8 s; the bridge itself polls at most 4 Hz. Pass.
4. **Traceability.** LKG1–LKG8 static, LKG9–LKG10 owner hardware, LKG11 CI. Pass.
5. **Scope.** No change to PAM, daemon, protocol, `soos-lockstatus`; KDE and installer untouched. Pass.

## 4. Findings

- F1 (MINOR, accepted): the change-set guard of LKG7 only runs in full clones (the CI test job is shallow); the PAM Docker matrix and the candid review cover the PR itself.
- F2 (INFO): the `shell-version` list, the card position, the binary candidates and the easing approximation are owner-to-confirm defaults (spec §7), not decisions invented here.

VALIDATION_VERDICT: APPROVED
