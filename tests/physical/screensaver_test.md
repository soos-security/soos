# Screen Locker, Display Manager, and Console Integration Validation Procedure

This operational manual documents the physical validation procedures, PAM stack configurations, expected behavior matrices, latency budgets, and security invariants for integrating `soos` local facial verification with Linux screen lockers, graphical display managers, and console elevation mechanisms.

---

## 1. Overview and Architecture Alignment

The `soos` biometric PAM module (`pam_soos.so`) provides local facial verification to PAM-aware applications without modifying application user interfaces. In accordance with `AI/ARCHITECTURE.md` §3 and §5:

- **Unprivileged PAM Boundary**: Lock screens and display managers execute `pam_soos.so` within unprivileged or worker process contexts. `pam_soos.so` never opens `/dev/video*` directly and performs zero AI inference.
- **Privileged Daemon Coordination**: Capture, face detection, alignment, PAD liveness verification, and embedding extraction occur inside the background `soos-daemon` root process via the local Unix domain socket (`/run/soos/daemon.sock`).
- **Fail-Closed Fallback Invariant**: Any anomaly, camera occlusion, multiple faces, unrecognized face, or daemon stoppage must degrade silently to standard password authentication (`PAM_IGNORE`). Under zero circumstances may an error yield `PAM_SUCCESS`.

---

## 2. Universal PAM Stack Architecture

Across all screen lockers and display managers, `pam_soos.so` is placed **immediately after account lockout preauth (`pam_faillock`) and before `pam_unix.so`**:

```pam
# 1. Account Lockout Pre-Authentication (enforces maximum failed attempts)
auth  required                       pam_faillock.so preauth

# 2. soos Local Biometric Facial Verification (deadline derived from the clamped timeout_ms)
auth  [success=done default=ignore]  pam_soos.so

# 3. Standard Password Authentication (executed if facial verification returns PAM_IGNORE)
auth  [success=done default=bad]     pam_unix.so try_first_pass nullok

# 4. Anti-Intrusion Telemetry Event (only reached on password authentication failure)
auth  optional                       pam_soos.so event=password-failed timeout_ms=20
```

---

## 3. Display Manager and Screen Locker Test Matrix

### 3.1 Swaylock (`swaylock`) — Wayland / wlroots

`swaylock` is the reference Wayland screen locker for wlroots-based compositors (Sway, River, Wayfire).

#### PAM Configuration: `/etc/pam.d/swaylock`
Keep the distribution file. It reaches the soos line through the base stack (Arch:
`swaylock` → `login` → `system-local-login` → `system-login` → `system-auth`; see
`Docs/DISTRIBUTION_DEPLOYMENT.md` §5.3). Do not add a second `pam_soos.so` line: the
`event=password-failed` line already lives in the base stack.
```pam
auth include login
```

#### Behaviour to Keep in Mind
`swaylock` calls `pam_authenticate` only when a password is submitted. The PAM path does no
background verification: locking, waiting or moving the mouse never makes `pam_soos.so` start
the camera, and nothing is shown on screen. With `ignore-empty-password` (`-e`), an empty
submission never reaches PAM. The daemon's presence auto-unlock (§3.6) scans in the background
only when logind reports the session locked, which a plain `swaylock` never does; the cases
below run without the §3.6 wrapper.

#### Test Procedure:
1. Ensure the user is enrolled: `soos-enroll list`.
2. Lock the active session **without** `-e`, and with no swaylock config file applied
   (`ignore-empty-password` may be set in `~/.swaylock/config`,
   `$XDG_CONFIG_HOME/swaylock/config` or `/etc/swaylock/config`):
   `swaylock -C /dev/null -c 000000`.
3. **Test Case 1 (Nominal Face Unlock)**:
   - Position face directly before the webcam.
   - Press `Enter` on the empty field.
   - **Expected Result**: Screen unlocks within the `timeout_ms` budget (default 1000 ms;
     about 0.3–0.5 s measured with the camera woken from auto-standby) without typing a
     password. The journal shows `verdict=Allow reason=FaceMatch`.
4. **Test Case 2 (Occluded / Unrecognized Face)**:
   - Lock screen again without `-e`: `swaylock -C /dev/null -c 000000`.
   - Cover the webcam lens or look away.
   - Press `Enter` on the empty field.
   - **Expected Result**: Screen remains locked and `swaylock` shows its failure state. The
     empty password failed `pam_unix.so`, so the base-stack event line sends one
     `PasswordFailed` event (journal: `Processing telemetry auth failure event`) and `faillock --user <user>` shows one more failure (stacks with
     `pam_faillock`). Entering the correct account password then unlocks the
     session. Do not repeat this `deny` times (Arch default 3): the account would be locked for
     `unlock_time` (default 600 s), and neither face nor password unlocks until it expires.
5. **Test Case 3 (Daemon Offline)**:
   - Stop daemon: `sudo systemctl stop soos-daemon`.
   - Lock screen: `swaylock -C /dev/null -c 000000`.
   - Type the account password and press `Enter`.
   - **Expected Result**: `pam_soos.so` returns `PAM_IGNORE` immediately and the password
     unlocks the session with zero UI freeze.
   - Restart daemon: `sudo systemctl start soos-daemon` (Test Case 4 needs it running).
6. **Test Case 4 (`ignore-empty-password`)**:
   - Check the daemon is running: `systemctl is-active soos-daemon` prints `active`.
   - Lock screen: `swaylock -C /dev/null -e -c 000000`.
   - Face the webcam and press `Enter` on the empty field.
   - **Expected Result**: The empty `Enter` does nothing and the `soos-daemon` journal shows
     no new `Rendered authentication response` line for it. Typing the password and pressing `Enter`
     then runs face verification first and unlocks the session (by face or by password).

---

### 3.2 Hyprlock (`hyprlock`) — Hyprland Wayland Compositor

`hyprlock` is the native multi-threaded GPU-accelerated screen locker for Hyprland. It is
**not validated on hardware**: it drives its PAM conversation differently from `swaylock`, so
this procedure first records when face verification runs.

#### PAM Configuration: `/etc/pam.d/hyprlock`
Keep the file shipped with `hyprlock`. Like `swaylock`, it reaches the soos line through the
distribution base stack (see `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.3). Do not add a
`pam_soos.so` line of its own: on a base stack that already carries one (Arch `system-auth`),
a face that does not match would be tried a second time.

#### Test Procedure:
1. Ensure the daemon is running: `systemctl is-active soos-daemon` prints `active`.
2. **Test Case 1 (When Face Verification Runs)**:
   - Launch `hyprlock`, face the camera and do not touch the keyboard for 5 s.
   - Press `Enter` on the empty field.
   - After unlocking (face or password), read the daemon journal:
     `journalctl -u soos-daemon --since "-5min" | grep "Rendered authentication response"`.
   - **Expected Result**: The session unlocks without a password only after a
     `verdict=Allow reason=FaceMatch` line. Record in the validation report whether that line
     was written when the lock screen appeared or when `Enter` was pressed.
3. **Test Case 2 (Password Fallback)**:
   - Cover the webcam, launch `hyprlock`, type the account password and press `Enter`.
   - **Expected Result**: The password unlocks the session within the `timeout_ms` budget.
4. **Test Case 3 (Multi-Monitor Integrity)**:
   - With multiple active displays, invoke `hyprlock`.
   - Authenticate via facial verification.
   - **Expected Result**: All displays unlock simultaneously without visual artifacts or hung surface textures.

---

### 3.3 GNOME Display Manager (`gdm` / `gdm-password`)

GDM manages graphical session logins and greeter screen unlocks for GNOME desktops.

#### PAM Configuration: `/etc/pam.d/gdm-password`
Do not edit the file by hand. `sudo soos-admin gdm enable` inserts the managed block (lockout
and login gates copied from the delegated stack, then `pam_soos.so timeout_ms=2500`) before the
first credential rule or delegation; see `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 for the exact
block, `gdm status`, `gdm disable` and `gdm restore`. Check the result with
`sudo soos-admin gdm status`.

When the delegated stack already carries the primary soos rule (Arch `system-auth` through
`gdm-password` → `system-local-login` → `system-login`, Debian `common-auth` with the soos
profile, Fedora `custom/soos`), `gdm enable` adds no managed block (and removes one left by an
earlier release); `sudo soos-admin gdm status` then shows `Shared soos Rule:  <stack>`. GDM
authenticates through that shared rule, with its deadline (module default 1000 ms) instead of
`timeout_ms=2500`: expect exactly one `Rendered authentication response` line per GDM attempt
(GitHub #331).

#### Test Procedure:
1. **Test Case 1 (Initial Greeter Login Never Uses Face)**:
   - Reboot, or log out to the GDM greeter, and select the enrolled user.
   - **Expected Result**: GDM asks for the password; the camera does not light. The daemon
     journal shows `Local-session policy refused auth request` with
     `reason="caller_session_unresolved"`: the user has no session yet, so the ADR 2026-09-30
     "Local Session Binding for Facial `Auth`" refuses face verification by design.
2. **Test Case 2 (Screen Unlock)**:
   - From an open GNOME session, lock the screen (`Super+L`), then raise the shield (any key).
   - **Expected Result**: The session unlocks without a password after a
     `verdict=Allow reason=FaceMatch` line. This relies on the GDM reauthentication worker
     running in the user's session scope (`cat /proc/<pid>/cgroup` shows
     `session-<id>.scope`); record the result, since the ADR leaves it to hardware validation.
   - **Finding the worker**: it exists only while the unlock prompt is up. In a terminal (or
     from a TTY / SSH session) start
     `sleep 15; pgrep -af 'gdm-session-worker \[pam/gdm-password\]'`, lock with `Super+L`,
     raise the shield within 15 s, then run `cat /proc/<pid>/cgroup` with the pid of the first
     column.
3. **Test Case 3 (Output Isolation Audit)**:
   - Check journal logs: `journalctl -u gdm -b`.
   - **Expected Result**: Zero stream pollution (`stdout` or `stderr` messages from `pam_soos.so`) that could corrupt GDM's JSON/DBus communication channel.

---

### 3.4 Linux Virtual Console (`login` TTY)

Text-mode virtual terminal authentication (`/dev/tty1` through `/dev/tty6`).

#### PAM Configuration: `/etc/pam.d/login`
Keep the distribution file. It reaches the soos line through the base stack (Arch:
`login` → `system-local-login` → `system-login` → `system-auth`). Do not add a
`pam_soos.so` line of its own.

#### Test Procedure:
1. Switch to a virtual console (`Ctrl+Alt+F3`).
2. At the login prompt, enter the username of the enrolled user while facing the webcam.
3. **Test Case 1 (Initial TTY Login Never Uses Face)**:
   - **Expected Result**: `Password:` is prompted; the camera does not light. The daemon
     journal shows `Local-session policy refused auth request` with
     `reason="caller_session_unresolved"` (the user has no session on this TTY yet; ADR
     2026-09-30 "Local Session Binding for Facial `Auth`").
4. **Test Case 2 (Password Login)**:
   - Enter the account password.
   - **Expected Result**: The shell opens normally.

---

### 3.5 Command-Line Elevation (`sudo`)

Administrative privilege elevation from terminal sessions.

#### PAM Configuration: `/etc/pam.d/sudo`
Keep the distribution file (Arch: `auth include system-auth`). Do not add a `pam_soos.so`
line of its own.

#### Test Procedure:
1. In a terminal of the local desktop session (not SSH), run `sudo -k`, then `sudo whoami`.
2. **Test Case 1 (Nominal Sudo Elevation)**:
   - Face the camera.
   - **Expected Result**: Command executes immediately and outputs `root` with zero password prompt.
3. **Test Case 2 (Terminal Password Fallback)**:
   - Cover camera, run `sudo -k`, then `sudo whoami`.
   - **Expected Result**: Prompt `[sudo] password for <user>:` appears once face verification
     gives up, within the `timeout_ms` budget (default 1000 ms). Entering valid password
     executes the command.

---

### 3.6 Presence Auto-Unlock of Locked Sessions (daemon path, no PAM) — matrix PAU21

`soos-daemon` unlocks a locked local session **without any keypress** when it verifies the face
of the session owner (ADR 2026-10-02 "Presence Auto-Unlock Through logind",
`Docs/DAEMON.md` §6, `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.5). This path does not go through
PAM: the daemon polls systemd-logind every second, scans only a session whose `LockedHint` is
`true` once its lock grace (`[presence] lock_grace_ms`, default 3000 ms) has passed, runs the
normal pipeline (PAD consensus, then match) and calls `UnlockSession`. It is enabled by default.
Everything in this section is **not yet validated on hardware**: record the observed behaviour
and timings in the validation report.

#### Preconditions
1. The user is enrolled (`soos-enroll list`) and the daemon runs:
   `systemctl is-active soos-daemon` prints `active`, and
   `journalctl -u soos-daemon -b | grep "Presence auto-unlock of locked local sessions started"`
   shows one line. A `Presence auto-unlock not started` line means `[presence] enabled = false`
   (or the test harness mode); fix `/etc/soos/daemon.toml` before continuing.
2. No kill switch is set: `ls /etc/soos/disabled /etc/soos/presence.disable` reports both missing.
3. The account is one the account guard can evaluate (otherwise presence never scans, by
   design):
   - `sudo grep -c '^<user>:' /etc/shadow` prints `1` (LDAP/SSSD and systemd-homed users have
     no line and never get presence unlock);
   - `grep -rn 'pam_faillock' /etc/pam.d /usr/lib/pam.d /usr/etc/pam.d 2>/dev/null` shows no
     line carrying `deny=`, `dir=`, `fail_interval=`, `unlock_time=`, `root_unlock_time=`,
     `admin_group=`, `conf=` or `even_deny_root` (such options belong in
     `/etc/security/faillock.conf`; `preauth`, `authfail`, `authsucc`, `silent` and `audit` are
     fine);
   - `sudo faillock --user <user>` lists no valid failure of the last `fail_interval`.
4. Keep a root shell that does not depend on the lock screen (another TTY with `Ctrl+Alt+F3`,
   or SSH from a second machine): several cases below lock the account on purpose.
5. Skip reasons are logged at `debug` only. To read them, set
   `log_level = "info,soos_daemon::presence=debug"` in `/etc/soos/daemon.toml` and restart the
   daemon (`sudo systemctl restart soos-daemon`); put `log_level = "info"` back at the end.
   Follow the journal during the whole section: `journalctl -u soos-daemon -f`.

Journal lines this section refers to (all in `crates/daemon/src`):

| Line | Level | Meaning |
|---|---|---|
| `Presence verified the session owner; locked session unlocked through logind` | `info` | `UnlockSession` succeeded; carries `session_id`, `uid` and `captures_evaluated` |
| `Presence scan vetoed by presentation attack detection; the session stays locked` | `warn` | A capture was classified as a spoof (once per lock period); preceded by `Presentation attack detected; vetoing request` |
| `Presence unlock refused by the account guard; the session stays locked` | `info` | The account check made after an `Allow` refused (`refusal=faillocked`, `account_expired`, `password_locked`, ...) |
| `Presence scan gated by the lid or the screen state` / `Presence scan gate open` | `info` | Lid or screen gate transition, with `lid_closed` and `display_state` |
| `The screen locker ignored UnlockSession; this lock period is no longer scanned` | `warn` | The session was still locked 5 s after a successful `UnlockSession` |
| `UnlockSession failed; the session stays locked` | `warn` | logind refused or timed out (never retried for the same `Allow`) |
| `systemd-logind unavailable; presence auto-unlock backs off (sessions stay locked)` | `warn` | No system bus or logind (once per outage) |
| `Presence tick skipped` (`reason=kill_switch`, `in_grace`, `account_refused`, `not_enrolled`, `lid_closed`, `display_off`, `no_locked_session`, ...) | `debug` | Why a tick did not scan (logged when the reason changes) |
| `Presence scan finished` (`outcome=NoMatch`, `SpoofVetoed`, `Unlocked { .. }`, ...) | `debug` | Result of a scan that recorded an attempt |

No line may contain a user name above `debug`, a score, an embedding, a frame or file content.

#### Test Procedure — GNOME / GDM (primary)
1. **Test Case 1 (Lock, Leave, Return)**:
   - Lock with `Super+L`, leave the camera field of view within 2 s and stay away 20 s.
   - Return and face the camera (move the mouse first if the screen has blanked).
   - **Expected Result**: The session unlocks without any keypress within about 4 s of facing the
     camera, and the journal shows one
     `Presence verified the session owner; locked session unlocked through logind` line with the
     session ID and UID. Record the delay.
2. **Test Case 2 (Lock While Seated)**:
   - Lock with `Super+L` and keep facing the camera.
   - **Expected Result**: The session unlocks again shortly after the 3 s grace. This is the
     expected behaviour (owner decision), not a defect. With
     `[presence] lock_grace_ms = 10000` and a restart, the unlock comes after about 10 s instead.
3. **Test Case 3 (Grace Period)**:
   - Lock with `Super+L` while facing the camera and watch the camera LED.
   - **Expected Result**: No scan (LED stays off unless the PAM path wakes it, see the note
     below; debug `reason="in_grace"`) during the first `lock_grace_ms`; the scan and the unlock
     follow afterwards.
4. **Test Case 4 (Unknown Face)**:
   - Lock, leave, and let a person who is not enrolled face the camera for 30 s.
   - **Expected Result**: The session stays locked; debug lines show `outcome=NoMatch` about
     every `scan_interval_ms` (2 s); no unlock line. The owner returning then unlocks it.
5. **Test Case 5 (Presentation Attack)**:
   - Lock, leave, and present a printed photo, then a phone or tablet replay of the owner.
   - **Expected Result**: The session never unlocks. A spoof capture produces one
     `Presence scan vetoed by presentation attack detection; the session stays locked` warning
     per lock period (no evidence snapshot is written for presence vetoes). Record any unlock as
     a security defect.
6. **Test Case 6 (Lid Closed / Screen Off)**:
   - On a laptop with an external monitor and `HandleLidSwitch=ignore` (or `HandleLidSwitchDocked=ignore`
     while docked), lock and close the lid. Separately, lock, leave, and let GNOME blank the
     screen (Settings → Power → Screen Blank set to a short delay).
   - **Expected Result**: `Presence scan gated by the lid or the screen state` with
     `lid_closed="true"` (or `display_state="off"`), no scan, and the camera LED goes off after
     `[camera] idle_timeout_secs` (default 10 s). Opening the lid or waking the screen logs
     `Presence scan gate open` and the scans resume. A state the daemon cannot read
     (`display_state="unknown"`) does not gate.
   - **Record**: the `display_state` value logged while GNOME blanks the screen, and
     `cat /sys/class/drm/card*-*/dpms` at that moment. On atomic-KMS drivers the sysfs `dpms`
     attribute may stay `On` while the compositor blanks (screen-off detection is best effort);
     if so, note it as a known limitation, not a failure, and check that the lid case still
     gates.
   - Also close the lid **during** a scan (after the camera LED turns on): the scan must end
     with no unlock (`ScanOutcome::LidClosed`, the lid re-check after the `Allow`).
7. **Test Case 7 (Sandboxed `UnlockSession`)**:
   - Confirm Test Case 1 ran with the packaged unit (`systemctl cat soos-daemon` shows
     `PrivateNetwork=yes` and `ProtectSystem=strict`).
   - **Expected Result**: The unlock line is present and no `UnlockSession failed` warning
     appears: logind accepts the call from the sandboxed root service.

Note: with `soos-admin gdm enable`, raising the GNOME shield also starts the PAM face path
(§3.3 Test Case 2). Cover the camera, or do not touch the keyboard and mouse, when a case must
observe presence alone.

#### Test Procedure — KDE Plasma
1. Lock with `Meta+L` (or `loginctl lock-session`); check the hint from a terminal of another
   session or over SSH:
   `busctl --system get-property org.freedesktop.login1 /org/freedesktop/login1/session/<id> org.freedesktop.login1.Session LockedHint`
   prints `b true` (`loginctl list-sessions` gives `<id>`).
2. Repeat GNOME Test Cases 1, 4 and 5.
3. **Expected Result**: Same as GNOME: `kscreenlocker` sets `LockedHint` and honours
   `UnlockSession`. A `The screen locker ignored UnlockSession; this lock period is no longer scanned`
   warning means the locker did not react; record it.

#### Test Procedure — `swaylock` / `hyprlock` (wlroots, `swayidle` hooks)
Plain `swaylock` and `hyprlock` never set `LockedHint` and never listen to logind, so presence
never scans for them (expected: no `Presence scan finished` line at all). Install the wrapper
and the `swayidle` hooks of `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.5 (for `hyprlock`, use
`hyprlock` in the wrapper and `pkill -USR1 hyprlock` as the `unlock` hook), then:
1. Lock with `loginctl lock-session`. Check `LockedHint` as in the KDE procedure (`b true`).
2. **Test Case 1 (Unlock Hook)**: leave, return and face the camera.
   - **Expected Result**: The journal shows the unlock line, `swayidle` runs its `unlock` hook,
     the locker exits, and the wrapper sets `LockedHint` back to `b false`.
3. **Test Case 2 (Missing Hook)**: run `swaylock -C /dev/null -c 000000` directly, without the
   wrapper.
   - **Expected Result**: No scan and no unlock; the camera stays off until a password (or an
     empty `Enter`, §3.1) is submitted.
4. **Test Case 3 (Hint Without Hook)**: start the wrapper but `swayidle` without the `unlock`
   hook, then face the camera.
   - **Expected Result**: `UnlockSession` succeeds but the locker stays up; 5 s later the journal
     shows `The screen locker ignored UnlockSession; this lock period is no longer scanned` and
     no further scan happens until the next lock.

#### Test Procedure — Account Guard (`pam_faillock`, expiry, locked password)
Run on the GNOME (or KDE) session; restore every change from the root shell of the
preconditions.
1. **Test Case 1 (`pam_faillock` Locked)**:
   - Lock the screen, cover the camera, and enter a wrong password `deny` times (default
     `deny = 3`, `/etc/security/faillock.conf`). From the root shell,
     `faillock --user <user>` lists the failures.
   - Uncover the camera and face it for 60 s.
   - **Expected Result**: No presence unlock and no camera wake by presence while the account is
     locked (debug `reason="account_refused"`; if the lock happened during a scan,
     `Presence unlock refused by the account guard; the session stays locked` with
     `refusal="faillocked"`). After `unlock_time` (default 600 s) has passed, presence unlocks
     again at the next scan; alternatively `sudo faillock --user <user> --reset` from the root
     shell makes it unlock within a few seconds (the guard re-reads the tally on every check).
   - Check that the presence unlock did not reset the tally: after a test with fewer than `deny`
     failures, `faillock --user <user>` still lists them (no tally reset by presence).
2. **Test Case 2 (Expired Account)**:
   - From the root shell: `sudo chage -E 0 <user>`, then lock the screen and face the camera.
   - **Expected Result**: No presence unlock (debug `reason="account_refused"`). Restore with
     `sudo chage -E -1 <user>`; presence unlocks at the next scan.
3. **Test Case 3 (Locked Password)**:
   - From the root shell: `sudo passwd -l <user>`, then lock the screen and face the camera.
   - **Expected Result**: No presence unlock. Restore with `sudo passwd -u <user>`; presence
     unlocks at the next scan.
4. **Test Case 4 (Policy Option on a PAM Line)**: add `deny=5` to the `pam_faillock.so preauth`
   line of a copy-safe test stack (for example a new file `/etc/pam.d/soos-presence-test`
   containing `auth required pam_faillock.so preauth deny=5`), then lock and face the camera.
   - **Expected Result**: No presence unlock for any user (state undeterminable). Delete the file;
     presence unlocks at the next scan.

Test Cases 2 and 3 also block the password at the lock screen (expired account, locked
password): restore them from the root shell before unlocking, and never leave any of these
changes in place.

#### Test Procedure — Kill Switches
1. **Test Case 1 (`presence.disable`)**:
   - Lock, leave, then `sudo touch /etc/soos/presence.disable` from the root shell; return and
     face the camera for 30 s.
   - **Expected Result**: No scan and no unlock within one second of the flag (debug
     `reason="kill_switch"`), no restart needed. Face PAM still works (raising the GNOME shield,
     `sudo`). `sudo rm /etc/soos/presence.disable`: the grace restarts and the session unlocks
     after about 3 s more.
2. **Test Case 2 (`disabled`)**:
   - Repeat Test Case 1 with `/etc/soos/disabled`.
   - **Expected Result**: No presence unlock, and `pam_soos.so` returns `PAM_IGNORE` too (the
     password is the only way in). Removing the flag restores both.
3. **Test Case 3 (`gdm.disable` Does Not Stop Presence)**:
   - `sudo touch /etc/soos/gdm.disable`, lock, leave, return.
   - **Expected Result**: Presence still unlocks the session (owner decision); only the GDM PAM
     face path is disabled. `sudo rm /etc/soos/gdm.disable` afterwards.
4. **Test Case 4 (`enabled = false`)**:
   - Set `[presence] enabled = false` in `/etc/soos/daemon.toml` and restart the daemon.
   - **Expected Result**: `Presence auto-unlock not started` at startup; locking never starts the
     camera. Restore `enabled = true` (or remove the key) and restart.

---

## 4. Operational Invariant and Behavior Matrix

| Scenario | Daemon State | Camera State | Face Alignment / Match | PAM Result | UI Response |
|---|---|---|---|---|---|
| **Nominal Unlock** | Active | Streaming MMAP | Single face, Score >= 0.50, PAD Pass | `PAM_SUCCESS` | Unlocks after the PAM call (Enter on `swaylock`) within the `timeout_ms` budget |
| **Unknown Person** | Active | Streaming MMAP | Single face, Score < 0.50 | `PAM_IGNORE` | Falls back to the password |
| **Presentation Attack** | Active | Streaming MMAP | Photo / Phone Screen / Video | `PAM_IGNORE` | Falls back to the password (rejection logged) |
| **Multiple Faces** | Active | Streaming MMAP | >= 2 faces detected in frame | `PAM_IGNORE` | Falls back to the password |
| **Lens Covered** | Active | Streaming MMAP | Zero faces detected | `PAM_IGNORE` | Falls back to the password |
| **Camera Hardware Unplugged** | Active | `ENODEV` hotplug | N/A (CameraManager backoff) | `PAM_IGNORE` | Falls back to the password within the `timeout_ms` budget |
| **Daemon Crashed / Stopped** | Inactive | N/A | N/A (Socket connection refused) | `PAM_IGNORE` | Falls back to the password at once (connection refused, no wait) |
| **Daemon Deadline Exceeded** | Busy | Stalled | Evaluation exceeds the client deadline (`timeout_ms`) | `PAM_IGNORE` | Deadline expiry -> falls back to the password |
| **Presence: Owner Returns** (§3.6) | Active | Woken by the presence scan | Single face, Score >= 0.50, PAD Pass (3 consecutive captures) | No PAM call | `UnlockSession` after the lock grace, no keypress |
| **Presence: Unknown Face, Spoof, Lid Closed, Screen Off, Account Locked or Expired, Kill Switch** (§3.6) | Active | Standby (or scanning without match) | Refused or not attempted | No PAM call | Session stays locked; the password path is unchanged |

---

## 5. Security & Isolation Guidelines

1. **Session Binding**:
   - `soos-daemon` verifies that the requesting UID owns an active local graphical seat via `/run/systemd/sessions/`. Unfocused or remote sessions (SSH) must not trigger webcam activation.
2. **Camera Snooping Prevention**:
   - The daemon maintains exclusive ownership of the capture device. When the screen is locked, third-party user processes cannot open `/dev/video*` or snoop on capture frames.
3. **Output Isolation**:
   - All PAM module output is suppressed. Caught panics log exclusively to syslog via `LOG_AUTHPRIV` without leaking credentials or tokens.

---

## 6. Emergency Recovery and Rollback Procedure

If a misconfigured PAM stack prevents login:

1. **Rescue Shell**:
   - Switch to TTY (`Ctrl+Alt+F2`) or reboot into systemd rescue target: `systemd.unit=rescue.target`.
2. **Disable Module in PAM Stack**:
   - Comment out `pam_soos.so` entries in the base stacks that carry them, following symlinks
     (authselect ships `system-auth` and `password-auth` as symlinks; a plain `sed -i` would
     replace them by regular files) and skipping missing files:
     ```bash
     for f in /etc/pam.d/common-auth /etc/pam.d/system-auth /etc/pam.d/password-auth; do
         [ -f "$f" ] && sudo sed -i --follow-symlinks 's/^auth.*pam_soos\.so/# &/' "$f"
     done
     ```
     `swaylock`, `hyprlock`, `login` and `sudo` keep their distribution files and reach soos
     only through these base stacks (§3.1–§3.5), so they need no edit.
   - GDM: `sudo soos-admin gdm disable` stops face verification at once (`/etc/soos/gdm.disable`).
     When GDM uses a managed block, `sudo soos-admin gdm restore` puts back
     `gdm-password.soos-backup`. When it uses the shared base-stack rule (`gdm status` shows
     `Shared soos Rule`), there may be no backup (`gdm restore` then reports that it does not
     exist): use `gdm disable` or the base-stack loop above.
   - Presence auto-unlock: `sudo touch /etc/soos/presence.disable` stops it within one second
     (`gdm.disable` does not); `/etc/soos/disabled` stops it together with every face PAM path.
3. **Service Rollback**:
   - Stop and disable the daemon:
     ```bash
     sudo systemctl stop soos-daemon
     sudo systemctl disable soos-daemon
     ```
4. **Standard Restoration**:
   - On Debian/Ubuntu: `sudo pam-auth-update --package --force`
   - On Fedora: `sudo authselect select sssd with-faillock --force`
