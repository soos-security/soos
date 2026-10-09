# Research: Live Camera View in `soos-remote` (iPhone home-screen web app)

- **Date**: 2026-10-07
- **Branch**: `feat/remote-live-camera` (GitHub #345)
- **Scope**: research only. No code, no ADR, no configuration change. Feeds the architect spec for
  the agreed direction: `soos-remote` fetches frames through the existing daemon
  `RequestKind::PreviewFrame` IPC (ADR 2026-09-29 "Camera Preview Stream Exception", GitHub #143),
  encodes JPEG in pure Rust and serves a stream to the home-screen web app; off by default, double
  opt-in, fresh UV passkey assertion per view, push notification on view start, bounded duration /
  fps / resolution, one concurrent view, no recording, no pixel logging, zeroized buffers, audit log.
- **Host commands run (read-only)**: `cat /run/systemd/sessions/*`, `loginctl list-sessions`,
  `loginctl show-session|show-user` (read properties), `id willi363`, `stat`/`ls` of `/run/soos`
  and `/etc/soos`, `cat /etc/soos/daemon.toml`, `cat ~/.config/soos/remote.toml`,
  `systemctl [--user] show` (properties only), `/proc/<pid>/status` of `soos-remote`,
  `v4l2-ctl --list-formats-ext` (format enumeration, no streaming, no control change),
  `journalctl -u soos-daemon` (format line only), `pacman -Q`, `rustc --version`, `cargo tree`.
  Nothing was installed, started, restarted or configured; `tailscale` was not run.
- **Status of each claim**: **[code]** read in this repository (file:line), **[host]** observed on
  the owner's host on 2026-10-07, **[src]** third-party source, **[doc]** vendor documentation,
  **[report]** community report, **[est]** estimate, **[rec]** recommendation,
  **owner check** = cannot be verified by an agent.

## 1. Versions and sources

| Component | Value | Where checked |
|---|---|---|
| systemd / Tailscale | `262-1` / `1.102.4-1` (CLI not run) | `pacman -Q` |
| Rust toolchain | `rustc 1.98.1 (2026-09-01)` | `rustc --version` |
| Camera | `/dev/v4l/by-id/usb-Azurewave_Integrated_Camera_SN0001-video-index0` ("Integrated Camera"), MJPG 320x180..1280x720 @30, YUYV 640x480/640x360/320x240 @30, 960x540 @20, 1280x720 @10 | `v4l2-ctl --list-formats-ext`, sysfs `name` |
| Daemon capture | `640x480, Yuyv, stride 1280, sensor Rgb` | `journalctl -u soos-daemon` |
| `jpeg-encoder` | 0.7.1, `(MIT OR Apache-2.0) AND IJG`, MSRV 1.87 (crates.io) / 1.89 (repo `main`), deps only optional `wide`/`bytemuck` | https://crates.io/api/v1/crates/jpeg-encoder, https://github.com/vstroebel/jpeg-encoder |
| `image` | 0.25.10 already in `Cargo.lock` (via `arboard`/`eframe` of `soos-gui`, no `jpeg` feature), `MIT OR Apache-2.0`, MSRV 1.88 | `Cargo.lock:1712`, `~/.cargo/registry/.../image-0.25.10/Cargo.toml:14,46,105-108` |
| Go `httputil.ReverseProxy` | `FlushInterval` doc | https://pkg.go.dev/net/http/httputil#ReverseProxy |
| Tailscale serve proxy | `ipn/ipnlocal/serve.go` (`main`) | https://github.com/tailscale/tailscale/blob/main/ipn/ipnlocal/serve.go |
| Tailscale Funnel | docs | https://tailscale.com/docs/features/tailscale-funnel |
| WebKit MJPEG | webkit-dev 2015-04 thread, 2019-04 thread | https://lists.webkit.org/pipermail/webkit-dev/2015-April/027393.html, https://lists.webkit.org/pipermail/webkit-dev/2019-April/030605.html |

## 2. Daemon side: how `PreviewFrame` is served today

### 2.1 Authorization (`ConnectionDispatcher::handle_preview_request`)

Routed at `crates/daemon/src/dispatcher.rs:735`; checks in order, each refusal is a standard
`Response` (`ProtocolError`) with zero pixel bytes [code]:

1. `authorize_preview` (`crates/daemon/src/preview.rs:121-142`): root peer always allowed;
   otherwise `peer_uid == uid_hint` (kernel `SO_PEERCRED` vs payload), `[preview] enabled`, and
   `peer_uid ∈ allowed_uids` (≤ 64 entries, `preview.rs:20,64-72`). Refusal → `UidMismatch`
   (`dispatcher.rs:1100-1115`).
2. Session check for non-root peers: `SessionValidator::is_active_session(peer_uid)`
   (`dispatcher.rs:1118`), which reads `/run/systemd/sessions/<id>` files (not D-Bus) and accepts
   any record with `UID == uid`, `ACTIVE=1` **or** `STATE=active`, and `REMOTE != 1`
   (`crates/daemon/src/session_policy.rs:72-101,127-129`, `session.rs:8-11,169`). It does **not**
   check `CLASS`, `SEAT` or `LockedHint`. Disabled when `enforce_active_session = false`
   (`dispatcher.rs:195-199`).
3. Per-peer-UID rate limit `[preview] max_requests_per_sec` (default 40 per 1 s sliding window,
   ≤ 64 tracked UIDs, root included; `preview.rs:26-32`, `dispatcher.rs:1150-1166`) →
   `RateLimited`.

**Locked session**: logind does not serialize `LockedHint` into the session file (already recorded
in ADR 2026-10-02 presence item (1), "verified on systemd 262"), and locking does not change
`ACTIVE`/`STATE`. A locked local session therefore **still counts as active** [code + host:
session 4 has `ACTIVE=1 STATE=active REMOTE=0`, `LockedHint=no` only visible over D-Bus]. **Not a
blocker.**

**Finding (looser than the ADR wording)** [host]: the owner has two sessions:
`1` = `CLASS=manager` (`SERVICE=systemd-user`, no `SEAT`, `ACTIVE=1 STATE=active REMOTE=0`) and
`4` = `CLASS=user`, `SEAT=seat0`, `TYPE=wayland`, `SERVICE=gdm-password`. `loginctl show-user`
reports **`Linger=yes`**. Because the preview check ignores `CLASS`/`SEAT`, the user-manager
session alone satisfies it; with linger, the manager session (and `soos-remote`) survives logout,
so a remote view could light the camera while **nobody is logged in at the desk**.
Owner check: after a full logout with linger on, does record `1` keep `STATE=active`? (Not tested:
it needs a logout.) Either way, the remote feature should not rely on this check meaning
"someone is logged in locally".

### 2.2 What the response contains

- `PreviewResponse { version, sequence: u64, width: u32, height: u32, format: u8,
  timestamp_monotonic_ns: u64, data: Vec<u8> }` (`crates/protocol/src/types.rs:268-285`). Format
  codes 0 RGB24, 1 Grey, 2 YUYV, 3 NV12, 4 MJPEG, 255 empty. **No stride field**: the data is the
  frame buffer as captured, so a packed layout is assumed (on this host YUYV 640x480 stride 1280 =
  packed, 614 400 bytes) [code + host].
- It is the **latest frame already captured** by the capture thread (`pipe.camera.latest_frame()`,
  `dispatcher.rs:1209`), not a dedicated capture; frames whose payload fits
  `MAX_PREVIEW_PIXEL_BYTES = 2 MiB - 1024` and width ≤ 640 (or any MJPEG) are forwarded unchanged,
  larger ones are decimated to ≤ 640 px (Grey stays Grey, colour → RGB24), IR-stamped frames are
  always sent as Grey (`crates/daemon/src/preview_image.rs:28-40,83-110`; ADR 2026-09-30 #196).
- `MAX_PREVIEW_MESSAGE_SIZE = 2 MiB` (`types.rs:21`); inbound frames are bounded by
  `MAX_MESSAGE_SIZE = 4096` (`types.rs:17`).
- Zeroization: `PreviewResponse` zeroizes on drop (`types.rs:287-304`), `PreviewImage` too
  (`preview_image.rs:67-71`), the encoded copy is `Zeroizing` (`dispatcher.rs:1180,1229-1237`).
  The daemon logs a served preview only at `debug` with the byte count (`dispatcher.rs:1239-1243`):
  **there is no info-level audit line on the daemon side** for preview frames.

### 2.3 Camera wake, idle and sharing

- After authorization, every preview request calls `camera.notify_activity()` and waits for
  readiness up to `min(connection_timeout - 100 ms, 1000 ms)` (`dispatcher.rs:1187-1198`); a
  frame not yet available yields `format = 255`. **A preview request wakes the camera.**
- Capture defaults (`crates/camera-v4l/src/config.rs:51-58,75-91`): 640x480, `fps = 30`,
  `idle_fps = 5` after half of `idle_timeout`, `idle_timeout = 10 s` (daemon key
  `[pipeline] idle_timeout_secs`, `Docs/DAEMON.md:79`), then standby (device released, LED off).
  While a viewer polls, the camera stays at full rate; after the last request it streams ≈ 5 s at
  30 fps, ≈ 5 s at 5 fps, then stops. Format priority RGB24 → YUYV → NV12 → MJPEG → Grey
  (`crates/camera-v4l/src/v4l_impl.rs:566-574`), hence YUYV on this host.
- **Sharing**: there is one capture thread and one `latest_frame` slot; PAM `Auth`, presence
  auto-unlock (`crates/daemon/src/presence/worker.rs:633-635`) and preview all read it. A preview
  never takes the camera from PAM or presence and consumes neither the inference slot nor the
  `Auth` rate limit. While the session is locked, presence scans every 2 s anyway, so a remote view
  of a locked PC changes nothing for presence; presence may unlock the session if the owner's face
  appears, exactly as without a viewer.

### 2.4 Connection model limits that matter for a long-lived viewer

`Docs/DAEMON.md:121-131,169-186`: `[peer_limits] max_connections_per_uid = 2` (unprivileged),
`max_requests_per_connection = 1024`, `max_connection_lifetime_ms = 30000`; persistent loop with
an idle timeout of `connection_timeout_ms` (2500 ms). Consequences [rec]:

- A viewer must **reconnect at least every 30 s** (at 10 fps the 1024-request cap is reached after
  ≈ 102 s, so the lifetime binds first).
- **UID 1000 has only 2 daemon connections.** `swaylock-plugin` (the owner's locker, run as uid
  1000, `remote.toml` `lock_screen_programs`) runs `pam_soos.so` as uid 1000; if `soos-remote` holds
  one connection and `soos-gui` another, the lock screen's face request is refused at admission and
  falls back to the password (`PAM_IGNORE`, fail-safe but a usability regression). The viewer must
  hold **at most one** connection and never while a second is open (or use one short-lived
  connection per frame).
- The 40/s preview rate limit is **per UID and shared** with `soos-gui` (which polls at ≈ 30/s,
  `crates/gui/src/ipc_camera.rs:40`): GUI + remote view at 15 fps would exceed it; the viewer must
  treat `RateLimited` as back-off, not as an error that ends the view.

### 2.5 Client code reuse

`soos-gui`'s client (`crates/gui/src/ipc_camera.rs:244-312`) is ≈ 70 lines: `getrandom` nonce,
`Request { kind: PreviewFrame, uid_hint: own uid, service: "soos-gui", deadline: u64::MAX }`,
`soos_protocol::message::encode_request` (adds the client tag trailer), 4-byte length read,
bounded `Zeroizing` buffer, a `Response` refusal path with `matches_request` +
`check_freshness(CLOCK_MONOTONIC, MAX_RESPONSE_FUTURE_SKEW_NS)`, else `decode_preview`. There is
**no shared client crate**; the GUI's code is tied to `soos-camera-v4l` `Frame`/egui types and must
be duplicated (small). Nothing else is needed (no extra tag, nonce or key): the protocol needs only
`soos-protocol` + `getrandom` + a `CLOCK_MONOTONIC` read (`nix`, already a dependency).

**BLOCKER (contract)**: `tests/invariants/src/remote_companion_contract.rs:612-658`
(`test_rmc_s4_remote_is_a_leaf_crate`) forbids `soos-protocol` (and `soos-camera-v4l`,
`soos-vision`, …) in `crates/remote/Cargo.toml`. Re-implementing the postcard wire format by hand
inside `soos-remote` would dodge the letter of that test and of
`protocol_codec_contract::test_no_crate_decodes_wire_payloads_with_postcard_directly` but defeat
their intent (single codec). Any design needs either an owner-approved ADR amending RMC-S4 (an
immutable contract test change) or a separate crate/process (§8, options A/B). Also note
`Docs/REMOTE_COMPANION.md:776-787` lists "Live camera or any frame … access" as out of scope
needing its own ADR, and ADR 2026-09-29 says frames go "exclusively towards the diagnostic GUI".

## 3. Host facts

| Fact | Value [host] |
|---|---|
| Groups of `willi363` (uid 1000) | `1000, 998 wheel, 992 input, 989 lp, 957 docker, 944 soos` |
| Daemon socket | `/run/soos` `drwxr-x--- root:soos`, `daemon.sock` `srw-rw---- root:soos` → reachable by group `soos` |
| Running `soos-remote` (pid 796339) | `Groups: 944 957 989 992 998 1000`, `NoNewPrivs: 1`, seccomp filters active → **has group `soos`** |
| `/etc/soos/daemon.toml` (47 bytes, `0644 root:root`) | `[preview]` `enabled = true`, `allowed_uids = [1000]` (set for `soos-gui`; no other section) |
| `soos-daemon` | active, `User=root` |
| `soos-remote.service` | `~/.config/systemd/user/soos-remote.service`, identical to `packaging/soos-remote.service` |

**Unit sandbox** (`packaging/soos-remote.service`): `NoNewPrivileges`, `RestrictAddressFamilies=AF_UNIX`,
`LockPersonality`, `MemoryDenyWriteExecute`, `RestrictRealtime`, `RestrictSUIDSGID`,
`SystemCallArchitectures=native`, `UMask=0077`. No `ProtectSystem`, `ProtectHome`,
`InaccessiblePaths`, `TemporaryFileSystem`, `PrivateUsers` or `BindPaths` (ADR 2026-10-06 push
item (3): filesystem sandboxing would imply `PrivateUsers=` and break journal reading through
`wheel`). Connecting to `/run/soos/daemon.sock` is a `AF_UNIX` connect by a process holding group
`soos`: **allowed today, no unit change needed**. (A future `PrivateUsers=` would map `soos` to
`nobody` and break it; a future `TemporaryFileSystem=/run` would need `BindReadOnlyPaths=/run/soos`.)
The invariant `test_rmc_s5_s11_*` pins the current directives but does not forbid additions.

**Double opt-in is already half satisfied**: the daemon side was enabled for `soos-gui`, and the
daemon cannot tell `soos-gui` from `soos-remote` (same uid; `Request.service` is an unverified
payload string). The remote key therefore is, in practice, the only gate that distinguishes "local
diagnostic preview" from "remote view" unless the daemon gains a distinct authorization (§8, D2).

## 4. `soos-remote` internals relevant to the feature

- **HTTP**: hand-written HTTP/1.1 over a `0600` Unix socket (`crates/remote/src/http.rs`,
  `server.rs`); every response carries `Connection: close` and the mandatory headers
  (`http.rs:261-269`); bodies are written with `write_bounded` under
  `RESPONSE_WRITE_TIMEOUT_MS = 2000` per write (`lib.rs:80`, `http.rs:319-330`). The request parser
  **drops the query string** (`http.rs:150,385`; `routes.rs:120,133,289`), so a view token cannot
  travel in `?t=`; it must be a path segment, a header (fetch only) or server-side state bound to
  the caller.
- **Long-lived body**: already exists for SSE: `encode_sse_head()` writes a head without
  `Content-Length` (`http.rs:299-303`), the body is delimited by connection close. Bounds:
  `MAX_CONNECTIONS = 16`, `MAX_SSE_STREAMS = 4` (`sse_slots`, `503 too_many_streams`),
  `MAX_FUNNEL_SSE_STREAMS = 2` (`enter_funnel_stream`, `server.rs:1179-1188`),
  `MAX_SSE_STREAM_MS = 30 min`, keep-alive 15 s, Funnel sessions re-validated before each alerts
  event (`lib.rs:67-84`, `server.rs:1-32`). A `multipart/x-mixed-replace` or
  `application/octet-stream` streaming body fits the same pattern (new head encoder, own slot).
- **Fresh UV assertion**: `POST /api/auth/unlock/options` issues a challenge
  (`ChallengePurpose::Unlock`, bound to the caller class/identity, `CHALLENGE_TTL_MS = 120 s`,
  `server.rs:1329-1352`); `POST /api/unlock` requires `X-Soos-Action: unlock` CSRF, `allow_unlock`,
  `rp_id`, a body, lockout check, then `check_assertion(.., ChallengePurpose::Unlock, binding)` with
  UP and UV flags required (`webauthn.rs:206-222`), counter persistence, audit
  `remote unlock requested` (`server.rs:1465-1535`). The same shape gives a camera flow: add
  `ChallengePurpose::CameraView` (`challenge.rs:26-33`), `POST /api/auth/camera/options`,
  `POST /api/camera/start` (`X-Soos-Action: camera-view`) that, on a verified assertion, mints a
  single-use 32-byte view token (TTL ≈ 10 s, bound to class + identity/session hash), consumed by
  the stream request. A distinct purpose prevents replaying an unlock challenge for a view and
  vice versa.
- **Web sessions** (Funnel): cookie `__Host-soos_session`, `Path=/; Secure; HttpOnly;
  SameSite=Strict`, idle 15 min, absolute 8 h, ≤ 4 sessions (`lib.rs:203-215`). Same-origin
  `<img>`/`fetch` requests carry it; tailnet requests carry `Tailscale-User-Login` set by Serve.
- **Audit**: `crates/remote/src/audit.rs` = fixed-text events without fields, dispatched through
  static callsites (`audit.rs:1-15,185-239`); new lines would be e.g. `camera view started`,
  `camera view ended`, `camera view refused`.
- **Push**: `PushDispatcher` builds a `Message::Alert`/`Message::Test` into an encrypted
  Declarative Web Push payload with its own `Topic` (`push.rs:1440-1530`); a `Message::CameraView`
  with its own topic (letters/digits only, e.g. `sooscamera`, see the `400 BadWebPushTopic` lesson
  in ADR 2026-10-06 item (8)) and a fixed generic text is a small extension. Push is only
  available when `push_notifications = true`, which itself requires `password_alerts` and `rp_id`
  (`config.rs:401-421`).
- **Config**: `FileConfig` is `#[serde(deny_unknown_fields)]` (`config.rs:273`); opt-in keys are
  `Option<bool>` defaulting to `false` with cross-key validation errors (pattern: `push_notifications`
  → `PushRequiresAlerts`/`PushRequiresRpId`, `config.rs:239-244,415-421`). A `camera_view` key
  would follow this pattern (e.g. requires `rp_id`, and `push_notifications` if the start
  notification is mandatory). The owner's `remote.toml` currently has `allow_unlock`, `rp_id`,
  `allow_funnel`, `password_alerts`, `push_notifications` all set [host].
- **CSP** (`http.rs:262-264`, pinned by `tests/remote/*` `CSP` constants in
  `crates/remote/tests/http_tests.rs:24`, `server_tests.rs:93`, `common/harness.rs:62`):
  `img-src 'self'` allows a same-origin MJPEG `<img src="/api/camera/stream/…">`; `blob:` is
  **not** allowed, so a fetch → `URL.createObjectURL` → `<img>` fallback would need a CSP change
  (pinned by tests), while fetch → `createImageBitmap(blob)` → `<canvas>` needs none
  (`connect-src 'self'` covers the fetch; `createImageBitmap` is not governed by `img-src`).
  `app.js` must not use `innerHTML`, `eval`, absolute URLs, etc.
  (`remote_companion_contract.rs:1015-1060`).
- **Dependency policy** (`deny.toml`): licence allow-list MIT, Apache-2.0, BSD-2/3, ISC,
  Unicode-3.0/DFS-2016, Zlib, BSL-1.0, CC0-1.0, CDLA-Permissive-2.0, OFL-1.1, Ubuntu-font-1.0
  (`deny.toml:26-40`); `multiple-versions = "deny"`, `wildcards = "deny"` (`deny.toml:58-60`).
  **IJG is not allowed** today. `soos-remote` deps: tokio, nix, tracing, serde/serde_json, toml,
  clap, httparse, zbus, zeroize, p256, sha2, ciborium, getrandom, base64ct, subtle, hmac, aes-gcm,
  soos-push-protocol (`crates/remote/Cargo.toml:18-42`).

## 5. Browser and transport (external)

### 5.1 iOS Safari / standalone web app and `multipart/x-mixed-replace`

- WebKit removed MJPEG as a **main resource** but kept it for subresources: "Having an IMG element
  in a page whose src attribute points to a Motion JPEG would still work as intended" [report:
  webkit-dev 2015-04, URL in §1]. The same thread reports iOS 8 showing only the first frame for a
  main-resource MJPEG, and that a home-screen app closes the connection when the user leaves the
  app but keeps downloading while the app stays open on another view.
- 2019 webkit-dev thread: Safari (macOS and iOS) failed on MJPEG served with **chunked**
  transfer encoding [report]. With Tailscale Serve the phone talks HTTP/2 to `tailscaled` (no
  chunked framing on that hop); the soos-remote → Serve hop uses close-delimited HTTP/1.1 like SSE.
- No authoritative statement for iOS 17–26 was found; community reports (e.g. Home Assistant
  behind nginx) are mixed [report]. Memory growth of long-lived `<img>` MJPEG on iOS is not
  documented. **Owner check (spike)**: a 60 s test MJPEG at 5 fps behind the real Serve/Funnel,
  in the standalone app, watching (a) frames update, (b) memory in Settings → Safari/Web
  Inspector, (c) behaviour after backgrounding and returning.
- Backgrounding: iOS suspends a standalone web app's page shortly after it leaves the
  foreground; the connection stalls or is closed [report, plus the 2015 thread]. Server-side,
  the 2 s bounded write ends a stalled stream; client-side, `visibilitychange`/`pagehide` must
  clear `img.src` / abort the fetch (the page already handles `visibilitychange` for SSE).
- **Fallbacks**: (F1) `fetch()` with `ReadableStream` reader parsing a simple length-prefixed
  JPEG stream, decoding with `createImageBitmap(blob)` into a `<canvas>` (Safari ≥ 15 supports
  `createImageBitmap`; works with the current CSP, allows sending the view token in a header and
  `AbortController` to stop) — https://developer.mozilla.org/en-US/docs/Web/API/Streams_API/Using_readable_streams,
  https://developer.mozilla.org/en-US/docs/Web/API/Window/createImageBitmap. (F2) Periodic
  `GET /api/camera/frame/<token>` polling (one JPEG per request, 2–5 fps): most robust, one HTTP
  request per frame through Serve, simplest to bound.

### 5.2 Tailscale Serve / Funnel with long-lived responses

- Serve proxies with Go `httputil.ReverseProxy` (no explicit `FlushInterval`), dialing the Unix
  socket directly; backend transport has `IdleConnTimeout: 90 s`, no `ResponseHeaderTimeout`
  [src, serve.go]. Go doc: "The FlushInterval is ignored when ReverseProxy recognizes a response as
  a streaming response, or if its ContentLength is -1; for such responses, writes are flushed to
  the client immediately" [doc]. A close-delimited body has `ContentLength == -1`, so each part is
  flushed immediately; consistent with the SSE stream that already works on the owner's phone.
- Funnel: "Traffic sent over a Funnel is subject to non-configurable bandwidth limits" (no number
  published); TLS terminated by `tailscaled` on the PC; ports 443/8443/10000 [doc]. Every Funnel
  byte crosses Tailscale's relays; DERP-relayed throughput reports go down to ≈ 2 Mbit/s [report:
  https://www.ssdnodes.com/learn/tailscale-funnel-limits-and-ports]. A tailnet connection
  (direct WireGuard when possible) has no such limit. **Owner check**: measured fps over Funnel
  on 4G/5G.

## 6. Pure-Rust JPEG encoder

| Candidate | Licence vs `deny.toml` | `unsafe` | Tree | MSRV | Notes |
|---|---|---|---|---|---|
| `jpeg-encoder` 0.7.1 (default features, no `simd`) | `(MIT OR Apache-2.0) AND IJG` → **needs `IJG` added** to the allow-list (owner/ADR) | `#![cfg_attr(not(feature = "simd"), forbid(unsafe_code))]` [src lib.rs] | zero required deps (`wide`/`bytemuck` optional) | 1.87–1.89 ≤ 1.98.1 | ≈ 3.4 kLoC, 8.1 M downloads, updated 2026-07; accepts `ColorType::Ycbcr`/`Luma`/`Rgb`, 4:2:0 sampling, quality 1–100 |
| `image` 0.25.10 `default-features = false, features = ["jpeg"]` | `MIT OR Apache-2.0` ✓ | encoder files contain no `unsafe` [host grep]; the `jpeg` feature also pulls the **decoder** `zune-jpeg` (96 `unsafe` occurrences, SIMD) and `zune-core` | `image` + `bytemuck`, `byteorder-lite`, `moxcms`, `num-traits`, `zune-*` (all already in `Cargo.lock`, `zune-core` in two versions 0.5.1/0.5.3 in the registry: `multiple-versions` must be checked with `cargo deny`) | 1.88 | feature unification turns `jpeg` on for `soos-gui`'s `image` too; heavier API (`ImageBuffer`) |
| In-house baseline encoder (≈ 400–600 lines: fixed Annex K tables, scalar AAN DCT, 4:2:0, Huffman) | project licence | `forbid(unsafe_code)` | none | n/a | precedent: in-house WebAuthn; most review and test effort, no external supply chain |
| Forward camera MJPEG | n/a | n/a | none | n/a | the daemon forwards MJPEG frames unchanged (`preview_image.rs:101-110`), but capture is YUYV (priority order, §2.3); switching capture to MJPEG would make every `Auth`/presence frame go through a JPEG decode and change PAD input: **not recommended** |

Encode cost [est, to be benchmarked in Phase 4]: scalar baseline encoders spend most time in the
DCT and Huffman stages; for 640x480 4:2:0 (≈ 460 k samples) expect ≈ 3–10 ms per frame on a recent
laptop core, ≈ 1–3 ms at 320x240; at 10 fps 640x480 this is ≲ 10 % of one core. YUYV → YCbCr is
a byte shuffle (no colour matrix), Grey → `Luma` directly.

## 7. Bandwidth estimate [est]

Typical webcam JPEG sizes (indoor, sensor noise): 640x480 ≈ 25 KB at q60, ≈ 45 KB at q80;
320x240 ≈ 8 KB at q60, ≈ 14 KB at q80 (multipart overhead ≈ 100 B/frame, negligible).

| Resolution / quality | 5 fps | 10 fps | 15 fps |
|---|---|---|---|
| 640x480 q60 | 1.0 Mbit/s | 2.0 Mbit/s | 3.0 Mbit/s |
| 640x480 q80 | 1.8 Mbit/s | 3.6 Mbit/s | 5.4 Mbit/s |
| 320x240 q60 | 0.3 Mbit/s | 0.6 Mbit/s | 1.0 Mbit/s |
| 320x240 q80 | 0.6 Mbit/s | 1.1 Mbit/s | 1.7 Mbit/s |

A 2-minute view at 640x480 q70 10 fps ≈ 40 MB of mobile data. Latency: capture age ≤ 33 ms +
IPC (614 KB local copy, < 1 ms) + encode 3–10 ms + network (tailnet direct 20–80 ms, Funnel/DERP
100–300 ms) ≈ 0.1–0.4 s glass-to-glass, plus browser decode. Over Funnel at ≈ 2 Mbit/s only
320x240 or ≤ 5 fps 640x480 q60 is sustainable; the server must drop frames (always send the
newest) rather than queue, which the 2 s bounded write and "latest frame" polling give for free.

## 8. Threat analysis (remote camera specific)

| Threat | Effect | Mitigation [rec] |
|---|---|---|
| Stolen Funnel web session (cookie) | Today: status, lock, alerts, push registration | View needs a **fresh UV assertion** (as unlock); session alone gives nothing; token single-use, ≤ 10 s TTL, bound to session hash |
| Compromised tailnet account in `allowed_logins` | Reaches the page without Funnel | Same fresh UV assertion; passkey is device-bound or iCloud-Keychain synced: **iCloud account compromise = passkey compromise** (accepted, as for unlock) |
| Funnel exposure (public Internet) | Anyone can reach the login page | Option: `camera_view` tailnet-only by default, `camera_view_funnel = true` as separate opt-in; Funnel stream slots already capped at 2 |
| Silent surveillance of people at the desk (incl. third parties) | Privacy / legal (recording others) | Camera LED (owner check: Azurewave UVC LED is normally hardware-tied to streaming), push to all registered devices at start, audit line, hard max duration (e.g. 120 s) + cooldown, one concurrent view, no recording path in code, optional on-screen indicator is out of scope (needs desktop integration) |
| Push payload on the lock screen | Reveals that a view started | Fixed generic text ("Camera view started on your PC"), no image, own topic, short TTL; an attacker's own registered device also receives it, so the page should list devices (already does) |
| Viewing when nobody is logged in (linger + manager session, §2.1) | Camera on with no local user | Remote-side check through logind (D-Bus, already used by `soos-remote`) that a local seat session `Class=user` exists; or daemon-side tightening of the preview session check to `check_local_seat_session_of` (affects `soos-gui` only positively) |
| Loosened daemon `allowed_uids` | Any uid-1000 process can already pull frames (existing since `soos-gui` opt-in) | Unchanged local exposure; remote feature widens **network** exposure only. Option: distinct daemon key (`[preview] remote_view = true`) checked via the caller's cgroup (`app.slice/soos-remote.service`), **not** a security boundary (the user controls its own units), useful as an explicit second opt-in and daemon audit line |
| Denial of face unlock | Viewer occupies a uid-1000 daemon connection / preview quota | ≤ 1 daemon connection, reconnect ≤ 30 s, back off on `RateLimited`, fps ≤ 10 |
| Memory disclosure | Frames in RAM / swap | `Zeroizing` IPC buffer, YCbCr scratch and JPEG output; no file, no cache (`Cache-Control: no-store` already); copies inside the kernel socket buffers and `tailscaled` (Go GC) are outside our control (accepted) |
| Logging | Pixel or token leakage | No `tracing` macro with fields in the camera module (as `push.rs`), fixed-text audit only |
| Screenshot / screen recording on the phone | Out-of-band recording | Accepted, cannot be prevented by a web app |

## 9. Design consequences and recommendations

**Blockers / prerequisites**

1. **RMC-S4** forbids `soos-protocol` in `soos-remote` (§2.5). Options:
   - **A. Amend RMC-S4** (owner-approved contract migration + ADR): allow `soos-protocol` only;
     still forbid `soos-daemon`, `soos-camera-v4l`, `soos-vision`, ORT, v4l. Smallest code
     (duplicate ≈ 70 lines of the GUI client), one process. **Recommended**.
   - **B. Separate user unit `soos-camera-relay`** (pattern of `soos-push-sender`): depends on
     `soos-protocol` + encoder, talks to the daemon, serves JPEG to `soos-remote` over a `0600`
     socket under `$XDG_RUNTIME_DIR`; `soos-remote` stays protocol-free and relays bytes. Isolates
     the encoder, but doubles units, sockets and the attack surface for pixels in transit.
   - C. Hand-written postcard in `soos-remote`: rejected (duplicated codec, defeats the codec
     invariant's intent).
2. **ADR amendments**: ADR 2026-09-29 ("exclusively towards the diagnostic GUI"), ADR 2026-10-05
   item (5) and `Docs/REMOTE_COMPANION.md` §9 ("Live camera … out of scope").
3. **JPEG licence**: `jpeg-encoder` needs `IJG` in `deny.toml` (owner decision); otherwise
   `image` + `jpeg` (pulls `zune-jpeg` decoder with `unsafe` SIMD, check `multiple-versions`) or
   an in-house encoder.
4. **iOS MJPEG `<img>`**: not proven for current iOS standalone apps — owner spike before
   committing (§5.1).

**Options per decision**

- **D1 Transport to the phone**: (a) MJPEG `multipart/x-mixed-replace` in `<img>` — simplest, CSP
  unchanged, token must be a path segment; stop = clear `src`; (b) fetch + `ReadableStream` +
  `createImageBitmap` → canvas — robust control (headers, abort, fps display), CSP unchanged;
  (c) per-frame polling — most robust, highest per-frame overhead. **Rec**: (b) as primary, or
  (a) if the spike succeeds; keep (c) as fallback.
- **D2 Daemon gate**: (a) keep current `[preview]` (already enabled for uid 1000) + new remote key
  only; (b) add a daemon key `[preview] remote_view` keyed on the caller's unit cgroup (explicit
  second opt-in, daemon `info` audit line on first preview of a connection; not a security
  boundary); (c) tighten the preview session check to a local seat session (`CLASS=user`, `SEAT`,
  `REMOTE=0`) for every unprivileged peer. **Rec**: (c) in any case (closes the linger gap), (b) if
  the owner wants a real daemon-side opt-in.
- **D3 Remote config**: `camera_view = false` default; requires `rp_id`; `camera_view_funnel`
  (default `false`); bounded `camera_max_view_s` (default 60–120, max 300), `camera_fps` (1..=10,
  default 5), `camera_width` ∈ {320, 640} (default 640, downscale by 2 for 320), `camera_quality`
  (50..=85, default 70). Require `push_notifications = true` if the start notification is
  mandatory (owner decision), else best effort.
- **D4 Authorization flow**: `POST /api/auth/camera/options` (purpose `CameraView`) →
  `POST /api/camera/start` (CSRF `X-Soos-Action: camera-view`, UV assertion) → single-use token
  (32 bytes, TTL 10 s) → stream route; one global view slot (`409 view_in_progress`), cooldown
  (e.g. 10 s), `POST /api/camera/stop` or connection close ends it; the stream also ends at the max
  duration, on Funnel session expiry (re-validate every few seconds), and on 2 s write timeout.
- **D5 Daemon client**: one persistent connection, reconnect before 30 s / on idle close, never a
  second concurrent connection, poll at `camera_fps`, skip unchanged `sequence`, back off on
  `RateLimited`, treat `format = 255` as "camera waking"; accept formats 1 (Grey) and 2 (YUYV),
  and 0 (RGB24) for decimated frames; refuse others (no decoder in `soos-remote`).
- **D6 Notifications and audit**: push "camera view started" (generic, topic `sooscamera`) to all
  subscriptions at start; audit `camera view started` / `camera view ended` /
  `camera view refused`; nothing else logged.
- **D7 Owner checks**: iOS spike (§5.1); Funnel throughput; camera LED behaviour while streaming
  and for the ≈ 10 s idle tail; `STATE` of the manager session after logout with linger; whether
  the owner wants Funnel access at all for the camera.
