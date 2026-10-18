# Remote Companion (`soos-remote`)

> Crate: `crates/remote` (`soos-remote`, GitHub #339, ADR 2026-10-05 "Remote Companion `soos-remote`",
> ADR 2026-10-06 "Remote Unlock in `soos-remote`", ADR 2026-10-06 "Tailscale Funnel Access and
> In-House Passkey Authentication for `soos-remote`", ADR 2026-10-06 "Failed-Password Alerts in
> `soos-remote` From the System Journal", ADR 2026-10-06 "Web Push Notifications for
> Failed-Password Alerts Through a Separate Sender Unit", ADR 2026-10-07 "Live Camera View in
> `soos-remote` Through the Daemon Preview Channel", ADR 2026-10-07 "Live Battery Level in
> `soos-remote`", GitHub #346)
> Scope: a **user-level** service that shows the owner's phone the real-time lock status of the
> desktop session, offers a remote **lock** and, only when `allow_unlock = true`, a remote
> **unlock** protected by a passkey (Face ID, fingerprint or screen lock) on every request (section 2a). Optionally it is
> also reachable from the internet through Tailscale Funnel, behind a passkey login (section
> 2b). When `password_alerts = true` the page also lists failed password attempts made on the
> PC (section 2c), never the typed password, and with `push_notifications = true` sends them to
> the phone as push notifications even when the app is closed (section 2d). With
> `camera_view = true` and the daemon's `[preview] remote_view = true` it also shows a live view
> of the PC camera after a passkey check (section 2f), never recorded. The status card also shows the
> PC's battery level, charge state and mains presence (section 2g, on by default,
> `battery_status = false` turns it off). Nothing else (see "Out of scope").
> Source of truth: `crates/remote/src/lib.rs` (constants), `crates/remote/src/config.rs`
> (configuration keys), `crates/remote/src/server.rs` (request handling). If this document and
> the code disagree, report the drift: the invariant `remote_companion_contract` pins the parts
> of this page the acceptance criteria rely on (matrix rows RMC1–RMC44, walkthroughs 185, 186
> and 187); `remote_passkey_contract` pins the Funnel and passkey parts; `remote_alerts_contract`
> pins the failed-password alerts (rows RMC45–RMC59, walkthrough 188); `remote_push_contract`
> pins the push notifications (rows RMC60–RMC74, walkthrough 189); `remote_camera_contract`
> pins the live camera view (rows RLC1–RLC16, walkthrough 191); `remote_android_contract` pins
> the Android support (rows RAN1–RAN12, walkthrough 193, section 2h).

---

## 1. What it is, and what it is not

`soos-remote` is a small HTTP/1.1 server that:

- listens on **one Unix socket** (`0600`, inside a `0700` directory under `$XDG_RUNTIME_DIR`)
  and never opens a network socket itself (`RestrictAddressFamilies=AF_UNIX` in the unit); the
  optional `soos-push-sender` unit (section 2d) is the only part of the companion that makes
  outbound HTTPS requests, to three push services only;
- is reached from the phone through the owner's Tailscale tailnet (`tailscale serve`
  terminates HTTPS with the node's `*.ts.net` certificate and proxies to the socket) and,
  only when `allow_funnel = true`, from the public internet through `tailscale funnel` on
  port 443 (same name, same certificate, section 2b);
- reads the session state from systemd-logind (`LockedHint`, `IdleHint`, `IdleSinceHint`,
  `Active`) over the pinned system bus, exactly like the daemon's presence worker;
- can ask logind to **lock** the owner's local session (`Manager.LockSession`);
- when `allow_unlock = true`, can ask logind to **unlock** it (`Manager.UnlockSession`,
  section 2a), only after a fresh passkey assertion with user verification (Face ID, fingerprint or screen lock);
- verifies WebAuthn passkeys itself (ES256, attestation `none`, pure-Rust RustCrypto code, no
  OpenSSL, no external service);
- when `password_alerts = true`, follows the system journal through a `journalctl` child
  process and shows failed password attempts made on the PC (section 2c);
- when `push_notifications = true`, encrypts a short summary of each new burst of failed
  attempts for the phone and hands it to `soos-push-sender`, which delivers it as a standard
  Web Push notification (section 2d);
- when `camera_view = true`, asks `soos-daemon` for preview frames over its Unix socket
  (`RequestKind::PreviewFrame`) and streams them as JPEG to the page after a fresh passkey
  assertion (section 2f).
- unless `battery_status = false`, reads the battery level, charge state and mains presence
  from `/sys/class/power_supply` (read-only, bounded) and shows them on the status card
  (section 2g).

It runs as the session owner, never as root. It is **not** a camera owner (it never opens
`/dev/video*`), a recorder (no frame, snapshot or video is ever stored), a second face
verifier or a way to view the camera after a full logout. `pam_soos.so` and the IPC wire format
are untouched; the daemon only gained the opt-in `[preview] remote_view` key and the local seat
session rule for previews (section 2f). Its only soos dependency is `soos-protocol`; no other
crate depends on `soos-remote`.

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

Reason 1 was verified on **Tailscale 1.102.4** (ADR 2026-10-12 "soos-remote Security
Hardening"): run Tailscale 1.102.4 or later on this PC, for `tailscale serve` as for Funnel
(check with `tailscale version`). The service cannot detect the proxy version.

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

## 2a. Remote unlock (off by default, on after `scripts/setup.sh`)

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
session is `Locked`) that asks for a confirmation tap, then for the passkey check, before sending the
request.

**Accepted risk** (owner decisions, ADR 2026-10-06 "Remote Unlock" as amended by the Funnel and
passkey ADR): an unlock needs the owner's passkey and its user verification (Face ID, fingerprint or screen lock)
on every request; the Tailscale identity alone (tailnet) or a web session alone (Funnel) is no
longer enough. Whoever can pass the user verification on a device holding the passkey (or the device passcode
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

Owner decision (ADR 2026-10-06): reach the PC from the phone without a permanent VPN and
without any paid service, through **Tailscale Funnel** on the node's own `*.ts.net` name,
**port 443** only. Authentication is in-house: WebAuthn passkeys (user verification required:
Face ID, fingerprint or screen lock), verified by `soos-remote` itself.

**Funnel is optional.** The code default and the manual install stay **tailnet-only**
(`tailscale serve`, the phone connected to the tailnet with the Tailscale VPN). The one-command
`scripts/setup.sh` turns Funnel on (ADR 2026-10-15 "Setup User Phase: Phone Companion Profile With
Default-On Features" item (3)). With `allow_funnel` absent or `false` every Funnel request is `403`, and the passkeys still protect
every unlock (section 2a). Enable Funnel only if you accept the extra public exposure described
under *Accepted risk* below. Other public exposure paths (for example Cloudflare Tunnel or
Cloudflare Access) are not covered by this page or by the ADR.

**Prerequisites for Funnel** (tailnet admin console, owner actions): Tailscale **1.102.4 or later**
on this PC (the release whose stripping of client-supplied `Tailscale-User-Login` and
`Tailscale-Funnel-Request` headers was verified; an older `tailscaled` might forward a forged
identity header from an internet client, which the service would then treat as a tailnet
caller; check with `tailscale version`), MagicDNS and HTTPS certificates enabled for the tailnet, and the `funnel` node attribute granted
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

1. **Register the phone's passkey over the tailnet** (VPN on). On the PC run
   `soos-remote enroll-code`: it prints a one-time code (10 symbols, valid 5 minutes, single
   use, 3 wrong attempts delete it) and stores only its hash in the `0700` socket directory.
   Open `https://<pc>.<tailnet>.ts.net`, type the code under *Add a passkey* and confirm with
   the phone's user verification (Face ID, fingerprint or screen lock). Registration is impossible from the internet: the register routes answer `403` on
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
3. With the VPN off, open the same address in the browser or the installed web app: *Sign in
   with your passkey* creates a web session (cookie `__Host-soos_session`, `Secure`, `HttpOnly`,
   `SameSite=Strict`, 15 min idle, 8 h at most, at most 4 sessions, stored on the PC only as a
   hash). Status, events and lock then work as on the tailnet; unlock asks for the passkey again.
   Sign-in names no credential (discoverable passkey, so the phone may offer a picker); the
   unlock and camera-start ceremonies, which already need a session or an allowed tailnet
   identity, name the registered passkeys (`allow_credentials`) so the phone goes straight to its
   biometric prompt (section 2h).

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

## 2c. Failed-password alerts (off by default, on after `scripts/setup.sh`)

When someone types a wrong password on the PC (lock screen, `sudo`, GDM or console login, any
other local PAM password check), the page shows a banner such as
"3 failed password attempts, last at 14:07 (lock screen)", a short history and an
*Acknowledge* button. The phone sees it on the tailnet and over a Funnel session; an anonymous
Funnel visitor never does.

**What is shown, and what never is.** Each history row carries only the time, the source class
(`lock_screen`, `sudo`, `login`, `other`), the account class ("your account", "root", "another
account"), the kind (wrong password, or an attempt while `pam_faillock` had locked the account)
and a count. It is **never the typed password**, nor any part, length or hash of it: no journal
line contains it (`unix_chkpwd` and `pam_unix` never log it), and soos does not add any PAM
component to capture it (`AGENTS.md`). The user-name field of a journal line can hold text typed
into a user-name box (a password typed at GDM by mistake), so it is reduced at once to the three
account classes and dropped: no user name, uid, service name, program path, journal cursor or
raw log line reaches the page, an event or a log.

**Setup** (nothing is read until you enable it):

1. The service account must be able to read the **system** journal: be a member of `wheel`,
   `adm` or `systemd-journal` (check with `id -nG`). A new group membership takes effect only in
   a new user-manager session (log out and in, or reboot); restarting the service is not enough.
   Without it the page says "Password alerts unavailable: the account cannot read the system
   journal", never "no attempts".
2. In `remote.toml`:

   ```toml
   password_alerts = true
   # Mandatory when the lock screen is not one of /usr/bin/{swaylock,hyprlock,gtklock,waylock}:
   lock_screen_programs = ["/home/<you>/.local/bin/swaylock-plugin"]
   ```

   `lock_screen_programs` lists at most 4 absolute paths of the lock-screen programs whose own
   `pam_unix` lines are trusted. On a host whose locker lives outside `/usr/bin` (for example the
   driftwm `swaylock-plugin` in `~/.local/bin`) this key is **mandatory**: when none of the
   configured paths exists the page shows "Lock screen not monitored — see setup" and lock-screen
   attempts are not counted (sudo, login and the other sources still are).
3. `systemctl --user restart soos-remote` (done by you; the unit file itself is unchanged).

**How it works.** The service spawns `/usr/bin/journalctl --follow --output=json` with a fixed
field list and the match `SYSLOG_FACILITY=10` (environment cleared, no shell, killed with the
service). It first probes that a root-owned journal entry is visible (2 s bound); the view is
`starting` while the last 24 hours are replayed and becomes `active` once caught up. A stopped
reader restarts with a 1 s → 60 s backoff and resumes after the last cursor. One attempt is
counted per real password check (`unix_chkpwd`), merged with the `pam_unix` failure line of the
same attempt; attempts refused while `pam_faillock` locked the account are counted separately.
Trust comes only from journald-set fields (`_UID`, `_EXE`, `_COMM`, `_TRANSPORT`): root-side
lines, the setuid `sudo`/`su`, and the configured lock-screen programs; lines of other accounts
and of other programs of yours (for example test binaries that write real
`pam_unix(swaylock:auth)` lines) are ignored.

**Routes.** `GET /api/alerts` returns the view (`state` `disabled`/`starting`/`active`/
`unavailable`, `reason`, `epoch`, `lock_screen`, the unacknowledged counts, the last attempt
time and source, `through`, `history`). `POST /api/alerts/ack` has no body and needs
`X-Soos-Action: alerts-ack` plus exactly one `X-Soos-Alerts-Epoch` and one
`X-Soos-Alerts-Through` header naming the view the page displayed; it acknowledges exactly the
attempts of that view. A view from before a service restart is refused with `409 stale_view`
(the page then shows the fresh view, and you acknowledge again after reading it). One
acknowledgement per second (`429`). The event stream adds `event: alerts` (at most one per
second). Only the acknowledgement marker is stored, in `remote-alerts.json` (`0600`, next to the
passkey store); the history itself is rebuilt from the journal at each start.

**Acknowledge removes the entries.** A successful acknowledgement removes the acknowledged
entries from memory, from the page and from the API at once (the `200` body, `GET /api/alerts`
and the next `event: alerts` no longer contain them; records carry no acknowledgement flag).
After a service restart, an attempt replayed from the journal whose time is at or before the
stored marker is discarded and never shown again; only newer attempts appear. An entry that
grew after the page displayed it (a new attempt merged into it) is kept as a whole with its new
count, and a list that was just cleared may flash back for under a second when an event
serialised just before the acknowledgement arrives (the next event corrects it). The residuals
all err towards showing **more** alerts, never fewer: when an older attempt was still
unacknowledged (or a lock-screen check still pending) at the last marker write, the marker stays
below it and acknowledged attempts above the marker are shown again after a restart; when
`remote-alerts.json` cannot be written, the entries disappear at once but reappear after a
restart; when more than 32 entries arrived between the displayed view and the acknowledgement,
the evicted attempts stay counted in the totals (counts only) until the next acknowledgement.
Push notifications are not affected: an acknowledgement neither cancels nor resends one.

**The system journal is never modified: journal entries are never deleted.** `journald` cannot
delete a single entry, and the only removal tools (`journalctl --vacuum-*`, `--rotate`, deleting
journal files) are root-only, act on whole files and would destroy unrelated logs together with
the evidence of the very intrusion attempts the page reports. *Acknowledge* only stops the page
from showing what you have seen; the raw history stays available to root through `journalctl`.

**Limits.** Any process running as you can write log lines that look like lock-screen failures,
so it can create **false alerts** (and, while it keeps doing so, hide the 2nd and later
lock-screen attempts that only `unix_chkpwd` logs); root-side attempts (`sudo`, login, polkit)
and the first failure of each lock-screen prompt are still counted. Such a process already runs
as you. Section 8 lists the other residual limitations.

## 2d. Push notifications (off by default, on after `scripts/setup.sh`)

With `push_notifications = true` the phone is told about failed passwords on the PC **even
when the app is closed**: a standard Web Push notification to the phone web app: the
home-screen web app on iPhone (iOS/iPadOS 16.4 or later; Safari tabs cannot receive push) or a
supporting browser on Android (Chrome, Samsung Internet, Firefox; see section 2h). The push
service is Apple's (`web.push.apple.com`) on iPhone, Google's (`fcm.googleapis.com`) for Chrome
and Samsung Internet, and Mozilla's (`updates.push.services.mozilla.com`) for Firefox. It is the same information as
section 2c, sent at most once per burst: "3 wrong passwords — lock screen, your account".

**What a notification carries, and what never.** Only the source class (lock screen, sudo,
login, other), the account class (your account, root, another account), the kind (wrong
password, or attempt while locked out) and counts — **never the typed password**, nor any
part, length or hash of it (no journal line contains it, and soos never captures it). With
`push_previews = "generic"` the visible text is only "Security alert on your PC — Open soos for
details". A notification is readable on the **locked** iPhone unless you set *Settings →
Notifications → Show Previews → When Unlocked* (or choose `push_previews = "generic"`) (Android:
section 2h). The message is end-to-end encrypted (RFC 8291): the push service (Apple, Google or Mozilla) sees
only its size, time and the PC's public address, never its content. The page stays the source of truth: push delivery
is best effort.

**When.** Only attempts recorded live (not the 24 h replay at start): the first attempt of a
burst is sent 3 s later as one summary; then at most one notification every 30 s and 20 per
hour, the counts accumulating meanwhile (a failed delivery is retried twice, then its counts
are added to the next notification, so the counts you receive add up). Any process running as
you can forge lock-screen journal lines and therefore cause **false notifications** (bounded by
these limits), exactly like the false alerts of section 2c.

**Setup** (owner steps, nothing is enabled by `scripts/install_remote.sh`; `scripts/setup.sh` of section 4 does steps 1–3 for you):

1. Make section 2c work first (`password_alerts = true`) and set `rp_id` (section 2b).
2. In `remote.toml`:

   ```toml
   push_notifications = true
   # Optional: the VAPID contact (default https://<rp_id>). Apple refuses localhost and
   # reserved names with 403 BadJwtToken; a mailto: address of yours also works.
   # vapid_subject = "mailto:you@example.com"
   # Optional: "generic" hides source and account on the lock screen.
   # push_previews = "detailed"
   ```

3. Run `scripts/install_remote.sh` (it installs `soos-push-sender` and its unit
   `soos-push-sender.service`), then `systemctl --user enable --now soos-push-sender` and
   `systemctl --user restart soos-remote`.
4. On the iPhone, open the app **from the home-screen icon** (section 4); on Android, open the
   page in Chrome, Samsung Internet or Firefox, installed or in a tab (section 2h). Tap
   **Enable notifications** and allow them. The card then lists the device ("Apple device,
   enabled …" with an endpoint on `web.push.apple.com`, "Android / Chrome (Google) device" on
   `fcm.googleapis.com`, "Firefox (Mozilla) device" on `updates.push.services.mozilla.com`).
5. Tap **Send test notification**. Then lock the phone and type a wrong password at the PC's lock
   screen: one notification arrives within about 10 s.

**Devices.** At most 4 devices receive notifications. The card lists each one (push service and
date). Anyone signed in to the page (your tailnet identity, or a Funnel passkey session) can add
a device, so check the list after enabling and whenever a device is unknown:
`soos-remote push list` prints `<N>  <push host>  created <unix time>`,
`soos-remote push remove N` removes one, and `soos-remote passkeys remove N` ends the Funnel
sessions of a passkey that is not yours. When the browser renews a subscription, the service
worker names the endpoint it replaces (`replacesEndpoint`) and the PC swaps the two in one store
update, so a renewal never leaves a stale device in one of the 4 slots. Some browsers (Android
and iPhone alike) give the worker no old subscription, or the two endpoints together exceed the
2048-byte request limit: the stale device then stays until the push service reports it gone
(404/410) or `soos-remote push remove N`. `soos-remote push reset` replaces the push key and
removes every device (each phone must tap *Enable notifications* again); use it as well if the
page says the notification store was removed.

**How it works.** All keys and all cryptography stay in `soos-remote`: the VAPID key and the
subscriptions live in `remote-push.json` (`0600`, next to the passkey store, at most 4
subscriptions, never rewritten when invalid; only the service start and `push reset` create
it). For each notification `soos-remote` encrypts the payload with the phone's keys, signs a
VAPID token (ES256, 12 h) and hands the encrypted body over a `0600` Unix socket to
`soos-push-sender`. The sender is a separate user unit: the only part of the companion with
network access (`AF_INET`/`AF_INET6`), it holds no key, accepts only `https://` endpoints on
`web.push.apple.com`, `fcm.googleapis.com` and `updates.push.services.mozilla.com`, refuses
the whole request if any resolved address is private (loopback, LAN, tailnet `100.64.0.0/10`
and `fd7a:115c:a1e0::/48`, …), follows no redirect, uses no proxy, and gives up after 10 s.
Its sandbox hides your home directory (except its own binary), `/run` (except its socket
directory) and the system logs (`/var/log`, the journal included, and
`/var/lib/systemd/coredump`), which your `wheel`/`adm`/`systemd-journal` membership would
otherwise let it read, so it cannot read your files, `remote.sock`, the session bus,
`tailscaled` or the system journal. At start-up it checks that sandbox itself (a read-only
`/run`, a read-only tmpfs over the home directory with nothing mounted back on top of it except
its own binary, and `/var/log` not listable) and refuses to start otherwise (exit 78).
Its logging filter is fixed in code (no `RUST_LOG`): the HTTP and TLS libraries never log the
endpoint or the request. A 404/410 answer removes the device; other refusals are shown as
"The push service refused the last notification".

**Limits.** A compromised sender keeps outbound network access, can reach **abstract**
Unix sockets of the host (not files, so no sandbox directive hides them), and can forge
answers (at worst a wrong delivery state or a removed device). On a host whose
`/etc/resolv.conf` points into `/run/systemd/resolve/` the sender cannot resolve names inside
its sandbox and every notification fails; `scripts/install_remote.sh` prints a warning when it
detects this. The workaround is a user drop-in (`systemctl --user edit soos-push-sender`) that adds
`BindReadOnlyPaths=-/run/systemd/resolve`; the packaged unit keeps `/run` fully hidden. The sandbox and the delivery to the iPhone are
verified on the owner's hardware (matrix rows RMC74 and RSH18); on a host whose systemd is older
than the supported range, check the unit with `systemd-analyze --user security soos-push-sender`.

The sender refuses to start (exit 78, not restarted; `journalctl --user -u soos-push-sender`
shows "sandbox not detected") when it is run by hand, when a drop-in removes
`ProtectHome=tmpfs`, `TemporaryFileSystem=/run:ro` or `InaccessiblePaths=`, when a drop-in mounts
the home directory (or a path inside it other than the sender binary) back on top of the hidden
home (for example `BindPaths=%h`), when its binary is not at `~/.local/bin/soos-push-sender`, and
when the home directory lies outside the paths `ProtectHome=` covers: the push sender needs a
home directory under `/home` (or a path `ProtectHome=` covers); elsewhere it refuses to start,
because its sandbox could not hide it. It derives the home directory from its own binary path,
never from the environment. A mount table containing a mount point that is not valid UTF-8
also makes it refuse to start (fail closed). A bind added under `/run` by a drop-in (for example
`BindReadOnlyPaths=/run/dbus`) is **not** detected: the resolver drop-in below legitimately adds
one there.

Opt-in drop-in for such hosts only (check first with `readlink -f /etc/resolv.conf`; nothing
to do when it does not start with `/run/systemd/resolve/`):

```bash
mkdir -p ~/.config/systemd/user/soos-push-sender.service.d
cat > ~/.config/systemd/user/soos-push-sender.service.d/resolved.conf <<'EOF'
[Service]
# Expose only the systemd-resolved directory (stub resolv.conf and its local socket),
# read-only; the rest of /run stays hidden.
BindReadOnlyPaths=-/run/systemd/resolve
EOF
systemctl --user daemon-reload
systemctl --user restart soos-push-sender
```

The drop-in widens the sandbox by that one read-only directory; the endpoint allowlist and the
refusal of private addresses are unchanged (the stub resolver at `127.0.0.53` is only queried
for names, never used as a push destination). Remove the file and run `daemon-reload` to undo.

## 2e. Page design (soos brand)

The phone page follows the soos brand, which is the brand of the whole project (ADR "[2026-10-06]
soos Brand Direction Applied to the `soos-remote` Web App" in `AI/DECISIONS.md`):

| Color | Value | Use on the page |
|---|---|---|
| Brand blue (Pantone 2728 C) | `#0047BB` | header band, status tile, primary buttons, card borders |
| Brilliant White | `#EDF1FF` | light body and cards, text on blue |
| Ink (Pantone Black 6 C) | `#101820` | text in light mode, body in dark mode |
| Pink (Pantone 244 C) | `#E59BDC` | the star accent on the status and login tiles |

- **One set of tokens with the desktop GUI.** `style.css` declares the palette as CSS custom
  properties named after the constants of `crates/gui/src/theme.rs` (`--blue` = `BLUE`,
  `--pale-2` = `PALE_2`, `--danger-text` = `DANGER_TEXT`, ...), with the same values, plus the
  GUI radii and strokes (`--r-card` 20 px, `--r-table` 14 px, `--r-input` 10 px, `--stroke-card`
  2 px). Fourteen web-only dark tints (`--ink-2`, `--blue-soft`, ...) are integer-percentage mixes
  of two palette colors. Components use role tokens only (`--page`, `--card`, `--tile`,
  `--primary`, `--danger-fg`, ...); no other color literal exists in the stylesheet.
- **Light by default, dark through `prefers-color-scheme`.** The dark variant keeps the blue band
  and tiles and switches the body to ink `#101820` with ink cards and a softened blue border. No
  manual switch (it would need storage).
- **Layout.** A blue header band carries the SOOS wordmark (inline SVG, the owner's vector) and a
  static "Remote" pill; the pale body is cut out of the band with 20 px top corners. The lock
  state is a big blue tile with a pale caption strip, a state dot (decorative: the state is
  always written in words), the state in large white type and the pink star. Cards have a 2 px
  blue border; lists sit in pale inner tables; alerts use banners (info, danger when attempts wait
  for acknowledgement, warn for the lock-screen coverage note).
- **Buttons.** `Lock now` is the primary (blue) button. `Unlock now` is a danger outline button
  (it lowers protection; it stays behind the confirmation dialog and Face ID). Its text and
  border use `--danger-text` (light) or `--danger-soft` (dark) so it meets WCAG AA; the GUI
  outline uses `DANGER`, which is below AA on pale, and should follow later. Disabled buttons use
  a grey fill, never a faded copy of the enabled look.
- **Notifications card.** It has no on/off switch: the page can only know that the PC has at
  least one registered device, not that this phone receives alerts, so `#push-state` (text) stays
  the only statement of the push state.
- **Accessibility.** Text contrast at least 4.5:1 in both themes, a 3 px focus ring for keyboard
  focus, 48 px buttons and 44 px links, safe-area insets on all sides, reduced motion under
  `prefers-reduced-motion` (below), zoom kept.
- **Motion** (ADR "[2026-10-13] Motion System for soos-gui and soos-remote", normative document
  `Docs/UI_MOTION.md`; the motion itself lands with `feat/remote-motion`, after the owner approves the
  prototype `Docs/prototypes/ui_motion_prototype.html`). Motion only explains a state change: the
  state word re-enters, the tile and dot colours cross-fade, the pink star turns 45° on a spring when
  the PC is confirmed unlocked and back when it locks, buttons press and release on the spring, a
  pending star ticks inside `Lock now` / `Unlock now` while the request waits (at most the existing
  5 s confirmation window), revealed cards rise 6 px, banners slide 8 px, lists stagger on their first
  render only. The tokens (`--t-fast` 120 ms unchanged, `--t-short`, `--t-medium`, `--t-long`,
  `--t-spring`, exit tokens at 0.7 ×, `--ease-standard`, `--ease-enter`, `--ease-exit`,
  `--ease-spring`) mirror `crates/gui/src/theme.rs`. Only `transform` and `opacity` move, plus colour
  cross-fades between role tokens: still no gradient and no decorative shadow. The state text,
  `aria-live` regions, `.state-*` / `data-state` and `disabled` change first, on the same frame; the
  page never shows "Unlocked" before the PC confirms, and the unlock confirmation stays the native
  dialog plus the passkey. `app.js` only toggles classes (`is-pending`, `stagger-once`) and, for the
  camera full-screen overlay on phones without element fullscreen, calls `document.startViewTransition`
  when the browser has it. Under `prefers-reduced-motion` every animation and transform is removed,
  only opacity and colour fades of at most 120 ms remain, and `startViewTransition` is skipped.
  Engines without `linear()`, `@starting-style` or View Transitions (section 2i) get a
  `cubic-bezier` spring fallback or today's instant change; no polyfill.
- **Icons.** `icon.svg` (also the favicon) is the owner's star mark. `apple-touch-icon.png`
  (180 x 180, opaque RGB, deterministic bytes) is generated from it on a developer machine:

  ```bash
  rsvg-convert -w 180 -h 180 crates/remote/assets/icon.svg \
    | magick png:- -background '#EDF1FF' -alpha remove -alpha off -strip \
        -define png:color-type=2 -define png:exclude-chunks=date,time,tIME \
        png:crates/remote/assets/apple-touch-icon.png
  ```

  The manifest uses `theme_color` `#0047BB` and `background_color` `#EDF1FF`; the
  `theme-color` meta is `#0047BB` and the iOS status bar is `black-translucent` over the band.
- **Android icons** (ADR 2026-10-09 "Android Support for the `soos-remote` Phone Companion and
  Web Push", section 2h). The manifest also carries `id` `/` (equal to `start_url`, so a later
  `start_url` change never installs a second app) and three PNG icons: `icon-192.png` and
  `icon-512.png` (`purpose` `any`, full-bleed renders like the touch icon) and
  `icon-maskable-512.png` (`purpose` `maskable`). The maskable icon draws the 508-unit mark at
  408 px centred on the brand blue `#0047BB`: the blue parts of the tile merge with the
  background, so the visible mark is the two pale spikes, whose tips lie 204 px from the centre,
  inside the maskable safe zone (a circle of radius 40 %, 204.8 px of 512), and circular or
  squircle launcher masks never cut it. The notification badge `badge-96.png` is an alpha-only
  white silhouette of the pale spikes (blue becomes transparent): Android draws a badge from its
  alpha channel only. The four files are generated deterministically from `icon.svg` on a
  developer machine (rsvg-convert 2.63.2, ImageMagick 7.1.2; two runs give byte-identical files
  that carry only `IHDR`, `IDAT` and `IEND`):

  ```bash
  A=crates/remote/assets
  # Full-bleed "any" icons (opaque RGB), like apple-touch-icon.png.
  for n in 192 512; do
    rsvg-convert -w "$n" -h "$n" "$A/icon.svg" \
      | magick png:- -background '#EDF1FF' -alpha remove -alpha off -strip \
          -define png:color-type=2 -define png:exclude-chunks=date,time,tIME \
          "png:$A/icon-$n.png"
  done
  # Maskable: the 508-unit mark at 408 px, centred on the brand blue (spike tips 204 px from the
  # centre, inside the 204.8 px safe-zone circle of a 512 px icon).
  rsvg-convert -w 408 -h 408 "$A/icon.svg" \
    | magick -size 512x512 'xc:#0047BB' png:- -gravity center -composite -alpha off -strip \
        -define png:color-type=2 -define png:exclude-chunks=date,time,tIME \
        "png:$A/icon-maskable-512.png"
  # Badge: alpha-only white silhouette of the pale star spikes (blue -> transparent, pale -> opaque).
  rsvg-convert -w 96 -h 96 "$A/icon.svg" \
    | magick png:- -alpha off -colorspace Gray -level 30%,90% -background white -alpha shape -strip \
        -define png:color-type=6 -define png:exclude-chunks=date,time,tIME \
        "png:$A/badge-96.png"
  ```

  The generated files are `crates/remote/assets/icon-192.png`, `icon-512.png`,
  `icon-maskable-512.png` and `badge-96.png`; the 180 px `apple-touch-icon.png` stays for iOS.
- **Unchanged by the brand redesign.** The Content Security Policy, `app.js`, `sw.js`, every
  element id and label, and the routes (the later Android support of section 2h kept the CSP and
  every element id, but changed the login label, `app.js`, `sw.js` and added the four PNG
  routes). The page uses the system font stack (no font file, no CDN) and contains no raster
  from the brand archive. The later camera card (section 2f) is built by `app.js` at runtime with
  the same tokens and adds no element id.

## 2f. Live camera view (off by default, on after `scripts/setup.sh`)

The page can show the PC's camera live (ADR "[2026-10-07] Live Camera View in `soos-remote`
Through the Daemon Preview Channel", GitHub #345): to check who is in front of the PC, or that
nobody is. `soos-remote` never opens the camera itself. `soos-daemon` stays its only owner and
`soos-remote` asks it for its latest frame over `/run/soos/daemon.sock`
(`RequestKind::PreviewFrame`, the same channel `soos-gui` uses), so face unlock, presence
auto-unlock and the view share one capture.

**Double opt-in, off by default on both sides.** No frame reaches the phone unless **all** of
these hold; every unknown or failed check refuses:

- in `/etc/soos/daemon.toml` (root, `Docs/DAEMON.md` §1.5): `[preview] enabled = true`, your
  UID in `allowed_uids`, and `remote_view = true` (default `false`, the daemon-side opt-in for
  the companion);
- your UID owns an active **local seat session** at the PC (`CLASS=user`, a seat, `REMOTE=0`,
  active; a locked session counts). There is no view after a full logout, from an SSH-only
  login or under a lingering user manager (the same rule now applies to the `soos-gui`
  preview);
- the requesting process itself runs inside such a local seat session, or under your own user
  manager (`soos-remote.service`) while your UID owns **no remote session** (ADR 2026-10-12, the
  rule the root face unlock path already applies): while an SSH session of yours is open, or a
  process left running from one (for example `tmux`), the view is refused (`camera_refused`);
- in `remote.toml` (section 5): `camera_view = true` (requires `rp_id`); over Funnel also
  `camera_view_funnel = true` (requires `camera_view` and `allow_funnel`), otherwise the view
  is tailnet only and a Funnel caller gets `403 camera_tailnet_only`.

| `remote.toml` key | Default | Bounds |
|---|---|---|
| `camera_view` | `false` | TOML boolean; requires `rp_id` |
| `camera_view_funnel` | `false` | TOML boolean; requires `camera_view = true` and `allow_funnel = true` |
| `camera_max_view_s` | `120` | 10..=300 seconds per view |
| `camera_fps` | `5` | 1..=10 frames per second |
| `camera_width` | `640` | `640` (source size, at most 640x480) or `320` (2x downscale) |
| `camera_quality` | `70` | 50..=85 (JPEG quality) |

A value out of range (`0` included, never "unlimited") is refused at start-up (exit 78), never
clamped; the range keys are checked even while `camera_view = false`. `camera_view` does not
require `push_notifications`.

**Fresh passkey check per view.** Every view, on the tailnet as over Funnel, starts with a fresh
passkey assertion with user verification (Face ID, fingerprint or screen lock) over a new single-use challenge
of purpose `CameraView` (`POST /api/auth/camera/options`), distinct from the unlock challenge;
a web session, a cookie or an earlier assertion never starts a view. Failures count against the
same 5-failures lockout as unlock. A successful `POST /api/camera/start` reserves the view and
returns a single-use stream token (256 random bits, valid 10 s, bound to the caller's class and
Funnel session, carried only in the path, never logged); the page then opens
`GET /api/camera/stream/<token>` with `X-Soos-Action: camera-stream`.

**One view, bounded.** One view at a time for the whole PC (`409 view_in_progress`) and a
maximum duration of `camera_max_view_s`. There is **no cooldown** between views (removed at
the owner's request on 2026-10-07): right after a view ends (stop, maximum duration, error) a
new one can start at once, still behind its own fresh Face ID assertion and the shared
failure lockout. The view also ends on *Stop camera view*
(`POST /api/camera/stop`, any signed-in caller may stop), when the page is hidden or closed,
when the connection closes, when a part cannot be written within 2 s, when the Funnel web
session expires (checked every 5 s), when no new frame could be sent for 5 s, on a daemon
refusal, on an unsupported frame format and when the service stops.

**Transport.** The stream is served as `application/octet-stream` and carries
`soosframe`-delimited multipart parts: one baseline JPEG part per frame, each with its `Content-Length` (at most 512 KiB), at most `camera_fps` parts
per second, over the existing `tailscale serve` / `tailscale funnel` path. The page reads it
with `fetch` and a `ReadableStream` (bounded to 1 MiB of buffered bytes) and draws each frame
into a canvas; the Content Security Policy is unchanged (no `blob:` or object URL, no `<img>`
stream). It is deliberately not labelled `multipart/x-mixed-replace`: the iOS network stack
handles that type itself, splits the response and makes the page's `fetch` fail ("Load
failed", seen on the owner's iPhone on 2026-10-07). The `200` head is sent only once the first JPEG is ready (within 5 s); before that a
failure is a JSON error the page can explain (`camera_refused`, `camera_unavailable`,
`camera_format_unsupported`). The daemon converts MJPEG, NV12 and frames larger than 640x480
for the companion (ADR 2026-10-12: Grey, YUYV or RGB24 within 640x480, decimated by an integer
step, so a 640x512 sensor is shown at 320x256); `soos-remote` itself still contains no image
decoder.

**Full screen, portrait and landscape.** A tap on the live image toggles **full screen**: the image
fills the screen on the brand ink background, inside the iPhone safe areas, scaled to fit
without cropping. Where the browser offers element fullscreen (iPad, desktop browsers) the
page also requests it; iPhone Safari has none, so there the page uses a fixed full-viewport
overlay, which in the home-screen app covers the whole screen. Full screen is left by tapping
the image again, by the **✕** button ("Exit full screen"), by the Escape key or the browser's
own exit gesture, and always when the view ends (stop, maximum duration, error, page hidden or
left). One button switches between two modes, in the card as in full screen: **landscape**
(the default, the image as the camera captures it) and **portrait** (the image turned by 90°
and refitted to the box); the button names the mode it switches to. The mode is applied with
a CSS class only (the Content Security Policy is unchanged), is kept in memory for the life
of the page, so the next view starts in the same mode, and is never stored.

**Awareness.** The camera **LED** lights while the daemon captures, and stays on for about 10 s
after the last frame; it is the only indicator at the PC (no on-screen indicator). A view sends
**no push notification**, even with push configured (section 2d): the notification sent at
view start was removed at the owner's request (2026-10-07); push stays for the failed-password
alerts only. The journal records `camera view started`, `camera view ended` and `camera view refused` (fixed
text, at most one `refused` line per 5 s), and the daemon logs one `info` line per companion
connection on its first frame, without any pixel data.

**No recording.** There is **no recording**, no snapshot and no still image: frames and JPEG
buffers live only in memory, the buffers `soos-remote` owns (daemon reply, converted pixels,
JPEG output) are zeroized on drop, nothing is written to disk, logged or sent through Web Push.
The encoder's internal working buffers are not zeroized (accepted residual). The phone itself
can still screen-record what it shows.

**Interaction with face unlock.** A view holds one daemon connection of your UID for its whole
duration and uses at most `camera_fps` of the per-UID preview quota (`[preview]
max_requests_per_sec`, 40 by default, shared with `soos-gui`). The daemon admits at most
`max_connections_per_uid` (`[peer_limits]`, default 2) connections per unprivileged UID, plus one
**auth-reserved** connection (ADR 2026-10-12): with a view open **and** `soos-gui` open, a face
request of a lock screen running as your user still gets that reserved connection, which serves
only one `Auth` request whose whole first request arrives within 200 ms (anything else is closed
unanswered). PAM shows "Looking for face" through the lock screen's conversation between
connecting and sending its request, so a lock screen that takes 200 ms or more to display that
message misses the reserve and falls back to the password (fail-safe, never an unlock). Root PAM callers (`sudo`,
the display manager) are not limited this way. The daemon closes an idle connection after
`connection_timeout_ms` (`[dispatcher]`, default 2500 ms) and every connection after its
lifetime and request caps; the client sends a request at least once per second, reconnects on
its own before those caps (after 25 s or 1000 requests, closing the old connection first and
waiting at most 200 ms for it) and abandons a daemon exchange that takes longer than 3 s, so
keep `connection_timeout_ms` at its default or above `1000 / camera_fps` ms.

**Not a security boundary.** The daemon recognises the companion by the peer process's cgroup
(the `soos-remote.service` user unit) and refuses it unless `remote_view = true`. This is an
administrative opt-in and an audit aid, **not a security boundary** against code already
running as you: such code can read the GUI preview with `allowed_uids` alone, start a unit
named `soos-remote.service`, or run `soos-remote` outside its unit (classified as a local
client, which bypasses `remote_view`).

**Third-party notice.** The JPEG encoder is the pure-Rust crate `jpeg-encoder` (licence
`(MIT OR Apache-2.0) AND IJG`, allowed through a crate-scoped `deny.toml` exception); its IJG
terms require the following attribution, reproduced here because it comes through
`jpeg-encoder`:

> This software is based in part on the work of the Independent JPEG Group.

**Setup** (owner steps; agents and the installer never change `daemon.toml`):

1. Make passkeys work first (`rp_id`, section 2b).
2. As root, add to `/etc/soos/daemon.toml` (replace `1000` with your UID, `id -u`) and restart
   the daemon:

   ```toml
   [preview]
   enabled = true
   allowed_uids = [1000]
   remote_view = true
   ```

   ```sh
   sudo systemctl restart soos-daemon
   ```

3. In `remote.toml`, set `camera_view = true` (and, for Funnel, `camera_view_funnel = true`),
   then `systemctl --user restart soos-remote`. `scripts/install_remote.sh` writes these keys
   commented out and prints the `daemon.toml` snippet; it never writes it.
4. On the iPhone, open the app from the home-screen icon; the **Camera** card appears. Tap
   **Start camera view**, confirm, pass Face ID.

**Owner hardware check (matrix row RLC16).** With the setup above, from the iPhone home-screen
app on the tailnet:

1. a view starts after Face ID and shows live video within about 1 s, at about 5 fps, for up to
   120 s; the camera LED is on;
2. with `soos-gui` closed, face unlock at the PC's lock screen still works during the view;
3. no push notification arrives for the view, even when section 2d is set up;
4. *Stop camera view* ends the view at once; a new start right after it succeeds (after Face
   ID, no countdown, no waiting);
5. a tap on the image shows it full screen; tapping again or **✕** returns to the card;
   **Portrait** / **Landscape** switches between the two modes, in the card and in full screen;
6. after a full logout at the PC, a start is refused (`camera_refused`);
7. optionally, over Funnel with `camera_view_funnel = true`, steps 1 and 4 again (the frame
   rate over Funnel is bounded by Tailscale's relay throughput).

## 2g. Battery level

ADR 2026-10-07 "Live Battery Level in `soos-remote`" (GitHub #346). On by default;
`battery_status = false` in `remote.toml` turns it off completely: no sysfs access at all, the
view is `{"state":"disabled",…}` and no `event: battery` is ever sent.

- **Source.** `soos-remote` reads the kernel power-supply class directory
  `/sys/class/power_supply` itself (the user service already runs as the owner and these
  attributes are world-readable). No daemon, D-Bus, UPower or dependency change. Only ten
  attributes are ever opened: `type`, `scope`, `present`, `capacity`, `status`, `energy_now`,
  `energy_full`, `charge_now`, `charge_full`, `online`. Supply names, serial numbers, model and
  manufacturer are never read or sent; nothing is ever written.
- **Bounds.** At most 64 class entries and 8 system batteries (one more → `unavailable`), entry
  names of at most 64 bytes of `[A-Za-z0-9_.:-]`, regular files only (opened with
  `O_NOFOLLOW | O_NONBLOCK`, then `fstat`), at most 32 bytes per value. One whole read runs on the
  blocking pool under 500 ms (the wait for a concurrent read included); at most one read is in
  flight, and every concurrent caller (the sampler, `GET /api/battery`, the first frame of a
  stream) shares that one read (single-flight). A read stuck in a hung driver makes every later
  attempt `unavailable` at once until it returns; no second thread is started.
- **Cadence.** While at least one event stream is open, a sampler reads every 5 s; with no
  stream open there is no sysfs traffic at all. `GET /api/battery` reuses a reading at most 5 s
  old.
- **Page.** The battery line of the status card is fed only by `event: battery` on the existing
  `GET /api/events` stream (the page never fetches `/api/battery`): one frame right after the
  first `status` (and `alerts`) frame of every stream, then a frame only when the view changes.
  The line is hidden while the PC is unreachable, while the stream is down (it reappears with the
  first battery frame of the reopened stream) and while signed out. Texts: "Battery 82%, on
  battery", "Battery 41%, charging", "Battery 100%, full", "Battery 80%, plugged in, not
  charging", "Battery 64%, plugged in" (charge state unknown, mains online), "No battery",
  "Battery: unknown".
- **Aggregation.** One battery: its firmware `capacity` (never recomputed from energy). Several:
  energy-weighted when every battery reports a usable pair of the same kind (`energy_*` or
  `charge_*`), else the floor mean of the capacities, else unknown. Charge state: any charging →
  charging; else any discharging → discharging; else all full → full; else any not charging →
  not charging; else unknown. `external_power` is `true` when any non-battery system supply
  reports `online` 1 or 2, `false` when all report 0, unknown otherwise. `scope=Device`
  batteries (mice, headsets) and empty bays (`present=0`) are ignored.
- **`unavailable` and `no_battery`.** `no_battery` means a complete, readable scan found no
  system battery (a desktop). `unavailable` means the class directory could not be read
  completely (missing or unreadable directory, an invalid entry name or an unreadable `type`
  without any battery, a bound exceeded, a timeout, a stuck or failed read). A malformed value
  never becomes a guessed one: it is an unknown field.
- **Visibility.** Exactly that of `/api/status`: tailnet callers in `allowed_logins`, or Funnel
  callers with a web session; an anonymous Funnel request gets `403 login_required`, and a
  stream re-validates a Funnel session before every battery event.

```json
{"state":"present","percent":82,"charge":"discharging","external_power":false}
```

`state` is one of `present`, `no_battery`, `unavailable`, `disabled`; `percent` (0..=100),
`charge` (`charging`, `discharging`, `full`, `not_charging`, `unknown`) and `external_power` are
`null` unless `present` (and known). Matrix rows RBS1–RBS12; invariant `remote_battery_contract`.

## 2h. Android phones (Chrome, Samsung Internet, Firefox)

Android has full parity with the iPhone (ADR 2026-10-09 "Android Support for the `soos-remote`
Phone Companion and Web Push"): status, lock, unlock, alerts, push notifications, live camera
and battery level work the same way. Owner steps and caveats:

1. **Tailnet access.** Install the Tailscale app (Google Play or F-Droid), sign in to the same
   tailnet with a login listed in `allowed_logins`, and keep the VPN on for tailnet use. Funnel
   access (section 2b) needs no Tailscale app.
2. **Install the web app.** Open `https://<pc>.<tailnet>.ts.net` in Chrome, then menu →
   **Install app** (or **Add to Home screen**); in Samsung Internet, menu → **Add page to** →
   **Home screen**. The launcher uses the maskable icon (section 2e). If Chrome only offers a
   shortcut, it still opens the page standalone. Push works in a normal Chrome tab too: unlike
   iPhone, no installation is needed for notifications.
3. **Notification permission.** Tap **Enable notifications**; on Android 13 or later the system
   also asks whether Chrome may post notifications (if refused: Settings → Apps → Chrome →
   Notifications). The site permission is under Chrome → Site settings → Notifications.
4. **Lock-screen privacy.** Notifications are shown on the **locked** phone with their full text by
   default (`push_previews` defaults to `"detailed"`: source, account and count). To hide it:
   Settings → Notifications → **Notifications on lock screen** → *Hide sensitive notifications*
   (or *Show conversations, default and silent* without content; names vary by vendor: Samsung
   "Lock screen → Notifications → Hide content"), and per app: Settings → Apps → Chrome →
   Notifications → the site's channel → *Lock screen* → *Hide sensitive content*. Or set
   `push_previews = "generic"` in `remote.toml`: the text is then only "Security alert on your
   PC — Open soos for details" on every phone.
5. **Passkey.** The passkey is stored by **Google Password Manager** (or Samsung Pass) and
   unlocked with **fingerprint or screen lock**; a screen lock is required. It is synced to the
   Google account (backup flags set), the same accepted risk as iCloud Keychain (section 8).
   Enroll it over the tailnet with `soos-remote enroll-code` as in section 2b. **Unlock** and
   **camera start** go straight to the fingerprint/face/screen-lock prompt: the page names the
   registered passkeys for these two signed-in ceremonies (ADR 2026-10-12 "Named Credentials on
   Authenticated Passkey Ceremonies of `soos-remote`"). **Sign-in** over Funnel still uses the
   discoverable flow (no credential is named to an anonymous client), so Android may show a
   passkey or provider picker there. Both phone-stored and password-manager passkeys are accepted.
   - **Preferred passkey service.** If a provider picker still appears at unlock, the passkey
     sits in another provider. Android 14 or later: Settings → **Passwords, passkeys & accounts**
     (Samsung: Settings → General management → **Passwords, passkeys and autofill**) →
     **Preferred service** → **Google Password Manager** (or Samsung Pass); under *Additional
     services*, turn off the other providers if the picker keeps appearing. Android 13 and older
     store passkeys in Google Password Manager only.
   - **Move a soos passkey out of Proton Pass** (or another third-party manager) to the phone:
     (1) first, in Proton Pass, open the item of `<pc>.<tailnet>.ts.net` and delete its passkey;
     (2) on the PC, `soos-remote passkeys list`, then `soos-remote passkeys remove N` for the old
     passkey (or keep it until the new one works, then remove it); (3) set the preferred passkey
     service as above; (4) with the phone on the tailnet (Tailscale VPN on), run
     `soos-remote enroll-code` on the PC and enroll **on the phone itself**: registration asks
     for the phone's own (platform) authenticator, so a desktop browser or a cross-device QR code
     flow may be refused; (5) check that unlock shows the fingerprint prompt directly. If the
     phone says the passkey **already exists**, the old soos passkey is still present in an
     enabled provider (for example Proton Pass): delete it there (step 1), or turn that provider
     off, then enroll again.
6. **Battery optimisation.** Pushes use `Urgency: high` and reach a phone in Doze, but vendor
   battery savers can still delay or drop them: set Chrome (and Firefox if used) to
   **Unrestricted** battery usage (Samsung: "Never sleeping apps").
7. **Push services.** Chrome and Samsung Internet use `fcm.googleapis.com` (device list label
   "Android / Chrome (Google)"); Firefox uses `updates.push.services.mozilla.com` ("Firefox
   (Mozilla)"). Both are in the existing allowlist; Google or Mozilla see delivery metadata,
   never the content (section 2d). Notifications carry the soos icon and a white star badge in
   the status bar, and a second burst under the same tag alerts again (`renotify`). The worker
   makes no request on push: the browser itself loads the two same-origin images when it shows
   the notification, and still shows it without them when the PC is unreachable.
8. **Firefox caveats.** The home-screen install opens like a browser shortcut on some versions;
   passkey support depends on the Firefox and Android versions (if no passkey prompt appears, use
   Chrome); Firefox must not be battery restricted for its push connection. Firefox fires
   `pushsubscriptionchange`; the page re-sends the subscription on every open anyway.
9. **Subscription renewal.** When the browser renews a push subscription
   (`pushsubscriptionchange`), the service worker re-sends it once to the PC with the page's
   exact request shape (`POST /api/push/subscribe`, `X-Soos-Action: push-subscribe`, aborted
   after 10 s, never retried). The server checks are unchanged, so it fails silently when the PC
   refuses it (off-tailnet, an expired Funnel session, four devices already registered); opening
   the app re-sends it. The request names the replaced endpoint when the browser provides it, so
   the stale device is removed in the same update (section 2d). Over Funnel, this single request refreshes the idle timer of a still-valid
   web session (its 8 h absolute lifetime is unchanged).
10. **Shortcuts/Tasker.** Not integrated; pointer only: the `POST /api/lock` call of section 4 can
   be issued by any HTTP automation app (Tasker, HTTP Shortcuts) over the tailnet with the same
   `X-Soos-Action: lock` header. Unlock still needs the page.
11. **Owner hardware check (matrix row RAN12).** On an Android phone (Chrome; Samsung Internet or
    Firefox if available): (1) over the tailnet with the Tailscale app, Chrome **Install app**
    adds the soos icon (maskable, not clipped) and opens standalone with the blue status bar;
    (2) enroll a passkey with `soos-remote enroll-code`, sign in over Funnel with fingerprint or
    screen lock; (3) **Enable notifications** (Android 13+ system prompt accepted), the device
    list shows "Android / Chrome (Google) device"; (4) **Send test notification** arrives with
    the soos icon and the white star badge; two failed-password bursts under the same tag both
    sound or vibrate; (5) a failed password at the PC with the phone screen off and Chrome set to
    Unrestricted arrives within about 1 min; (5b) with the Tailscale VPN and Funnel off, a
    failed-password notification still appears (the icon may fall back to the browser default);
    (6) lock and unlock from the phone work; (7) the iPhone home-screen app still works as
    before; (8) **RDB10**: after re-enrolling a passkey stored on the phone (item 5), **unlock**
    and **camera start** show the fingerprint/face prompt directly with no provider picker, over
    the tailnet and over Funnel; sign-in may still show the picker.

## 2i. Browser support

Policy (ADR 2026-10-13 "Browser Support Policy for the `soos-remote` Web App", GitHub #366,
owner decisions of 2026-10-13 on #355):

- **Mandatory browsers**: Safari on iPhone, iPad and macOS, Chrome, Firefox and Samsung Internet.
  Every other browser is **best effort**: it either works or each affected feature says, in a
  fixed sentence, why it is off here. Nothing is left as a silent, stuck button.
- **Floor**: features that are *Baseline widely available*; there is no explicit Firefox ESR 115
  commitment (older engines get the CSS fallbacks below where they understand `@supports`).
- **Feature detection only**: one pure probe in `app.js` (`browserCapabilities()`) checks
  `PublicKeyCredential`, the service worker, `PushManager`, `Notification`, `EventSource`,
  streamed `fetch` bodies, `createImageBitmap` and the home-screen display mode. The page never
  reads the User-Agent (not even for wording) and the server never reads `User-Agent` either.
  The flags only hide or disable controls in the page: every server check (identity headers,
  CSRF, `Origin`, `Sec-Fetch-Site`, passkey user verification, web session) is the same for every
  browser, so a capability flag can never open a server path.
- **No polyfill, no CDN, no framework, no bundler**: the CSP and the eleven-file asset set are
  unchanged; per-feature messages only (no "this browser" panel).

What each feature does when the browser lacks it:

| Feature | Needs | Without it |
|---|---|---|
| Status, live updates, **Lock now** | `fetch`; `EventSource` for live updates | Without `EventSource` the page fetches the status every 15 s instead (no battery line). Lock never depends on passkeys. |
| Sign-in, **Unlock now**, *Add a passkey*, camera start | WebAuthn (`PublicKeyCredential`) | The buttons stay disabled with "This browser cannot use passkeys, so sign-in, unlock, passkey enrollment and the camera view are off here; lock and status still work on the tailnet". A cancelled or timed-out passkey check shows a fixed sentence, never the browser's own error text. |
| Notifications | service worker, `PushManager`, `Notification` | iPhone or iPad outside the home-screen app: "On iPhone and iPad, notifications only work in the home-screen app: tap Share, then Add to Home Screen, …"; no service worker (private window): "Notifications need a service worker, which this window does not allow …"; other browsers: the existing "this browser does not support them" sentence. |
| Notifications, at subscribe time | a working push service the PC accepts | Permission refused: "Notifications are blocked …"; no push service or subscribe refused (Brave with Google push off, Chromium builds without Google keys): "This browser has no working push service: in Brave, turn on Use Google services for push messaging; …"; an older subscription in the way: "reload the page, then enable notifications again"; the PC's `unsupported_push_service` (Edge on a computer, `*.notify.windows.com`): "The PC does not accept this browser's push service (Edge on a computer, for example) …", and the refused subscription is dropped again. The whole tap ends within 10 s of the permission answer. |
| Live camera view | streamed `fetch` (`ReadableStream`, `AbortController`), `createImageBitmap` | The start button is hidden: "This browser cannot show the live camera view (no streamed download or image decoding) …". |
| Styling | `:has()`, container query units, `dvh` | `@supports not selector(:has(*))` repeats the status dot colours and the feedback spacing on hooks set by `app.js`; `@supports not (container-type: size)` sizes the portrait camera canvas with percentages and viewport units; `100vh` precedes `100dvh`. |

Compatibility matrix. "OK (row)" is verified on hardware by that row; "Expected (row)" is the
design target, pending the owner's hardware row; "Message" means the feature is not possible
there and the page says why. Automated cross-engine runs (Playwright) are a separate follow-up.

| Browser | Page + live status + lock | Passkey sign-in / unlock | Notifications | Live camera |
|---|---|---|---|---|
| Safari, iPhone/iPad home-screen app (iOS 16.4+) | OK (RMC43, RMC88) | OK (RMC43, RMC44) | Expected (RMC74, RXB12) | Expected (RLC16) |
| Safari in a tab, or any other iOS browser (all WebKit) | OK (RMC43) | OK in Safari (RMC43); Expected elsewhere (RXB12) | Message: home-screen app needed (RXB12); Add to Home Screen also works from other iOS browsers on iOS 16.4+ | Expected (RXB12) |
| Safari macOS 13+ | Expected (RXB13) | Expected (RXB13) | Expected (Apple, RXB13) | Expected (RXB13) |
| Chrome desktop and Android | Expected (RXB14, RAN12) | Expected (RXB14, RDB10) | Expected (FCM, RXB14) | Expected (RXB14) |
| Firefox desktop | Expected (RXB15) | Expected; platform passkeys depend on the OS (RXB15) | Expected (Mozilla, RXB15) | Expected (RXB15) |
| Firefox Android | Expected (RXB16) | Expected (RXB16) | Expected (Mozilla, RXB16) | Expected (RXB16) |
| Samsung Internet | Expected (RXB17) | Expected (RXB17) | Expected (FCM, RXB17) | Expected (RXB17) |
| Brave desktop / Android | Best effort (RXB18) | Best effort (RXB18) | Message until *Use Google services for push messaging* is on (desktop, RXB18) | Best effort (RXB18) |
| Edge desktop | Best effort (RXB19) | Best effort (RXB19) | Message: push service not accepted, out of scope (section 9, RXB19) | Best effort (RXB19) |
| Edge Android, Opera, Vivaldi, DuckDuckGo, other WebViews | Best effort | Best effort (WebAuthn in a WebView is up to the app) | Best effort (expected FCM where available; else a message) | Best effort |
| Any private / incognito window | Expected (RXB20) | Expected with synced passkeys (RXB20) | Message (RXB20): blocked permission (Chrome) or no service worker (Firefox) | Expected (RXB20) |

Notes:

- **iPhone and iPad**: Web Push exists only in the home-screen app (iOS/iPadOS 16.4 or later).
  Every iOS browser is WebKit; since iOS 16.4 *Add to Home Screen* is also offered by third-party
  browsers. Safari and the home-screen app keep separate cookies (section 7).
- **Edge on a computer** subscribes through the Windows push service (`*.notify.windows.com`),
  which is not in the push allowlist: notifications are out of scope there (section 9); every
  other feature is best effort.
- **Brave** desktop needs *Settings → Privacy and security → Use Google services for push
  messaging*; without it the subscribe is refused and the page says so.
- **Private windows**: Chrome incognito refuses the notification permission, Firefox private
  windows have no service worker; status, lock, passkey sign-in (synced passkeys) and the camera
  view still work.
- **Browser secure DNS** (Firefox DNS-over-HTTPS, Chrome or Edge *Use secure DNS*) can bypass
  MagicDNS, so a tailnet-only `*.ts.net` name does not resolve and the page cannot load at all
  (section 7). The Funnel name is public and is not affected.

**Owner hardware checks** (matrix rows, pending; agents never install, enable or restart units):
RXB12 iPhone: Safari tab and one third-party iOS browser show the home-screen sentence for
notifications, the home-screen app subscribes; RXB13 Safari macOS: page, passkey, notifications,
camera; RXB14 Chrome desktop and Android; RXB15 Firefox desktop: notifications (Mozilla), the
private-window sentence, the secure-DNS note; RXB16 Firefox Android; RXB17 Samsung Internet;
RXB18 Brave: the push-service sentence, then a working subscription once the Google push setting
is on; RXB19 Edge desktop: page, passkey and camera work, *Enable notifications* shows the Edge
sentence and nothing stays subscribed (`soos-remote push list` unchanged); RXB20 a private
window (Chrome incognito and Firefox private): the matching push sentence while status and lock
keep working.

## 2j. Offline history and back-online notifications (off by default, on after `scripts/setup.sh`)

ADR 2026-10-13 "Offline History and Back-Online Push Notifications in `soos-remote`" (GitHub #376,
tier 1 of #359 Part B). The PC cannot report its own disappearance while it is offline, so this
feature reports **after the fact**: it records when the PC was offline and, once the PC is online
again, sends one notification to the phone. A real-time "PC stopped responding" alert needs an
observer outside the PC (tier 2 watchdog and tier 3 Android heartbeat, later PRs).

**Enable** in `remote.toml` (the one-command setup of #362 will write both keys):

```toml
push_notifications = true   # required; the service refuses to start (exit 78) otherwise
offline_alerts = true
```

Turning push notifications off later means turning `offline_alerts` off as well.

**What is recorded**: at most 32 intervals (the oldest is dropped), in `remote-network.json` next
to the passkey store (`0600`, at most 8 KiB, `O_NOFOLLOW`, owner-checked, written under `flock`
through a temp file, `fsync` and `rename`). Each interval has a start and an end time, a duration,
and one of four reasons:

| Reason | Meaning | Notification |
|---|---|---|
| `network_lost` | NetworkManager reported no usable connection (state disconnected, connecting or asleep, or connectivity `NONE`) | yes |
| `wifi_disabled` | the same while the Wi-Fi radio was switched off (`WirelessEnabled = false`) | yes |
| `sleep` | the PC was suspended or hibernated | no (recorded only) |
| `shutdown_or_crash` | the previous run ended in a system shutdown, a crash or a power loss | yes |

A captive portal (`PORTAL`) or limited connectivity (`LIMITED`) counts as online. Intervals
shorter than 30 s are not recorded (Wi-Fi roaming, DHCP renewal, the reconnection right after a
resume).

**How it watches, read-only**: every 5 s the service asks the D-Bus bus driver whether
NetworkManager is running (`NameHasOwner`, which never starts a stopped service) and, if it is,
reads three properties of the NetworkManager manager object with `Properties.Get`: `State`,
`Connectivity` and `WirelessEnabled`. It installs no signal match, calls no NetworkManager method
that changes anything, and reads no connection, access-point or device object. A suspend shows as
the gap between `CLOCK_BOOTTIME` (which keeps counting during a suspend) and `CLOCK_MONOTONIC`
(which stops). Durations within one boot come from `CLOCK_BOOTTIME`, so a clock correction after
reconnecting does not change them. On a clean stop the service reads logind's
`PreparingForShutdown` once:

- `false` (a logout or `systemctl --user restart`) writes the marker `stopped`, so the next start
  records nothing.
- `true`, or a failed read, writes `shutdown`.

A run that ended without any marker (crash or power loss) is found at the next start through its
last heartbeat (written at most once a minute). In both cases the next start records one
`shutdown_or_crash` interval. Hosts without NetworkManager (iwd-only, systemd-networkd) record
`sleep` and `shutdown_or_crash` only; the page says so.

**The notification** goes through the existing sender on topic `soosonline`, so it never replaces
an undelivered failed-password alert. It shares the alert budget: at most one notification per
30 s and 20 per hour across both kinds. It is sent only once NetworkManager reports the PC online
again, or after 60 s without any NetworkManager reading. The text uses a fixed vocabulary, for
example "Your PC is back online — Offline for 12 min (network lost)" or "Offline 3 times, latest
for 2 h 5 min (Wi-Fi turned off)". With `push_previews = "generic"` the lock screen shows only
"Update from your PC — Open soos for details". The clock times stay inside the encrypted payload,
and the page shows them in the phone's time zone. A notification whose delivery keeps failing is
retried after 5 s, 30 s, 2 min and 10 min, then dropped; the history keeps the interval.

**Privacy**: no SSID, BSSID, IP address, MAC address or location is ever read, stored, logged,
sent or shown. Only reasons and times are kept. A "network changed" notification is not provided,
because it would need connection objects whose properties include the network name.

**Page**: the "Offline history" card (`GET /api/offline`, the visibility of `/api/alerts`)
lists the 10 newest intervals with start, end, duration and reason.

**Owner hardware checks** (matrix rows ROA12–ROA14):

- ROA12: turn Wi-Fi off for more than a minute, then on again. The phone receives one back-online
  notification, and the card shows a `wifi_disabled` row.
- ROA13: suspend the PC, then resume it. The card shows a `sleep` row, and no notification
  arrives.
- ROA14: hard power-off, then boot. At the next start, a `shutdown_or_crash` row appears and one
  notification arrives.

## 2j. Native Android app (Kotlin, first version)

A native Android app (ADR 2026-10-13 "Native Android App for `soos-remote` (Kotlin and Jetpack
Compose)", GitHub #382, first PR of #357) is an alternative to the web app: each user chooses.
This first version shows the PC status, locks the PC, signs in over Funnel with a passkey and,
when turned on, alerts you when the PC stops answering (PC offline alerts, below); the second
(GitHub #404, PR 2 of #357) adds the failed-password alerts with acknowledgement.
Unlock, the live camera view, push notifications, the battery level and the offline history stay
in the web app for now. The app is sideloaded only (no store) and its application id is
`org.soos.remote` (owner to confirm).

**What the app does.**

- **PC address.** The settings screen accepts only `https://<pc>.<tailnet>.ts.net`, the same
  rule as `rp_id`: HTTPS, no port, no path, no user name.
- **Status and lock.** Over the tailnet (Tailscale app on, a login in `allowed_logins`), the app
  reads `GET /api/status` and calls `POST /api/lock` like the web page. Each request carries
  `Origin: https://<rp_id>` and the route's `X-Soos-Action`.
- **Failed-password alerts** (section 2c; same server, no server change). While signed in (a
  tailnet identity or the Funnel web session), the home screen shows a *Password alerts* card.
  The app reads `GET /api/alerts` when the screen opens, on *Refresh* and on pull to refresh
  (swipe down). It shows one fixed sentence per state (`disabled`, `starting`, `active`,
  `unavailable` with a fixed reason), the unacknowledged counts, the last attempt and the history
  (time, source, account class, count). Times are rendered in the phone's time zone and locale.
  *Acknowledge* sends `POST /api/alerts/ack` with no body, `X-Soos-Action: alerts-ack`, exactly one
  `X-Soos-Alerts-Epoch` and one `X-Soos-Alerts-Through` header naming the displayed view, and
  `Origin`, like the web page. The `200` answer is the fresh view. On `409 stale_view` (the service
  restarted) the app shows the fresh view and asks you to acknowledge again; it never acknowledges
  a view you have not seen. The parser is strict and bounded: an unknown state or class, a wrong
  type, more than 32 entries or a body over 16 KiB is treated as unreadable ("Alerts could not be
  loaded"). Every text on the card is a fixed string of the app: no server text, status number or
  exception text is ever displayed.
  Every refresh clears the acknowledgement message (GitHub #420): opening the screen, *Refresh*,
  pull to refresh, sign-in, sign-out and *Lock PC* all reload the view without the previous
  message, so "Acknowledged." never stays under a newer view. A refused acknowledgement keeps its
  fixed text under the fresh view it loads (for example the `stale_view` text). If the
  acknowledgement is refused with `login_required` and the refresh finds the session signed out,
  the alerts card is hidden and the screen message says "Your session ended. Sign in again."
  The *Acknowledge* button follows the web app's rule: it is shown while counts are pending and
  the view has an epoch, whatever the state (a `starting` or `unavailable` state still
  acknowledges; without any data the view has no epoch and no count). The decision is the pure
  helper `alerts/AlertsFeedback.kt`, covered by JVM unit tests (matrix AAF1–AAF4).
- **Sign-in over Funnel.** The app uses Android Credential Manager (Android 14 or later, no
  Google Play services library in the app). It runs the existing login ceremony: login options,
  then the passkey prompt with user verification, then login verify. The server still uses the
  discoverable flow and never names a passkey to an anonymous client. The app remembers the id
  of the passkey from its last successful sign-in and names it on the next one, so Android goes
  straight to the fingerprint, face or screen-lock prompt. Google Password Manager is the
  provider to validate first; Proton Pass and other providers are best effort.
- **Session.** The session cookie is kept in memory only and lost when the app process ends.
  The app never logs anything. Its storage is excluded from backups and device transfer.

**Server binding (opt-in, needed for the Funnel sign-in).** A passkey used by a native app
carries the origin `android:apk-key-hash:<base64url SHA-256 of the signing certificate>`.
Android also requires the site to vouch for the app at `/.well-known/assetlinks.json`. Add this
table at the **end of `remote.toml`**: TOML puts every key written after a table header inside
that table, and an unknown key there is refused.

```toml
[android]
package = "org.soos.remote"
sha256_cert_fingerprints = ["AA:BB:...:FF"]   # 1 or 2 entries, upper-case, 32 pairs
```

The table requires `rp_id` and `allow_funnel = true`. The login is Funnel-only, and Google's
Digital Asset Links check needs the public Funnel host. With the table, `soos-remote` does two
more things:

- It serves `GET|HEAD /.well-known/assetlinks.json`: one statement with the single relation
  `delegate_permission/common.get_login_creds`, reachable anonymously like the page's assets.
- It accepts the app origin for the **login** ceremony only. Unlock, the camera view and
  passkey registration still require the web origin.

Without the table nothing changes: the path answers `404`, exactly like any unknown path. An
invalid table stops the service at start-up with exit status 78. Nothing about the table is
logged.

**Signing key (accepted risk).** The key whose fingerprint is listed becomes a login trust
anchor. Whoever holds that private key, and gets a matching APK onto a phone where you then pass
the passkey prompt, receives a Funnel web session: status, events, lock, alerts, push and
battery, but no unlock and no camera.

- Use a **dedicated** key. Never use the default `~/.android/debug.keystore` (its password is
  `android`) or any CI key. Create one with
  `keytool -genkeypair -keystore ~/soos-android.jks -alias soos -keyalg EC -groupname secp256r1 -validity 3650`.
- Keep the key outside the repository. `*.jks`, `*.keystore`, `keystore.properties` and
  `local.properties` are git-ignored and rejected by `scripts/secret_scan.sh`.
- Read the fingerprint with `keytool -list -v -keystore ~/soos-android.jks -alias soos`
  (the `SHA256:` line is already in the `AA:BB:..` form), or with
  `apksigner verify --print-certs app.apk`, then upper-case it and add a `:` between each pair.
- **Key rotation:** list the new fingerprint next to the old one (two entries), install the
  newly signed app, then remove the old entry.
- **Switch the binding off:** remove the table and restart the unit. Passkeys are bound to
  `rp_id`, not to the app, so they keep working in the browser.

**Build (owner).** Use `scripts/android_build.sh` on a machine with a JDK 17 and the Android
SDK (`ANDROID_HOME`). It downloads Gradle 8.14.5, checks its SHA-256 and runs the debug build,
the unit tests and lint with strict dependency verification. Gradle signs
`android/app/build/outputs/apk/debug/app-debug.apk` with the default debug key. Re-sign it with
your dedicated key before you install it:
`apksigner sign --ks ~/soos-android.jks --ks-key-alias soos app-debug.apk` (`apksigner` is in the
SDK build tools). Release signing is a later PR. The debug APK is `debuggable`: with USB debugging
enabled, `adb` can inspect it. Install it with `adb install` or a file transfer,
grant "Install unknown apps" to the file manager only, and revoke it afterwards.

**CI and dependency verification.** The `Android (debug build)` job
(`.github/workflows/android.yml`) runs only when `android/`, its build script or the workflow
change. It is not part of `CI Success`, but `scripts/pr_loop.sh` blocks a pull request that
touches those paths while the job fails. The first run happens while
`android/gradle/verification-metadata.xml` lists no component:

1. The run generates the SHA-256 metadata, uploads it as the artifact
   `android-verification-metadata`, and **fails**.
2. Review the file (every dependency comes from Google Maven, Maven Central or the Gradle Plugin
   Portal) and commit it on the branch (trust on first use).
3. Re-run the candid review for the new head. Never push with `--no-verify`.
4. Note the run id in the pull request.

After that, every run is strict. The first bootstrap run was 38446125267 and the first strict
green run of the foundation pull request (#391) was 38772439975. In `ci` mode the script then
checks the APK permissions with `aapt2 dump permissions`: every request line, in the
`uses-permission: name='…'` form and in the `uses-permission-sdk-23: name='…'` form (a permission
requested only on Android 6 and later), must name an allowlisted permission, and any other
`uses-permission*` line fails the job (GitHub #398). To regenerate after a dependency change, empty the component
list again, or run `scripts/android_build.sh bootstrap` locally. The file is generated on Linux
x86_64, so platform-specific entries for other systems may be missing.

**PC offline alerts (opt-in, GitHub #415, part of #359).** The app can check your PC in the
background and notify you when it stops answering. It is off by default. Turn on
**PC offline alerts** in the settings screen (shown once a PC address is saved). Android then
asks for the notification permission (`POST_NOTIFICATIONS`); this is the only time the app asks
for it, and the alerts stay off if you refuse.

- **Checks.** Every 15 minutes (the Android minimum; two checks are 10 to 20 minutes apart),
  one `GET /api/status` on the saved address, with the same HTTPS rules as the screens (no
  redirect, 5 s timeouts, 16 KiB, system certificates). No other request is made. Any answer
  from `soos-remote`, including a refusal, counts as reachable; only no answer counts as a
  failure. In airplane mode the check is skipped.
- **Notifications.** "PC went offline" once after two checks in a row fail, then
  "PC back online" once when the PC answers again. The texts are fixed English and never name
  the PC, its address or any status value. Tapping one opens the app.
- **Sign-in.** Over Funnel the check uses the in-memory web session of the app while the app
  process is alive; the session cookie is never written to storage. When no session exists,
  `soos-remote` answers `403 login_required`: the PC is reachable and the offline detection
  keeps working. The app shows "Sign-in needed" once after you turn the alerts on (again after
  you turn them off and on, or change the PC address), never after every app run. It does not
  retry faster.
- **Reboot.** The job is persisted (`RECEIVE_BOOT_COMPLETED`), so it survives a phone reboot.
  The app declares no broadcast receiver; the system scheduler alone may start the job, which
  is not exported (`BIND_JOB_SERVICE`).
- **Limits (best effort).** Doze and App Standby defer the checks while the phone is idle, so
  an alert can come late. A force stop in the Android settings cancels the job until you open
  the app again, which schedules it again. If you block notifications during an outage and
  allow them again later, that outage is not reported. Battery Saver, Data Saver or the
  restricted battery setting can block the app's background network, so checks fail and a
  false "PC went offline" can follow: set the app's battery use to unrestricted. With the
  Tailscale VPN off, or with no network outside airplane mode, the PC is unreachable from the
  phone and an alert follows. The ADR amendment of 2026-10-17 explains
  why no network constraint is used.
- **Turn it off** with the same switch: the job is cancelled and the stored count is cleared.
  Nothing is logged, and the stored state is only the switch, a count and two flags.

**Owner hardware check (matrix ANA14–ANA16, AHB8).**

1. Install the debug APK signed with the dedicated key. The launcher shows the adaptive soos icon
   (themed icons OK), with blue system bars.
2. Over the tailnet, status and lock work.
3. With `[android]` configured and Funnel on, sign in from the app with fingerprint or screen
   lock (record the passkey provider used). The second sign-in goes straight to the biometric
   prompt.
4. Over Funnel, status and lock work after sign-in.
5. The web app and the iPhone home-screen app still work.
6. PC offline alerts (AHB8): turn them on and allow notifications. Disconnect the PC from the
   network for more than 30 minutes: exactly one "PC went offline" arrives. Reconnect it:
   exactly one "PC back online" arrives. Reboot the phone with the alerts on and repeat the
   disconnection: the alert still arrives. Leave the phone idle for more than 30 minutes with
   the PC up, once normally and once with Battery Saver on: no alert arrives (record any false
   alert; it would argue for a network constraint, see the ADR amendment). Turn the alerts off: no further notification.

**Owner hardware check of the alerts (matrix ANA21).** With `password_alerts = true`, type a wrong
password at the lock screen or in `sudo`, then:

1. Over the tailnet, open the app: the *Password alerts* card shows the count, the last attempt
   and the history, with the times of your phone's time zone. Pull down to refresh after a new
   attempt.
2. Tap *Acknowledge*: the card says "Acknowledged." and the entries disappear, in the app and in
   the web page.
3. Repeat over Funnel after a passkey sign-in.
4. Restart `soos-remote` between reading and acknowledging: the app reports the stale view, shows
   the fresh one, and the next *Acknowledge* works.
5. Acknowledgement messages (matrix AAF4): after "Acknowledged.", tap *Refresh* (and, separately,
   pull to refresh): the message disappears. Over Funnel, with the PC offline alerts off, let the
   app show pending counts, then leave it untouched for more than 15 minutes so the web session
   expires (or restart `soos-remote`), and tap *Acknowledge*: the alerts card disappears and the
   screen says "Your session ended. Sign in again."

## 2k. Feature tour (phone web app)

ADR 2026-10-17 "Phone Onboarding Tour in the `soos-remote` Web App and Its Single Storage
Exception" (GitHub #414, PR 2 of #363; the `soos-gui` tour is PR 1, `Docs/GUI_APPLICATION.md`).
A short tour explains every feature of the phone page, what it exposes and where to turn it off.

- **When.** It opens once on its own: on the first visit over the tailnet, or right after the
  first passkey sign-in over Funnel (never over the sign-in screen). **Feature tour** at the
  bottom of the page reopens it at any time. **Skip tour**, **Done** and Escape close it.
- **Steps.** An introduction, then one step per feature: Phone companion, Battery level, Remote
  lock and unlock, Internet access, Failed-password history, Push alerts, Live camera view,
  Offline alerts. The six features the PC tour also covers keep its exact titles, explanations
  and "Off:" lines (`crates/gui/src/onboarding.rs`). Each step says where to turn the feature
  off: **Features** in `soos-gui` where it has a switch, otherwise the exact `remote.toml` key,
  always with the keys that depend on it (for example `push_notifications = false` together
  with `offline_alerts = false`, `allow_funnel = false` together with `camera_view_funnel =
  false` and no `[android]` table), because `soos-remote` refuses a file that leaves a dependent
  key on (exit 78). **Show it on this page** moves to the matching card when it is shown (the
  status card, Sign out, the failed-password card, Notifications, Camera, Offline history).
- **What it never does.** The tour switches nothing on its own, makes no request and shows fixed
  English text only: no server data (alert counts, devices, battery level, camera state) and no
  user data. The card and the footer button are built by `app.js` (no new markup id, no inline
  style, CSP unchanged). Motion: the card enters with the `Docs/UI_MOTION.md` W4 entry
  (`--t-medium`, `--ease-enter`, `--rise`) where `@starting-style` exists, and at once otherwise
  or with reduced motion.
- **What the browser stores.** The only storage of the web app, a narrow exception approved by
  the owner on 2026-10-17 (#363): one `localStorage` key `soos-remote-tour-v1` whose only value
  is the fixed word `seen`, written when the tour is closed. It holds nothing personal and is
  read and written only by one guarded helper in `app.js`; clearing the site data of the page
  shows the tour again. When the browser blocks site data (some private windows, "block all
  cookies"), the tour simply opens on every visit and closes as usual, without an error.
- **Native Android app.** The same tour in the native Android app (section 2j) is a follow-up
  of #357; the app has no tour yet.

Matrix rows PTR1–PTR11; invariant `remote_tour_contract`.

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

**One command.** `./scripts/setup.sh --tailscale-login you@example.com --rp-id <this-pc>.<tailnet>.ts.net`
(run as yourself, not with `sudo`) installs soos and then this companion with every feature on
(Funnel, remote unlock, camera view also over Funnel, password alerts, push, offline alerts; owner
decisions of 2026-10-12 and 2026-10-15, GitHub #400): it writes a setup-managed `remote.toml`
(mode 0600; a key you turned off later, for example in the soos-gui feature tour, stays off),
runs `scripts/install_remote.sh`, validates the file with `soos-remote passkeys list`, enables
and starts the user units, creates the daemon `[preview]` section when `/etc/soos/daemon.toml`
does not exist yet, and prints the steps below that need your Tailscale account (it runs no
`tailscale` command except `tailscale version`). Details: `Docs/PACKAGING_AND_PROVISIONING.md`
§3.0. The manual procedure follows.

```bash
scripts/install_remote.sh
```

The binaries are copied from the cargo target directory the build used: `CARGO_TARGET_DIR`
when set (relative to the checkout when relative), else `target/` of the checkout.

The script refuses to run as root and never calls `sudo`. It builds `soos-remote` (release,
`--locked`), installs the binary to `~/.local/bin/soos-remote`, the user unit
`packaging/soos-remote.service` to `${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/soos-remote.service`,
writes a commented configuration template to `${XDG_CONFIG_HOME:-$HOME/.config}/soos/remote.toml`
(mode `0600`, **only when absent**, with `allowed_logins = []` so the service refuses to start
until you fill it in; it ends with a commented `# battery_status = true` line, section 2g),
and runs `systemctl --user daemon-reload`. It refuses to write through a `remote.toml` that is a
symbolic link and creates the template without ever overwriting a file. It never enables the
unit and never runs `tailscale`; those are your steps (the tailnet path relies on the same header
stripping as Funnel, so Tailscale 1.102.4 or later applies to `tailscale serve` too; check with
`tailscale version`):

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
4. Open `https://<this-pc>.<tailnet>.ts.net` on the phone (the phone must be logged in to the
   same tailnet) and add it to the home screen (iPhone: Safari → Share → **Add to Home Screen**
   → Add; Android: section 2h). The page then opens as a standalone web app with its own icon; it reconnects every
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
Screen*. The icon opens the page full screen (live status and the *Lock now* button). iOS caches
the home-screen icon: after an icon change (such as the soos brand star of section 2e), remove
the web app from the home screen and add it again to see the new icon.

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
| `password_alerts` | no | `false` | TOML boolean; `true` follows the system journal and shows failed password attempts (section 2c) |
| `lock_screen_programs` | no | `["/usr/bin/swaylock", "/usr/bin/hyprlock", "/usr/bin/gtklock", "/usr/bin/waylock"]` | 0..=4 absolute paths (at most 4096 bytes each, no trailing `/`, no `..`, no control byte, not ending in ` (deleted)`), duplicates removed; the lock-screen programs whose `pam_unix` lines are trusted (section 2c) |
| `push_notifications` | no | `false` | TOML boolean; `true` sends failed-password alerts as Web Push notifications (section 2d); requires `password_alerts = true` and `rp_id` |
| `vapid_subject` | no | `https://<rp_id>` | `mailto:<you>@<domain>` or `https://<host>[/path]`, at most 256 printable bytes, no space, no port, no reserved name (`localhost`, `.local`, `.test`, `.example`, `.invalid`, `.internal`, `.home.arpa`) |
| `push_socket_path` | no | `$XDG_RUNTIME_DIR/soos-push/push.sock` | absolute, at most 107 bytes; the socket of `soos-push-sender` |
| `push_previews` | no | `"detailed"` | `"detailed"` (source, account, counts) or `"generic"` (fixed text on the lock screen) |
| `camera_view` | no | `false` | TOML boolean; `true` enables the live camera view (section 2f); requires `rp_id`, and the daemon must allow it (`[preview] enabled`, `allowed_uids`, `remote_view`) |
| `camera_view_funnel` | no | `false` | TOML boolean; `true` also allows the view over Funnel; requires `camera_view = true` and `allow_funnel = true` |
| `camera_max_view_s` | no | `120` | 10..=300; maximum duration of one view in seconds |
| `camera_fps` | no | `5` | 1..=10; frames per second sent to the phone |
| `camera_width` | no | `640` | `640` (source size, at most 640x480) or `320` (2x downscale); any other value → refused |
| `camera_quality` | no | `70` | 50..=85; JPEG quality |
| `offline_alerts` | no | `false` | TOML boolean; `true` records offline intervals and sends a back-online notification (section 2j); requires `push_notifications = true` (exit 78 otherwise) |
| `battery_status` | no | `true` | TOML boolean; false disables every sysfs read (no battery line, `GET /api/battery` answers `disabled`, section 2g); any other type → refused |
| `[android]` | no | absent (no binding) | table at the end of the file: `package` (Android application id, at most 128 bytes) and `sha256_cert_fingerprints` (1 or 2 upper-case `AA:BB:..` SHA-256 digests); requires `rp_id` and `allow_funnel = true`; serves `/.well-known/assetlinks.json` and accepts the app origin for the login ceremony (section 2j) |
| `socket_path` | no | `$XDG_RUNTIME_DIR/soos-remote/remote.sock` | absolute, at most 107 bytes; `XDG_RUNTIME_DIR` unset or relative without `socket_path` → refused (no `/tmp` fallback) |

A missing file, an unknown key or an invalid value exits with status 78 (`EX_CONFIG`), which the
unit does not retry (`RestartPreventExitStatus=78`); runtime failures exit 1 and are restarted.
The service also refuses to start when its real or effective uid is 0.

`soos-gui` can turn `allow_unlock`, `allow_funnel`, `push_notifications` and `camera_view` **off**
from its feature tour (`Docs/GUI_APPLICATION.md` §1e, GitHub #379): it rewrites only the
`key = true` line to `key = false` (and `camera_view_funnel` with `allow_funnel` or
`camera_view`), verifies the result with a TOML parse, writes it atomically with mode 0600 and
runs `systemctl --user try-restart soos-remote.service`. Turning a key back on is shown as the
exact edit, because each widens exposure and has prerequisites only the service validates.

## 6. HTTP surface

| Method + path | Response |
|---|---|
| `GET /`, `/index.html`, `/app.js`, `/style.css`, `/manifest.webmanifest`, `/icon.svg`, `/apple-touch-icon.png`, `/icon-192.png`, `/icon-512.png`, `/icon-maskable-512.png`, `/badge-96.png` | the embedded page (strict CSP `default-src 'self'`, no inline script, no CDN) |
| `GET /api/status` | `200 application/json` `{state, active, idle, idle_since_unix_s, checked_unix_ms}`; one fresh logind read |
| `GET /api/events` | `200 text/event-stream`; first event = the stream's own fresh read; then an event on every change and a keep-alive at least every 15 s; `503` beyond 4 streams; closed after 30 min (the browser reconnects). With the battery level on, an `event: battery` follows the first status (and alerts) event, then one on every change of the battery view (section 2g) |
| `POST /api/lock` | `202 {"result":"lock_requested"}`, `409 no_session` / `already_locked`, `429 rate_limited` (one lock per 2 s), `503 unavailable` |
| `POST /api/unlock` | JSON assertion body (section 2a). `403 unlock_disabled` (default), `403 passkeys_not_configured`, `403 passkey_required` (no body), `403 passkey_rejected`, `400 bad_request`, `413 body_too_large`, `202 {"result":"unlock_requested"}`, `409 no_session` / `already_unlocked`, `429 rate_limited` (one unlock per 2 s, independent of the lock; or the failure lockout), `503 unavailable` / `store_unavailable` |
| `GET /api/auth/state` | `{"mode":"tailnet"\|"funnel","authenticated",…}`; an anonymous Funnel caller gets only `{"mode":"funnel","authenticated":false}` |
| `POST /api/auth/login/options`, `POST /api/auth/login/verify` | Funnel only: passkey login (`200 logged_in` + `Set-Cookie`), `403 passkey_rejected`, `409 no_passkey`, `429 rate_limited` / `too_many_sessions`, `503 busy` |
| `POST /api/auth/logout` | Funnel only: `200 logged_out`, clears the cookie |
| `POST /api/auth/unlock/options` | tailnet, or Funnel with a session: an `unlock` challenge (`200 {allow_credentials, challenge, rp_id, timeout_ms}`; `allow_credentials` = every registered passkey as `{"id": <base64url>, "transports": [..]}`, store order, the stored transports or `["internal", "hybrid"]`). The login options stay `{challenge, rp_id, timeout_ms}` (no credential id) |
| `POST /api/auth/register/options`, `POST /api/auth/register/verify` | tailnet only, with the code of `soos-remote enroll-code`; the verify body may carry the optional `transports` of `getTransports()` (at most 8 entries of 16 bytes, else `400 bad_request`; only the six WebAuthn names are stored): `200 registered`, `403 enroll_code_rejected`, `409 passkey_limit` / `already_registered` / `registration_conflict` |
| `GET /api/alerts` | tailnet, or Funnel with a session (`403 login_required` otherwise): `200` the alerts view (section 2c); `{"state":"disabled",…}` while `password_alerts` is off |
| `POST /api/alerts/ack` | no body; `X-Soos-Action: alerts-ack`, `X-Soos-Alerts-Epoch`, `X-Soos-Alerts-Through`: `200` the new view, `403 forbidden` (CSRF), `403 alerts_disabled`, `400 bad_request` (headers, or a `through` beyond the newest attempt), `429 rate_limited` (one per second), `409 stale_view`, `503 unavailable` |
| `GET /api/battery` | tailnet, or Funnel with a session (`403 login_required` otherwise): `200 application/json` `{state, percent, charge, external_power}` (section 2g); `{"state":"disabled",…}` while `battery_status` is `false`; a reading at most 5 s old, or one bounded read |
| `GET /api/offline` | tailnet, or Funnel with a session (`403 login_required` otherwise): `200 application/json` `{state, reason, network_monitor, intervals}` (section 2j); `state` is `disabled`, `active` or `unavailable` (`reason` `store_failed`), `network_monitor` is `unknown`, `active` or `unavailable`, `intervals` newest first as `{reason, start_unix_ms, end_unix_ms, duration_ms}` |
| `GET /.well-known/assetlinks.json` | public (also anonymous over Funnel): `404` exactly like an unknown path unless `[android]` is configured; then `200 application/json` with one Digital Asset Links statement (`delegate_permission/common.get_login_creds`, the package and its certificate digests), `HEAD` the same head, `405` for other methods (section 2j) |
| `GET /sw.js` | the service worker (public asset, `text/javascript`); it shows notifications and, on `pushsubscriptionchange` only, re-sends the renewed subscription once to `POST /api/push/subscribe`; the worker makes no request on push |
| `GET /api/push` | tailnet, or Funnel with a session: `{state, reason, public_key, subscriptions, devices, last_delivery, sender}`; `{"state":"disabled",…}` while `push_notifications` is off |
| `POST /api/push/subscribe` | `X-Soos-Action: push-subscribe`, the browser's subscription JSON (at most 2 KiB): `200 subscribed`, `400 bad_request` / `unsupported_push_service`, `403 forbidden` / `push_disabled`, `409 too_many_subscriptions`, `413 body_too_large`, `429 rate_limited` (one per second, shared with unsubscribe), `503 unavailable` / `store_unavailable` |
| `POST /api/push/unsubscribe` | `X-Soos-Action: push-unsubscribe`, `{"endpoint":…}`: `200 unsubscribed` (also for an unknown endpoint), `400`, `403`, `429`, `503` as above |
| `POST /api/push/test` | no body; `X-Soos-Action: push-test`: `202 test_queued`, `409 no_subscriptions`, `429 rate_limited` (one per 10 s), `403`, `503` as above |
| `GET /api/camera` | tailnet, or Funnel with a session: `200 {enabled, reachable, state, max_view_s, fps, width}`; `state` is `disabled`, `idle`, `pending`, `starting` or `streaming` (no cooldown state since 2026-10-07); never a token or a frame property |
| `POST /api/auth/camera/options` | `X-Soos-Action: camera-options`, `Origin` = `https://<rp_id>`: a `CameraView` challenge (`200 {allow_credentials, challenge, rp_id, timeout_ms}`, `allow_credentials` as for the unlock options), `403 forbidden` / `camera_disabled` / `camera_tailnet_only` / `passkeys_not_configured`, `409 no_passkey`, `429 rate_limited` / `too_many_challenges`, `503 unavailable` / `store_unavailable` |
| `POST /api/camera/start` | `X-Soos-Action: camera-view`, JSON assertion body (section 2f): `200 {"result":"view_ready","stream_path":"/api/camera/stream/<token>","token_ttl_ms":10000,…}`, `403 forbidden` / `camera_disabled` / `camera_tailnet_only` / `passkeys_not_configured` / `passkey_required` / `passkey_rejected`, `400 bad_request`, `413 body_too_large`, `409 view_in_progress`, `429 rate_limited`, `503 unavailable` / `store_unavailable` |
| `GET /api/camera/stream/<token>` | `X-Soos-Action: camera-stream`: `200 application/octet-stream` (`soosframe` JPEG parts until the view ends), or before the head `403 forbidden` / `camera_disabled` / `camera_tailnet_only` / `view_token_rejected` / `camera_refused`, `503 camera_unavailable` / `camera_format_unsupported`; `HEAD` → `405` |
| `POST /api/camera/stop` | no body; `X-Soos-Action: camera-stop`: `200 stopped` / `no_view`, `403 forbidden` / `camera_disabled` |
| `HEAD` of a `GET` route | same headers, empty body |

`state` is one of `locked`, `unlocked`, `no_session`, `unavailable`. Any logind failure is
`unavailable`: the service never reports `unlocked` unless a fresh read returned
`LockedHint == false`. The body never contains the uid, user name, session id or seat.

Every response carries `Cache-Control: no-store`, the CSP, `X-Content-Type-Options: nosniff`,
`Referrer-Policy: no-referrer`, `X-Frame-Options: DENY` and `Connection: close`. Requests are
bounded: 16 connections, 8 KiB head, 32 headers, 256-byte path, 5 s to send the head, 2 s per
write, no request body except on the four passkey body routes, `POST /api/camera/start` and the two
push subscription routes (`413 body_not_allowed` / `400`; on those routes at most 8 KiB, `Content-Length` or strict `Transfer-Encoding: chunked`,
5 s to send it, `413 body_too_large`), 1.5 s per logind snapshot, 2 s per lock flow, 2 s per
unlock flow. A camera stream keeps its connection (one of the 16, and one of the 8 Funnel
connections over Funnel) for its whole lifetime, takes no event-stream slot, writes each part
within 2 s and has no `Content-Length` (the body ends when the connection closes).

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
| `POST /api/unlock` answers `403 passkey_required` | The request carried no passkey assertion (an old "Unlock PC" shortcut, section 4) | Unlock from the web page (passkey check) |
| `403 passkeys_not_configured` | `rp_id` is not set | Section 2b |
| Funnel page shows the sign-in screen again / `403 login_required` | The web session expired (15 min idle, 8 h at most), was signed out, or its passkey was removed | Sign in with your passkey again |
| `409 no_passkey` | No passkey registered yet | `soos-remote enroll-code`, then register over the tailnet (section 2b) |
| `403 enroll_code_rejected` | No code, a wrong one (3 attempts delete it) or an expired one (5 min) | Run `soos-remote enroll-code` again |
| `503 store_unavailable` | The passkey store is unreadable, not `0600`, not yours, or busy | Fix the file mode/owner (`chmod 600`), check the journal |
| `503 busy` over Funnel | The Funnel capacity is in use (many simultaneous internet requests) | Retry; the tailnet path is not affected |
| `421` over Funnel only | The address used is not exactly `rp_id` (another name, a trailing dot, port 8443 or 10000) | Open `https://<rp_id>` exactly; keep Funnel on port 443 |
| The address does not resolve with the VPN off | Funnel is not active (`tailscale funnel status` empty), the `funnel` node attribute is missing, *shields up* is on, or the public DNS name is younger than about 10 minutes | Section 2b prerequisites; wait and retry |
| `tailscale funnel` refuses to start | The tailnet lacks HTTPS certificates or MagicDNS, or the node lacks the `funnel` attribute | Enable them in the admin console (section 2b) |
| Passkey prompt (Face ID, fingerprint or screen lock) never appears / `passkey_rejected` on every try | The page was not opened on `https://<rp_id>` (the passkey is bound to that exact name), the passkey was removed, or user verification was skipped | Open the exact `rp_id` address; `soos-remote passkeys list`; register again over the tailnet |
| No **Camera** card on the page | `camera_view` is absent or `false` | Section 2f |
| Camera: "The PC refused the camera view" (`403 camera_refused`) | The daemon refused the preview: `[preview] enabled`, `allowed_uids` or `remote_view` not set, or nobody is logged in at the PC's seat (SSH only, full logout) | Section 2f setup, step 2; log in at the PC |
| Camera: `503 camera_unavailable` | The daemon is stopped, the camera gives no frame, or the daemon connection failed | `systemctl status soos-daemon`; retry |
| Camera: `503 camera_format_unsupported` | A daemon older than the 2026-10-12 hardening (it forwards NV12 or MJPEG previews, which `soos-remote` does not convert), or a frame the daemon could not convert | Update and restart `soos-daemon`; check the camera with `soos-gui` |
| Camera: `403 camera_refused` while an SSH session of yours is open | An SSH session of yours, or a process left running from one (e.g. `tmux`), is open: the daemon refuses previews to processes under your user manager while your UID owns a remote session (a `soos-gui` started as a user unit is refused for the same reason) | `loginctl list-sessions` (a `remote` session of your user, possibly in state `closing`); end that process or run `loginctl terminate-session <id>` |
| Push sender unit `failed`, exit status 78, "sandbox not detected" | The sender runs outside its packaged sandbox: started by hand, a drop-in weakened `ProtectHome=`, `TemporaryFileSystem=/run:ro` or `InaccessiblePaths=`, or mounted the home back, the binary is not at `~/.local/bin/soos-push-sender`, the binary was replaced while the sender was starting (its path then ends in ` (deleted)`; the self-check fails closed and a restart on the new binary passes), or the home directory lies outside `/home` | Reinstall with `scripts/install_remote.sh`, remove the drop-in, then `systemctl --user restart soos-push-sender` (section 2d, Limits) |
| Camera: `403 camera_tailnet_only` | Over Funnel without `camera_view_funnel = true` | Use the tailnet, or section 2f |
| Camera: `409 view_in_progress` | Another view is open (one view for the whole PC; there is no cooldown after a view) | Stop the other view, then start again |
| Face unlock falls back to the password during a view | The view and `soos-gui` hold both daemon connections of your UID (`max_connections_per_uid` = 2) and the lock screen did not send its request on the auth-reserved connection within 200 ms (a slow PAM conversation), or another process of yours holds that reserve | Close `soos-gui` during a view, or raise `max_connections_per_uid` (section 2f) |
| The page shows "Battery: unknown" | The power-supply class could not be read completely within the bounds (an entry without a readable `type`, more than 64 entries or 8 batteries, a driver slower than 500 ms) | `cat /sys/class/power_supply/*/type`; a desktop without a battery shows "No battery" |
| Signed in in Safari but the home-screen app asks again | Safari and the home-screen app keep separate cookies | Sign in once in each |
| Android: no notification while the screen is off | A battery saver restricts Chrome (or Firefox) | Set the browser to **Unrestricted** battery usage (section 2h) |
| "Notifications could not start: the service worker did not register" | `/sw.js` could not be registered or did not activate within 10 s | Reload the page and try again; check that `/sw.js` answers `200` |
| The tailnet address does not load in one browser, but works in another or with `tailscale ping` | The browser's own secure DNS (Firefox DNS-over-HTTPS, Chrome or Edge *Use secure DNS*) bypasses MagicDNS, so the tailnet-only `*.ts.net` name does not resolve | Turn the browser's secure DNS off, or add an exception for your tailnet domain (Firefox: *Settings → Privacy & Security → DNS over HTTPS*, *Manage Exceptions*); see section 2i |
| "The PC does not accept this browser's push service (Edge on a computer, for example)…" | The browser's push service is not one of the three allowed hosts (`unsupported_push_service`; Edge desktop uses `*.notify.windows.com`) | Use Chrome, Firefox, Safari or Samsung Internet for notifications (section 2i); the refused subscription is dropped again |
| "This browser has no working push service…" | The browser refused the subscription (`AbortError` or `NotSupportedError`): Brave without *Use Google services for push messaging*, Chromium builds without Google keys | Turn the Brave setting on, or use a mandatory browser (section 2i) |
| "Notifications need a service worker, which this window does not allow…" | A private window, or a browser without service workers | Open the page in a normal window |
| "On iPhone and iPad, notifications only work in the home-screen app…" | The page is open in a Safari tab or another iOS browser | Share → **Add to Home Screen**, then open soos from the icon (iOS 16.4 or later) |
| "The browser's push service did not answer in time" / "The PC did not confirm the subscription in time" / "The PC could not be reached to save the subscription" | The subscribe or the hand-off to the PC did not finish within 10 s of the permission answer, or the PC was unreachable | Check the connection (tailnet, Funnel) and tap **Enable notifications** again |
| Passkey buttons disabled with "This browser cannot use passkeys…" | The browser (or an in-app WebView) has no WebAuthn | Use a mandatory browser; lock and status keep working on the tailnet |

## 8. Residual limitations

- A suspended iOS web app keeps its stream slot until `tailscaled` closes the proxied connection
  or the 30 min stream lifetime elapses; with four slots, the owner's own reconnects are the only
  consumer. The page shows `Unreachable` when no event arrived for 45 s (measured on the phone
  from the last event's arrival, not from `checked_unix_ms`).
- The status is only as truthful as the desktop's `LockedHint` (section 3).
- Identity headers are trusted because `tailscaled` strips client copies (verified on Tailscale
  1.102.4): a downgrade of `tailscaled` below 1.102.4, or a future release that stops stripping
  them, re-opens the forged-header risk, and the service cannot detect the proxy version.
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
- An allowed tailnet identity or a signed-in Funnel session learns the registered credential
  ids through the unlock and camera options (`allow_credentials`, ADR 2026-10-12); a credential
  id is an opaque public identifier that cannot sign, and an anonymous client never sees one.
  When the passkey sits in a third-party provider and several providers are enabled, Android may
  still show a picker at unlock (section 2h); sign-in keeps the picker by design.
- A device-bound (non-synced) passkey presenting the same counter twice concurrently is accepted
  twice (clone detection only, never an authentication bypass; synced iCloud Keychain and Google Password Manager passkeys report
  `0/0` and are unaffected; candid review MINOR finding, follow-up).
- Failed-password alerts (section 2c): a process running as the owner can create false
  lock-screen alerts, relabel the class of a helper-only attempt, and, while it keeps forging
  untrusted owner-side `pam_unix` failures, hide the lock-screen attempts that only `unix_chkpwd`
  logs; only `_UID=0` lines cannot be forged without root (lines trusted through `_EXE`, such as
  `sudo`, `su` or a configured locker, can be forged through journald's PID-reuse race: false
  alerts and relabelling only, never the suppression of a trusted failure). A helper-only check
  can carry the class of another recent attempt of the same side and account (the count stays
  right). The service needs a group that reads the whole system journal. With volatile journal
  storage the history does not survive a reboot. An attempt whose journal line or user-name
  field exceeds the bounds (24 KiB line, 4 KiB field) is skipped; this can only concern accounts
  other than yours. The kernel pipe buffer and `serde_json`'s escape scratch buffer are not
  wiped. A `journalctl` stalled for more than 2 s inside the 24 h replay, or silent for 2 s right
  after a cold start, can report `active` before the replay ends (later lines still count). A
  lock screen outside `lock_screen_programs` is not monitored; the page says so only when none
  of the configured paths exists. `journalctl` inside the exact unit sandbox is verified on the
  owner's hardware (matrix row RMC59).
- Push notifications (section 2d): a detailed notification is readable on the locked phone
  (iPhone: use *Show Previews: When Unlocked*; or `push_previews = "generic"`); vendor battery
  savers on Android may delay pushes (section 2h); a renewed subscription is lost until the next
  app open when the PC refuses the worker's single request (section 2h); Apple (or Google,
  Mozilla) sees delivery metadata, never the content; an owner-uid process can cause false
  notifications (at most 20 per hour) and can use the sender socket to post to the three push
  hosts; a compromised sender can reach abstract-namespace sockets (for example Xwayland's
  `@/tmp/.X11-unix/X0`) and forge replies; a client of your uid that delays `soos-remote`
  beyond 18 s can cause a duplicate notification; a newer summary replaces a pending
  `Retry-After` retry and is sent at once; a stolen Funnel session can register a device (check
  the device list); a renewal without the old subscription (some browsers) or above the
  2048-byte limit leaves the stale device until a 404/410 or `push remove`; the sender still
  reads world-readable files outside its hidden paths (`/etc`, `/usr`) and the `/proc` entries of
  your other processes, and its start-up check does not detect a bind added under `/run`; `ring`, `rustls` and the RustCrypto crates are not independently audited.
- Live camera view (section 2f):
  - Any process of your UID can already read frames through the GUI preview
    (`allowed_uids`), start a unit named `soos-remote.service`, or run `soos-remote` outside
    its unit (then classified as a local client, bypassing `remote_view`): `remote_view` is an
    administrative opt-in and an audit aid, not a boundary.
  - Whoever can pass the user verification (Face ID, fingerprint or screen lock) on a device
    holding your passkey (iCloud Keychain or Google Password Manager compromise included) can watch the camera; a stolen Funnel cookie alone gives no view, but can call
    `POST /api/camera/stop` (harmless).
  - A view holds one of your UID's 2 daemon connections and up to 10 of the 40 preview requests
    per second shared with `soos-gui`: with `soos-gui` open as well, a lock-screen face request
    uses the single auth-reserved connection; a process of your UID can occupy that reserve for
    up to 200 ms per connection, and a lock screen whose PAM conversation takes 200 ms or more
    misses it (password fallback). At each proactive reconnect (at
    most every 25 s) the client waits at most 200 ms for the daemon to release the old
    connection; a PAM request in that window can still be refused at admission (password
    fallback).
  - Pixels transit `tailscaled` (and, over Funnel, Tailscale's relays, TLS terminated on the
    PC); kernel socket buffers and `tailscaled`'s memory are outside soos' control; the phone
    can screen-record.
  - The camera LED is the only local indicator; there is no on-screen indicator.
  - The daemon reads the peer cgroup by PID after `SO_PEERCRED`; PID reuse within a connection
    is theoretically possible and only affects this administrative classification.
- Battery level (section 2g): the value is only as fine as the kernel and the firmware report
  it, and reaches the page up to about 5 s late; `external_power` is unknown without a readable
  `online` attribute; a single battery without a readable `capacity` shows "Battery level
  unknown" even if it exposes an energy pair (fail closed, never recomputed). At service stop
  the runtime waits at most `RUNTIME_SHUTDOWN_TIMEOUT_MS` (1 s) for blocking tasks: a battery
  read stuck in a hung driver is abandoned, and the same bound applies to every other blocking
  task still running then (a push store write, which is temp file + `fsync` + `rename` and so
  never leaves a torn store).
- Browser support (section 2i): capability flags are hints for the page only (every server check is the
  same for every browser); feature detection cannot see a push service that is present but broken until
  `subscribe` answers (bounded by 10 s); a browser without `EventSource` polls the status every 15 s, which
  refreshes a Funnel web session's idle timer while the page is open (as an open stream does) and shows no
  battery line; engines without container queries get an approximate portrait camera box (clipped when the card
  stage is capped by `max-height: 70vh`; in full screen the box ignores the safe-area padding and `100vh` is the
  large viewport on mobile); pre-Baseline engines (Firefox ESR 115 included) are best effort; there is no
  automated cross-engine run yet (hardware rows RXB12–RXB20).
- Offline history (section 2j): the notification comes after the fact; a PC kept offline is never
  reported in real time (tiers 2 and 3). An outage shorter than 30 s, or one that starts and ends
  between two 5 s readings, is not recorded. Wall times are as good as the PC's clock. After a
  crash or a power loss, a `shutdown_or_crash` interval starts at the last heartbeat (up to 60 s
  early). A nightly shutdown gives one `shutdown_or_crash` notification each morning. A laptop that
  sleeps often fills the 32 rows with `sleep` rows. A notification not yet sent when the service
  stops is lost (the interval stays in the history). A wired PC with the Wi-Fi radio off reports
  an unplugged cable as `wifi_disabled`. A process of your UID can edit `remote-network.json`
  (forged history, bounded to 32 rows and 8 KiB; false notifications bounded by the shared budget
  of 20 per hour). The service reads 4 bus properties every 5 s (about 69 000 calls a day) and
  writes the heartbeat at most once a minute.
- The `bind` → `chmod 0600` window of the socket is closed by the `0700` parent directory and
  the unit's `UMask=0077`; the service must therefore be started through the unit (or with the
  same umask).
- Native Android app (section 2j): the listed signing key is a login trust anchor (accepted
  risk); the session cookie lives in the app process only; the first Gradle verification
  metadata is trusted on first use; the debug APK is debuggable; Credential Manager providers
  other than Google Password Manager are best effort; the Digital Asset Links check needs the
  public Funnel host.
- PC offline alerts of the Android app (section 2j): the heartbeat is best effort. Doze and App
  Standby defer its checks, so an alert can come well after 30 minutes; a force stop cancels it
  until the app is opened again; with the phone's Tailscale VPN off, without any network
  outside airplane mode, or with Battery Saver, Data Saver or the restricted battery setting
  blocking background network, checks fail and can raise "PC went offline"; while the app process
  lives, a check over Funnel refreshes the web session idle timer (accepted risk, the 8 h
  absolute lifetime is unchanged).

## 9. Out of scope (each needs its own ADR)

- An automatic re-lock after a remote unlock (owner decision D-F). The crate still never names
  `SetLockedHint`, the `Unlock` signal or a session-ending method (invariant RMC-S3).
- Funnel on ports 8443 or 10000, other attestation formats than `none`, other algorithms than
  ES256, several users.
- Any other public exposure path (Cloudflare Tunnel, Cloudflare Access, a custom domain, a
  reverse proxy): explored separately; it needs its own ADR before it may reach this socket.
- **Lock/unlock-state notifications**, camera view notifications (removed 2026-10-07),
  real-time "monitoring lost" notifications (section 2j covers only the after-the-fact back-online
  notification and the opt-in PC offline alerts of the Android app; the watchdog needs its own
  ADR), a "network changed"
  notification, notification actions, and push hosts other than Apple,
  Google and Mozilla (section 2d covers failed-password alerts and the test notification, section
  2j the back-online notification).
- Any recording, snapshot or still image of the **live camera** view (section 2f), audio,
  H.264/WebRTC or any other live camera transport, an on-screen indicator, any image decoder
  inside `soos-remote` (the daemon converts frames for it), and any embedding or evidence access.
- Battery time-to-empty or time-to-full, battery health or cycle count, peripheral batteries
  (`scope=Device`), low-battery notifications, any write to sysfs, UPower.
- Features of the native Android app beyond its first version (section 2j: unlock, camera, push, Keystore device key and release signing are later PRs under the ADR of 2026-10-13; the first version is no longer out of scope), FCM direct API keys, Edge desktop push (`*.notify.windows.com`; the page says so,
  section 2i), Android Shortcuts/Tasker integration (pointer in section 2h only).
- System-wide packaging (`install.sh`, deb/rpm/Arch): deferred until the owner approves the
  merge; `scripts/install_remote.sh` is the only installer.
