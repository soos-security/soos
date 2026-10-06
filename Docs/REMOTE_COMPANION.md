# Remote Companion (`soos-remote`)

> Crate: `crates/remote` (`soos-remote`, GitHub #339, ADR 2026-10-05 "Remote Companion `soos-remote`",
> ADR 2026-10-06 "Remote Unlock in `soos-remote`")
> Scope: a **user-level** service that shows the owner's phone the real-time lock status of the
> desktop session, offers a remote **lock** and, only when `allow_unlock = true`, a remote
> **unlock** (section 2a). Nothing else: no camera, no push notification (see "Out of scope").
> Source of truth: `crates/remote/src/lib.rs` (constants), `crates/remote/src/config.rs`
> (configuration keys), `crates/remote/src/server.rs` (request handling). If this document and
> the code disagree, report the drift: the invariant `remote_companion_contract` pins the parts
> of this page the acceptance criteria rely on (matrix rows RMC1–RMC25, walkthroughs 183 and 184).

---

## 1. What it is, and what it is not

`soos-remote` is a small HTTP/1.1 server that:

- listens on **one Unix socket** (`0600`, inside a `0700` directory under `$XDG_RUNTIME_DIR`)
  and never opens a network socket (`RestrictAddressFamilies=AF_UNIX` in the unit);
- is reached from the phone **only through the owner's Tailscale tailnet**: `tailscale serve`
  terminates HTTPS with the node's `*.ts.net` certificate and proxies to the socket;
- reads the session state from systemd-logind (`LockedHint`, `IdleHint`, `IdleSinceHint`,
  `Active`) over the pinned system bus, exactly like the daemon's presence worker;
- can ask logind to **lock** the owner's local session (`Manager.LockSession`);
- when `allow_unlock = true`, can ask logind to **unlock** it (`Manager.UnlockSession`,
  section 2a).

It runs as the session owner, never as root. `soos-daemon`, `pam_soos.so` and the IPC protocol
are untouched; no other crate depends on `soos-remote`.

## 2. Trust model (read this before exposing anything)

Every request must carry exactly one `Tailscale-User-Login` header whose value is listed in
`allowed_logins`; everything else is `403`. That header is trusted for two reasons, and only
while both hold:

1. **Tailscale Serve sets it and strips any client-supplied copy.** The header is absent for
   Funnel traffic and for tagged (non-user) devices, which are therefore refused.
2. **The backend is a `0600` Unix socket in a `0700` directory owned by the session owner.**
   Only `tailscaled` (root) and the owner can connect to it, so a local process cannot inject a
   forged identity, and the owner forging their own login is no escalation.

Consequences:

- **Never** put any other reverse proxy in front of the socket, and never run
  `tailscale serve --http` (plain HTTP) or `tailscale funnel` (public internet) for it: each one
  voids the model (forgeable headers, no tailnet identity, or exposure beyond the owner's
  devices).
- The **effective host** (`X-Forwarded-Host` as set by `tailscale serve`, or `Host` for a
  direct local client) must be an allowed `*.ts.net` name (or a name from `allowed_hosts`);
  anything else is `421 Misdirected Request`, which defeats DNS rebinding even if the socket
  were ever reached over plain HTTP. When `X-Forwarded-Host` is present the `Host` header
  (the proxy's backend name, `localhost` today) is not inspected at all.
- A proxied request (one carrying `X-Forwarded-Host`) must also carry exactly one
  `X-Forwarded-Proto: https`; anything else is `421`, so the `--http` ban above is enforced,
  not only documented. `tailscale serve` was observed (Tailscale 1.102.4, 2026-10-06) to
  overwrite a client-supplied `X-Forwarded-Host`, `X-Forwarded-Proto` and
  `Tailscale-User-Login` with the real values; these three are the only headers the checks
  rely on (`X-Forwarded-For` is sent too but ignored by the service).
- `POST /api/lock` additionally needs `X-Soos-Action: lock` (a non-simple header a cross-origin
  page cannot send without a CORS preflight, which is never answered), `Sec-Fetch-Site` absent or
  `same-origin`, and `Origin` absent or equal to `https://<effective host>`.
- `POST /api/unlock` needs the same three CSRF conditions with `X-Soos-Action: unlock`; the lock
  route never accepts the unlock value and the unlock route never accepts `lock`.
- Nothing about a request is logged: no identity, no `Host`, no path, no header value, no
  session id. Denials log their reason class only. An accepted unlock adds one `info` line,
  `remote unlock requested`, without any of these.

## 2a. Remote unlock (opt-in)

`POST /api/unlock` asks systemd-logind to unlock the owner's local session (the same session
the lock picks: `Class=user`, `Remote=false`, on a seat, the active one first). logind emits its
`Unlock` signal; GNOME and Plasma close their lock screen, and on a sway-family desktop the
`swayidle` `unlock` hook of section 3 stops the locker, exactly as for the daemon's presence
auto-unlock. The service gains no right the owner's account does not already have: logind lets
a user unlock their own session.

It is **disabled by default**. Enable it in `remote.toml`, then restart the unit:

```toml
allow_unlock = true
```

```sh
systemctl --user restart soos-remote
```

While it is disabled, `POST /api/unlock` answers `403 {"result":"unlock_disabled"}` without
reading logind. The page shows an *Unlock now* button (enabled only while the session is
`Locked`) that asks for a confirmation tap before sending the request.

**Accepted risk** (owner decision, ADR 2026-10-06): the only authentication is the Tailscale
identity. Anyone who holds the owner's unlocked phone, or any other device logged in to an
allowed Tailscale login, while it is connected to the tailnet, can unlock the PC, and the PC
stays unlocked until it locks again by itself (no automatic re-lock). There is no passkey, PIN
or Face ID step. To revoke the capability at once: set `allow_unlock = false` (or remove the
login from `allowed_logins`) and restart the unit, or remove the device from the tailnet in the
Tailscale admin console.

## 3. Requirements on the desktop

- **`LockedHint` must be set by the desktop.** GNOME, Plasma and niri set it natively;
  swaylock, swaylock-plugin, hyprlock and swayidle never do, and `loginctl lock-session` does
  not either (it only emits the `Lock` signal). On a sway-family desktop a small wrapper must
  call `org.freedesktop.login1.Session.SetLockedHint` with `true` before starting the locker and
  with `false` after it exits, for example through `busctl --system call org.freedesktop.login1
  /org/freedesktop/login1/session/auto org.freedesktop.login1.Session SetLockedHint b true`; the
  complete `swaylock-presence` wrapper is given in `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.5 (it
  is the same hint the presence auto-unlock relies on). Without it the page can only ever show
  `Unlocked`. The status is exactly as truthful as `LockedHint`. Check the hint with
  `busctl --system get-property org.freedesktop.login1 /org/freedesktop/login1/session/auto org.freedesktop.login1.Session LockedHint`.
- **The desktop must honour the logind `Lock` signal.** `Manager.LockSession` only emits the
  `Lock` signal on the session; GNOME and Plasma react natively. A sway-family desktop needs a
  `swayidle` `lock` hook that starts the wrapper above, started once from the compositor
  configuration (the `unlock` hook belongs to the presence auto-unlock and is optional here):

  ```sh
  exec swayidle -w lock 'swaylock-presence &' unlock 'pkill -USR1 swaylock'
  ```

  A desktop that ignores the signal yields `202 lock_requested` and no `locked`: the page then
  reports "The desktop did not confirm the lock (LockedHint unchanged)" after 5 s.
- `soos-remote` only looks at the owner's own sessions with `Class == "user"`, `Remote == false`
  and a seat; SSH and other remote sessions are never reported or locked.

## 4. Installation (per user, no root)

```bash
scripts/install_remote.sh
```

The script refuses to run as root and never calls `sudo`. It builds `soos-remote` (release,
`--locked`), installs the binary to `~/.local/bin/soos-remote`, the user unit
`packaging/soos-remote.service` to `${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/soos-remote.service`,
writes a commented configuration template to `${XDG_CONFIG_HOME:-$HOME/.config}/soos/remote.toml`
(mode `0600`, **only when absent**, with `allowed_logins = []` so the service refuses to start
until you fill it in), and runs `systemctl --user daemon-reload`. It never enables the unit and
never runs `tailscale`; those are your steps:

1. Put your Tailscale login in `remote.toml`:

   ```toml
   allowed_logins = ["you@example.com"]
   ```

2. Start the service: `systemctl --user enable --now soos-remote`
   (`journalctl --user -u soos-remote` shows the socket path on start-up).
   *Optional:* `loginctl enable-linger "$USER"` keeps your user manager, and with it
   `soos-remote` and `$XDG_RUNTIME_DIR`, alive while no session of yours is open, so the page
   still answers (`No session`) from the login screen after a reboot. Without linger the socket
   disappears with your last session and comes back at the next login; nothing else changes.
3. Publish the socket on your tailnet:
   `tailscale serve --bg unix:$XDG_RUNTIME_DIR/soos-remote/remote.sock`
   (`--bg` persists the configuration across reboots; `tailscale serve status` shows it).
   *Suggested tailnet policy (defence in depth):* the identity check already refuses every
   login outside `allowed_logins`, but an ACL rule in the tailnet policy file stops anyone
   else from reaching the HTTPS port of your PC at all, for example

   ```jsonc
   {"action": "accept", "src": ["you@example.com"], "dst": ["you@example.com:443"]}
   ```

   (your own identity may reach port 443 of your own devices; adapt the `dst` to a host alias
   or tag if your policy uses them). Keep the default `tailscale serve` HTTPS listener on 443;
   never use `tailscale serve --http` or `tailscale funnel` for this socket.
4. Open `https://<this-pc>.<tailnet>.ts.net` in Safari on the iPhone (the phone must be logged
   in to the same tailnet) and add it to the home screen: Share → **Add to Home Screen** →
   Add. The page then opens as a standalone web app with its own icon; it reconnects every
   time it is brought back to the foreground. No App Store, no account, no password.

`scripts/install_remote.sh --uninstall` stops and disables the unit and removes the binary and
the unit; the configuration is kept. To undo step 3 run `tailscale serve reset` (or remove only
this handler with `tailscale serve --https=443 off`).

### What `tailscale serve` forwards to the socket (verified)

Verified by the owner on 2026-10-06 with Tailscale 1.102.4 (`tailscale serve --bg unix:`,
tailnet only): `tailscale serve unix:` does **not** forward the original `Host`. A request for
`https://<pc>.<tailnet>.ts.net/api/status` reaches the socket as

```text
GET /api/status HTTP/1.1
Host: localhost
Tailscale-User-Login: <login>
X-Forwarded-For: 100.x.y.z
X-Forwarded-Host: <pc>.<tailnet>.ts.net
X-Forwarded-Proto: https
```

so the service takes the effective host from `X-Forwarded-Host` and requires
`X-Forwarded-Proto: https` on such a request (section 2). A client that sent forged
`X-Forwarded-Host`, `X-Forwarded-Proto` and `Tailscale-User-Login` values reached the socket
with the real ones: Serve overwrites all three. Post-install check, from a logged-in device on
the tailnet:

```bash
curl -sS -o /dev/null -w '%{http_code}\n' https://<pc>.<tailnet>.ts.net/api/status
```

Expected `200`. A `421` means the effective host is not accepted or the transport is not
`https`: see section 7.

### iPhone home screen and Shortcuts

Every option below goes through the Tailscale VPN, so the iPhone must be connected to the
tailnet (the Tailscale app, optionally with *VPN On Demand*). Replace `<pc>.<tailnet>.ts.net`
with the name printed by `tailscale serve status` (the same name Safari shows in the address
bar). If you read it from `tailscale status --json` (`Self.DNSName`) instead, drop the trailing
dot: `Self.DNSName` ends with `.` (`pc.<tailnet>.ts.net.`), and the service refuses a host with
a trailing dot (`421`).

**Web app icon.** Open `https://<pc>.<tailnet>.ts.net` in Safari, then *Share* → *Add to Home
Screen*. The icon opens the page full screen (live status and the *Lock now* button).

**"Lock PC" shortcut (one tap, or Siri).** In the Shortcuts app, create a shortcut with:

1. *Get Contents of URL*
   - URL: `https://<pc>.<tailnet>.ts.net/api/lock`
   - Method: `POST`
   - Headers: `X-Soos-Action` = `lock`
   - Request Body: *File*, with no file selected (the documented way to send an empty body;
     the service refuses any request body with `413`, and an empty JSON body `{}` is a body;
     a `Content-Length: 0` header is accepted, so a client that declares an empty body is fine)
2. *Get Dictionary Value* `result` from *Contents of URL*.
3. *Show Notification* with the *Dictionary Value* (`lock_requested`, `already_locked`,
   `no_session`, `rate_limited`, `unavailable`, `forbidden` or `misdirected_request`; any
   other value, or a `421`/`403`, is a configuration problem: see section 7).

Name it "Lock PC", then *Share* → *Add to Home Screen* for a one-tap button. Siri runs it by name
("Hey Siri, Lock PC"). The shortcut passes the CSRF check because it sends the action header and
no `Origin` or `Sec-Fetch-Site` header (the server accepts an absent `Origin`); the identity
still comes from Tailscale, so it works only on a device logged in with an allowed login. If
the shortcut reports an error instead of a notification, the service refused the request
(`403`, `421`, `413`): check section 7 and that the body is empty.

**"Unlock PC" shortcut** (only with `allow_unlock = true`, section 2a). The same three actions
as "Lock PC" with:

- URL: `https://<pc>.<tailnet>.ts.net/api/unlock`
- Method: `POST`
- Headers: `X-Soos-Action` = `unlock`
- Request Body: *File*, with no file selected (empty body)

The result is `unlock_requested`, `already_unlocked`, `no_session`, `rate_limited`,
`unavailable`, `unlock_disabled` or `forbidden`. Put a *Choose from Menu* step first (prompt
"Unlock the PC?", items "Unlock" and "Cancel") and place *Get Contents of URL* and the
following actions under "Unlock" only: Siri and home-screen shortcuts run without the page's
confirmation tap, so this step is the shortcut's protection against a stray tap or an
unintended Siri match.

**"PC status" shortcut.** *Get Contents of URL* `https://<pc>.<tailnet>.ts.net/api/status`
(method `GET`), then *Get Dictionary Value* `state`, then *Show Notification* (or *Speak Text*).
`state` is `locked`, `unlocked`, `no_session` or `unavailable`; if the request itself fails,
the PC is unreachable (off, asleep, or Tailscale disconnected).

## 5. Configuration (`remote.toml`)

| Key | Required | Default | Bounds / meaning |
|---|---|---|---|
| `allowed_logins` | yes | — | 1..=8 Tailscale logins (`Tailscale-User-Login` values), printable ASCII, compared case-insensitively; empty → the service refuses to start |
| `allowed_hosts` | no | `[]` (any valid `*.ts.net` name) | 0..=4 DNS names accepted as the effective host (`X-Forwarded-Host`, or `Host`) (lowercased, no port, no scheme) |
| `allow_unlock` | no | `false` | TOML boolean; `true` enables `POST /api/unlock` (section 2a); any other type → refused |
| `poll_interval_ms` | no | 1000 | 250..=10000; logind polling cadence **while a stream is open**; out of range → refused, never clamped |
| `socket_path` | no | `$XDG_RUNTIME_DIR/soos-remote/remote.sock` | absolute, at most 107 bytes; `XDG_RUNTIME_DIR` unset or relative without `socket_path` → refused (no `/tmp` fallback) |

A missing file, an unknown key or an invalid value exits with status 78 (`EX_CONFIG`), which the
unit does not retry (`RestartPreventExitStatus=78`); runtime failures exit 1 and are restarted.
The service also refuses to start when its real or effective uid is 0.

## 6. HTTP surface

| Method + path | Response |
|---|---|
| `GET /`, `/index.html`, `/app.js`, `/style.css`, `/manifest.webmanifest`, `/icon.svg`, `/apple-touch-icon.png` | the embedded page (strict CSP `default-src 'self'`, no inline script, no CDN) |
| `GET /api/status` | `200 application/json` `{state, active, idle, idle_since_unix_s, checked_unix_ms}`; one fresh logind read |
| `GET /api/events` | `200 text/event-stream`; first event = the stream's own fresh read; then an event on every change and a keep-alive at least every 15 s; `503` beyond 4 streams; closed after 30 min (the browser reconnects) |
| `POST /api/lock` | `202 {"result":"lock_requested"}`, `409 no_session` / `already_locked`, `429 rate_limited` (one lock per 2 s), `503 unavailable` |
| `POST /api/unlock` | `403 unlock_disabled` (default), `202 {"result":"unlock_requested"}`, `409 no_session` / `already_unlocked`, `429 rate_limited` (one unlock per 2 s, independent of the lock), `503 unavailable` |
| `HEAD` of a `GET` route | same headers, empty body |

`state` is one of `locked`, `unlocked`, `no_session`, `unavailable`. Any logind failure is
`unavailable`: the service never reports `unlocked` unless a fresh read returned
`LockedHint == false`. The body never contains the uid, user name, session id or seat.

Every response carries `Cache-Control: no-store`, the CSP, `X-Content-Type-Options: nosniff`,
`Referrer-Policy: no-referrer`, `X-Frame-Options: DENY` and `Connection: close`. Requests are
bounded: 16 connections, 8 KiB head, 32 headers, 256-byte path, 5 s to send the head, 2 s per
write, no request body (`413` / `400`), 1.5 s per logind snapshot, 2 s per lock flow, 2 s per
unlock flow.

## 7. Troubleshooting

Start with `systemctl --user status soos-remote` and `journalctl --user -u soos-remote -e`.
Logs never contain the identity, the `Host`, the effective host, a path or a header value, only
reason classes.

| Symptom | Cause | Fix |
|---|---|---|
| Unit `failed`, exit status 78, not restarted | Configuration refused: `configuration file not found`, `allowed_logins is empty`, `poll_interval_ms out of range`, `socket_path must be absolute…`, `XDG_RUNTIME_DIR is not set or not absolute`, `configuration is not valid TOML for soos-remote` (unknown key included), `soos-remote must not run as root`, or a socket directory that is a symlink, a file or not yours | Fix `remote.toml` (section 5) or the socket directory, then `systemctl --user restart soos-remote` |
| Unit restarts every 5 s with exit status 1 | Runtime failure: the socket directory could not be created or bound (`$XDG_RUNTIME_DIR` missing, see linger in section 4), or the poller or accept loop ended | Check the journal line before the exit; make sure `$XDG_RUNTIME_DIR` exists for your user |
| Phone gets `403` on every page | Your login is not in `allowed_logins` (compared case-insensitively, the value is the Tailscale account login, usually the e-mail), the request did not come through `tailscale serve` (no `Tailscale-User-Login`: Funnel, a tagged device, a direct `curl` on the socket), or the header was repeated | Fix the list; use a device logged in with your account; never expose the socket another way |
| Phone gets `421 Misdirected Request` | The effective host (`X-Forwarded-Host`, section 4) is not an allowed `*.ts.net` name / not in `allowed_hosts`, or `X-Forwarded-Proto` is missing or not `https` (`tailscale serve --http`, a port other than 443) | Use `tailscale serve --bg unix:` on port 443; set `allowed_hosts` if the name differs |
| Page shows `Unreachable` | No event for 45 s: the PC is off, asleep or off the tailnet, `tailscaled` is stopped, the unit is down, or iOS suspended the web app | Bring the page back to the foreground (it reconnects), check `tailscale status` and the unit |
| Status `Unavailable` | logind could not be read within the bounds (system bus down, `ListSessions`/`GetAll` failed or timed out, too many sessions) | `busctl --system status org.freedesktop.login1`; the status never falls back to a stale value |
| Status `No session` | None of your sessions is `Class=user`, `Remote=false` and on a seat (only SSH, a greeter, or you are logged out) | Expected from the login screen; log in locally |
| Status stays `Unlocked` while the screen is locked | The desktop does not set `LockedHint` (sway family without the wrapper) | Section 3, first bullet |
| "Lock requested…" then "The desktop did not confirm the lock (LockedHint unchanged)" | The desktop ignores the logind `Lock` signal, or the locker runs without the `SetLockedHint` wrapper | Section 3, second bullet |
| `POST /api/lock` answers `429` | A lock was accepted less than 2 s ago | Wait and retry |
| `POST /api/lock` answers `409` | `no_session` (see above) or `already_locked` | Nothing to do |
| `POST /api/unlock` answers `403 unlock_disabled` | `allow_unlock` is absent or `false` | Section 2a, if you accept its risk |
| "Unlock requested…" then "The desktop did not confirm the unlock (LockedHint unchanged)" | logind emitted `Unlock` but the locker ignored it (no `swayidle` `unlock` hook), or the wrapper did not clear `LockedHint` | Section 3; check `swayidle` runs with `unlock 'pkill -USR1 swaylock'` |
| `GET /api/events` answers `503` | Four streams are already open (old suspended tabs) | Close other tabs; a stream ends by itself after 30 min |

## 8. Residual limitations

- A suspended iOS web app keeps its stream slot until `tailscaled` closes the proxied connection
  or the 30 min stream lifetime elapses; with four slots, the owner's own reconnects are the only
  consumer. The page shows `Unreachable` when no event arrived for 45 s (measured on the phone
  from the last event's arrival, not from `checked_unix_ms`).
- The status is only as truthful as the desktop's `LockedHint` (section 3).
- The host and transport checks rely on `tailscale serve` setting `X-Forwarded-Host` and
  `X-Forwarded-Proto` (verified with Tailscale 1.102.4, 2026-10-06; section 4); a Serve
  release that stops sending them makes every proxied request `421` (fail-closed, section 7),
  never `200`.
- The `bind` → `chmod 0600` window of the socket is closed by the `0700` parent directory and
  the unit's `UMask=0077`; the service must therefore be started through the unit (or with the
  same umask).

## 9. Out of scope (each needs its own ADR)

- **A second factor for the remote unlock** (passkey / Face ID through WebAuthn) and an
  automatic re-lock after a remote unlock: the owner chose the Tailscale identity alone
  (section 2a). The crate still never names `SetLockedHint`, the `Unlock` signal or a
  session-ending method (invariant RMC-S3).
- **Push notifications** (lock/unlock alerts while the page is in the background).
- **Live camera** or any frame, embedding or evidence access.
- System-wide packaging (`install.sh`, deb/rpm/Arch): deferred until the owner approves the
  merge; `scripts/install_remote.sh` is the only installer.
