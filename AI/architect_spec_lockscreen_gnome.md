# Architect Spec — GitHub #405: GNOME Shell Unlock-Dialog Extension `soos-lockscreen@soos`

- **Date**: 2026-10-17 — **Branch**: `feat/lockscreen-gnome` — **Base**: `origin/main` `9e82795`
- **Issue**: GitHub #405 = PR 4 of the epic #352 (lock-screen face-scan widget). GitHub-only: no
  `AI/BACKLOG.md` entry, not registered in `scripts/sync_issue.py`; the commit carries `Closes #405`
  and `Refs #352`.
- **Owner decisions applied** (#352): comment of 2026-10-12 ("one adapter per environment … GNOME
  Shell system extension (unlock-dialog session mode)"; extensions.gnome.org out of scope); Q1 of
  2026-10-13 (swaylock-plugin and hyprlock first — merged as #403 —, then GNOME, then KDE); Q2 (only
  Scanning, Idle and Not recognized, every failure reason collapsed); Q3 (animate for PAM face
  requests and presence scans alike).
- **ADRs applied without amendment**: 2026-10-13 "Read-Only Lock-Screen State Channel", 2026-10-15
  "Lock-Screen Adapters for swaylock-plugin and hyprlock" (its item 8 defers the
  `/etc/soos/lockwidget.disable` kill switch to this adapter), 2026-10-13 "Motion System for soos-gui
  and soos-remote" (tokens), 2026-10-06 "soos Brand Direction" (palette).
- **Out of scope**: KDE Plasma (PR 5), installer opt-in and packaging of the extension and of
  `soos-lockstatus` (PR 6), extensions.gnome.org, any change to `pam_soos.so`, the daemon, the
  protocol or `soos-lockstatus`.

## 1. Context

`soos-lockstatus --watch` (PR #403) prints one JSON line per state change: keys exactly `state`
(`idle`, `scanning`, `not_recognized`), `available` and `sequence`, at most 96 bytes, polling the
daemon at most 4 Hz with shared per-user coordination and back-off. GNOME Shell does not use
`ext-session-lock-v1` from a client: the shell itself is the locker, and the only supported way to
add content to its lock screen is an extension whose `session-modes` include `unlock-dialog`. The
adapter is therefore a GJS extension that runs the existing bridge and maps its three states to one
actor. It adds no Rust code.

## 2. Files

| Path | Role |
|---|---|
| `packaging/gnome-shell/soos-lockscreen@soos/metadata.json` | uuid, name, description, `session-modes`, `shell-version`, `url` |
| `packaging/gnome-shell/soos-lockscreen@soos/extension.js` | ESM extension (`export default class … extends Extension`) |
| `packaging/gnome-shell/soos-lockscreen@soos/stylesheet.css` | brand palette, ring, label, card |
| `tests/invariants/src/lockscreen_gnome_contract.rs` | static contract (CI has no gnome-shell) |
| `tests/invariants/src/lib.rs` | `mod lockscreen_gnome_contract;` (one new declaration, no assertion touched) |
| `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md` (LKG1–LKG11), `Docs/LOCKSCREEN_WIDGET.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md` §5, `AI/walkthroughs/215_lockscreen_gnome.md` | records |

## 3. `metadata.json`

```json
{
  "uuid": "soos-lockscreen@soos",
  "name": "soos lock screen",
  "description": "…read-only face-scan status on the GNOME lock screen…",
  "session-modes": ["user", "unlock-dialog"],
  "shell-version": ["45", "46", "47", "48", "49", "50", "51"],
  "url": "https://github.com/soos-security/soos"
}
```

- `session-modes` exactly `["user", "unlock-dialog"]`: never `gdm` (the greeter never shows face
  status; initial login never uses face, epic Out of scope). `user` must be listed: an extension
  that lists only `unlock-dialog` is not enabled in the regular session and would be enabled and
  disabled at every lock, losing nothing but adding churn; listing both keeps it enabled across the
  lock (the shell's `_sessionUpdated` keeps an extension whose modes contain the new mode).
- `shell-version`: 45 is the first ESM release (the `import` / `export default` format this file
  uses does not load before 45). Every shell API the extension uses was verified in the GNOME 45.0
  sources (`js/ui/layout.js`, `js/ui/sessionMode.js`, `js/misc/animationUtils.js`,
  `js/extensions/sharedInternals.js`, `js/ui/extensionSystem.js`) and in the GNOME 51.1 sources
  embedded in the locally installed `libshell-51.so`: `Main.layoutManager.screenShieldGroup` (an
  `St.Widget` with a `Clutter.BinLayout`, identical in both), `Main.sessionMode.isLocked` and its
  `updated` signal, `St.Settings.get().enable_animations` (the shell's mirror of
  `org.gnome.desktop.interface enable-animations`, read by `adjustAnimationTime` in both),
  `St.DrawingArea` `repaint` with `get_context()` / `get_surface_size()` / `cr.$dispose()`,
  `Gio.Subprocess.new`, `TextDecoder`. The declared list is therefore 45 through 51. **Owner to
  confirm** the list (the task proposed "45–48+"); the owner's Ubuntu host is the hardware check
  (LKG8). A version outside the list is refused by the shell (the stock lock screen stays as is).

## 4. Behaviour of `extension.js`

### 4.1 Lifecycle

- `enable()`: creates nothing visible; stores a `St.Settings` handle, connects
  `Main.sessionMode` `updated` and `St.Settings` `notify::enable-animations` (both through
  `connect` with the returned ids stored), then calls `_sync()`.
- `_sync()` (also the `updated` handler): `wanted = Main.sessionMode.isLocked && !killSwitch()`.
  When wanted and not shown: build the actor, add it to `Main.layoutManager.screenShieldGroup`
  (last child = drawn on top), start the bridge. When not wanted: stop the bridge, destroy the
  actor. The kill switch is re-read at every lock (`GLib.file_test(KILL_SWITCH_PATH, EXISTS)`), so
  creating or removing the file takes effect at the next lock without a shell restart.
- `disable()`: carries the review-required comment explaining the `unlock-dialog` use
  ("this extension shows read-only face-scan status on the unlock dialog, so it stays enabled in
  the unlock-dialog session mode; everything it created is removed here"); disconnects both
  signal ids, removes every GLib source (turn timer, respawn timer), stops the bridge
  (`Gio.Cancellable.cancel()`, `force_exit()`), destroys the actor, drops every reference. Every
  step is in its own `try`/`catch` so one failure never leaves another resource behind.

### 4.2 Bridge

- Binary: the first candidate of `SOOS_LOCKSTATUS_CANDIDATES = ['/usr/bin/soos-lockstatus',
  '/usr/local/bin/soos-lockstatus']` that is a regular file owned by root (`unix::uid` 0), not
  writable by group or others (`unix::mode & 0o022 == 0`) and executable
  (`access::can-execute`). No `PATH` lookup, no home directory, no shell: a same-user process can
  already rewrite a user extension, but a system extension running inside the shell must not
  execute a user-writable file. None found → Idle forever for this lock (nothing logged
  repeatedly; one `console.warn` per lock without any state). **Owner to confirm** the candidate
  list (the installer PR decides where the package puts the binary; the manual steps of
  `Docs/LOCKSCREEN_WIDGET.md` install it to `/usr/local/bin`).
- Process: `Gio.Subprocess.new([path, '--watch'], STDOUT_PIPE | STDERR_SILENCE)` (stdin is
  `/dev/null` by default). argv is a fixed array; `GLib.spawn_*` and `/bin/sh` are never used.
- Reading: `read_bytes_async(READ_CHUNK_BYTES = 256)` on the stdout pipe with the bridge's
  `Gio.Cancellable`; bytes are accumulated in a pending buffer split on `\n`. A pending line longer
  than `MAX_LINE_BYTES = 256` (the bridge's lines are at most 96 bytes) is a protocol error: the
  view becomes Idle and the bridge is stopped and respawned after the back-off. EOF or a read error
  (not a cancellation) → Idle, respawn after back-off.
- Parsing (`parseLine`): `JSON.parse` in `try`; accepted only if the value is a plain object whose
  key set is exactly `{state, available, sequence}`, `state` is one of the three strings,
  `available` is a boolean and `sequence` a non-negative safe integer. Anything else → the line is
  ignored (the last view is kept; a valid line follows within the bridge's interval). Only these
  three fields are read.
- Mapping (`present`): `available === false` → `idle`; else `state`. There is no other visible
  state and no success rendering.
- Respawn: when the bridge ends while the session is still locked, a `GLib.timeout_add` source
  restarts it after `RESPAWN_DELAYS_MS = [1000, 2000, 4000, 8000]` (index saturating, reset once a
  valid line arrives). At most one bridge process and one respawn source exist at any time.

### 4.3 Actor

One non-reactive `St.Widget` (`reactive: false`, `can_focus: false`, `track_hover: false`) named
`soos-lockscreen`, style class `soos-lock-card`, horizontal `Clutter.BoxLayout`, aligned
top-centre (`x_align CENTER`, `y_align START`, `margin-top` in CSS) inside a non-reactive frame
(`BinLayout`) that carries `Layout.MonitorConstraint({primary: true})` — the constraint the
shell's own unlock dialog box uses (`unlockDialog.js` in 51.1), so the card sits on the primary
monitor and never between two monitors of the stage-wide shield group —, itself the last child of
the shield group: below the top panel, above the centred clock and prompt stack (GNOME's unlock layout
keeps the top tenth of the height free). **Owner to confirm** the position. Children:

- ring: `St.Widget` (style class `soos-lock-ring` plus `soos-idle` / `soos-scanning` /
  `soos-not-recognized`), CSS circle with a 3 px border; inside it the star:
- star: `St.DrawingArea` (style class `soos-lock-star`, `color` = PINK in CSS) drawing the two
  spikes of `STAR_SPIKES` (`crates/gui/src/brand.rs`, view box 508) with cairo, pivot at its
  centre;
- label: `St.Label` (style class `soos-lock-label`), text `""`, `"Scanning"` or
  `"Not recognized"`; hidden while Idle.

Nothing connects any input, key or pointer signal; the only signals connected anywhere are
`Main.sessionMode` `updated`, `St.Settings` `notify::enable-animations` and the star's `repaint`.

### 4.4 Motion (`Docs/UI_MOTION.md` tokens, as in soos-lock-widget)

Constants: `T_SHORT_MS = 180`, `T_MEDIUM_MS = 260`, `T_PENDING_MS = 1200`, `SLIDE_PX = 8`,
`PENDING_LOOP_MAX_MS = 4800`.

- Words change at once; the ring colour cross-fades through the CSS `transition-duration: 180ms`
  of `.soos-lock-ring`, overridden to `0ms` by `.soos-reduced-motion`.
- Scanning: a repeating `GLib.timeout_add(T_PENDING_MS)` source eases the star by +90° per tick
  (`Clutter.AnimationMode.EASE_OUT_CUBIC`, the closest stock mode to `EASE_STANDARD`) while the
  state is Scanning and less than `PENDING_LOOP_MAX_MS` passed since the current scan's start (a
  new `sequence` while scanning restarts that start, as presence repeats); once those stop holding
  it adds one more quarter only if the angle is not on a half-turn (the mark repeats every 180°),
  then removes itself.
- Not recognized: a new non-zero `sequence` shakes the card once: `translation_x` 8, -8, 4, 0 over
  four legs of `T_MEDIUM_MS / 4`.
- Reduced motion (`!St.Settings.get().enable_animations`): no turn timer, no shake, the
  `soos-reduced-motion` class on the card; changes of the setting take effect at once (a running
  turn finishes its current ease and stops).

### 4.5 Kill switch

`KILL_SWITCH_PATH = '/etc/soos/lockwidget.disable'`: when it exists at the time of a lock, nothing
is built and no process is started. Removing the extension, `gnome-extensions disable
soos-lockscreen@soos`, or the kill switch each leave the stock unlock dialog unchanged.

### 4.6 Error containment

Every callback (`updated`, `notify::enable-animations`, `repaint`, async read, `wait_async`, GLib
sources, ease `onComplete`) is a `try`/`catch` whose `catch` renders Idle or stops quietly and logs
one fixed message with `console.warn` — never the state, a line, a path from the bridge or an
exception carrying bridge output. An exception escaping `enable()`/`disable()` is caught by the
shell's extension system, which marks the extension as errored and leaves the lock screen to the
shell (owner hardware row LKG9).

## 5. Security invariants

| # | Invariant | Enforcement |
|---|---|---|
| S1 | cannot unlock: no `Main.screenShield` method call, no `unlock`, `deactivate`, `ensureUnlockDialog`, `_onUnlockFailed`, `LoginManager`, `Gdm`, `AuthPrompt` | static test LKG3 |
| S2 | no input: no key, button, motion, scroll, touch, `captured-event`, `event`, `grab` signal or call; actor non-reactive | static test LKG3 |
| S3 | no shell spawn: `Gio.Subprocess` with a fixed argv only; no `GLib.spawn`, `/bin/sh`, `bash`, `-c` | static test LKG4 |
| S4 | only `state`, `available`, `sequence` read; exactly three states | static test LKG5 |
| S5 | bounded reading (`MAX_LINE_BYTES`, `READ_CHUNK_BYTES`) | static test LKG5 |
| S6 | disable cleans up: `disable()` comment, `force_exit`, `cancel`, `destroy`, `GLib.Source.remove`, `disconnect` | static test LKG6 |
| S7 | kill switch path present and checked before building | static test LKG7 |
| S8 | PAM, daemon, protocol and `soos-lockstatus` unchanged | static test LKG7 (`git diff` against the merge base when available) and the PAM Docker matrix (LKG10) |
| S9 | no network, no D-Bus proxy, no `Soup`, no `Gio.DBus`, no `GSettings` write | static test LKG4 |

## 6. Acceptance mapping

| Row | Criterion | Evidence |
|---|---|---|
| LKG1 | metadata: uuid, `session-modes` exactly `user` + `unlock-dialog`, `shell-version` a list of major versions ≥ 45, no `gdm` | `lockscreen_gnome_contract::test_lkg1_*` |
| LKG2 | ESM format and syntax: `export default class`, `extends Extension`, imports only from `gi://` and `resource:///org/gnome/shell/`; `node --check` when available | `test_lkg2_*` |
| LKG3 | no unlock path, no input signal, non-reactive actor | `test_lkg3_*` |
| LKG4 | subprocess without shell, absolute candidate paths, no network or D-Bus | `test_lkg4_*` |
| LKG5 | three states, three fields, bounded line | `test_lkg5_*` |
| LKG6 | `disable()` cleanup and comment; every callback in `try`/`catch` | `test_lkg6_*` |
| LKG7 | kill switch; reduced motion via `enable_animations`; PAM/daemon/protocol untouched | `test_lkg7_*` |
| LKG8 | brand palette in CSS mirrors `theme.rs`; motion tokens mirror `Docs/UI_MOTION.md`; star spikes mirror `brand.rs`; documentation and ADR | `test_lkg8_*` |
| LKG9 | owner hardware (Ubuntu GNOME): the actor shows on the lock screen and the unlock dialog; Scanning / Not recognized follow PAM and presence scans; reduced motion honoured | Pending |
| LKG10 | owner hardware: an extension exception, `gnome-extensions disable` and the kill switch each leave the stock unlock dialog and password unlock working | Pending |
| LKG11 | CI and the PAM Docker matrix green (PAM, daemon, protocol unchanged) | Pending (first CI run on the pull request) |

## 7. Owner to confirm (defaults assumed)

1. `shell-version` list `45`–`51`.
2. Actor position: top centre below the panel.
3. Binary candidates `/usr/bin` then `/usr/local/bin`, root-owned only.
4. `EASE_OUT_CUBIC` as the stock approximation of `EASE_STANDARD` (no custom timeline).
