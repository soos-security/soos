# Remote Companion (`soos-remote`)

> Crate: `crates/remote` (`soos-remote`, GitHub #339, ADR 2026-10-05 "Remote Companion `soos-remote`",
> ADR 2026-10-06 "Remote Unlock in `soos-remote`", ADR 2026-10-06 "Tailscale Funnel Access and
> In-House Passkey Authentication for `soos-remote`")
> Scope: a **user-level** service that shows the owner's phone the real-time lock status of the
> desktop session, offers a remote **lock** and, only when `allow_unlock = true`, a remote
> **unlock** protected by a passkey (Face ID) on every request (section 2a). Optionally it is
> also reachable from the internet through Tailscale Funnel, behind a passkey login (section
> 2b). Nothing else: no camera, no push notification (see "Out of scope").
> Source of truth: `crates/remote/src/lib.rs` (constants), `crates/remote/src/config.rs`
> (configuration keys), `crates/remote/src/server.rs` (request handling). If this document and
> the code disagree, report the drift: the invariant `remote_companion_contract` pins the parts
> of this page the acceptance criteria rely on (matrix rows RMC1–RMC44, walkthroughs 183, 184
> and 185); `remote_passkey_contract` pins the Funnel and passkey parts.

---

## 1. What it is, and what it is not

`soos-remote` is a small HTTP/1.1 server that:

- listens on **one Unix socket** (`0600`, inside a `0700` directory under `$XDG_RUNTIME_DIR`)
  and never opens a network socket (`RestrictAddressFamilies=AF_UNIX` in the unit);
- is reached from the phone through the owner's Tailscale tailnet (`tailscale serve`
  terminates HTTPS with the node's `*.ts.net` certificate and proxies to the socket) and,
  only when `allow_funnel = true`, from the public internet through `tailscale funnel` on
  port 443 (same name, same certificate, section 2b);
- reads the session state from systemd-logind (`LockedHint`, `IdleHint`, `IdleSinceHint`,
  `Active`) over the pinned system bus, exactly like the daemon's presence worker;
- can ask logind to **lock** the owner's local session (`Manager.LockSession`);
- when `allow_unlock = true`, can ask logind to **unlock** it (`Manager.UnlockSession`,
  section 2a), only after a fresh passkey assertion with user verification (Face ID / Touch ID);
- verifies WebAuthn passkeys itself (ES256, attestation `none`, pure-Rust RustCrypto code, no
  OpenSSL, no external service).

It runs as the session owner, never as root. `soos-daemon`, `pam_soos.so` and the IPC protocol
are untouched; no other crate depends on `soos-remote`.

## 2. Trust model (read this before exposing anything)

Every request is classified first. A **tailnet** request carries exactly one
`Tailscale-User-Login` header whose value is listed in `allowed_logins`. A **Funnel** request
carries exactly one `Tailscale-Funnel-Request: ?1` header and no identity; it is accepted only
when `allow_funnel = true` (section 2b). Both markers together, a repeated marker, any other
marker value, or neither marker is `403`. The identity header is trusted for two reasons, and
only while both hold:

1. **`tailscaled` sets it and strips any client-supplied copy.** It is absent for Funnel
   traffic (which carries the Funnel marker instead, also set by `tailscaled` after deleting
   any client copy) and for tagged (non-user) devices, which are therefore refused.
2. **The backend is a `0600` Unix socket in a `0700` directory owned by the session owner.**
   Only `tailscaled` (root) and the owner can connect to it, so a local process cannot inject a
   forged identity, and the owner forging their own login is no escalation.

Consequences:

- **Never** put any other reverse proxy in front of the socket, and never run
  `tailscale serve --http` (plain HTTP) for it, nor `tailscale funnel` on a port other than
  443: each one voids the model (forgeable headers, or an origin the passkeys are not bound
  to). `tailscale funnel` on port 443 is supported only together with `allow_funnel = true`
  and `rp_id` (section 2b); without them a Funnel request is `403 forbidden`.
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
  rely on. `X-Forwarded-For` is read for Funnel requests only, and only to choose an
  anonymous rate-limit bucket (never an authorization input, never logged).
- `POST /api/lock` additionally needs `X-Soos-Action: lock` (a non-simple header a cross-origin
  page cannot send without a CORS preflight, which is never answered), `Sec-Fetch-Site` absent or
  `same-origin`, and `Origin` absent or equal to `https://<effective host>`.
- `POST /api/unlock` needs the same three CSRF conditions with `X-Soos-Action: unlock`; the lock
  route never accepts the unlock value and the unlock route never accepts `lock`.
- Every passkey route (`/api/auth/*`) additionally needs its own `X-Soos-Action` value and an
  `Origin` equal to `https://<rp_id>` (required, not optional).
- Nothing about a request is logged: no identity, no `Host`, no path, no header value, no
  session id, no challenge, cookie, credential id, public key, user handle or enrollment code.
  Denials log their reason class only. An accepted unlock adds one `info` line,
  `remote unlock requested`, an accepted Funnel login one line `remote login accepted`, a new
  passkey one line `passkey registered`, without any of these.

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

Unlock also needs `rp_id` (section 2b) and at least one registered passkey: **every** unlock,
on the tailnet as over Funnel, carries a fresh passkey assertion with user verification over an
`unlock`-purpose challenge (`POST /api/auth/unlock/options`, 120 s, single use). Without a body
the answer is `403 {"result":"passkey_required"}`; an invalid, replayed or UV-less assertion is
`403 passkey_rejected`; without `rp_id` every unlock is `403 passkeys_not_configured`.

While it is disabled, `POST /api/unlock` answers `403 {"result":"unlock_disabled"}` without
reading the body or logind. The page shows an *Unlock now* button (enabled only while the
session is `Locked`) that asks for a confirmation tap, then for Face ID, before sending the
request.

**Accepted risk** (owner decisions, ADR 2026-10-06 "Remote Unlock" as amended by the Funnel and
passkey ADR): an unlock needs the owner's passkey and its user verification (Face ID / Touch ID)
on every request; the Tailscale identity alone (tailnet) or a web session alone (Funnel) is no
longer enough. Whoever can pass Face ID on a device holding the passkey (or the device passcode
fallback the platform offers) can unlock the PC, and the PC stays unlocked until it locks again
by itself (no automatic re-lock). Residual risks also accepted: a distributed attacker can keep
the anonymous Funnel capacity busy and delay a Funnel **login** (the tailnet path and a
signed-in owner keep their own capacity and limiters); a process running as the owner's uid can
reach the socket directly and read a pending enrollment code, which grants nothing it could not
already do (it can call logind on its own session). To revoke the capability at once: set
`allow_unlock = false` (or remove the login from `allowed_logins`) and restart the unit, remove
the passkey with `soos-remote passkeys remove N`, or remove the device from the tailnet in the
Tailscale admin console.

## 2b. Internet access through Tailscale Funnel, and passkeys

Owner decision (ADR 2026-10-06): reach the PC from the iPhone without a permanent VPN and
without any paid service, through **Tailscale Funnel** on the node's own `*.ts.net` name,
**port 443** only. Authentication is in-house: WebAuthn passkeys (Face ID / Touch ID on the
iPhone, user verification required), verified by `soos-remote` itself.

**Funnel is optional.** The default, and the recommended deployment, stays **tailnet-only**
(`tailscale serve`, the iPhone connected to the tailnet with the Tailscale VPN): with
`allow_funnel` absent or `false` every Funnel request is `403`, and the passkeys still protect
every unlock (section 2a). Enable Funnel only if you accept the extra public exposure described
under *Accepted risk* below. Other public exposure paths (for example Cloudflare Tunnel or
Cloudflare Access) are not covered by this page or by the ADR.

**Prerequisites for Funnel** (tailnet admin console, owner actions): Tailscale 1.38.3 or later,
MagicDNS and HTTPS certificates enabled for the tailnet, and the `funnel` node attribute granted
to this PC in the tailnet policy file, for example

```jsonc
"nodeAttrs": [{"target": ["autogroup:member"], "attr": ["funnel"]}]
```

(narrow the target to your own user or the PC if your policy allows it). Funnel is refused while
*shields up* is on, works on the free Personal plan, and needs no domain of your own.

Configuration (`remote.toml`, then restart the unit):

```toml
rp_id = "<pc>.<tailnet>.ts.net"   # the full node name, never the bare tailnet name
allow_funnel = true               # opt-in; false (the default) keeps Funnel requests at 403
allow_unlock = true               # optional, section 2a
```

1. **Register the iPhone's passkey over the tailnet** (VPN on). On the PC run
   `soos-remote enroll-code`: it prints a one-time code (10 symbols, valid 5 minutes, single
   use, 3 wrong attempts delete it) and stores only its hash in the `0700` socket directory.
   Open `https://<pc>.<tailnet>.ts.net`, type the code under *Add a passkey* and confirm with
   Face ID. Registration is impossible from the internet: the register routes answer `403` on
   the Funnel path, and need an allowed Tailscale identity **and** the local code.
2. **Publish the socket on the internet** (owner action; neither the installer nor any
   agent runs `tailscale`):

   ```sh
   tailscale funnel --bg unix:$XDG_RUNTIME_DIR/soos-remote/remote.sock
   tailscale funnel status        # must list https://<pc>.<tailnet>.ts.net (port 443) only
   ```

   The public DNS name can take up to 10 minutes to resolve after the first activation. Tailnet
   clients (VPN on) keep reaching the same address directly and keep their Tailscale identity.
   To stop the public exposure: `tailscale funnel --https=443 off`, then, if
   `tailscale serve status` no longer lists the socket, restore the tailnet-only handler with
   `tailscale serve --bg unix:$XDG_RUNTIME_DIR/soos-remote/remote.sock`; or simply set
   `allow_funnel = false` and restart the unit (Funnel requests are then `403`).
3. With the VPN off, open the same address in Safari or the home-screen app: *Sign in with
   Face ID* creates a web session (cookie `__Host-soos_session`, `Secure`, `HttpOnly`,
   `SameSite=Strict`, 15 min idle, 8 h at most, at most 4 sessions, stored on the PC only as a
   hash). Status, events and lock then work as on the tailnet; unlock asks for Face ID again.

Rules on the Funnel path (requests carrying `Tailscale-Funnel-Request: ?1`):

- the effective host must be exactly `rp_id` (`421` otherwise);
- without a valid session only the page, its assets, `GET /api/auth/state`, the login ceremony
  and `POST /api/auth/logout` are reachable; status, events, lock and unlock answer
  `403 {"result":"login_required"}`; the register routes answer `403 forbidden`;
- limits: 10 challenges per minute and 5 failures per 5 minutes per limiter key (tailnet,
  signed-in Funnel owner, each anonymous client address), at most 8 of the 16 connections for
  Funnel, 4 of them anonymous, 2 anonymous request bodies, 2 Funnel event streams; a refused
  request is `503 busy` and never counts as a failure;
- removing a passkey (`soos-remote passkeys remove N`) ends the sessions it opened at their next
  request or keep-alive.

Passkey management on the PC: `soos-remote passkeys list` prints, per passkey, its number, its
registration time, its backup state and an 8-hex fingerprint (never the credential id);
`soos-remote passkeys remove N` removes one. The store is
`<directory of remote.toml>/remote-passkeys.json` (`credentials_path` overrides it), mode `0600`,
at most 4 passkeys, written atomically under a lock. Every subcommand refuses root.
A stored sign counter never goes down: two concurrent assertions of one device-bound passkey
re-check the counter under the store lock, so the higher value is kept.

The service reads and writes this store synchronously on its single-threaded runtime. This is an
accepted, bounded stall: each Funnel request re-reads at most 16 KiB of the local file (parsed
again only when it changed), and a write (registration, removal, counter change of a device-bound
passkey) is one small atomic write waited for at most 500 ms on the lock.

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
   never use `tailscale serve --http` for this socket, and use `tailscale funnel` only on port
   443 together with `allow_funnel = true` and `rp_id` (section 2b).
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

**"Unlock PC" shortcut: no longer possible.** Every unlock now needs a fresh passkey assertion
with Face ID, which only the web page can produce (Shortcuts cannot run WebAuthn). A request
with URL `https://<pc>.<tailnet>.ts.net/api/unlock`, method `POST` and the header
`X-Soos-Action` = `unlock` but no assertion body is answered `403 passkey_required` (or
`unlock_disabled`, or `403 passkeys_not_configured` while `rp_id` is unset; `already_unlocked`
and the other results need an assertion). Replacement: an
*Open URLs* shortcut on `https://<pc>.<tailnet>.ts.net` that opens the web app, then *Unlock
now* and Face ID.

The "Lock PC" and "PC status" shortcuts keep working **on the tailnet only** (VPN on): over
Funnel they get `403 login_required`, because Shortcuts does not share the web app's session
cookie.

**"PC status" shortcut.** *Get Contents of URL* `https://<pc>.<tailnet>.ts.net/api/status`
(method `GET`), then *Get Dictionary Value* `state`, then *Show Notification* (or *Speak Text*).
`state` is `locked`, `unlocked`, `no_session` or `unavailable`; if the request itself fails,
the PC is unreachable (off, asleep, or Tailscale disconnected).

## 5. Configuration (`remote.toml`)

| Key | Required | Default | Bounds / meaning |
|---|---|---|---|
| `allowed_logins` | yes | — | 1..=8 Tailscale logins (`Tailscale-User-Login` values), printable ASCII, compared case-insensitively; empty → the service refuses to start |
| `allowed_hosts` | no | `[]` (any valid `*.ts.net` name) | 0..=4 DNS names accepted as the effective host (`X-Forwarded-Host`, or `Host`) (lowercased, no port, no scheme) |
| `allow_unlock` | no | `false` | TOML boolean; `true` enables `POST /api/unlock` (section 2a); any other type → refused; without `rp_id` every unlock is refused (a warning is logged at start-up) |
| `rp_id` | no | absent (passkeys off) | the full node name `<pc>.<tailnet>.ts.net` (lowercased, at least two labels before `.ts.net`, no port, no scheme; a member of `allowed_hosts` when that list is set); the WebAuthn relying party, origin `https://<rp_id>` |
| `allow_funnel` | no | `false` | TOML boolean; `true` accepts `Tailscale-Funnel-Request: ?1` requests (section 2b); requires `rp_id` |
| `credentials_path` | no | `<dir of remote.toml>/remote-passkeys.json` | absolute path of the passkey store, at most 4096 bytes |
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
| `POST /api/unlock` | JSON assertion body (section 2a). `403 unlock_disabled` (default), `403 passkeys_not_configured`, `403 passkey_required` (no body), `403 passkey_rejected`, `400 bad_request`, `413 body_too_large`, `202 {"result":"unlock_requested"}`, `409 no_session` / `already_unlocked`, `429 rate_limited` (one unlock per 2 s, independent of the lock; or the failure lockout), `503 unavailable` / `store_unavailable` |
| `GET /api/auth/state` | `{"mode":"tailnet"\|"funnel","authenticated",…}`; an anonymous Funnel caller gets only `{"mode":"funnel","authenticated":false}` |
| `POST /api/auth/login/options`, `POST /api/auth/login/verify` | Funnel only: passkey login (`200 logged_in` + `Set-Cookie`), `403 passkey_rejected`, `409 no_passkey`, `429 rate_limited` / `too_many_sessions`, `503 busy` |
| `POST /api/auth/logout` | Funnel only: `200 logged_out`, clears the cookie |
| `POST /api/auth/unlock/options` | tailnet, or Funnel with a session: an `unlock` challenge (`200 {challenge, rp_id, timeout_ms}`) |
| `POST /api/auth/register/options`, `POST /api/auth/register/verify` | tailnet only, with the code of `soos-remote enroll-code`: `200 registered`, `403 enroll_code_rejected`, `409 passkey_limit` / `already_registered` / `registration_conflict` |
| `HEAD` of a `GET` route | same headers, empty body |

`state` is one of `locked`, `unlocked`, `no_session`, `unavailable`. Any logind failure is
`unavailable`: the service never reports `unlocked` unless a fresh read returned
`LockedHint == false`. The body never contains the uid, user name, session id or seat.

Every response carries `Cache-Control: no-store`, the CSP, `X-Content-Type-Options: nosniff`,
`Referrer-Policy: no-referrer`, `X-Frame-Options: DENY` and `Connection: close`. Requests are
bounded: 16 connections, 8 KiB head, 32 headers, 256-byte path, 5 s to send the head, 2 s per
write, no request body except on the four passkey body routes (`413 body_not_allowed` /
`400`; on those routes at most 8 KiB, `Content-Length` or strict `Transfer-Encoding: chunked`,
5 s to send it, `413 body_too_large`), 1.5 s per logind snapshot, 2 s per lock flow, 2 s per
unlock flow.

## 7. Troubleshooting

Start with `systemctl --user status soos-remote` and `journalctl --user -u soos-remote -e`.
Logs never contain the identity, the `Host`, the effective host, a path or a header value, only
reason classes.

| Symptom | Cause | Fix |
|---|---|---|
| Unit `failed`, exit status 78, not restarted | Configuration refused: `configuration file not found`, `allowed_logins is empty`, `poll_interval_ms out of range`, `socket_path must be absolute…`, `XDG_RUNTIME_DIR is not set or not absolute`, `configuration is not valid TOML for soos-remote` (unknown key included), `soos-remote must not run as root`, or a socket directory that is a symlink, a file or not yours | Fix `remote.toml` (section 5) or the socket directory, then `systemctl --user restart soos-remote` |
| Unit restarts every 5 s with exit status 1 | Runtime failure: the socket directory could not be created or bound (`$XDG_RUNTIME_DIR` missing, see linger in section 4), or the poller or accept loop ended | Check the journal line before the exit; make sure `$XDG_RUNTIME_DIR` exists for your user |
| Phone gets `403` on every page | Your login is not in `allowed_logins` (compared case-insensitively, the value is the Tailscale account login, usually the e-mail), the request did not come through `tailscale serve` (no `Tailscale-User-Login`: a tagged device, a direct `curl` on the socket, or a Funnel request while `allow_funnel` is `false`), or the header was repeated | Fix the list; use a device logged in with your account; enable Funnel only as in section 2b; never expose the socket another way |
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
| `GET /api/events` answers `503` | Four streams are already open (old suspended tabs; two at most over Funnel) | Close other tabs; a stream ends by itself after 30 min |
| `POST /api/unlock` answers `403 passkey_required` | The request carried no passkey assertion (an old "Unlock PC" shortcut, section 4) | Unlock from the web page (Face ID) |
| `403 passkeys_not_configured` | `rp_id` is not set | Section 2b |
| Funnel page shows the sign-in screen again / `403 login_required` | The web session expired (15 min idle, 8 h at most), was signed out, or its passkey was removed | Sign in with Face ID again |
| `409 no_passkey` | No passkey registered yet | `soos-remote enroll-code`, then register over the tailnet (section 2b) |
| `403 enroll_code_rejected` | No code, a wrong one (3 attempts delete it) or an expired one (5 min) | Run `soos-remote enroll-code` again |
| `503 store_unavailable` | The passkey store is unreadable, not `0600`, not yours, or busy | Fix the file mode/owner (`chmod 600`), check the journal |
| `503 busy` over Funnel | The Funnel capacity is in use (many simultaneous internet requests) | Retry; the tailnet path is not affected |
| `421` over Funnel only | The address used is not exactly `rp_id` (another name, a trailing dot, port 8443 or 10000) | Open `https://<rp_id>` exactly; keep Funnel on port 443 |
| The address does not resolve with the VPN off | Funnel is not active (`tailscale funnel status` empty), the `funnel` node attribute is missing, *shields up* is on, or the public DNS name is younger than about 10 minutes | Section 2b prerequisites; wait and retry |
| `tailscale funnel` refuses to start | The tailnet lacks HTTPS certificates or MagicDNS, or the node lacks the `funnel` attribute | Enable them in the admin console (section 2b) |
| Face ID prompt never appears / `passkey_rejected` on every try | The page was not opened on `https://<rp_id>` (the passkey is bound to that exact name), the passkey was removed, or user verification was skipped | Open the exact `rp_id` address; `soos-remote passkeys list`; register again over the tailnet |
| Signed in in Safari but the home-screen app asks again | Safari and the home-screen app keep separate cookies | Sign in once in each |

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
- Funnel classification relies on `tailscaled` setting `Tailscale-Funnel-Request: ?1` (verified
  in the Tailscale 1.102.4 source, undocumented): a release that stops sending it makes Funnel
  requests `403` (fail-closed), never authenticated.
- Over Funnel an anonymous `POST /api/auth/login/options` answers `409 no_passkey` when no
  passkey is registered, so an internet client can learn whether one exists (no credential id,
  user handle or identity is disclosed; candid review MINOR finding, follow-up).
- A device-bound (non-synced) passkey presenting the same counter twice concurrently is accepted
  twice (clone detection only, never an authentication bypass; synced iCloud passkeys report
  `0/0` and are unaffected; candid review MINOR finding, follow-up).
- The `bind` → `chmod 0600` window of the socket is closed by the `0700` parent directory and
  the unit's `UMask=0077`; the service must therefore be started through the unit (or with the
  same umask).

## 9. Out of scope (each needs its own ADR)

- An automatic re-lock after a remote unlock (owner decision D-F). The crate still never names
  `SetLockedHint`, the `Unlock` signal or a session-ending method (invariant RMC-S3).
- Funnel on ports 8443 or 10000, other attestation formats than `none`, other algorithms than
  ES256, several users.
- Any other public exposure path (Cloudflare Tunnel, Cloudflare Access, a custom domain, a
  reverse proxy): explored separately; it needs its own ADR before it may reach this socket.
- **Push notifications** (lock/unlock alerts while the page is in the background).
- **Live camera** or any frame, embedding or evidence access.
- System-wide packaging (`install.sh`, deb/rpm/Arch): deferred until the owner approves the
  merge; `scripts/install_remote.sh` is the only installer.
