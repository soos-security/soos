# Lock-Screen Widget — `soos-lockstatus` and `soos-lock-widget`

Shows whether soos is scanning your face while the screen is locked, in the soos brand, on the
lock screens that allow it. Epic GitHub #352; this page covers PR 2 (GitHub #401): the
`soos-lockstatus` bridge, the **hyprlock** label and the **swaylock-plugin** background widget,
PR 4 (GitHub #405): the **GNOME Shell** system extension `soos-lockscreen@soos` (§11), and PR 6
(GitHub #432): the installer opt-in through `scripts/install.sh` and `scripts/setup.sh` (§3). The
KDE Plasma adapter follows in a later pull request (owner decision of 2026-10-13: swaylock-plugin
and hyprlock first, then GNOME, then KDE).

Decisions: ADR 2026-10-13 "Read-Only Lock-Screen State Channel" (the daemon side, PR 1, GitHub
#380) and ADR 2026-10-15 "Lock-Screen Adapters for swaylock-plugin and hyprlock" in
`AI/DECISIONS.md`, ADR 2026-10-17 "GNOME Shell Lock-Screen Extension" for §11, and ADR
2026-10-18 "Installer Opt-In of the Lock-Screen Adapters and the Shipped Widget Build" for §3. Wire contract: `Docs/IPC_PROTOCOL.md` §14. Motion: `Docs/UI_MOTION.md`.

## 1. Why adapters

Wayland's `ext-session-lock-v1` gives the locker exclusive ownership of every locked output, so no
other program can draw over a lock screen. That is a security property soos keeps. The daemon
therefore exposes one read-only state (`RequestKind::LockState`) and each locker gets a thin
adapter that reads it:

| Locker | Adapter | Animation |
|---|---|---|
| swaylock-plugin (`--command`, wlr-layer-shell background) | `soos-lock-widget` | full: star, ring, words |
| hyprlock (`label` with `cmd[update:N]`) | `soos-lockstatus --once` and `packaging/lockscreen/hyprlock-soos.conf` | coarse: a glyph and words per poll |
| GNOME Shell 45 or later (lock screen and unlock dialog) | system extension `soos-lockscreen@soos` (`packaging/gnome-shell/`), through `soos-lockstatus --watch` (§11) | full: star, ring, words |
| KDE Plasma | later PR, through `soos-lockstatus --watch` | — |
| plain swaylock, i3lock, xscreensaver | none (no extension point); use swaylock-plugin | — |

No adapter can unlock, start a scan, wake the camera or send input to the locker. Removing or
killing one leaves password and presence unlock exactly as they were.

## 2. What the lock screen shows

Only three states are ever shown (owner decision Q2 of 2026-10-13):

| State | When | swaylock-plugin widget | hyprlock label |
|---|---|---|---|
| Idle | no scan running, nothing to report — and also whenever the daemon cannot be asked (stopped, older than #380, refusing, socket missing, no private runtime directory) | pale ring, pink star at rest, no words | `✦` |
| Scanning | a face scan of **your** user is running: a PAM face request (Enter in the locker) or a presence auto-unlock scan (owner decision Q3) | bright ring, the star turns a quarter turn per 1.2 s, "Scanning" | turning `◜◝◞◟` (static `◌` with reduced motion) and "Scanning" |
| Not recognized | the last scan ended without verifying your face, **whatever the reason** (no face, low score, presentation attack, camera or model failure, timeout) | amber ring, one short shake, "Not recognized" | `✕` and "Not recognized" |

There is no "recognized" state: the screen unlocking is the success signal. The daemon reveals a
failure no earlier than 3.24 s after the scan started and holds it 3 s (`Docs/IPC_PROTOCOL.md`
§14), so the screen never tells a presentation attack from an ordinary non-match. Transport errors
look exactly like Idle on the screen; only `--watch` reports `"available":false` for diagnostics.

## 3. Build and install

Both programs run as the locked user; they need no privilege beyond the `soos` group membership
face unlock already requires (`/run/soos/daemon.sock` stays `0660 root:soos`).

**With the installer (default).** `scripts/install.sh` (and the packages, which stage through it
or list the same files) installs, root-owned:

| File | Mode |
|---|---|
| `/usr/bin/soos-lockstatus`, `/usr/bin/soos-lock-widget` (when built; a build without them installs and says so) | 0755 |
| `/usr/share/gnome-shell/extensions/soos-lockscreen@soos/` (`metadata.json`, `extension.js`, `stylesheet.css`) | 0755 / 0644 |
| `/usr/share/soos/lockscreen/hyprlock-soos.conf` (the snippet of §5) | 0644 |

Every file is journaled: a failed install removes them again, and `scripts/uninstall.sh` removes
exactly these files (never through a symbolic link; a file you added to the extension directory
keeps it). The installer enables nothing. `scripts/setup.sh` then opts you in, on by default:

- it detects `swaylock-plugin`, `hyprlock` and `gnome-shell` (`command -v` only; the dry run and
  the plan show what it found and what it will do);
- **GNOME**: as you (never root), from a local seat whose desktop is GNOME
  (`XDG_CURRENT_DESKTOP`), after the system phase it runs
  `gnome-extensions enable soos-lockscreen@soos`; GNOME Shell reads a new system extension at
  login, so on the first install the enable is usually refused: log out and back in, then run
  that command (setup prints it; a refusal never fails setup). From SSH, as root or from another
  desktop the command is printed instead;
- **swaylock-plugin** and **hyprlock**: setup prints the exact line (§4, §5) and never edits a
  locker configuration;
- `--lock-widget off` creates `/etc/soos/lockwidget.disable` (no GNOME card for any user; setup
  enables no adapter and prints no locker line, only an off notice saying that the swaylock-plugin
  widget and the hyprlock label do not read the file), `--lock-widget on` removes it, and without
  the option the current state is kept (`Docs/PACKAGING_AND_PROVISIONING.md` §3.0). The extension
  reads only `/etc/soos/lockwidget.disable`: with another `--sysconfdir` setup still records the
  request in `<sysconfdir>/soos/lockwidget.disable` but reports the step "not applied" and prints
  the exact `sudo touch` or `sudo rm -f` command for `/etc/soos/lockwidget.disable` (GitHub #451).

**Shipped build.** Every installer path (`install.sh --build`, `setup.sh`, the PKGBUILD, the Debian
rules and the RPM spec) runs `cargo build --release --locked --workspace`. That whole-workspace build
unifies features with `soos-gui`, which enables the `libwayland-client` backend of
`wayland-backend` loaded at run time (`dlopen`): the installed widget needs `libwayland-client.so.0`
(present wherever swaylock-plugin runs, since swaylock-plugin itself links it; a missing library
is reported as a setup error on stderr). Built on its own (`-p soos-lock-widget`) the widget uses
the pure-Rust backend and links no Wayland C library; that variant is not installed by any
script.

**By hand (user level, swaylock-plugin and hyprlock only).**

```sh
cargo build --locked --release -p soos-lockstatus -p soos-lock-widget
install -Dm0755 target/release/soos-lockstatus  ~/.local/bin/soos-lockstatus
install -Dm0755 target/release/soos-lock-widget ~/.local/bin/soos-lock-widget
```

## 4. swaylock-plugin (driftwm, sway, any wlroots compositor)

swaylock-plugin runs a wlr-layer-shell program as the animated lock background; the program must
"promptly provide a full-size background layer surface for each Wayland output" and is restarted
whenever it closes its connection (`swaylock-plugin(1)`). Add the widget to the command that
starts the locker, for example in the `SetLockedHint` wrapper of `Docs/DISTRIBUTION_DEPLOYMENT.md`
§5.5 (needed for presence auto-unlock on wlroots lockers):

```sh
swaylock-plugin --command 'soos-lock-widget'
```

After the installer, `scripts/setup.sh` prints this line with the absolute path
(`swaylock-plugin --command '/usr/bin/soos-lock-widget'`).

`--command` (one process for every output) is recommended. `--command-each` also works (one
instance per output); every instance shares the same coordination, so the daemon still sees one
connection at a time.

The widget draws the brand blue background, the pink star mark and a ring at 78 % of the screen
height, below swaylock's own indicator, with the words under the ring. It binds no seat (no
keyboard, pointer or touch), draws a `background` layer with `KeyboardInteractivity::None`, and
never covers the password prompt (swaylock-plugin draws its indicator on top).

Options: `--reduce-motion`, `--interval-ms N` (clamped to 250..=2000, default 500), `--socket PATH`
(diagnostics).

Budgets: at most 8 outputs; buffers at the logical size (scale 1; a HiDPI compositor upscales),
one buffer per output, a second only while the compositor still holds the first (never more than
two), at most 8192 pixels per side, 33 554 432 pixels per output and 67 108 864 pixels in total (an
output over budget gets no widget surface; that swaylock-plugin then shows its own colour there is
an assumption to confirm on hardware, matrix LKA14). Shared memory is therefore 4 bytes per pixel
of each output (128 MiB for an output at the per-output budget, 256 MiB for two of them) when the
compositor releases each buffer promptly, and at most twice that (512 MiB) on a compositor that
keeps holding the buffer it last received. The widget redraws at most 30 times per second and only
while something moves; once the state is stable it draws nothing.

## 5. hyprlock

Append `packaging/lockscreen/hyprlock-soos.conf` (installed as
`/usr/share/soos/lockscreen/hyprlock-soos.conf`) to `~/.config/hypr/hyprlock.conf`, for example
`cat /usr/share/soos/lockscreen/hyprlock-soos.conf >> ~/.config/hypr/hyprlock.conf` (the line
`scripts/setup.sh` prints; it never runs it):

```ini
label {
    monitor =
    text = cmd[update:500] soos-lockstatus --once --format label
    color = rgba(237, 241, 255, 1.0)
    font_size = 18
    position = 0, -160
    halign = center
    valign = center
}
```

`soos-lockstatus --once` prints one rendering without a newline and exits 0 within 300 ms in
every case (daemon down, socket missing, stalled answer), never writes to stderr (hyprlock renders
stdout as Pango markup; the output never contains `<`, `>` or `&`). Formats: `text` (words only,
nothing while idle), `glyph`, `label` (glyph, two spaces, words). Keep `update:500` or slower. The
glyphs come from the hyprlock host's font fallback (DejaVu Sans covers them).

## 6. `soos-lockstatus --watch` (desktop adapters, diagnostics)

```sh
soos-lockstatus --watch
{"state":"scanning","available":true,"sequence":3}
```

One JSON line (at most 96 bytes) for the first state and for every change; keys exactly `state`
(`idle`, `scanning`, `not_recognized`), `available` and `sequence` (the daemon's per-user transition
counter, restarting with the daemon). Polls every 500 ms by default (`--interval-ms`, clamped to
250..=2000: at most 4 Hz), backs off 1 s, 2 s, then 4 s while the daemon fails, and exits 0 when
stdout closes. The JSON is for programs; a lock screen must render `state` only.

## 7. Per-user coordination

The daemon allows each user two regular connections, one extra connection reserved for an `Auth`
request, and 8 `LockState` requests per second. Uncoordinated pollers (one hyprlock label per
monitor, one widget per output) could hold the connection `pam_soos.so` needs. Every
`soos-lockstatus` and `soos-lock-widget` process of a user therefore:

- takes an exclusive lock on `$XDG_RUNTIME_DIR/soos-lockstatus.lock` before contacting the daemon
  (so at most one connection exists at a time, one request per connection);
- reuses an answer at most 250 ms old stored in that file (so all clients together make at most
  about 4 requests per second), and after daemon failures waits 1 s, 2 s, then 4 s before the next
  attempt, shared by all clients;
- keeps showing the last state for a poll in which another client holds the lock (`--watch` and the
  widget; `--once` reads Idle when it cannot take the lock within its budget);
- never contacts the daemon when `XDG_RUNTIME_DIR` is missing, relative, a symlink, not owned by
  the user or accessible by group or others, or when the lock file is not a private (`0600`),
  single-link regular file of the user; the state then reads Idle (`--once` stays silent; the widget
  writes one line to stderr at start when `XDG_RUNTIME_DIR` is missing or not private, and stays
  silent for an unsafe lock file).

The file holds one line such as `v1 scanning 3 0 123456 6db2722693fa5425` (state, sequence,
failure count, `CLOCK_BOOTTIME` milliseconds, a hash of the socket path). It is the user's own
coarse state; another process of the same user can make the adapters show a wrong state or none,
never a success state and never another user's state.

Accepted residual: when two other clients of the same user (for example a `soos-gui` camera
preview and a `soos-remote` camera view) already hold both regular connections, the single
lock-status connection falls back to the reserved one for its short exchange; the daemon refuses
`LockState` there and the clients back off. The daemon cannot refuse it when the connection is
accepted, because the lock-status clients and `pam_soos.so` in the locker run as the same user; it
refuses it as soon as it has read and classified the request, without answering, without counting
it against the `LockState` quota, and it frees the reserved connection right away for the next
face `Auth`.

## 8. Reduced motion

`--reduce-motion`, else `SOOS_REDUCE_MOTION=1` (reduced) or `SOOS_REDUCE_MOTION=0` (full motion),
else full motion. Reduced motion: colours change at once, the star never turns, no shake, static
glyphs. The desktop portal setting is not read on the lock screen (that would need a D-Bus client
in this path, which the lock-state ADR excludes).

Motion with full motion (`Docs/UI_MOTION.md` tokens): the words change on the first frame; the ring
colour cross-fades in 180 ms; the star turns one quarter per 1200 ms with the standard easing,
never later than 4.8 s after the scan started, and always stops on a half-turn (the star repeats every 180°); a new
failure shakes the widget once by at most 8 px over 260 ms. While presence keeps failing (a new
scan about every 4.24 s), the turn continues from scan to scan.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Always idle | daemon stopped or older than #380; user not in `soos`; `XDG_RUNTIME_DIR` not private | `systemctl status soos-daemon`; `id`; `soos-lockstatus --watch` shows `"available":false` |
| Never "Scanning" on swaylock-plugin | presence needs `LockedHint` (wrapper of `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.5); PAM face runs only on Enter | add the wrapper and `swayidle` hooks |
| Widget missing, swaylock-plugin colour shown | no wlr-layer-shell in swaylock-plugin's nested server, `libwayland-client.so` missing (workspace build only), or an output over the pixel budget or out of shared memory | read the widget's stderr (swaylock-plugin forwards it): one line names the setup error or the skipped output, never the state |
| Always idle although the daemon runs and scans | the daemon's logind session check (at most 250 ms, `LOCK_STATE_SESSION_CHECK_TIMEOUT_MS`) answers after the client's 250 ms budget: every poll times out and backs off (the daemon would answer Idle on such a timeout anyway) | check `systemd-logind` health; `soos-lockstatus --watch` shows `"available":false` |
| GNOME: no card on the lock screen | extension not enabled for this user, shell version not in `metadata.json`, kill switch present, or no root-owned `soos-lockstatus` in `/usr/bin` or `/usr/local/bin` (the card then stays Idle) | §11: `gnome-extensions info soos-lockscreen@soos`; `journalctl --user -b \| grep soos-lockscreen` |
| hyprlock label empty | `soos-lockstatus` not on hyprlock's `PATH`; idle with `--format text` | use an absolute path; use `--format label` |

## 10. Security notes

- Nothing biometric leaves the daemon: the answer is three states and a counter. Neither program
  logs the state; neither has a logger.
- No new socket, no permission change, no privileged component, no network, no D-Bus.
- `pam_soos.so`, the daemon and the protocol are unchanged by these PRs.
- Kill switch: remove the `--command` argument or the hyprlock label. The `/etc/soos/lockwidget.disable`
  file (`scripts/setup.sh --lock-widget off`) applies to the GNOME adapter, where an extension
  runs inside the shell (§11); the swaylock-plugin widget and the hyprlock label do not read it.

## 11. GNOME Shell (system extension `soos-lockscreen@soos`)

PR 4 of #352 (GitHub #405, ADR 2026-10-17 "GNOME Shell Lock-Screen Extension"). GNOME Shell is
its own locker, so the adapter is a GJS extension in the `unlock-dialog` session mode (owner
decision of 2026-10-12: a **system** extension; extensions.gnome.org is out of scope). Files:
`packaging/gnome-shell/soos-lockscreen@soos/` (`metadata.json`, `extension.js`, `stylesheet.css`).

**What it does.** While the session is locked (lock screen and unlock dialog), the extension
runs `soos-lockstatus --watch` with `Gio.Subprocess` (fixed argument list, no shell), reads its
JSON lines (at most 256 bytes per line; a longer one stops the bridge) and shows one card at the
top centre of the primary monitor, below the top bar: brand blue, a ring, the pink star and the words
"Scanning" or "Not recognized", exactly the three states of §2. `"available":false` looks like
Idle. The card is not reactive and the extension connects no input signal: it cannot take
input, unlock, start a scan or wake the camera. When the session unlocks it stops the bridge
and removes the card; when the bridge ends it shows Idle and restarts it after 1 s, 2 s, 4 s,
then every 8 s while the session stays locked.

**Requirements.** GNOME 45 or later (the ESM extension format; `shell-version` lists 45 to 51).
`soos-lockstatus` must be a root-owned file, not writable by group or others, in `/usr/bin` or
`/usr/local/bin`: the extension never looks in `PATH` or in your home directory, so the
`~/.local/bin` install of §3 is not used here. The user must be in the `soos` group (§3).

**Install.** `scripts/install.sh` copies the extension to `/usr/share/gnome-shell/extensions/` and
`soos-lockstatus` to `/usr/bin`, and `scripts/setup.sh` runs `gnome-extensions enable` for you from
a local GNOME seat (§3). **Manual install**, the same steps by hand:

```sh
cargo build --locked --release -p soos-lockstatus
sudo install -Dm0755 -o root -g root target/release/soos-lockstatus /usr/local/bin/soos-lockstatus
sudo cp -r packaging/gnome-shell/soos-lockscreen@soos /usr/share/gnome-shell/extensions/
sudo chown -R root:root /usr/share/gnome-shell/extensions/soos-lockscreen@soos
# Log out and back in (the shell reads system extensions at start), then, as the user:
gnome-extensions enable soos-lockscreen@soos
gnome-extensions info soos-lockscreen@soos   # State: ACTIVE (ENABLED on GNOME 45)
```

Uninstall: `gnome-extensions disable soos-lockscreen@soos`, then remove
`/usr/share/gnome-shell/extensions/soos-lockscreen@soos`.

**Kill switch.** `sudo touch /etc/soos/lockwidget.disable` turns the card off for every user
without uninstalling anything; it is read at every lock, so it applies from the next lock.
Remove the file to turn the card back on. Disabling the extension, removing it, the kill switch
or an error inside the extension each leave the stock GNOME unlock dialog and password unlock
exactly as they are (presence unlock does not depend on the extension either).

**Reduced motion.** The extension follows `org.gnome.desktop.interface enable-animations`
(Settings → Accessibility → Reduce Animation, or
`gsettings set org.gnome.desktop.interface enable-animations false`), read through the shell:
the ring colour changes at once, the star does not turn and the card does not shake.
`SOOS_REDUCE_MOTION` and `--reduce-motion` (§8) do not apply to GNOME.

**Motion** (tokens of `Docs/UI_MOTION.md`, as in §8): ring colour cross-fade 180 ms; while
scanning the star turns one quarter per 1200 ms, at most 4.8 s after the scan started, and
always comes to rest on a half turn; a new Not recognized shakes the card once by at most 8 px
over 260 ms; the card fades in over 180 ms when the screen locks.

**Troubleshooting.**

| Symptom | Check |
|---|---|
| `gnome-extensions enable` prints that the extension does not exist | the shell reads system extensions at start: log out and back in after copying |
| State `OUT OF DATE` | the running GNOME version is not in `shell-version`; nothing is shown and the stock lock screen is unchanged |
| State `ERROR` | `journalctl --user -b -g soos-lockscreen` (or `journalctl -b /usr/bin/gnome-shell`); the stock lock screen is unchanged |
| Card always idle | `soos-lockstatus` missing from `/usr/bin` and `/usr/local/bin`, not root-owned, or writable by group or others (one warning per lock in the journal); otherwise as in §9 |
| No card at all | kill switch `/etc/soos/lockwidget.disable` present; extension disabled for this user |
