# Remote Companion (`soos-remote`)

> Crate: `crates/remote` (`soos-remote`, GitHub #339, ADR 2026-10-05 "Remote Companion `soos-remote`",
> ADR 2026-10-06 "Remote Unlock in `soos-remote`", ADR 2026-10-06 "Tailscale Funnel Access and
> In-House Passkey Authentication for `soos-remote`", ADR 2026-10-06 "Failed-Password Alerts in
> `soos-remote` From the System Journal", ADR 2026-10-06 "Web Push Notifications for
> Failed-Password Alerts Through a Separate Sender Unit", ADR 2026-10-07 "Live Camera View in
> `soos-remote` Through the Daemon Preview Channel")
> Scope: a **user-level** service that shows the owner's phone the real-time lock status of the
> desktop session, offers a remote **lock** and, only when `allow_unlock = true`, a remote
> **unlock** protected by a passkey (Face ID) on every request (section 2a). Optionally it is
> also reachable from the internet through Tailscale Funnel, behind a passkey login (section
> 2b). When `password_alerts = true` the page also lists failed password attempts made on the
> PC (section 2c), never the typed password, and with `push_notifications = true` sends them to
> the phone as push notifications even when the app is closed (section 2d). With
> `camera_view = true` and the daemon's `[preview] remote_view = true` it also shows a live view
> of the PC camera after Face ID (section 2f), never recorded. Nothing else (see "Out of scope").
> Source of truth: `crates/remote/src/lib.rs` (constants), `crates/remote/src/config.rs`
> (configuration keys), `crates/remote/src/server.rs` (request handling). If this document and
> the code disagree, report the drift: the invariant `remote_companion_contract` pins the parts
> of this page the acceptance criteria rely on (matrix rows RMC1–RMC44, walkthroughs 185, 186
> and 187); `remote_passkey_contract` pins the Funnel and passkey parts; `remote_alerts_contract`
> pins the failed-password alerts (rows RMC45–RMC59, walkthrough 188); `remote_push_contract`
> pins the push notifications (rows RMC60–RMC74, walkthrough 189); `remote_camera_contract`
> pins the live camera view (rows RLC1–RLC16, walkthrough 191).

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
  section 2a), only after a fresh passkey assertion with user verification (Face ID / Touch ID);
- verifies WebAuthn passkeys itself (ES256, attestation `none`, pure-Rust RustCrypto code, no
  OpenSSL, no external service);
- when `password_alerts = true`, follows the system journal through a `journalctl` child
  process and shows failed password attempts made on the PC (section 2c);
- when `push_notifications = true`, encrypts a short summary of each new burst of failed
  attempts for the phone and hands it to `soos-push-sender`, which delivers it as a standard
  Web Push notification (section 2d);
- when `camera_view = true`, asks `soos-daemon` for preview frames over its Unix socket
  (`RequestKind::PreviewFrame`) and streams them as JPEG to the page after a fresh Face ID
  (section 2f).

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

## 2c. Failed-password alerts (opt-in)

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

## 2d. Push notifications (opt-in)

With `push_notifications = true` the phone is told about failed passwords on the PC **even
when the app is closed**: a standard Web Push notification to the home-screen web app
(iOS/iPadOS 16.4 or later; Safari tabs cannot receive push). It is the same information as
section 2c, sent at most once per burst: "3 wrong passwords — lock screen, your account".

**What a notification carries, and what never.** Only the source class (lock screen, sudo,
login, other), the account class (your account, root, another account), the kind (wrong
password, or attempt while locked out) and counts — **never the typed password**, nor any
part, length or hash of it (no journal line contains it, and soos never captures it). With
`push_previews = "generic"` the visible text is only "Security alert on your PC — Open soos for
details". A notification is readable on the **locked** iPhone unless you set *Settings →
Notifications → Show Previews → When Unlocked* (or choose `push_previews = "generic"`). The
message is end-to-end encrypted (RFC 8291): Apple's push service sees only its size, time and
the PC's public address, never its content. The page stays the source of truth: push delivery
is best effort.

**When.** Only attempts recorded live (not the 24 h replay at start): the first attempt of a
burst is sent 3 s later as one summary; then at most one notification every 30 s and 20 per
hour, the counts accumulating meanwhile (a failed delivery is retried twice, then its counts
are added to the next notification, so the counts you receive add up). Any process running as
you can forge lock-screen journal lines and therefore cause **false notifications** (bounded by
these limits), exactly like the false alerts of section 2c.

**Setup** (owner steps, nothing is enabled by the installer):

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
4. On the iPhone, open the app **from the home-screen icon** (section 4), tap
   **Enable notifications** and allow them. The card then lists the device ("Apple device,
   enabled …"); its endpoint is on `web.push.apple.com`.
5. Tap **Send test notification**. Then lock the phone and type a wrong password at the PC's lock
   screen: one notification arrives within about 10 s.

**Devices.** At most 4 devices receive notifications. The card lists each one (push service and
date). Anyone signed in to the page (your tailnet identity, or a Funnel passkey session) can add
a device, so check the list after enabling and whenever a device is unknown:
`soos-remote push list` prints `<N>  <push host>  created <unix time>`,
`soos-remote push remove N` removes one, and `soos-remote passkeys remove N` ends the Funnel
sessions of a passkey that is not yours. `soos-remote push reset` replaces the push key and
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
Its sandbox hides your home directory (except its own binary) and `/run` (except its socket
directory), so it cannot read your files, `remote.sock`, the session bus or `tailscaled`.
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
verified on the owner's hardware (matrix row RMC74).

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
  focus, 48 px buttons and 44 px links, safe-area insets on all sides, no motion under
  `prefers-reduced-motion`, zoom kept.
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
- **Unchanged.** The Content Security Policy, `app.js`, `sw.js`, every element id and label, and
  the routes. The page uses the system font stack (no font file, no CDN) and contains no raster
  from the brand archive. The later camera card (section 2f) is built by `app.js` at runtime with
  the same tokens and adds no element id.

## 2f. Live camera view (opt-in)

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

**Fresh Face ID per view.** Every view, on the tailnet as over Funnel, starts with a fresh
passkey assertion with user verification (Face ID / Touch ID) over a new single-use challenge
of purpose `CameraView` (`POST /api/auth/camera/options`), distinct from the unlock challenge;
a web session, a cookie or an earlier assertion never starts a view. Failures count against the
same 5-failures lockout as unlock. A successful `POST /api/camera/start` reserves the view and
returns a single-use stream token (256 random bits, valid 10 s, bound to the caller's class and
Funnel session, carried only in the path, never logged); the page then opens
`GET /api/camera/stream/<token>` with `X-Soos-Action: camera-stream`.

**One view, bounded.** One view at a time for the whole PC (`409 view_in_progress`), a 10 s
cooldown after a view that showed pixels (`429 camera_cooldown` with `retry_after_ms`), and a
maximum duration of `camera_max_view_s`. The view also ends on *Stop camera view*
(`POST /api/camera/stop`, any signed-in caller may stop), when the page is hidden or closed,
when the connection closes, when a part cannot be written within 2 s, when the Funnel web
session expires (checked every 5 s), when no new frame could be sent for 5 s, on a daemon
refusal, on an unsupported frame format and when the service stops.

**Transport.** The stream is `multipart/x-mixed-replace; boundary=soosframe`: one baseline JPEG
part per frame, each with its `Content-Length` (at most 512 KiB), at most `camera_fps` parts
per second, over the existing `tailscale serve` / `tailscale funnel` path. The page reads it
with `fetch` and a `ReadableStream` (bounded to 1 MiB of buffered bytes) and draws each frame
into a canvas; the Content Security Policy is unchanged (no `blob:` or object URL, no `<img>`
stream). The `200` head is sent only once the first JPEG is ready (within 5 s); before that a
failure is a JSON error the page can explain (`camera_refused`, `camera_unavailable`,
`camera_format_unsupported`). Only Grey, YUYV and RGB24 preview frames are converted;
`soos-remote` contains no image decoder.

**Awareness.** The camera **LED** lights while the daemon captures, and stays on for about 10 s
after the last frame; it is the only indicator at the PC (no on-screen indicator). With push
configured (section 2d) a view start sends one generic notification "soos camera view — The
live camera view of your PC was started" (no image, best effort, never delaying the view). The
journal records `camera view started`, `camera view ended` and `camera view refused` (fixed
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
`max_connections_per_uid` (`[peer_limits]`, default 2) connections per unprivileged UID: with a
view open **and** `soos-gui` open, a face request of a lock screen running as your user is
refused at admission and falls back to the password (fail-safe, never an unlock). Close
`soos-gui` during a view, or raise `max_connections_per_uid` to 3. Root PAM callers (`sudo`,
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
3. the push "camera view started" arrives (when section 2d is set up);
4. *Stop camera view* ends the view at once; a second start within 10 s is refused
   (`camera_cooldown`);
5. after a full logout at the PC, a start is refused (`camera_refused`);
6. optionally, over Funnel with `camera_view_funnel = true`, steps 1 and 4 again (the frame
   rate over Funnel is bounded by Tailscale's relay throughput).

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
| `GET /api/alerts` | tailnet, or Funnel with a session (`403 login_required` otherwise): `200` the alerts view (section 2c); `{"state":"disabled",…}` while `password_alerts` is off |
| `POST /api/alerts/ack` | no body; `X-Soos-Action: alerts-ack`, `X-Soos-Alerts-Epoch`, `X-Soos-Alerts-Through`: `200` the new view, `403 forbidden` (CSRF), `403 alerts_disabled`, `400 bad_request` (headers, or a `through` beyond the newest attempt), `429 rate_limited` (one per second), `409 stale_view`, `503 unavailable` |
| `GET /sw.js` | the service worker (public asset, `text/javascript`); it only shows notifications |
| `GET /api/push` | tailnet, or Funnel with a session: `{state, reason, public_key, subscriptions, devices, last_delivery, sender}`; `{"state":"disabled",…}` while `push_notifications` is off |
| `POST /api/push/subscribe` | `X-Soos-Action: push-subscribe`, the browser's subscription JSON (at most 2 KiB): `200 subscribed`, `400 bad_request` / `unsupported_push_service`, `403 forbidden` / `push_disabled`, `409 too_many_subscriptions`, `413 body_too_large`, `429 rate_limited` (one per second, shared with unsubscribe), `503 unavailable` / `store_unavailable` |
| `POST /api/push/unsubscribe` | `X-Soos-Action: push-unsubscribe`, `{"endpoint":…}`: `200 unsubscribed` (also for an unknown endpoint), `400`, `403`, `429`, `503` as above |
| `POST /api/push/test` | no body; `X-Soos-Action: push-test`: `202 test_queued`, `409 no_subscriptions`, `429 rate_limited` (one per 10 s), `403`, `503` as above |
| `GET /api/camera` | tailnet, or Funnel with a session: `200 {enabled, reachable, state, cooldown_ms, max_view_s, fps, width}`; `state` is `disabled`, `idle`, `pending`, `starting`, `streaming` or `cooldown`; never a token or a frame property |
| `POST /api/auth/camera/options` | `X-Soos-Action: camera-options`, `Origin` = `https://<rp_id>`: a `CameraView` challenge (`200 {challenge, rp_id, timeout_ms}`), `403 forbidden` / `camera_disabled` / `camera_tailnet_only` / `passkeys_not_configured`, `409 no_passkey`, `429 rate_limited` / `too_many_challenges`, `503 unavailable` / `store_unavailable` |
| `POST /api/camera/start` | `X-Soos-Action: camera-view`, JSON assertion body (section 2f): `200 {"result":"view_ready","stream_path":"/api/camera/stream/<token>","token_ttl_ms":10000,…}`, `403 forbidden` / `camera_disabled` / `camera_tailnet_only` / `passkeys_not_configured` / `passkey_required` / `passkey_rejected`, `400 bad_request`, `413 body_too_large`, `409 view_in_progress`, `429 rate_limited` / `{"result":"camera_cooldown","retry_after_ms":N}`, `503 unavailable` / `store_unavailable` |
| `GET /api/camera/stream/<token>` | `X-Soos-Action: camera-stream`: `200 multipart/x-mixed-replace; boundary=soosframe` (JPEG parts until the view ends), or before the head `403 forbidden` / `camera_disabled` / `camera_tailnet_only` / `view_token_rejected` / `camera_refused`, `503 camera_unavailable` / `camera_format_unsupported`; `HEAD` → `405` |
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
| No **Camera** card on the page | `camera_view` is absent or `false` | Section 2f |
| Camera: "The PC refused the camera view" (`403 camera_refused`) | The daemon refused the preview: `[preview] enabled`, `allowed_uids` or `remote_view` not set, or nobody is logged in at the PC's seat (SSH only, full logout) | Section 2f setup, step 2; log in at the PC |
| Camera: `503 camera_unavailable` | The daemon is stopped, the camera gives no frame, or the daemon connection failed | `systemctl status soos-daemon`; retry |
| Camera: `503 camera_format_unsupported` | The camera delivers NV12 or MJPEG previews, which `soos-remote` does not convert | Not supported in this version |
| Camera: `403 camera_tailnet_only` | Over Funnel without `camera_view_funnel = true` | Use the tailnet, or section 2f |
| Camera: `429 camera_cooldown` / `409 view_in_progress` | A view ended less than 10 s ago, or another view is open | Wait, or stop the other view |
| Face unlock falls back to the password during a view | The view and `soos-gui` hold both daemon connections of your UID (`max_connections_per_uid` = 2) | Close `soos-gui` during a view, or raise `max_connections_per_uid` (section 2f) |
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
- Push notifications (section 2d): a detailed notification is readable on the locked iPhone
  (use *Show Previews: When Unlocked* or `push_previews = "generic"`); Apple (or Google,
  Mozilla) sees delivery metadata, never the content; an owner-uid process can cause false
  notifications (at most 20 per hour) and can use the sender socket to post to the three push
  hosts; a compromised sender can reach abstract-namespace sockets (for example Xwayland's
  `@/tmp/.X11-unix/X0`) and forge replies; a client of your uid that delays `soos-remote`
  beyond 18 s can cause a duplicate notification; a newer summary replaces a pending
  `Retry-After` retry and is sent at once; a stolen Funnel session can register a device (check
  the device list); `ring`, `rustls` and the RustCrypto crates are not independently audited.
- Live camera view (section 2f):
  - Any process of your UID can already read frames through the GUI preview
    (`allowed_uids`), start a unit named `soos-remote.service`, or run `soos-remote` outside
    its unit (then classified as a local client, bypassing `remote_view`): `remote_view` is an
    administrative opt-in and an audit aid, not a boundary.
  - Whoever can pass Face ID on a device holding your passkey (iCloud Keychain compromise
    included) can watch the camera; a stolen Funnel cookie alone gives no view, but can call
    `POST /api/camera/stop` (harmless).
  - A view holds one of your UID's 2 daemon connections and up to 10 of the 40 preview requests
    per second shared with `soos-gui`: with `soos-gui` open as well, a lock-screen face request
    is refused at admission and falls back to the password. At each proactive reconnect (at
    most every 25 s) the client waits at most 200 ms for the daemon to release the old
    connection; a PAM request in that window can still be refused at admission (password
    fallback).
  - Pixels transit `tailscaled` (and, over Funnel, Tailscale's relays, TLS terminated on the
    PC); kernel socket buffers and `tailscaled`'s memory are outside soos' control; the phone
    can screen-record.
  - The camera LED is the only local indicator; there is no on-screen indicator.
  - The daemon reads the peer cgroup by PID after `SO_PEERCRED`; PID reuse within a connection
    is theoretically possible and only affects this administrative classification.
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
- **Lock/unlock-state notifications**, "monitoring lost" notifications, notification
  actions, and push hosts other than Apple, Google and Mozilla (section 2d covers only
  failed-password alerts and the test notification).
- Any recording, snapshot or still image of the **live camera** view (section 2f), audio,
  H.264/WebRTC or any other live camera transport, an on-screen indicator, NV12/MJPEG
  conversion, and any embedding or evidence access.
- System-wide packaging (`install.sh`, deb/rpm/Arch): deferred until the owner approves the
  merge; `scripts/install_remote.sh` is the only installer.
