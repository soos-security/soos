# Research: Web Push notifications for soos-remote (feature level 2)

Status: research note for GitHub #339 (branch `feat/remote-auth-alerts`), 2026-10-06.
Scope: every external fact the "real push notifications on the iPhone" design (owner decisions
O-1 to O-5, feature level 2 on top of the level-1 failed-password alerts of ADR 2026-10-06 and
walkthrough 188) depends on. Only read-only commands were run on the host (`pacman -Q`,
`journalctl` queries printing counts only, `getent ahosts`, `ip route get`,
`systemctl --user show`, `systemd-analyze --user security`, `sysctl`, `man`, registry
metadata from crates.io and the local Cargo registry cache). Nothing was installed, started,
restarted or configured; `tailscale` was not run; `~/.config` and `/etc` were not touched; no
push request was sent.

## 0. Relation to the relayed owner request

The relayed request asks to "see on the phone the passwords that were tested remotely". As
for level 1 (ADR 2026-10-06 "Failed-Password Alerts", item 1), the typed password is **not
available** to `soos-remote` (no journal producer contains it, research_alerts §2) and
capturing `PAM_AUTHTOK` is forbidden by `AGENTS.md` and O-2. Level 2 therefore pushes only the
level-1 safe subset (time, source class, account class, count). A Web Push payload transits
Apple's servers (encrypted end to end, §3) and lands on the lock screen of the phone, where it
is readable without unlocking unless the owner sets "Show Previews: When Unlocked": an even
stronger reason to never include any password-derived data. Any wish to see the typed text
needs an amendment of `AGENTS.md` and its own ADR; it is not a research input here.

## 1. Versions and sources

| Component | Value | Where checked |
|---|---|---|
| systemd | `262 (262-1-arch)` | `pacman -Q systemd`, `journalctl --version` |
| Tailscale | `1.102.4-1` (package only, the CLI was not run) | `pacman -Q tailscale` |
| curl / openssl / glibc | `8.22.0-1` / `3.6.5-1` / `2.44+r50` | `pacman -Q` |
| CA store | `ca-certificates 20240618-1`, `ca-certificates-mozilla 3.130-1`; `/etc/ssl/certs/ca-certificates.crt -> ../../ca-certificates/extracted/tls-ca-bundle.pem` | `pacman -Q`, `ls -l` |
| Rust toolchain | `rustc 1.98.1`, `cargo 1.98.1` | `rustc --version` |
| Kernel userns | `kernel.unprivileged_userns_clone = 1`, `user.max_user_namespaces = 62443` | `sysctl` |
| RFC 8030 (Web Push protocol), RFC 8188 (aes128gcm), RFC 8291 (message encryption), RFC 8292 (VAPID) | text | rfc-editor.org |
| Apple | "Sending web push notifications in web apps and browsers" (developer.apple.com, JSON of the doc page), WebKit blogs 12945 "Meet Web Push", 13878 "Web Push for Web Apps on iOS and iPadOS", 16535 "Meet Declarative Web Push" | fetched 2026-10-06 |
| Mozilla | autopush-rs error codes | mozilla-services.github.io/autopush-rs/errors.html |
| MDN | `PushManager.subscribe`, `PushSubscription.toJSON`, `ServiceWorkerContainer.register`, `Notification.requestPermission`, CSP `worker-src` | developer.mozilla.org |

## 2. Browser side (iPhone home-screen web app)

### 2.1 Platform requirements

- Web Push for **Home Screen web apps** exists from **iOS / iPadOS 16.4**; Safari tabs on iOS
  cannot subscribe (WebKit 13878; Apple doc: "Add web push to Home Screen web apps in iOS 16.4
  or later"). The owner's iOS version is not known to the agent: **owner check**.
- The site must be a web app, not a bookmark: manifest `display` `standalone` or `fullscreen`
  (WebKit 13878). Checked: `crates/remote/assets/manifest.webmanifest` already has
  `"display": "standalone"`, `"start_url": "/"`, `"scope": "/"`.
- A home-screen web app has its own storage and its own push subscription, separate from
  Safari; the owner's existing home-screen icon (installed during the Funnel work) is the app
  that must subscribe.
- No Apple Developer Program account is needed ("You don't need to join the Apple Developer
  Program to send web push notifications", Apple doc; same in WebKit 12945).
- **User gesture**: permission and subscription must be requested from a direct user action
  ("call the push subscription method immediately from the gesture's event handler code",
  Apple doc; WebKit 13878 "in response to direct user interaction — such as tapping on a
  'subscribe' button"). Doing an `await fetch(...)` before `subscribe()` inside the handler
  risks losing the transient activation: fetch the VAPID public key before the tap (or embed
  it in a prior response) and call `Notification.requestPermission()` /
  `pushManager.subscribe()` first in the click handler.
- **No invisible push**: "Safari doesn't support invisible push notifications. Present push
  notifications to the user immediately after your service worker receives them. If you
  don't, Safari revokes the push notification permission for your site." (Apple doc;
  WebKit 12945: "Violations of the `userVisibleOnly` promise will result in a push
  subscription being revoked."). The service worker `push` handler must always call
  `event.waitUntil(self.registration.showNotification(...))`, also when the payload is
  missing or malformed (show a generic text then).
- `userVisibleOnly: true` is required by Chrome/Edge and by Safari's model (MDN, WebKit 12945).
- Notifications integrate with Focus; the Badging API (`navigator.setAppBadge`,
  `clearAppBadge`) is available to home-screen apps from 16.4 (WebKit 13878, Apple doc).
- **Declarative Web Push** (iOS / iPadOS 18.4, macOS 15.5): a payload that is JSON with the
  magic member `"web_push": 8030` and a `notification` object (non-empty `title` required,
  `body`, `navigate` URL, `silent`, `app_badge`, `lang`, `dir`) is displayed by the system
  even without a service worker; with a service worker the `push` event receives the proposed
  notification, and if the handler fails the declarative notification is still shown
  ("removing penalties for service worker failures", WebKit 16535). Using the declarative
  JSON shape as the plaintext of every push costs nothing on older iOS (the service worker
  parses the same JSON) and protects against the revocation rule above on 18.4+. Same VAPID
  and RFC 8291 encryption (WebKit 16535 subscribes with the same `applicationServerKey`).

### 2.2 Web APIs

- `ServiceWorkerContainer.register(scriptURL)`: secure context, `scriptURL` same origin as
  the page, served with a JavaScript MIME type; the default and maximum scope is the
  script's directory unless `Service-Worker-Allowed` widens it; `updateViaCache` default
  `'imports'` (the main script is always fetched from the network) (MDN). Consequence: serve
  the worker at `/sw.js` (scope `/`, no `Service-Worker-Allowed` header needed) as
  `text/javascript`; the existing mandatory `Cache-Control: no-store` is compatible.
- CSP: `worker-src` governs `ServiceWorker` registration and, when absent, falls back to
  `child-src`, then `script-src`, then `default-src` (MDN). The current header
  (`crates/remote/src/http.rs:263`) is `default-src 'self'; script-src 'self'; ...`, so a
  same-origin `/sw.js` is already allowed; an explicit `worker-src 'self'` is a minimal,
  self-documenting addition. A service worker has its own CSP taken from the response
  headers of the worker script itself, so `/sw.js` gets the same mandatory headers.
- `PushManager.subscribe({ userVisibleOnly: true, applicationServerKey })`:
  `applicationServerKey` is the 65-byte uncompressed P-256 public key, as a base64url string
  or an `ArrayBuffer`; secure context; call from a user gesture (MDN). Subscribing again with
  a different key while a subscription exists fails (`InvalidStateError`): a VAPID key
  rotation needs `unsubscribe()` then `subscribe()` on the phone.
- `PushSubscription.toJSON()` returns `{ endpoint, expirationTime (number|null),
  keys: { p256dh, auth } }`, keys in unpadded base64url (MDN). `p256dh` decodes to 65 bytes
  starting with `0x04`, `auth` to 16 bytes (RFC 8291 §3.2).
- `Notification.requestPermission()` resolves to `"granted"`, `"denied"` or `"default"`
  (treated as denied); secure context; user activation (MDN). Pages on iOS must display
  through `ServiceWorkerRegistration.showNotification`, not `new Notification()`.

### 2.3 Interaction with the existing authentication (code facts)

- On Funnel, `server.rs` lets only `is_funnel_public` routes (static assets, login) through
  without a web session (`403 login_required` otherwise); the session cookie is
  `__Host-soos_session` with `SameSite=Strict; HttpOnly; Secure; Path=/`, at most
  `MAX_WEB_SESSIONS = 4`, `Max-Age=28800` (8 h). `/sw.js` must be a public static asset
  (the browser fetches it for update checks without user action, and an expired session must
  not break the worker); the subscribe / unsubscribe / test routes must be authenticated like
  `/api/alerts` (tailnet identity or Funnel web session, CSRF header like the lock flow).
- A push notification must keep working while the web session is expired: the payload is
  self-contained, and a tap opens `start_url` (`/`), where the passkey login appears.
- Level-1 alerts are rebuilt from the last 24 h of the journal at each start
  (`alerts.rs`, ADR item 4): replayed attempts (journal time before the service start) must
  **not** be pushed again, only attempts recorded live. `AlertBook` exposes a
  `watch::Sender<u64>` version (`alerts.rs:794`, `subscribe()` at `:895`), a natural trigger.

## 3. Server side cryptography (RFC 8291, RFC 8188, RFC 8292)

### 3.1 Message encryption (RFC 8291, aes128gcm of RFC 8188)

Key schedule, verbatim from RFC 8291 §3.4 (application server side):

```
ecdh_secret = ECDH(as_private, ua_public)          -- as_* is a fresh ephemeral key per message
auth_secret = <from user agent, 16 octets>
salt        = random(16)
PRK_key  = HMAC-SHA-256(auth_secret, ecdh_secret)
key_info = "WebPush: info" || 0x00 || ua_public || as_public
IKM      = HMAC-SHA-256(PRK_key, key_info || 0x01)
PRK      = HMAC-SHA-256(salt, IKM)
cek_info = "Content-Encoding: aes128gcm" || 0x00
CEK      = HMAC-SHA-256(PRK, cek_info || 0x01)[0..15]
nonce_info = "Content-Encoding: nonce" || 0x00
NONCE    = HMAC-SHA-256(PRK, nonce_info || 0x01)[0..11]
```

- Body = RFC 8188 header `salt (16) || rs (uint32 BE) || idlen (1) = 65 || keyid = as_public
  (65 octets, starts with 0x04)` followed by **one** record: `AES-128-GCM(CEK, NONCE,
  plaintext || 0x02 || padding-zeros)` with its 16-byte tag; no associated data (RFC 8291 §4:
  "An application server MUST encrypt a push message with a single record"; `rs` must exceed
  plaintext + 1 + padding + 16; the receiver discards a delimiter other than `0x02`).
  Header overhead = 86 bytes; the example uses `rs = 4096` (`AAAQAA` in the body).
- `Content-Encoding: aes128gcm` exactly (RFC 8291 §4).
- Size: "A push service is not required to support more than 4096 octets of payload body"
  (RFC 8291 §4), so plaintext + padding ≤ 4096 − 86 − 1 − 16 = 3993 bytes; Apple:
  `PayloadTooLarge` "over the limit of 4 KB"; Mozilla: `413` at 4028 characters. A minimal
  alert payload is well under 300 bytes.
- **Known-answer test vector** (RFC 8291 §5 and Appendix A), base64url:
  plaintext `V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24`,
  as_public `BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8`,
  as_private `yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw`,
  ua_public `BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4`,
  ua_private `q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94`,
  salt `DGv6ra1nlYgDCS1FRnbzlw`, auth_secret `BTBZMqHH6r4Tts7J_aSIgg`;
  ecdh_secret `kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs`,
  PRK_key `Snr3JMxaHVDXHWJn5wdC52WjpCtd2EIEGBykDcZW32k`,
  IKM `S4lYMb_L0FxCeq0WhDx813KgSYqU26kOyzWUdsXYyrg`,
  PRK `09_eUZGrsvxChDCGRCdkLiDXrReGOEVeSCdCcPBSJSc`,
  CEK `oIhVW04MRdy2XN9CiKLxTg`, NONCE `4h_95klXJ5E_qnoN`; body (145 bytes, no padding, rs 4096):
  `DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN`.
  The encryptor needs injectable ephemeral key and salt to reproduce it (test seam, like the
  crate's existing CSPRNG seam).

### 3.2 VAPID (RFC 8292)

- Header: `Authorization: vapid t=<JWT>, k=<base64url uncompressed P-256 public key>`
  (RFC 8292 §3, §3.1, §3.2).
- JWT: JWS compact, header `{"typ":"JWT","alg":"ES256"}`; claims `aud` = Unicode
  serialization of the **origin of the push resource URL** (e.g. `https://web.push.apple.com`,
  no path, no trailing slash), `exp` ≤ 24 h after the request, `sub` = `mailto:` or `https:`
  URI (RFC 8292 §2, §2.1). The ES256 signature is the 64-byte raw `R || S` (RFC 7518 §3.4),
  not DER.
- **Apple makes `sub` mandatory and validates it**: `BadJwtToken` covers "The JWT subject
  claim isn't a URL or `mailto:`", wrong audience, `exp` more than one day ahead, missing JWT
  or wrong key (Apple doc). Field reports: `mailto:...@localhost`, a `.invalid` domain, a bare
  host name and a doubled `mailto:mailto:` were all refused with `403 {"reason":"BadJwtToken"}`
  while `https://github.com/<repo>` was accepted (alphastorm/omp-session-gateway#173,
  openclaw#83134, videah.net/web-push-pitfall). Practical choices: the owner's Funnel origin
  `https://<host>.<tailnet>.ts.net` (already in the config as `rp_id`) or the public
  repository URL; never a placeholder default. This is the most likely first-run failure:
  **owner hardware check**.
- Apple: "Don't refresh your JWT more frequently than once per hour" (Apple doc). Cache one
  JWT per audience origin, regenerate after ≥ 1 h, with `exp` ≈ now + 12 h (< 24 h with clock
  skew margin).
- Apple `VapidPkHashMismatch`: the `k` key differs from the subscription's
  `applicationServerKey` (RFC 8292 §3.2: SHOULD be `400`/`403`). A lost or rotated VAPID key
  silently invalidates every subscription: the private key must persist (0600) and its loss
  means "re-enable notifications on the phone".

### 3.3 Protocol (RFC 8030) and push-service behaviour

- `POST <endpoint>` with body, `TTL` header **mandatory** (RFC 8030 §5.2; Apple `BadTtl` when
  missing or not positive; Apple stores up to 30 days at most depending on TTL).
- `Urgency`: `very-low`, `low`, `normal` (default), `high` (RFC 8030 §5.3; Apple: "To attempt
  to deliver the notification immediately, specify `high`"; `BadUrgency` otherwise).
- `Topic`: ≤ 32 characters of the base64url alphabet (observed 2026-10-06: Apple is stricter and refuses `-` with `400 BadWebPushTopic`, so soos uses ASCII letters and digits only); a newer message with the same topic
  replaces an undelivered older one (RFC 8030 §5.4; Apple `BadWebPushTopic`). A fixed topic
  for alerts coalesces on the push service while the phone is offline.
- Responses: `201 Created` success (RFC 8030 §5; Apple `201`). Apple: `400` bad request,
  `403` authentication error, `404` "invalid `:path`" (`BadPath`), `405` non-POST, `410`
  "The device token has expired" (`Unregistered`-class), `413` payload too large, `429`
  "too many consecutive requests to the same device token", `500`, `503`; the JSON body has a
  `reason` key; responses carry `apns-id`. Mozilla: `404` (errno 102) and `410` (errno
  103/105/106) mean the URL must never be used again; `401` bad VAPID; `413`; `429` and
  `503` with `Retry-After` / backoff. RFC 8030 §7.3 / §6.2: `404`/`410` for expired
  subscriptions. → Remove the subscription on `404` and `410`; keep it and stop retrying on
  `400`/`401`/`403`/`413` (configuration or code fault, audit event); bounded retry with
  backoff (respect `Retry-After`, capped) on `429`/`500`/`503`/timeouts.
- Redirects: RFC 8030 §7.1 lets a push service redirect **user agents** (307) for load
  balancing; nothing in RFC 8030, Apple or Mozilla docs describes redirects of message
  delivery to application servers. Refusing every redirect (treat 3xx as a failure) is
  therefore safe and closes an SSRF bypass.
- Apple transport: TLS with **SNI mandatory**, HTTP/1.1 (default) or HTTP/2 via ALPN, up to
  100 unacknowledged pipelined HTTP/1.1 requests, `IdleTimeout` on idle connections
  (Apple doc). One short-lived HTTP/1.1 connection per push is sufficient at this volume.

## 4. Push endpoints and SSRF facts

- Endpoint hosts by browser (Pushpad "What are the browser push services", Apple doc, Mozilla
  docs): Safari / iOS `https://web.push.apple.com/<token>` (Apple asks to allow
  `https://*.push.apple.com`); Chrome and Chromium Android `https://fcm.googleapis.com/fcm/send/...`
  (also `/wp/...`); Firefox `https://updates.push.services.mozilla.com/wpush/v2/...`;
  desktop Edge `https://*.notify.windows.com/...`. The owner only needs the iPhone, so the
  minimal allowlist is `web.push.apple.com` (exact host; widen to `*.push.apple.com` only if
  a real subscription shows another host).
- Host DNS (read-only `getent ahosts`, 2026-10-06): `web.push.apple.com` → CNAME
  `web-vs.push-apple.com.akadns.net`, `17.188.143.78`, `17.188.178.139` (Apple's 17.0.0.0/8);
  `fcm.googleapis.com` → `216.239.38.55`, `216.239.36.57`;
  `updates.push.services.mozilla.com` → Fastly `199.232.169.91`, `2a04:4e42:6a::347`.
  Addresses rotate (CDN / GeoDNS): an IP allowlist is not viable, only a host allowlist plus a
  "public address" check of the resolved addresses.
- The host resolver is Tailscale MagicDNS (`/etc/resolv.conf`: `nameserver 100.100.100.100`,
  `fd7a:115c:a1e0::53`, `search <tailnet>.ts.net ...`). Consequences: (a) DNS for the sender
  goes to a CGNAT/ULA address, so a network sandbox must still allow it; (b) the resolver can
  answer tailnet names with `100.64.0.0/10` / `fd7a:115c:a1e0::/48` addresses, which must be
  in the private-address deny list together with loopback, RFC 1918, link-local, ULA,
  multicast, unspecified, `0.0.0.0/8`, IPv4-mapped IPv6 and documentation ranges.
- Route: `ip route get 17.188.143.78` → `via <LAN gateway> dev <wlan>` (direct, not through an
  exit node at the time of the check); no IPv6 default route on this host, so IPv6 endpoints
  addresses must not be required (try IPv4 results).
- Known SSRF pitfall to avoid: a filter that applies IPv6 prefix tests (`fc`, `fd`, `ff`) to
  **host names** rejects `fcm.googleapis.com` (docs-plus/docs.plus#398). Checks must apply
  to parsed IP addresses only; the host name is checked against the allowlist by exact match.
- DNS rebinding: resolve once, validate every address, then connect to the validated address
  while sending SNI / `Host` with the allowlisted name (ureq's `Resolver` seam, §5.2).
- Endpoint validation on subscribe: `https` scheme only, no userinfo, no explicit port other
  than 443, host in allowlist, path length bounded (Apple tokens are well under 512 bytes),
  no fragment; `p256dh` must decode to a valid uncompressed P-256 point (on-curve check with
  `p256::PublicKey::from_sec1_bytes`), `auth` to exactly 16 bytes.

## 5. Rust dependencies, licences and the no-OpenSSL rule

### 5.1 Already locked (Cargo.lock)

- `p256 0.13.2` (workspace feature `ecdsa`: signing + verifying, RFC 6979 deterministic
  signatures via `rfc6979` / `hmac 0.12.1`), `elliptic-curve 0.13.8`, `ecdsa 0.16.9`,
  `sha2 0.10.9`, `hmac 0.12.1` (transitive), `aes-gcm 0.10.3` (used by biometric-store and
  evidence-store, `features = ["zeroize"]`), `getrandom 0.4` (crate CSPRNG), `base64ct 1.8`
  (base64url without padding: `Base64UrlUnpadded`), `subtle`, `zeroize`, `serde_json`.
- `ureq 3.4.2` is already locked, but only as a **build dependency** of `ort-sys` with the
  `native-tls` feature, which pulls `native-tls 0.2.18` → `openssl 0.10.81` (deny.toml skip
  reasons for `foreign-types`). With Cargo resolver 2, build-dependency features are not
  unified with normal-dependency features, so a normal dependency on `ureq` with
  `default-features = false, features = ["rustls"]` reuses the same version (no
  `multiple-versions` violation) and does **not** link OpenSSL into `soos-remote`.
- **No rustls, ring, webpki or hkdf crate is in the lock today.** `rustls-pki-types 1.15.1`
  is (ureq).

### 5.2 Candidates (crates.io metadata, 2026-10-06)

| Crate | Latest | Licence | MSRV | Note |
|---|---|---|---|---|
| `ureq` | 3.4.2 | MIT OR Apache-2.0 | 1.85 | blocking; feature `rustls` = `rustls-no-provider` + `_ring` + `rustls-webpki-roots`; default features `rustls`, `gzip` (disable gzip) |
| `rustls` | 0.23.45 | Apache-2.0 OR ISC OR MIT | 1.71 | ureq requires `^0.23.22` with `logging`, `std`, `tls12` |
| `ring` | 0.17.14 | Apache-2.0 AND ISC | 1.66 | C/asm build via `cc` (already locked 1.4.6; `cc`, `gcc`, `clang` present); RUSTSEC-2025-0009 fixed in 0.17.12 |
| `rustls-webpki` | 0.103.15 | ISC | 1.71 | |
| `untrusted` | 0.9.0 | ISC | — | |
| `webpki-roots` | 1.0.9 | CDLA-Permissive-2.0 | 1.70 | compiled-in Mozilla roots; licence already in `deny.toml` allow list |
| `rustls-native-certs` | 0.8.4 | Apache-2.0 OR ISC OR MIT | 1.71 | alternative: read `/etc/ssl/certs/ca-certificates.crt` |
| `rustls-platform-verifier` | 0.7.1 | MIT OR Apache-2.0 | 1.85 | heavier; not needed |
| `aws-lc-rs` / `aws-lc-sys` | 1.18.1 / 0.45.0 | ISC AND (Apache-2.0 OR ISC) [and more for -sys] | 1.71 | rustls's default provider; needs CMake, vendors an OpenSSL-derived C library; avoid (use the `ring` provider) |
| `hkdf` | 0.12.4 (0.13.0 needs `digest 0.11`) | MIT OR Apache-2.0 | — | 0.12 matches `hmac 0.12` / `sha2 0.10`; `p256` feature `ecdh` → `elliptic-curve/ecdh` pulls `hkdf 0.12` |
| `web-push` | n/a | — | — | not evaluated: crates.io API returned no record for the query; existing crates pull `isahc`/`hyper` + OpenSSL; in-house is ~200 lines over the crates above |

- All licences above are in `deny.toml`'s allow list (`MIT`, `Apache-2.0`, `ISC`,
  `CDLA-Permissive-2.0`); `cargo deny check` must still be run by the developer after the
  change (`[graph] all-features = true`, `multiple-versions = "deny"`: ring 0.17.14 uses
  `getrandom 0.2`, already skipped in `deny.toml`; its reason text must then name ring too).
- ECDH options: (a) enable `p256` feature `ecdh` (`p256::ecdh::diffie_hellman`,
  `SharedSecret::raw_secret_bytes`; adds `hkdf 0.12.4`); (b) without new crates:
  `(PublicKey::to_projective() * NonZeroScalar).to_affine().x()` with the `arithmetic`
  feature already enabled, HKDF written as the five HMAC calls of §3.1 over `hmac 0.12`
  (promoted to a direct dependency). Both are pure Rust RustCrypto.
- Randomness: `p256::SecretKey::random` wants a `rand_core 0.6` RNG; the crate's single
  CSPRNG seam is `getrandom 0.4`, so draw 32 bytes and use `SecretKey::from_slice` /
  `NonZeroScalar::from_repr` (retry on the negligible zero / ≥ n case) and 16 bytes of salt
  from the same seam; this also gives the test seam for the RFC 8291 vector.
- `ureq 3.4.2` API facts (local registry source `src/config.rs`, `src/agent.rs`):
  `Config::builder().https_only(true)`, `.max_redirects(0)`, `.timeout_global(Some(..))`,
  `.max_response_header_size(..)`, `.http_status_as_error(false)`, `.proxy(None)` — the
  default config calls `Proxy::try_from_env()` (`config.rs:948`), so proxy environment
  variables are honoured unless disabled explicitly; `Agent::with_parts(config, connector,
  resolver)` with the `unversioned::resolver::Resolver` trait allows a resolver that
  validates addresses (marked *unversioned*: may change in a minor ureq release; pin with
  `=3.4.2` or wrap behind a small module). ureq is blocking: in the current-thread Tokio
  `soos-remote` it would need `spawn_blocking`; in a separate sender process it is natural.

## 6. Process isolation facts (systemd user units)

- `soos-remote.service` (user unit) today: `RestrictAddressFamilies=AF_UNIX`,
  `NoNewPrivileges=yes`, `PrivateUsers=no` (`systemctl --user show`); `systemd-analyze --user
  security` rates it 8.0 "EXPOSED" (mostly options unavailable to user units) and confirms
  "Service cannot allocate Internet sockets".
- `RestrictAddressFamilies=` is a seccomp filter on `socket(2)` and is inherited by children
  (systemd.exec(5)); a child spawned by `soos-remote` cannot open `AF_INET` sockets, so an
  outbound sender cannot be a child process of the current unit: it must be either a
  **separate unit** or the main unit relaxed to `AF_UNIX AF_INET AF_INET6`.
- `IPAddressAllow=` / `IPAddressDeny=` are cgroup eBPF filters; they "might not be supported
  on some systems ... These settings will have no effect in that case" (systemd.resource-control(5));
  for **user** managers they are not available because "the kernel restricts [eBPF] to
  privileged processes, which means --user systemd will have a hard time" (L. Poettering,
  systemd-devel 2022, marc.info `m=167122362810659`). So no kernel IP firewall is available
  to a user unit; the private-address and host allowlist checks must be done in-process. A
  **system** unit (root-installed, `DynamicUser=`) could use `IPAddressDeny=` for private
  ranges, but that contradicts the per-user, no-root install of `scripts/install_remote.sh`.
- Filesystem sandboxing (`ProtectHome=`, `ReadOnlyPaths=`, `InaccessiblePaths=`,
  `PrivateTmp=`, capabilities options) in a user unit implicitly enables `PrivateUsers=`,
  which needs unprivileged user namespaces (systemd.exec(5)); available here
  (`kernel.unprivileged_userns_clone = 1`). Caveat for the existing unit: inside
  `PrivateUsers=` the supplementary group `wheel` is unmapped, which would break the level-1
  journal reading; such options must therefore go on a **separate sender unit only**, never
  on `soos-remote.service`.
- `PrivateNetwork=` would remove the network entirely (not usable for the sender).
- Separate-sender sketch the facts support: user unit `soos-remote-push.service`,
  `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`, `NoNewPrivileges`,
  `MemoryDenyWriteExecute` (ring needs no JIT), `ProtectHome=read-only` +
  `ReadWritePaths=` of its own state only (credential store of soos-remote unreadable),
  listening on a `0600` Unix socket in the same `0700` runtime directory; `soos-remote` keeps
  `AF_UNIX` only and sends it bounded, already-authorized requests. Same UID: the isolation
  is against bugs and parser exploits in the TLS/HTTP stack, not against a malicious owner
  process (which already has the owner's rights, as in level 1's accepted risks).

## 7. Journal facts re-checked for the trigger (read-only, counts only)

- `journalctl` 262; the owner is in `wheel`; the level-1 probe conditions still hold.
- Last 7 days, `SYSLOG_FACILITY=10 _COMM=unix_chkpwd`: 9 entries with `_UID=0` and 9 with
  `_UID=1000`, all `_TRANSPORT=syslog` (no message, user name or other field printed). The
  trusted-field model of research_alerts.md §2–§4 is unchanged; level 2 consumes level-1
  `Attempt` records and needs no new journal field.
- Residual spoofing (O-3) carries over: a process running as the owner can forge
  lock-screen-looking lines and therefore trigger **false push notifications**; with rate
  limiting and coalescing it can at most cause bounded notification noise, never read data.

## 8. Facts the design must not assume (owner checks)

1. The owner's iOS version (≥ 16.4 required; ≥ 18.4 gives declarative fallback).
2. That Apple accepts the chosen VAPID `sub` (Funnel origin or repository URL): first test
   notification from the phone, `201` vs `403 BadJwtToken`.
3. The actual endpoint host of the owner's subscription (expected `web.push.apple.com`).
4. Notification preview setting on the iPhone ("Show Previews"), Focus filters.
5. That the sender unit starts and reaches Apple from the owner's network (agents never
   start units or send real pushes).
