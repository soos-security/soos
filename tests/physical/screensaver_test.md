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
`swaylock` calls `pam_authenticate` only when a password is submitted. There is no background
verification: locking, waiting or moving the mouse never starts the camera, and nothing is
shown on screen. With `ignore-empty-password` (`-e`), an empty submission never reaches PAM.

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

When the delegated stack also carries a soos line (Arch: `gdm-password` →
`system-local-login` → `system-login` → `system-auth`), a face that does not match at the
managed block is tried again by the base-stack line. Record whether the journal shows one or
two `Rendered authentication response` lines for one GDM attempt.

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
     running in the user's session scope (`cat /proc/<worker pid>/cgroup` shows
     `session-<id>.scope`); record the result, since the ADR leaves it to hardware validation.
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
   - Comment out `pam_soos.so` entries in `/etc/pam.d/`:
     ```bash
     sudo sed -i 's/^auth.*pam_soos\.so/# &/' /etc/pam.d/common-auth /etc/pam.d/system-auth /etc/pam.d/swaylock /etc/pam.d/hyprlock /etc/pam.d/sudo
     ```
   - GDM: `sudo soos-admin gdm disable` stops face verification at once (`/etc/soos/gdm.disable`);
     `sudo soos-admin gdm restore` puts back `gdm-password.soos-backup`.
3. **Service Rollback**:
   - Stop and disable the daemon:
     ```bash
     sudo systemctl stop soos-daemon
     sudo systemctl disable soos-daemon
     ```
4. **Standard Restoration**:
   - On Debian/Ubuntu: `sudo pam-auth-update --package --force`
   - On Fedora: `sudo authselect select sssd with-faillock --force`
