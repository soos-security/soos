# Architect Spec — GitHub #432: Installer Opt-In of the Lock-Screen Adapters (PR 6 of #352)

- **Epic**: GitHub #352 (lock-screen widget). Owner decisions: #352 (2026-10-12) "installer detection and opt-in of the adapters"; #362 (2026-10-12/13) the installer enables everything by default and each feature can be turned off. KDE stays deferred (#421).
- **Branch**: `feat/lockscreen-installer`. **ADR**: 2026-10-18 "Installer Opt-In of the Lock-Screen Adapters and the Shipped Widget Build". **Matrix**: LKI1–LKI12. **Walkthrough**: 233.
- **Out of scope**: no change to `crates/pam`, `crates/daemon`, `crates/protocol`, `crates/lockstatus`, `crates/lock-widget` or `packaging/gnome-shell/` (LKG7 change-set guard); no Tailscale command (SET3); no PAM activation.

## 1. Files

| File | Change |
|---|---|
| `scripts/install.sh` | optional artifacts `soos-lockstatus`, `soos-lock-widget`; GNOME extension and hyprlock snippet copied, journaled |
| `scripts/uninstall.sh` | removes the above, never through a symbolic link |
| `scripts/setup.sh` | `--lock-widget on\|off`, locker detection, plan lines, root phase `lock widget`, user step `lock screen` |
| `packaging/arch/PKGBUILD`, `packaging/rpm/soos.spec` | install and list the same files |
| `tests/invariants/src/lockscreen_installer_contract.rs` | new contract LKI1–LKI10 |
| `Docs/LOCKSCREEN_WIDGET.md` §3, §4, §5, §10, §11; `Docs/PACKAGING_AND_PROVISIONING.md` §3.0, §3.2, §5 | documentation |

## 2. install.sh

- `OPTIONAL_ARTIFACTS=(soos-lockstatus soos-lock-widget)`: looked up in the resolved artifact directory with the rule of the required ones (`-f` and `-s`); present → `ARTIFACT_PATHS`, absent → `SKIPPED_ADAPTERS` and one `[INFO]` line. Never a preflight failure, never part of `MISSING_ARTIFACTS` (so neither `--allow-missing` nor the `INCOMPLETE` banner is involved). Rationale: the existing contracts (`installer_contract`, `packaging_ownership_contract`, `setup_contract`) stub only the five required artifacts and must stay green unmodified.
- Step 4b, after the required binaries and before the models (so the model-failure rollback covers it): `tracked_install 0755` of each present adapter into `TARGET_BIN_DIR`; the extension's three files (`metadata.json`, `extension.js`, `stylesheet.css`, an explicit list, never a glob) with `tracked_install 0644` into `${DESTDIR}${PREFIX}/share/gnome-shell/extensions/soos-lockscreen@soos` (`ensure_dir ... 0755 owned`; the parents are system directories, never re-moded); the snippet with `tracked_install 0644` into `${DESTDIR}${PREFIX}/share/soos/lockscreen/hyprlock-soos.conf`. A checkout without the sources skips with a notice.
- A symbolic link in place of either target directory: `error` and `exit 1` (the EXIT trap rolls back).
- `--prefix` other than `/usr` or `/usr/local`: two warnings (the extension trusts only `/usr/bin` and `/usr/local/bin`; GNOME Shell searches `/usr/share` and `/usr/local/share`).
- Plan block and epilogue mention the lock screen. Nothing is enabled.

## 3. uninstall.sh

Step 3b: `rm -f` both programs; if the extension directory is a symbolic link → warning, untouched; if a directory → `rm -f` of the three files, `rmdir` (warning when other files remain); snippet removed unless its directory is a link; `rmdir` of `share/soos/lockscreen` and `share/soos` (quiet). The kill switch stays (configuration, like `presence.disable`).

## 4. setup.sh

### 4.1 Option and paths

`--lock-widget <on|off>` → `LOCK_WIDGET` (empty: keep). `LOCK_WIDGET_FLAG=<sysconfdir>/soos/lockwidget.disable`, `GNOME_EXT_DIR=<prefix>/share/gnome-shell/extensions/soos-lockscreen@soos`, `HYPRLOCK_SNIPPET=<prefix>/share/soos/lockscreen/hyprlock-soos.conf`, `LOCK_WIDGET_BIN=<prefix>/bin/soos-lock-widget`. Passed to `--system-phase` as `--lock-widget <v>`.

### 4.2 Root half

`phase_lock_widget` right after `phase_presence`, same shape: symlink refusal, `off` → `ensure_soos_conf_dir`, `set -C` create, 0644, journal `lock_widget_flag_created`; `on` → `rm`, journal `lock_widget_flag_removed`; empty → report. `rollback_journal` reverses both. `root_preflight` refuses a symlinked flag. `print_summary` adds `Lock widget: <report>` and, for a root start (no user half), the locker lines.

### 4.3 State file (format 1 unchanged)

Optional lines, written only away from the default and placed before `"enrolled"`: `"lock_widget_flag_created_by_setup": true` (sticky: `PREV_LW_FLAG_CREATED` or created now) and `"lock_widget": "off"` (flag present). `read_state` accepts exactly these values; anything else is `state_invalid`. A default run therefore writes the exact SET10 content.

### 4.4 Detection and plan (read-only)

`detect_lockers`: `command -v` of `swaylock-plugin`, `hyprlock`, `gnome-shell` → `LOCKERS`. `[DETECT] lock screen: ...`. Plan: `lock widget` (like presence) and `print_lockscreen_plan` after the companion plan: no locker → one skipped line; widget off → one skipped line; else per locker: GNOME `will run` when `gnome_enable_blocker` is empty (not root, `LOCAL_SEAT`, `XDG_CURRENT_DESKTOP` contains `GNOME` case-insensitively, `gnome-extensions` found), otherwise `printed only (<reason>)`; swaylock-plugin and hyprlock `printed only` with the exact line.

### 4.5 User step

After the root half returned 0 or 3, before the companion: `run_lockscreen_step` → when on and unblocked, `run_gnome_enable_step`: extension installed? → `gnome-extensions list --enabled` (bounded; exact-line match) → already done; else `gnome-extensions enable soos-lockscreen@soos` (bounded) → done, or the log-out-and-run step. Never changes `rc`. Then `print_lockscreen_lines` (swaylock-plugin and hyprlock lines; the GNOME command when blocked; when off, the notice that those two adapters do not read the flag).

## 5. Security invariants

1. No locker program is ever executed; detection is `command -v` only.
2. `gnome-extensions` runs only in the user half (never root), only from a local GNOME seat, with a fixed argument list and a timeout.
3. No write to `$HOME` or any locker configuration; no `gsettings`/`dconf`.
4. Flag files: created with `set -C` (no clobber, no symlink follow), 0644, journaled; symlinks refused.
5. Installed files root-owned on a live install (root preflight), 0755/0644, no group/other write (the GNOME extension's trust rule for `soos-lockstatus`).
6. Every new install.sh write journaled before the late steps; no write through a symbolic link.
7. Messages print paths with `%s` only; no secret is involved anywhere.

## 6. Plan-evaluator self-check

- ARCHITECTURE/ADR alignment: no daemon/PAM/protocol change; IPC untouched; ADR 2026-10-15 item 8 (the two wlroots adapters do not read the kill switch) kept, so no crate change. VERDICT APPROVED.
- Existing contracts: SET10 exact state content (optional keys only), SET5 help (option documented), SET3 forbidden strings (none added), SET7/SET11 (no extra writes; `system_up_to_date` compares the adapter files only when their sources exist, which the setup harnesses do not copy), installer and packaging stubs (optional artifacts), LKG7 guard (no `packaging/gnome-shell/` change), `installer_templates_contract` empty `share/soos` removal (still removed).
- Risk: GNOME Shell does not see a system extension installed in the running session; handled as a printed step, never a failure.

## 7. Auditor constraints (Phase 3)

- A1: no `eval`, no unquoted expansion, every new function errexit-safe (conditions in `if`, no trailing `&&` as the last command of a function).
- A2: every external call in the user step bounded by `run_bounded "${QUERY_TIMEOUT_S}"`.
- A3: no new write outside `<sysconfdir>/soos`, the state file and install.sh's journaled targets.
- A4: no change to an existing test assertion; new contracts must fail on origin/main (9 of 10 do; LKI3 is a regression guard that passes vacuously on origin/main because nothing touched the adapters).
