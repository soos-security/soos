# Architect Spec — GitHub #345: Live Camera View in `soos-remote` Through the Daemon Preview Channel

- **Date**: 2026-10-07
- **Round**: 2. Revises round 1 after `AI/plan_evaluator_report.md` (REVISION_REQUIRED: MAJOR F1–F3, MINOR F5,
  F6, F8–F12, wording). Every finding is resolved in place; §19 maps each finding to the changed sections and tests.
  Orchestrator resolutions applied: Q-1 = S-3 (runtime-built card, no RMC-S50 migration); D-1 resolved by the ADR
  amendment of item (7); D-2 resolved by the ADR amendment of item (4a) (`REMOTE=0`).
- **Branch**: `feat/remote-live-camera` (GitHub #345). Merged only with the owner's explicit go. Agents never
  commit, push, install, restart host services, run `tailscale`, or touch `/etc/soos` or `~/.config`.
- **Binding inputs**: ADR "[2026-10-07] Live Camera View in `soos-remote` Through the Daemon Preview Channel"
  (`AI/DECISIONS.md`, owner decisions LC-1 to LC-4; the ADR text is authoritative), research note
  `AI/research_live_camera.md` (§9 options; the ADR picked D1(b), D2(a)+(b)+(c), D3, D4, D5, D6), GitHub #345.
- **Orchestrator defaults (binding for this spec)**: maximum view 120 s (cap 300), 5 fps (1..=10), width 640 or 320
  (2x downscale), JPEG quality 70 (50..=85), one global view slot, a cooldown between views, push best effort (never
  blocks the view), no recording.
- **Code read**: `crates/remote/src/{lib,server,routes,http,challenge,auth,config,push,audit,webauthn}.rs`,
  `crates/remote/assets/{app.js,index.html,sw.js}`, `crates/daemon/src/{dispatcher,preview,preview_image,session,
  session_policy,config,main}.rs`, `crates/gui/src/ipc_camera.rs`, `crates/protocol/src/{types,codec,message}.rs`,
  `tests/invariants/src/remote_{companion,passkey,push,brand}_contract.rs`, `tests/invariants/src/daemon_docs_contract.rs`,
  `crates/daemon/tests/preview_authorization_tests.rs`, `deny.toml`, `packaging/soos-remote.service`,
  `scripts/install_remote.sh`, `Docs/{REMOTE_COMPANION,DAEMON}.md`.
- **Matrix**: new rows `RLC1`–`RLC16` (§14). **Walkthrough**: `AI/walkthroughs/191_remote_live_camera.md`.
- **Test prefixes**: `test_rlc_` (behaviour), `test_rlc_s<N>_` (static invariants, new file
  `tests/invariants/src/remote_camera_contract.rs`).
- **Backlog**: #345 is a GitHub-only issue like #339 (no `AI/BACKLOG.md` entry); the branch is **not** registered in
  `BRANCH_TO_ISSUE` (`scripts/sync_issue.py`), the PR body says `Closes #345`.

---

## 0. Owner decisions and spec-level decisions

### 0.1 Owner decisions (restated, not reopened)

| Id | Decision |
|---|---|
| LC-1 | `soos-remote` may depend on `soos-protocol`, and on no other soos crate; RMC-S4 is migrated with the owner's approval to allow exactly that crate. |
| LC-2 | JPEG in pure Rust with `jpeg-encoder` (default features, no `simd`); `deny.toml` allows `IJG` through a crate-scoped exception for `jpeg-encoder` only. No image decoder in `soos-remote`. |
| LC-3 | Daemon: (a) preview session check tightened to a local seat session for **every** unprivileged peer; (b) new `[preview] remote_view` (default `false`) keyed on the peer cgroup `soos-remote.service`; one `info` line without pixels on the first frame of a remote connection. Not a security boundary. |
| LC-4 | `camera_view` (default `false`, requires `rp_id`); tailnet only unless `camera_view_funnel = true` (requires `allow_funnel`). Fresh UV passkey assertion with purpose `CameraView` per view on every path. |

### 0.2 Spec-level decisions (to be restated in the walkthrough; none contradicts the ADR)

| Id | Decision | Rationale |
|---|---|---|
| S-1 | Module layout: four flat files `camera.rs` (runtime + view loop), `camera_slot.rs` (pure state machine + token), `camera_jpeg.rs` (pure conversion + encoder), `camera_ipc.rs` (daemon client). | Crate is flat; pure parts testable without I/O. |
| S-2 | The stream answers its `200` head **only after the first JPEG is ready** (≤ `CAMERA_FIRST_FRAME_TIMEOUT_MS`); a daemon refusal / no frame / unsupported format before that is a normal JSON error. | The page can show *why* a view failed; `started` audit and push mean "pixels were shown". |
| S-3 | The view UI is built at runtime by `app.js` (`document.createElement`, no `id`), appended after `#push`; `index.html` gains no id. | `test_rmc_s50_ui_contract_ids_labels_and_attributes_are_kept` pins exactly 29 ids in `index.html` and the `getElementById` set of `app.js`. No contract migration needed. Alternative (static markup) needs an owner-approved migration of RMC-S50, see §16 Q-1. |
| S-4 | `Docs/REMOTE_COMPANION.md` §9 keeps one line containing "live camera" (recording, snapshots and any other live-camera transport stay out of scope). | `test_rmc_s39_push_is_documented` asserts §9 still names the live camera; `test_rmc_s7_*` needs the phrase. No migration. |
| S-5 | The camera challenge shares the **limiter keys** of unlock (`LimitKey::Tailnet` / `LimitKey::FunnelSession`): one lockout and one options budget for every privileged ceremony. Its **challenge pool** is separate. | A camera failure must count against the same 5-failures lockout as unlock. |
| S-6 | Daemon peer classification is computed once per connection, lazily on its first `PreviewFrame`; an unreadable or malformed cgroup refuses the preview (fail closed). | ADR LC-3b; cheap (≤ 16 KiB read per connection). |
| S-7 | Seat predicate = the root-peer `Auth` predicate, reused as is: `SessionRecord::is_local_seat_session_of(uid)` is defined as `self.check_local_seat_session_of(uid).is_ok()` (`UID == uid`, active, `REMOTE=0` exactly — absent or malformed `REMOTE` refuses —, non-empty `SEAT`, `CLASS=user`). No second predicate. | Amended ADR item (4a) (D-2, round 2, F1); one source of truth. |
| S-8 | Wire format codes become `pub const` in `soos-protocol` (`PREVIEW_FORMAT_*`); the daemon's `PREVIEW_FORMAT_EMPTY` and the GUI's private copy become aliases of it. | One source of truth for the new consumer. |
| S-9 | A refused-view audit line is rate-gated (one per `CAMERA_REFUSED_AUDIT_MIN_INTERVAL_MS`). | An authenticated caller could otherwise flood the journal with bad tokens. |
| S-10 | Conversion + JPEG encoding run in `tokio::task::spawn_blocking` (the service runs a current-thread runtime). A panic ends the view (`JoinError` → `Internal`). | Keeps every other connection responsive; at most one blocking task (one view). |

---

## 1. Scope & blast radius

### 1.1 Crates, modules, files

| Area | Change |
|---|---|
| `crates/protocol/src/types.rs` | + `PREVIEW_FORMAT_RGB24/GREY/YUYV/NV12/MJPEG/EMPTY` constants (§3.1). No wire change. |
| `crates/daemon/src/preview.rs` | `PreviewConfig.remote_view: bool` (default `false`). |
| `crates/daemon/src/preview_peer.rs` (new) | `PreviewPeerOrigin`, `PreviewPeerError`, `REMOTE_COMPANION_UNIT`, pure `classify_preview_peer_cgroup`. |
| `crates/daemon/src/session_policy.rs` | `SessionRecord::is_local_seat_session_of`. `systemd_hierarchy_path`/`parse_strict_uid` become `pub(crate)` for reuse. |
| `crates/daemon/src/session.rs` | `SessionValidator::has_local_seat_session(uid)`. `is_active_session` kept (used by tests only after the change). |
| `crates/daemon/src/dispatcher.rs` | preview gate order (§9.3), per-connection `PreviewPeerState`, `with_preview_peer_source` builder, new log lines. |
| `crates/daemon/src/preview_image.rs` | `PREVIEW_FORMAT_EMPTY` aliases the protocol constant. |
| `crates/daemon/src/config.rs`, `main.rs` | `[preview] remote_view` parsing; start-up info line includes it. |
| `crates/gui/src/ipc_camera.rs` | private `PREVIEW_FORMAT_EMPTY` replaced by the protocol constant (no behaviour change). |
| `crates/remote/Cargo.toml` | + `soos-protocol = { workspace = true }`, + `jpeg-encoder = { workspace = true }`, `nix = { workspace = true, features = ["time"] }`. |
| root `Cargo.toml` | `[workspace.dependencies]` + `jpeg-encoder = { version = "=0.7.1" }` (default features = `std` only; never `simd`). |
| `crates/remote/src/lib.rs` | constants §3.2 + compile-time relations; `pub mod camera; camera_ipc; camera_jpeg; camera_slot;`. |
| `crates/remote/src/config.rs` | `CameraConfig`, `CameraWidth`, 6 keys, 7 `ConfigError` variants. |
| `crates/remote/src/challenge.rs` | `ChallengePurpose::CameraView`, two pool entries. |
| `crates/remote/src/routes.rs` | 5 routes, `camera_stream_token`, `check_camera_csrf`, `allow_header`/`accepts_body`/`is_funnel_public`. |
| `crates/remote/src/http.rs` | `encode_camera_stream_head`, `encode_camera_part_head`, `CAMERA_STREAM_TRAILER`; `accepts_body` doc. |
| `crates/remote/src/server.rs` | `ServerState::with_camera`, `Shared.camera`, route dispatch, `camera_options`, `camera_start`, `camera_stream`, `camera_stop`, `camera_view_response`. |
| `crates/remote/src/push.rs` | `Message::CameraView`, `camera_queued`, `queue_camera_view`, `camera_payload`, `send_camera`. |
| `crates/remote/src/audit.rs` | `camera_view_started` (INFO), `camera_view_ended` (INFO), `camera_view_refused` (WARN). |
| `crates/remote/src/main.rs` | wires `DaemonPreviewClient` factory when `camera.enabled`. |
| `crates/remote/assets/app.js`, `style.css`, `sw.js` | camera card (runtime DOM), canvas reader; `.camera*` styles from the palette; `kind === "camera"` tag. |
| `deny.toml` | `[licenses] exceptions = [{ crate = "jpeg-encoder", allow = ["IJG"] }]`. |
| `scripts/install_remote.sh` | commented template keys + printed `daemon.toml` snippet (never written). |
| Docs | `Docs/REMOTE_COMPANION.md` (§1, §2f new, §5, §6, §8, §9), `Docs/DAEMON.md` (§1.5, §3 table, session wording), `Docs/IPC_PROTOCOL.md` (preview consumers, format constants), `Docs/GUI_APPLICATION.md` (seat-session requirement), `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` (IJG exception, preview consumers), `.claude/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md` §13 (traceability phase). |
| Tests | new files §13; migrations §12. |

`packaging/soos-remote.service` is **unchanged**: connecting to `/run/soos/daemon.sock` is an `AF_UNIX` connect by a
process in group `soos` (verified on the host, research §3). Pinned by RMC-S5/S11/S26/S35, so a change would fail.

### 1.2 Consumers of changed public items

- `PreviewConfig` gains a field: every struct literal without `..PreviewConfig::default()` breaks
  (`crates/daemon/tests/preview_authorization_tests.rs:508`, §12).
- `RemoteConfig` gains `camera: CameraConfig`: four test literal sites (§12).
- `Route` and `ChallengePurpose` gain variants: `server.rs` matches `Route` exhaustively (compile-checked);
  `challenge::pool_of` has a `_ => None` arm (extend explicitly).
- `PREVIEW_FORMAT_EMPTY` (daemon) stays exported with the same value; `preview_downscale_tests` and GUI tests unaffected.

### 1.3 Out of scope (each needs its own ADR)

Recording, snapshots/still images, audio, H.264/WebRTC, an on-screen desktop indicator, viewing after a full
logout, NV12/MJPEG support in `soos-remote`, any change to the soos-remote unit sandbox.

---

## 2. Architecture overview

```
iPhone page ──fetch(POST /api/auth/camera/options)──▶ soos-remote ──challenge(CameraView)
            ──navigator.credentials.get (UV)
            ──fetch(POST /api/camera/start, assertion)──▶ verify → slot Pending(token, 10 s) → 200 {stream_path}
            ──fetch(GET /api/camera/stream/<token>, X-Soos-Action: camera-stream)──▶ slot Starting
                                         │ one tokio UnixStream ──PreviewFrame──▶ soos-daemon
                                         │   (gate: authorize_preview → cgroup origin → remote_view → seat → rate)
                                         │◀── PreviewResponse (Grey/YUYV/RGB24, ≤ 640x480)
                                         │ spawn_blocking: convert (+2x) → JPEG (bounded sink)
            ◀── 200 multipart/x-mixed-replace; boundary=soosframe ── part, part, ... (≤ fps, ≤ max_view)
  reader.read() → parse Content-Length parts → createImageBitmap(Blob) → canvas.drawImage
```

---

## 3. Constants

### 3.1 `soos-protocol` (`crates/protocol/src/types.rs`, the only definition)

```rust
/// Preview wire format codes of `PreviewResponse::format` (ADR 2026-09-29, 2026-10-07).
pub const PREVIEW_FORMAT_RGB24: u8 = 0;
pub const PREVIEW_FORMAT_GREY: u8 = 1;
pub const PREVIEW_FORMAT_YUYV: u8 = 2;
pub const PREVIEW_FORMAT_NV12: u8 = 3;
pub const PREVIEW_FORMAT_MJPEG: u8 = 4;
/// Explicit empty preview (no frame ready, or not convertible).
pub const PREVIEW_FORMAT_EMPTY: u8 = 255;
```

Daemon: `pub const PREVIEW_FORMAT_EMPTY: u8 = soos_protocol::types::PREVIEW_FORMAT_EMPTY;` (same name, same value;
`wire_format_code` may keep its literals or use the constants — values must be identical; test 49 pins them).

### 3.2 `soos-remote` (`crates/remote/src/lib.rs`, new section "Live camera view")

| Constant | Value | Meaning / bound |
|---|---|---|
| `MAX_CAMERA_VIEWS` | `1` | global view slots |
| `DEFAULT_CAMERA_MAX_VIEW_S` / `MIN_` / `MAX_` | `120` / `10` / `300` | `camera_max_view_s` |
| `DEFAULT_CAMERA_FPS` / `MIN_` / `MAX_` | `5` / `1` / `10` | `camera_fps` |
| `CAMERA_FULL_WIDTH` / `CAMERA_HALF_WIDTH` | `640` / `320` | accepted `camera_width` values |
| `DEFAULT_CAMERA_QUALITY` / `MIN_` / `MAX_` | `70` / `50` / `85` | `camera_quality` |
| `CAMERA_MIN_SOURCE_DIM` | `16` | smallest accepted source width/height |
| `CAMERA_MAX_SOURCE_WIDTH` / `_HEIGHT` | `640` / `480` | largest accepted source frame |
| `CAMERA_MAX_SCRATCH_BYTES` | `640 * 480 * 3` = 921 600 | largest converted buffer |
| `MAX_CAMERA_JPEG_BYTES` | `524_288` | JPEG sink capacity; never grows |
| `CAMERA_VIEW_TOKEN_BYTES` | `32` | CSPRNG token |
| `CAMERA_VIEW_TOKEN_B64_LEN` | `43` | unpadded base64url length |
| `CAMERA_VIEW_TOKEN_TTL_MS` | `10_000` | start → stream request |
| `CAMERA_VIEW_COOLDOWN_MS` | `10_000` | after a view that showed pixels |
| `CAMERA_FIRST_FRAME_TIMEOUT_MS` | `5_000` | stream request → first JPEG (else JSON error) |
| `CAMERA_STALL_TIMEOUT_MS` | `5_000` | no new frame sent for this long → view ends |
| `CAMERA_SESSION_CHECK_MS` | `5_000` | Funnel web-session re-validation period |
| `CAMERA_MAX_BAD_FRAMES` | `10` | consecutive geometry/encode failures → view ends |
| `CAMERA_REFUSED_AUDIT_MIN_INTERVAL_MS` | `5_000` | at most one `camera view refused` line per interval |
| `CAMERA_DAEMON_SOCKET_PATH` | `"/run/soos/daemon.sock"` | daemon socket (never configurable) |
| `CAMERA_DAEMON_SERVICE` | `"soos-remote"` | `Request.service` (≤ `MAX_SERVICE_LEN`) |
| `CAMERA_DAEMON_CONNECT_TIMEOUT_MS` | `500` | connect + peer-credential check |
| `CAMERA_DAEMON_IO_TIMEOUT_MS` | `3_000` | one exchange (write + read); > daemon wake 1000 ms + processing |
| `CAMERA_DAEMON_RECONNECT_AFTER_MS` | `25_000` | proactive reconnect (< daemon default lifetime 30 000) |
| `CAMERA_DAEMON_MAX_REQUESTS_PER_CONNECTION` | `1_000` | proactive reconnect (< daemon default 1024) |
| `CAMERA_DAEMON_BACKOFF_MIN_MS` / `_MAX_MS` | `200` / `2_000` | doubling back-off after Io/Protocol/Unavailable |
| `CAMERA_RATE_LIMITED_BACKOFF_MS` | `250` | back-off on `RateLimited` (shared 40/s quota with `soos-gui`) |
| `CAMERA_STREAM_BOUNDARY` | `"soosframe"` | multipart boundary (no byte of it can begin a JPEG part header) |
| `PUSH_CAMERA_TOPIC` | `"sooscamera"` | Web Push `Topic` (letters/digits only) |
| `ACTION_CAMERA_OPTIONS` / `ACTION_CAMERA_VIEW` / `ACTION_CAMERA_STREAM` / `ACTION_CAMERA_STOP` | `"camera-options"` / `"camera-view"` / `"camera-stream"` / `"camera-stop"` | `X-Soos-Action` values |

Compile-time relations (added next to the existing ones):

```rust
const _: () = assert!(MAX_SSE_STREAMS + MAX_CAMERA_VIEWS < MAX_CONNECTIONS);
const _: () = assert!(MAX_FUNNEL_SSE_STREAMS + MAX_CAMERA_VIEWS < MAX_FUNNEL_CONNECTIONS);
const _: () = assert!(MIN_CAMERA_MAX_VIEW_S <= DEFAULT_CAMERA_MAX_VIEW_S && DEFAULT_CAMERA_MAX_VIEW_S <= MAX_CAMERA_MAX_VIEW_S);
const _: () = assert!(MIN_CAMERA_FPS <= DEFAULT_CAMERA_FPS && DEFAULT_CAMERA_FPS <= MAX_CAMERA_FPS);
const _: () = assert!(MIN_CAMERA_QUALITY <= DEFAULT_CAMERA_QUALITY && DEFAULT_CAMERA_QUALITY <= MAX_CAMERA_QUALITY);
const _: () = assert!(CAMERA_MAX_SCRATCH_BYTES == (CAMERA_MAX_SOURCE_WIDTH * CAMERA_MAX_SOURCE_HEIGHT * 3) as usize);
const _: () = assert!(CAMERA_MAX_SCRATCH_BYTES + 1024 <= soos_protocol::types::MAX_PREVIEW_MESSAGE_SIZE);
const _: () = assert!(CAMERA_HALF_WIDTH * 2 == CAMERA_FULL_WIDTH && CAMERA_FULL_WIDTH == CAMERA_MAX_SOURCE_WIDTH);
const _: () = assert!(CAMERA_VIEW_TOKEN_B64_LEN == (CAMERA_VIEW_TOKEN_BYTES * 4).div_ceil(3));
const _: () = assert!(CAMERA_DAEMON_RECONNECT_AFTER_MS < 30_000);          // daemon default lifetime
const _: () = assert!(CAMERA_DAEMON_MAX_REQUESTS_PER_CONNECTION < 1024);   // daemon default cap
const _: () = assert!(CAMERA_DAEMON_BACKOFF_MIN_MS < CAMERA_DAEMON_BACKOFF_MAX_MS);
const _: () = assert!(CAMERA_DAEMON_BACKOFF_MAX_MS < CAMERA_STALL_TIMEOUT_MS);
const _: () = assert!(CAMERA_DAEMON_IO_TIMEOUT_MS < CAMERA_FIRST_FRAME_TIMEOUT_MS);
const _: () = assert!(CAMERA_DAEMON_SERVICE.len() <= soos_protocol::types::MAX_SERVICE_LEN);
const _: () = assert!(is_ascii_alphanumeric_topic(PUSH_CAMERA_TOPIC.as_bytes()));
const _: () = assert!(PUSH_CAMERA_TOPIC.len() <= soos_push_protocol::MAX_TOPIC_LEN);
const _: () = assert!(!const_bytes_eq(PUSH_CAMERA_TOPIC.as_bytes(), PUSH_TOPIC.as_bytes()));
const _: () = assert!(!const_bytes_eq(PUSH_CAMERA_TOPIC.as_bytes(), PUSH_TEST_TOPIC.as_bytes()));
const _: () = assert!(CAMERA_FULL_WIDTH as usize * 2 <= u16::MAX as usize);  // jpeg-encoder takes u16 dims
```

Note: the 30 000 / 1024 literals mirror the daemon defaults `max_connection_lifetime_ms` / `max_requests_per_connection`
(`soos-remote` must not depend on `soos-daemon`). An operator who lowers them below 25 s / 1000 only causes extra
reconnects (the client survives a daemon-side close, §5.2 steps 1 and 3a).

Two further constants (round 2, F5/F12):

| Constant | Value | Meaning |
|---|---|---|
| `CAMERA_DAEMON_CLOSE_WAIT_MS` | `200` | after `shutdown(Write)` of a connection being replaced, wait at most this long for the daemon's EOF before connecting the next one |
| `CAMERA_DAEMON_IDLE_RETRIES` | `1` | immediate retries on a fresh connection after an `Io` on a **reused** connection before any reply byte |

```rust
const _: () = assert!(CAMERA_DAEMON_CLOSE_WAIT_MS < CAMERA_DAEMON_CONNECT_TIMEOUT_MS);
const _: () = assert!(CAMERA_DAEMON_IDLE_RETRIES == 1);
```

The first-frame phase stays bounded as a whole by `CAMERA_FIRST_FRAME_TIMEOUT_MS` (one `timeout_at` around it), so
the retry never extends it.

### 3.3 Daemon

| Constant | Location | Value |
|---|---|---|
| `REMOTE_COMPANION_UNIT` | `crates/daemon/src/preview_peer.rs` | `"soos-remote.service"` |

---

## 4. Configuration

### 4.1 `remote.toml` (`crates/remote/src/config.rs`)

```rust
/// Output width of the live view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraWidth {
    /// Source size (at most 640x480).
    #[default]
    Full,
    /// 2x2 box downscale when the source is wider than `CAMERA_HALF_WIDTH`.
    Half,
}

/// Live camera view settings; `Default` = off with the default bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraConfig {
    /// `camera_view`; default `false`.
    pub enabled: bool,
    /// `camera_view_funnel`; default `false` (tailnet only).
    pub funnel: bool,
    /// `camera_max_view_s` ∈ MIN..=MAX_CAMERA_MAX_VIEW_S, default 120.
    pub max_view_s: u32,
    /// `camera_fps` ∈ 1..=10, default 5.
    pub fps: u32,
    /// `camera_width` ∈ {320, 640}, default 640.
    pub width: CameraWidth,
    /// `camera_quality` ∈ 50..=85, default 70.
    pub quality: u8,
}
// impl Default: { enabled: false, funnel: false, max_view_s: 120, fps: 5, width: Full, quality: 70 }

pub struct RemoteConfig { /* existing fields */ pub camera: CameraConfig }
```

`FileConfig` (still `deny_unknown_fields`) gains: `camera_view: Option<bool>`, `camera_view_funnel: Option<bool>`,
`camera_max_view_s: Option<u32>`, `camera_fps: Option<u32>`, `camera_width: Option<u32>`, `camera_quality: Option<u8>`.
A wrong TOML type (e.g. `camera_fps = "5"`, `camera_quality = 300` overflowing `u8`) → `ConfigError::Syntax`.

New `ConfigError` variants (fixed texts, never echo a value; every one exits `EXIT_CONFIG`):

| Variant | Text | Trigger |
|---|---|---|
| `CameraRequiresRpId` | `camera_view requires rp_id` | `camera_view = true` without `rp_id` |
| `CameraFunnelRequiresCameraView` | `camera_view_funnel requires camera_view` | `camera_view_funnel = true`, `camera_view` false/absent |
| `CameraFunnelRequiresFunnel` | `camera_view_funnel requires allow_funnel` | `camera_view_funnel = true`, `allow_funnel` false/absent |
| `CameraMaxViewOutOfRange` | `camera_max_view_s out of range` | outside 10..=300 |
| `CameraFpsOutOfRange` | `camera_fps out of range` | outside 1..=10 |
| `InvalidCameraWidth` | `camera_width must be 320 or 640` | any other value |
| `CameraQualityOutOfRange` | `camera_quality out of range` | outside 50..=85 |

Sentinels: absent key → default; `0` for any numeric key is out of range (refused, never "unlimited", never clamped);
range keys are validated even when `camera_view = false` (fail closed on a typo). Check order inside
`camera_config(...)`: `CameraRequiresRpId` → `CameraFunnelRequiresCameraView` → `CameraFunnelRequiresFunnel` → max view
→ fps → width → quality. `camera_view` does **not** require `push_notifications` (push is best effort, ADR item 8).

### 4.2 `daemon.toml` `[preview]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `remote_view` | bool | `false` | Serve preview frames to a peer whose cgroup is the `soos-remote.service` user unit. No effect unless `enabled = true` and the peer UID is in `allowed_uids`. |

`PreviewConfigFile` gains `remote_view: Option<bool>` (the `daemon_docs_contract` test then requires the row in
`Docs/DAEMON.md` §1.5). `PreviewConfig::validate` is unchanged (`remote_view = true` with `enabled = false` is accepted
and inert). `main.rs` start-up line adds `remote_view = config.preview.remote_view` to the existing `info!`.

---

## 5. Daemon client (`crates/remote/src/camera_ipc.rs`)

### 5.1 Types

```rust
/// One preview frame copied out of a `PreviewResponse`. No `Debug`, no `Clone`.
pub struct PreviewFrame {
    pub sequence: u64,
    pub width: u32,
    pub height: u32,
    /// `soos_protocol::types::PREVIEW_FORMAT_*`.
    pub format: u8,
    /// Moved out with `std::mem::take` (the `PreviewResponse` zeroizes its remainder).
    pub data: Zeroizing<Vec<u8>>,
}

/// Why a frame could not be fetched. Fixed texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PreviewError {
    /// Daemon refusal (`ProtocolError`/`Deny` other than `RateLimited`): preview disabled, UID not listed,
    /// `remote_view = false`, no local seat session. Ends the view.
    #[error("preview refused by the daemon")]
    Refused,
    /// `ProtocolError` + `RateLimited`: back off `CAMERA_RATE_LIMITED_BACKOFF_MS`, keep the view.
    #[error("preview rate limited")]
    RateLimited,
    /// `Verdict::Unavailable`.
    #[error("daemon unavailable")]
    Unavailable,
    /// Connect/read/write failure or timeout, peer not root.
    #[error("daemon connection failed")]
    Io,
    /// Malformed, oversize, stale or unbound reply; unknown verdict.
    #[error("daemon protocol error")]
    Protocol,
}

/// Seam of the view loop (tests inject a scripted source).
pub trait PreviewSource: Send {
    /// Fetches the latest frame. Must be cancel-safe (§5.3).
    fn next_frame(&mut self) -> Pin<Box<dyn Future<Output = Result<PreviewFrame, PreviewError>> + Send + '_>>;
}

/// Builds one source per view (production: `DaemonPreviewClient`).
pub type PreviewSourceFactory = Arc<dyn Fn() -> Box<dyn PreviewSource> + Send + Sync>;

/// Production client: at most one daemon connection.
pub struct DaemonPreviewClient {
    socket_path: PathBuf,
    uid: u32,                       // own uid (uid_hint)
    expected_daemon_uid: u32,       // 0 in production
    conn: Option<Conn>,             // taken during an exchange
}
struct Conn { stream: tokio::net::UnixStream, opened_at: tokio::time::Instant, requests: u32 }

impl DaemonPreviewClient {
    #[must_use] pub fn new(socket_path: PathBuf, uid: u32) -> Self;            // expected_daemon_uid = 0
    #[doc(hidden)] #[must_use] pub fn with_expected_daemon_uid(self, uid: u32) -> Self; // test hook
}
impl PreviewSource for DaemonPreviewClient { /* §5.2 */ }
```

### 5.2 Exchange (one `next_frame`)

1. `let conn = self.conn.take()`; if `None`, or `opened_at` older than `CAMERA_DAEMON_RECONNECT_AFTER_MS`, or
   `requests >= CAMERA_DAEMON_MAX_REQUESTS_PER_CONNECTION` → **graceful close** of the old connection (if any):
   `shutdown(Write)`, then read into a fixed 64-byte sink until EOF/error, bounded by `CAMERA_DAEMON_CLOSE_WAIT_MS`,
   then drop it (F5: the daemon has observed EOF and released its per-UID permit before the next connect in the
   normal case; a daemon slower than 200 ms leaves the residual window of R-3). Only then connect under
   `CAMERA_DAEMON_CONNECT_TIMEOUT_MS`; then `stream.peer_cred()?.uid() == expected_daemon_uid` else `Io` (refuses a
   non-root listener). At most one connection exists at any instant.
2. Request: `Request { version: CURRENT_VERSION, kind: RequestKind::PreviewFrame, request_id: <32 bytes getrandom>,
   uid_hint: self.uid, service: CAMERA_DAEMON_SERVICE.into(), deadline_monotonic_ns: u64::MAX }`, encoded with
   `soos_protocol::message::encode_request` (tagged frame). RNG failure → `Io`.
3. Under one `tokio::time::timeout(CAMERA_DAEMON_IO_TIMEOUT_MS)`: `write_all`, read 4-byte BE length;
   `declared == 0 || declared > MAX_PREVIEW_MESSAGE_SIZE` → `Protocol` (nothing allocated); allocate
   `Zeroizing<Vec<u8>>` of exactly `declared + 4`, read the payload; timeout or EOF → `Io`.
3a. **Idle-closed reuse (F12)**: if the connection was **reused** (not opened by this call) and the write fails or
   EOF/reset arrives before the first reply byte, the connection is dropped and the exchange is retried at once
   (`CAMERA_DAEMON_IDLE_RETRIES` = 1, no back-off) on a fresh connection (step 1 path, still one connection at a
   time) with a **new** nonce. A failure of the retry, or any failure on a fresh connection, is `Io`. This covers an
   operator `connection_timeout_ms` below `1000 / camera_fps` ms; `Docs/REMOTE_COMPANION.md` §2f recommends
   `connection_timeout_ms` > `1000 / camera_fps` ms (default 2500 > 1000).
4. If `declared <= MAX_MESSAGE_SIZE` and `codec::decode::<Response>` succeeds and `matches_request(&request_id)`:
   `check_freshness(monotonic_now_ns(), MAX_RESPONSE_FUTURE_SKEW_NS)` failing → `Protocol`; else map verdict:
   `ProtocolError`+`RateLimited` → `RateLimited`; `ProtocolError`/`Deny` → `Refused`; `Unavailable` → `Unavailable`;
   `Allow` → `Protocol`. (Same rules as `crates/gui/src/ipc_camera.rs`, duplicated ≈ 70 lines per ADR item 2.)
5. Else `codec::decode_preview::<PreviewResponse>` → failure `Protocol`; success: `PreviewFrame { data:
   Zeroizing::new(std::mem::take(&mut resp.data)), .. }`.
6. On `Ok`/`Refused`/`RateLimited`/`Unavailable` the connection is put back (`requests += 1`); on `Io`/`Protocol` it is
   dropped.

`monotonic_now_ns()` reads `nix::time::clock_gettime(CLOCK_MONOTONIC)` (0 on failure → rejected by `check_freshness`).
`SystemTime::now` is never used (RMC test `test_rmc_production_code_never_panics_or_prints`).

### 5.3 Cancel safety

The connection is held in a local during the exchange; a cancelled future drops it (a half-read connection is never
reused). The view loop drops an in-flight `next_frame` only through a terminal arm (§8.7 rule 1); non-terminal arms
never cancel it.

### 5.4 Frame acceptance (in the view loop, not in the client)

| Frame | Action |
|---|---|
| `format == PREVIEW_FORMAT_EMPTY` (or `data` empty) | camera waking: not an error, counts toward the stall/first-frame timers |
| `sequence == last_sent_sequence` | skip (never re-encode or re-send a frame: no stale replay) |
| format ∉ {RGB24, GREY, YUYV} | view ends `UnsupportedFrame` (NV12/MJPEG/unknown are never decoded) |
| geometry error (§6.2) or encode error | skip; `CAMERA_MAX_BAD_FRAMES` consecutive → view ends `UnsupportedFrame` |

---

## 6. JPEG path (`crates/remote/src/camera_jpeg.rs`, pure, no I/O, no logging)

### 6.1 Types

```rust
/// A finished JPEG image. No `Debug`, no `Clone`; zeroized on drop.
pub struct JpegFrame { pub bytes: Zeroizing<Vec<u8>> }

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("unsupported preview format")]   UnsupportedFormat,
    #[error("preview geometry out of bounds")] Geometry,
    #[error("jpeg larger than the bound")]  TooLarge,
    #[error("jpeg encoding failed")]         Encode,
}

/// Encoder settings resolved from `CameraConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegSettings { pub width: CameraWidth, pub quality: u8 }

/// Pure: validates, converts (+ optional 2x2 box downscale) and encodes one frame.
pub fn encode_preview_jpeg(format: u8, width: u32, height: u32, data: &[u8], settings: JpegSettings)
    -> Result<JpegFrame, FrameError>;

/// Pure conversion step (exposed for tests): returns (pixels, out_w, out_h, kind).
pub fn convert_frame(format: u8, width: u32, height: u32, data: &[u8], width_mode: CameraWidth)
    -> Result<(Zeroizing<Vec<u8>>, u16, u16, PixelKind), FrameError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelKind { Luma, Rgb, Ycbcr }

/// `std::io::Write` sink over a `Zeroizing<Vec<u8>>` preallocated to `MAX_CAMERA_JPEG_BYTES`;
/// a write beyond the capacity fails with `io::ErrorKind::WriteZero` and never reallocates.
pub struct JpegSink { buf: Zeroizing<Vec<u8>> }
impl JpegSink { #[must_use] pub fn new() -> Self; #[must_use] pub fn capacity(&self) -> usize; pub fn into_frame(self) -> JpegFrame; }
impl std::io::Write for JpegSink { /* no growth */ }
```

### 6.2 Validation (before any allocation)

`CAMERA_MIN_SOURCE_DIM <= width <= CAMERA_MAX_SOURCE_WIDTH`, `CAMERA_MIN_SOURCE_DIM <= height <=
CAMERA_MAX_SOURCE_HEIGHT`; **YUYV only** additionally requires an even `width` (a pixel pair is the unit); Grey and
RGB24 accept odd widths and heights (the daemon's integer decimation can emit e.g. 533x400 RGB24 from 1600x1200,
F6). `data.len()` must equal exactly `w*h` (GREY), `w*h*2` (YUYV), `w*h*3` (RGB24), computed with checked arithmetic;
else `Geometry`. Other format codes → `UnsupportedFormat`.

### 6.3 Conversion (one `Zeroizing` output of exactly the output size, ≤ `CAMERA_MAX_SCRATCH_BYTES`)

- **GREY** → `Luma`, 1 byte/pixel.
- **RGB24** → `Rgb`, 3 bytes/pixel, byte copy.
- **YUYV** (packed `Y0 U Y1 V`) → `Ycbcr` 4:4:4, pixel pair → `(Y0,U,V),(Y1,U,V)` (byte shuffle, no colour matrix:
  JPEG's YCbCr is full-range BT.601 like UVC YUYV; colour is approximate and that is accepted).
- **Half** (`CameraWidth::Half` and `width > CAMERA_HALF_WIDTH`): 2x2 box average with round-half-up
  `(a+b+c+d+2)/4` per channel; for YUYV the luma of the 4 pixels and the two rows' `U` (resp. `V`) of the pair:
  `(U_row0 + U_row1 + 1) / 2`. Output `floor(w/2) x floor(h/2)`: a last odd column and/or a last odd row is
  dropped (for YUYV only the row can be odd). `Half` with `width <= 320`: unchanged. Every loop index and product uses
  checked/`get` access (workspace lints `indexing_slicing`, `arithmetic_side_effects` are errors).

### 6.4 Encoding

`let mut sink = JpegSink::new(); let mut enc = jpeg_encoder::Encoder::new(&mut sink, quality);
enc.set_progressive(false); enc.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
enc.encode(&pixels, w_u16, h_u16, ColorType::{Luma|Rgb|Ycbcr})`. An encoder error caused by the sink bound →
`TooLarge`; any other → `Encode`. Output is baseline JFIF (SOI `FF D8`, SOF0 `FF C0`, EOI `FF D9`).
Quality range is already validated by the configuration. (API verified on docs.rs for 0.7.1: `Encoder::new(w, u8)`,
`encode(self, &[u8], u16, u16, ColorType)`, `SamplingFactor::R_4_2_0` aliases `F_2_2`.)

**Residual (amended ADR item 7, D-1 resolved)**: `jpeg-encoder` keeps internal working buffers (blocks,
coefficients) that are freed without zeroization; the amended ADR accepts it. Every buffer owned by soos-remote (IPC
reply, converted pixels, JPEG output, part buffer) is `Zeroizing`.

---

## 7. View slot and token (`crates/remote/src/camera_slot.rs`, pure; `Instant` passed in)

### 7.1 Types

```rust
/// 32-byte single-use stream token. No derived `Debug` (manual `ViewToken(<redacted>)`), zeroized on drop,
/// compared in constant time.
pub struct ViewToken([u8; CAMERA_VIEW_TOKEN_BYTES]);
impl ViewToken {
    pub fn from_bytes(bytes: [u8; CAMERA_VIEW_TOKEN_BYTES]) -> Self;
    /// Unpadded base64url, exactly `CAMERA_VIEW_TOKEN_B64_LEN` chars.
    #[must_use] pub fn encode(&self) -> Zeroizing<String>;
    /// Strict: exactly 43 chars of `[A-Za-z0-9_-]`, canonical (re-encoding equals the input), 32 bytes.
    #[must_use] pub fn parse(text: &str) -> Option<Self>;
}

/// Who may consume a token: path class and, on the Funnel, the web-session hash. `Debug` redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct ViewOwner { pub class: PathClass, pub session: Option<[u8; 32]> }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotPhase { Idle, Pending, Starting, Streaming, Cooldown }

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SlotError {
    /// `409 view_in_progress`.
    #[error("a camera view is in progress")] Busy,
    /// `429 camera_cooldown` (remaining ms in the body).
    #[error("camera view cooldown")] Cooldown { remaining_ms: u64 },
    /// `403 view_token_rejected`.
    #[error("view token rejected")] TokenRejected,
}

/// Identifies one started view (never logged: RC-5 forbids `*_id` fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewTicket { pub view: u64, pub ends_at: Instant }

pub struct ViewSlot { state: SlotState, next_view: u64 }
enum SlotState {
    Idle { cooldown_until: Option<Instant> },
    Pending { token: ViewToken, owner: ViewOwner, expires: Instant },
    Starting { view: u64, owner: ViewOwner, ends_at: Instant, stop: bool },
    Streaming { view: u64, owner: ViewOwner, ends_at: Instant, stop: bool },
}
```

### 7.2 Transitions (every method first expires a `Pending` whose `expires <= now` to `Idle{cooldown_until: None}`)

| Method | From | Result |
|---|---|---|
| `check(now) -> Result<(), SlotError>` (pre-check, no change) | Idle (no/elapsed cooldown) | `Ok` |
| | Idle in cooldown | `Cooldown{remaining_ms}` |
| | Pending / Starting / Streaming | `Busy` |
| `reserve(now, owner, token) -> Result<(), SlotError>` | as `check`; on `Ok` → `Pending{expires = now + TOKEN_TTL}` | |
| `begin(now, owner, &presented, max_view) -> Result<ViewTicket, SlotError>` | Pending, token equal (ct), not expired, `owner` equal (class equal; session hashes ct-equal) | → `Starting{ends_at = now + max_view}` (token dropped: single use) |
| | Pending with a different token or owner | `TokenRejected`, Pending **kept** (a stray request cannot cancel the owner's view) |
| | Pending expired / Idle / Starting / Streaming | `TokenRejected` |
| `mark_streaming(view)` | Starting(view) | → Streaming (same `ends_at`) |
| `stop()` -> `bool` | Pending → Idle (no cooldown), true; Starting/Streaming → `stop = true` (durable flag, never cleared until `end`), true; Idle → false | |
| `stop_requested(view) -> bool` | Starting/Streaming(view) with `stop`; **also true** when the slot no longer holds `view` (fail closed) | |
| `end(now, view, shown: bool)` | Starting/Streaming(view) → `Idle{cooldown_until = shown.then(now + COOLDOWN)}`; other view id → no-op | |
| `phase(now) -> (SlotPhase, Option<u64> remaining_cooldown_ms)` | read-only view | |

`next_view` uses `checked_add`; overflow → `begin` returns `Busy` forever (fail closed; unreachable in practice).
`ViewSlot` lives in `CameraRuntime` behind a `std::sync::Mutex` (never held across an await); a poisoned lock is
handled as `Busy`/no-op (no `unwrap`).

A `ViewGuard { runtime, view, shown: Cell/bool }` returned at `begin` calls `slot.end(now, view, shown)` and emits
`audit::camera_view_ended()` if `shown` on `Drop` — every exit path (error, cancel, panic of the connection task)
frees the slot.

---

## 8. HTTP surface

### 8.1 Routes (`crates/remote/src/routes.rs`)

| Route variant | Method + path | Body | Funnel-public | `Allow` on 405 |
|---|---|---|---|---|
| `Camera` | `GET|HEAD /api/camera` | no | no | `GET, HEAD` |
| `CameraOptions` | `POST /api/auth/camera/options` | no | no | `POST` |
| `CameraStart` | `POST /api/camera/start` | **yes** (assertion) | no | `POST` |
| `CameraStop` | `POST /api/camera/stop` | no | no | `POST` |
| `CameraStream` | `GET /api/camera/stream/<43 base64url chars>` | no | no | `GET` (HEAD → 405) |

```rust
pub const CAMERA_PATH: &str = "/api/camera";
pub const CAMERA_OPTIONS_PATH: &str = "/api/auth/camera/options";
pub const CAMERA_START_PATH: &str = "/api/camera/start";
pub const CAMERA_STOP_PATH: &str = "/api/camera/stop";
pub const CAMERA_STREAM_PREFIX: &str = "/api/camera/stream/";

/// The token of a stream path. `None` unless the path is exactly the prefix + a strict 43-char token.
#[must_use] pub fn camera_stream_token(path: &str) -> Option<ViewToken>;

/// Pure: lock CSRF rules with `action` ∈ {camera-stream, camera-stop}; anything else → `MissingActionHeader`.
pub fn check_camera_csrf(head: &RequestHead, normalized_host: &str, action: &str) -> Result<(), CsrfError>;
```

`route()`: `CameraStream` is matched by prefix + `ViewToken::parse` shape check (alphabet and length only, the token is
not kept in `Route`, which is `Debug`-logged by `debug!(?resolved, "request")`); `/api/camera/stream/` followed by
anything else → `NotFound`; a `POST`/`HEAD` on a well-formed stream path → `MethodNotAllowed`. `accepts_body` becomes
true for the seven paths (adds `/api/camera/start`). `is_funnel_public` unchanged (no camera route added).
`allow_header` returns `"GET"` for a stream path. `CameraOptions` and `CameraStart` use `check_auth_csrf` (Origin
**required**, equal to `https://<rp_id>`); `CameraStream` and `CameraStop` use `check_camera_csrf` (Origin optional).

### 8.2 Dispatch-level gates (existing, unchanged)

Host (`421`) → classification (`403`) → Funnel capacity and Funnel host = `rp_id` → Funnel session (camera routes are
not public: `403 login_required` without a session) → route handler.

### 8.3 `GET|HEAD /api/camera` — state

`200 application/json`:

```json
{"enabled":true,"reachable":true,"state":"idle","cooldown_ms":0,"max_view_s":120,"fps":5,"width":640}
```

`enabled` = `camera_view`; `reachable` = enabled and (tailnet, or Funnel with `camera_view_funnel`); `state` ∈
`disabled` (when `!enabled`) | `idle` | `pending` | `starting` | `streaming` | `cooldown`; `width` 640 or 320. Never
contains a token, a sequence, an identity or a frame property.

### 8.4 `POST /api/auth/camera/options`

Order: `check_auth_csrf(head, rp_id or host, "camera-options")` (`403 forbidden`) → `camera_view` (`403
camera_disabled`) → Funnel and `!camera_view_funnel` (`403 camera_tailnet_only`) → `rp_host_gate` (`403
passkeys_not_configured` / `421`) → `issue_options(ctx.unlock_key(), ctx.class, ChallengePurpose::CameraView,
ctx.unlock_binding())` → `200 {challenge, rp_id, timeout_ms}` / `409 no_passkey` / `429 rate_limited` /
`429 too_many_challenges` / `503 unavailable|store_unavailable`.

### 8.5 `POST /api/camera/start`

Order (no body byte read before step 8):

1. `check_auth_csrf(head, rp_id or host, "camera-view")` → `403 forbidden`
2. `camera_view` → `403 camera_disabled`
3. Funnel and `!camera_view_funnel` → `403 camera_tailnet_only`
4. `rp_id` configured → `403 passkeys_not_configured`
5. body absent → `403 passkey_required`
6. `rp_host_gate` → `421 misdirected_request`
7. lockout of `ctx.unlock_key()` → `429 rate_limited`; slot `check(now)` → `409 view_in_progress` /
   `429 {"result":"camera_cooldown","retry_after_ms":N}` (counted refusal audit, not an auth failure)
8. bounded body read (`read_request_body`; `400 bad_request` counted, `413 body_too_large`, close)
9. `decode_assertion` → `400 bad_request` (counted)
10. `auth.check_assertion(rp, ctx.class, ChallengePurpose::CameraView, &ctx.unlock_binding(), ..)` (UP **and** UV
    required, challenge single-use) → `403 passkey_rejected` (counted) / `503 store_unavailable`
11. `persist_counter` → `503 store_unavailable`
12. draw 32 CSPRNG bytes (`state.random`) → `503 unavailable`
13. `slot.reserve(now, ViewOwner{class, session}, token)` → `409 view_in_progress` / `429 camera_cooldown`
14. `200 {"result":"view_ready","stream_path":"/api/camera/stream/<token>","token_ttl_ms":10000,"max_view_s":…,"fps":…,"width":…}`

Every non-200 answer from step 7 on emits `audit::camera_view_refused()` through the rate gate (S-9).

### 8.6 `GET /api/camera/stream/<token>`

1. `check_camera_csrf(head, normalized_host, "camera-stream")` → `403 forbidden` (requires the custom header, so a
   cross-site `<img>`/navigation cannot open it and a cross-origin `fetch` needs a CORS preflight that never succeeds)
2. `camera_view` → `403 camera_disabled`; Funnel and `!camera_view_funnel` → `403 camera_tailnet_only`
3. `slot.begin(now, owner, &token, max_view)` → `403 view_token_rejected` (+ refused audit)
4. `ViewGuard` created; a `PreviewSource` is built from the factory; no factory wired (should not happen when
   enabled) → `503 camera_unavailable`
5. **First frame** within `CAMERA_FIRST_FRAME_TIMEOUT_MS` (fetch at the fps cadence, accept per §5.4, encode in
   `spawn_blocking`), run as the same loop structure as §8.7 (pinned frame future polled by `&mut`, durable stop
   checked at the top of every iteration, read half drained into a fixed sink; F2/F3), without the write step.
   Outcomes before any head byte: `Refused` → `403 camera_refused`; timeout / repeated
   `Io|Protocol|Unavailable` → `503 camera_unavailable`; `UnsupportedFrame` → `503 camera_format_unsupported`;
   stop/EOF/shutdown → closed without a response. Each is a refused audit (rate gated); `shown = false` (no cooldown).
6. `slot.mark_streaming`, write `encode_camera_stream_head()` + first part (one `write_bounded` each); `shown = true`;
   `audit::camera_view_started()`; `push.queue_camera_view()` if push is active (never awaited).
7. View loop (§8.7) until an end condition; best-effort trailer `--soosframe--\r\n`; `ViewGuard` drop → `ended`
   audit + cooldown.

**Permits held by a stream (F10)**: the connection permit (one of `MAX_CONNECTIONS`) and, on the Funnel, the
`funnel` class permit (one of `MAX_FUNNEL_CONNECTIONS`) are kept in the connection's `ClassPermits` for the whole
stream lifetime (the `MAX_FUNNEL_SSE_STREAMS + MAX_CAMERA_VIEWS < MAX_FUNNEL_CONNECTIONS` assertion relies on it);
the stream route reads no body, so it holds no body-read guard (`/api/camera/start` releases its guard,
`permits.body_read = None`, right after the assertion check, as `unlock_route` does); a session holder never holds an
anonymous permit; the stream takes **no** SSE slot (`sse_slots`, `enter_funnel_stream`) — its only slot is the
global view slot. All permits are released when the connection task ends (also on refusal through `answer`).

Head (`http.rs`):

```text
HTTP/1.1 200 OK\r\n
Content-Type: multipart/x-mixed-replace; boundary=soosframe\r\n
<MANDATORY_HEADERS unchanged: Cache-Control: no-store, the unchanged CSP, nosniff, no-referrer, DENY, Connection: close>\r\n
```

No `Content-Length`, no `Transfer-Encoding` (close-delimited body, flushed per write by Tailscale's
`httputil.ReverseProxy`, research §5.2). Part (`encode_camera_part_head(len)`):

```text
--soosframe\r\nContent-Type: image/jpeg\r\nContent-Length: <len>\r\n\r\n<len JPEG bytes>\r\n
```

The part head and the JPEG are written as one `write_bounded` call of a `Zeroizing` buffer
(`RESPONSE_WRITE_TIMEOUT_MS` = 2 s per part). Trailer constant `CAMERA_STREAM_TRAILER = "--soosframe--\r\n"`.

### 8.7 View loop (inside the connection task, after the head)

**Cancellation and stop rules (round 2, F2/F3).**

1. **Frame step, never dropped mid-flight (F2).** One iteration of the frame pipeline is a future
   `frame_step = sleep_until(next_tick) → source.next_frame() → spawn_blocking(encode)` that returns a
   `StepOutcome` (`Frame(JpegFrame)`, `Skip`, `Waiting`, `RateLimited`, `Failed(PreviewError)`, `Unsupported`,
   `Internal`). It is created once, `pin!`ned outside the `select!` and polled by `&mut` from the loop
   (`frame = &mut frame_step`); a **non-terminal** arm (session check that passes, read-half bytes) completes without
   touching it, so an in-flight daemon exchange or encode is never cancelled by them. It is recreated
   (`frame_step.set(..)`) only after it completed. The **part write** is not inside the future: it happens in the
   arm body (`write_bounded(part)`, ≤ 2 s), exactly like `serve_stream`, so no other arm can interrupt a partial
   part. Only **terminal** arms (shutdown, stop, client close, max duration, session expired, stall) drop the
   step; the stream ends right after, so a cancelled exchange only drops the daemon connection (§5.3) and a part is
   never left half-written followed by another part.
2. **Read half (F2b).** The read-half arm reads into a fixed `[u8; STREAM_SINK_BYTES]` sink (64 bytes, the
   `serve_stream` pattern), discards the bytes, and ends the view only on `Ok(0)` or `Err(_)`.
3. **Durable stop (F3).** The stop signal is the slot's durable `stop` flag (§7.2) plus a
   `tokio::sync::watch::Sender<u64>` (`stop_epoch`) in `CameraRuntime` that `POST /api/camera/stop` increments
   (`send_modify`) after `slot.stop()`. The view task subscribes **before** `begin` returns its ticket (receiver
   created inside the same slot critical section), and: (a) checks `stop_requested(view)` at the **top of every loop
   iteration** (first-frame phase included) and after every arm body (in particular after a part write that took up
   to 2 s); (b) has a `stop_rx.changed()` arm. A `watch` change is never lost (the version is stored), and the flag
   check covers a stop issued while an arm body was running.

`tokio::select! { biased; … }` over (after the top-of-iteration stop check):

| Arm | Kind | Effect |
|---|---|---|
| `closing.changed()` true | terminal | end `Shutdown` |
| `stop_rx.changed()` and `stop_requested(view)` | terminal | end `Stopped` |
| read half into the 64-byte sink: `Ok(0)`/`Err` | terminal | end `ClientClosed` (bytes are discarded, loop continues) |
| `sleep_until(ticket.ends_at)` | terminal | end `MaxDuration` |
| Funnel only: `sleep_until(next_session_check)` | non-terminal | `session_still_valid(hash)` false → end `SessionExpired`; else next check `+CAMERA_SESSION_CHECK_MS` |
| `sleep_until(last_sent_at + CAMERA_STALL_TIMEOUT_MS)` | terminal | end `CameraUnavailable` |
| `outcome = &mut frame_step` | non-terminal | `Frame` → `write_bounded(part)` in the arm body (error → end `WriteFailed`), `last_sent_*` updated; `Failed(Refused)` → end `DaemonRefused`; `Unsupported` (format, or `CAMERA_MAX_BAD_FRAMES` consecutive) → end `UnsupportedFrame`; `Internal` (`JoinError`) → end `Internal`; then `frame_step.set(..)` with the next tick |

**Tick rule (no burst catch-up).** `next_tick = max(previous_tick + 1000/fps ms, now)`, computed when the step
completes; a slow exchange is followed by the next request at once but never by several, so consecutive parts are
≥ `1000/fps` ms apart except right after a slow exchange, where the gap is ≥ the exchange time. `RateLimited` →
`next_tick ≥ now + CAMERA_RATE_LIMITED_BACKOFF_MS`; `Io/Protocol/Unavailable` → doubling back-off 200..2000 ms
(reset on a frame).

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewEnd { Stopped, MaxDuration, ClientClosed, WriteFailed, SessionExpired, Shutdown,
                   DaemonRefused, CameraUnavailable, UnsupportedFrame, Internal }
```

`ViewEnd` is used for tests and for one fixed-text `debug!` per variant (no field). At most one frame is in flight
(fetch → encode → write are sequential), so no queue.

### 8.8 `POST /api/camera/stop`

No body (the generic `413 body_not_allowed` applies). `check_camera_csrf(.., "camera-stop")` → `403 forbidden`;
`camera_view` → `403 camera_disabled`; then `slot.stop()` (durable flag) and, if it returned `true`,
`stop_epoch.send_modify(|e| *e = e.wrapping_add(1))` → `200 {"result":"stopped"}`, else `200 {"result":"no_view"}`.
A `stopped` answer guarantees the view ends within one loop iteration (≤ one part write, 2 s). Any authenticated
caller may stop (stopping never reveals pixels). No Funnel-flag gate.

### 8.9 Status codes added to `Docs/REMOTE_COMPANION.md` §6

`camera_disabled`, `camera_tailnet_only`, `view_in_progress`, `camera_cooldown`, `view_ready`, `view_token_rejected`,
`camera_refused`, `camera_unavailable`, `camera_format_unsupported`, `stopped`, `no_view`.

---

## 9. Daemon changes

### 9.1 Seat predicate (`session_policy.rs`, `session.rs`)

```rust
impl SessionRecord {
    /// ADR 2026-10-07 LC-3a (amended): exactly `self.check_local_seat_session_of(uid).is_ok()` — `UID == uid`,
    /// active (`ACTIVE=1` or `STATE=active`), `REMOTE=0` (absent or malformed refuses), non-empty `SEAT`,
    /// `CLASS=user`. A locked seat session qualifies (logind does not serialize `LockedHint`).
    #[must_use] pub fn is_local_seat_session_of(&self, uid: u32) -> bool;
}
impl SessionValidator {
    /// Same directory scan and fail-closed rules as `is_active_session`, with `is_local_seat_session_of`;
    /// `true` when enforcement is disabled (`enforce_active_session = false`, mock harnesses only).
    #[must_use] pub fn has_local_seat_session(&self, uid: u32) -> bool;
}
```

The two scans share one private helper `any_session_matches(uid, predicate)`; `is_active_session` keeps its behaviour.

### 9.2 Peer origin (`crates/daemon/src/preview_peer.rs`, new, `pub mod` in `lib.rs`)

```rust
pub const REMOTE_COMPANION_UNIT: &str = "soos-remote.service";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewPeerOrigin { Local, RemoteCompanion }

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PreviewPeerError {
    #[error("peer pid unavailable")] MissingPid,
    #[error("peer cgroup unreadable")] Unreadable,
    #[error("peer cgroup malformed")] Malformed,
}

/// Pure. Considers only the systemd hierarchies (`0::` and `name=systemd`, `systemd_hierarchy_path`). For each
/// line: a path below `/user.slice/user-<u>.slice/user@<u>.service/` (strict UIDs, `<u>` equal on both, malformed →
/// `Malformed`) whose first non-`.slice` component after the manager service equals `REMOTE_COMPANION_UNIT` exactly is
/// `RemoteCompanion` iff `<u> == peer_uid` (else `Malformed`); any other path is `Local`. Considered lines that
/// disagree → `Malformed`; no considered line → `Local`.
pub fn classify_preview_peer_cgroup(content: &str, peer_uid: u32) -> Result<PreviewPeerOrigin, PreviewPeerError>;
```

Examples: `0::/user.slice/user-1000.slice/user@1000.service/app.slice/soos-remote.service` → `RemoteCompanion`;
`…/app.slice/soos-gui.service`, `…/session-4.scope`, `/user.slice/user-1000.slice/session-4.scope`, `0::/` → `Local`;
`…/app.slice/soos-remote@x.service`, `…/soos-remote.service.d` → `Local` (exact match only);
`…/user-1000.slice/user@1001.service/…` → `Malformed`.

### 9.3 Dispatcher preview gate (`handle_preview_request`)

```rust
/// Per-connection preview state (private; lives in `handle_connection`, passed `&mut` down to the preview handler).
struct PreviewPeerState { origin: Option<PreviewPeerOrigin>, first_frame_announced: bool }

impl ConnectionDispatcher {
    /// Test hook: the source of `/proc/<pid>/cgroup` for preview peer classification.
    /// Default: `SystemLogind::with_paths(config.logind_sessions_dir, DEFAULT_PROC_ROOT)`, **independent of**
    /// `enforce_active_session`.
    #[must_use] pub fn with_preview_peer_source(self, source: Arc<dyn LogindSource>) -> Self;
}
```

Order (each refusal = standard `Response` `ProtocolError`, zero pixel bytes):

1. `authorize_preview` (unchanged) → `UidMismatch`.
2. Unprivileged peer: origin = cached, else `cgroup_of_pid(pid)` (`None`/pid ≤ 0 → `MissingPid`, `Err` →
   `Unreadable`) → `classify_preview_peer_cgroup`; error → `UidMismatch`, `warn!(peer_uid, "Preview peer cgroup
   unreadable or malformed; refusing preview request")`.
3. `RemoteCompanion && !preview.remote_view` → `UidMismatch`, `warn!(peer_uid, "Preview request from soos-remote
   refused: [preview] remote_view is false")`.
4. Unprivileged peer: `session_validator.has_local_seat_session(peer_uid)` false → `UidMismatch`,
   `warn!(peer_uid, "Preview peer has no active local seat session; refusing preview request")`.
5. Rate limit (unchanged, per peer UID, shared with the GUI).
6. Serve (unchanged). If origin is `RemoteCompanion`, the response carries a non-empty frame and
   `!first_frame_announced`: `info!(peer_uid = peer_uid, "Remote camera view: first preview frame served to
   soos-remote on this connection")`, set the flag. No size, dimension, sequence or byte field.

Root peers skip 2–4 (unchanged ADR 2026-09-29 rule). `Auth`, `Status`, `Event` paths are untouched: the cgroup is read
only for `PreviewFrame`, so the PAM latency budget is unchanged (§11).

---

## 10. Push, audit, page

### 10.1 Push (`push.rs`)

- `enum Message { Alert(PushSummary), Test, CameraView }`; topic `PUSH_CAMERA_TOPIC`.
- `PushRuntime.camera_queued: AtomicBool`; `pub(crate) fn queue_camera_view(&self)` — no-op unless `active`; sets the
  flag, `wake.notify_one()`; never awaits, never fails.
- `run_dispatcher`: after the queued test, `if camera_queued.swap(false) { send_camera(&mut state).await; continue }`.
  `send_camera` = `send_test` with `Message::CameraView`: one attempt per subscription, no retry, never touches alert
  retries/carries/hourly cap; outcomes recorded like the test (`gone` removes the subscription).
- `pub fn camera_payload(rp_id: &str) -> Result<Vec<u8>, WebPushError>`: Declarative Web Push, `title: "soos camera
  view"`, `body: "The live camera view of your PC was started"`, `navigate: https://<rp_id>/`, `soos.kind: "camera"`,
  counts 0, no source/account/time. Always generic (independent of `push_previews`). ≤ `MAX_PUSH_PLAINTEXT_BYTES`.
- `sw.js`: `kind === "camera"` → `tag = "soos-camera"` (one more branch next to `"test"`).
- Bound: at most one camera notification per started view; views are ≥ `CAMERA_VIEW_COOLDOWN_MS` apart.

### 10.2 Audit (`audit.rs`, same static-callsite pattern, no field)

| Function | Level | Message | When |
|---|---|---|---|
| `camera_view_started()` | INFO | `camera view started` | head + first part written |
| `camera_view_ended()` | INFO | `camera view ended` | `ViewGuard` drop of a shown view |
| `camera_view_refused()` | WARN | `camera view refused` | a refusal of §8.5 step ≥ 7 or §8.6 steps 3–5, at most one per `CAMERA_REFUSED_AUDIT_MIN_INTERVAL_MS` (`AtomicU64` gate in `CameraRuntime`) |

### 10.3 Page (`app.js`, `style.css`; `index.html` and the CSP unchanged)

- Constants: `CAMERA_PATH = "/api/camera"`, `CAMERA_OPTIONS_PATH = "/api/auth/camera/options"`,
  `CAMERA_START_PATH = "/api/camera/start"`, `CAMERA_STOP_PATH = "/api/camera/stop"`,
  `CAMERA_BOUNDARY = "--soosframe"`, `CAMERA_MAX_PART_BYTES = 524288`, `CAMERA_MAX_BUFFER_BYTES = 1048576`,
  `CAMERA_START_LABEL = "Start camera view"`, `CAMERA_STOP_LABEL = "Stop camera view"`.
- Card built once by `buildCameraCard()` with `document.createElement` (`section.card.camera`, `h2` "Camera",
  a `p.detail` state line, a `canvas.camera-canvas`, two `button type="button"`, a `p.feedback role="status"`),
  inserted after the `#push` section, hidden until `GET /api/camera` says `enabled`; `reachable === false` shows
  "Camera view is available on the tailnet only". Text only through `textContent`.
- Start: `window.confirm(...)` → `postJson(CAMERA_OPTIONS_PATH, "camera-options")` → `getAssertion(options)`
  (`userVerification: "required"`, existing helper) → `fetch(CAMERA_START_PATH, {method: "POST", headers:
  {"X-Soos-Action": "camera-view", "Content-Type": "application/json"}, body: assertionBody(..)})` →
  `fetch(stream_path, {headers: {"X-Soos-Action": "camera-stream"}, signal: controller.signal, cache: "no-store"})`.
- Reader: `response.body.getReader()`; a bounded `Uint8Array` accumulator (> `CAMERA_MAX_BUFFER_BYTES` → abort);
  parse `--soosframe\r\n`, header lines until `\r\n\r\n`, `Content-Length` (digits only, ≤ `CAMERA_MAX_PART_BYTES`,
  else abort), the JPEG bytes, the trailing `\r\n`; `createImageBitmap(new Blob([jpeg], {type: "image/jpeg"}))` →
  size the canvas once → `drawImage` → `bitmap.close()`; a frame still decoding is skipped (latest wins). Never
  `URL.createObjectURL`, never an `<img>`, never storage.
- Stop: `controller.abort()` + `fetch(CAMERA_STOP_PATH, {method: "POST", headers: {"X-Soos-Action": "camera-stop"},
  keepalive: true})`; also on `visibilitychange` (hidden) and `pagehide`. The canvas is cleared
  (`clearRect`) and its size reset to 0 at every end.
- Errors map `camera_refused` → "The PC refused the camera view (check the [preview] settings and that you are logged
  in at the PC)", `camera_unavailable`, `camera_format_unsupported`, `camera_cooldown`, `view_in_progress`,
  `camera_tailnet_only`, `passkey_rejected` to fixed texts.
- `style.css`: `.camera-canvas { width: 100%; height: auto; }` plus layout only; any colour through the existing
  palette variables (`test_rmc_s45_*`); no `.switch`.

---

## 11. Latency budget

The PAM `Auth` path is untouched (no new read on `Auth` connections; `DECISION_BUDGET_MS` 900, PAM `timeout_ms`
10..=5000 unchanged). Preview path: + one bounded (`MAX_CGROUP_FILE_SIZE` 16 KiB) `/proc/<pid>/cgroup` read per
connection and one bounded sessions-directory scan per request (as before). Camera-to-glass (research §7): capture age
≤ 33 ms + IPC < 1 ms + encode 3–10 ms (640x480) + network 20–300 ms ≈ 0.1–0.4 s.
Shared resources while a view is open: 1 of the 2 `max_connections_per_uid` daemon slots and ≤ 10 of the 40/s
per-UID preview quota (§15 R-3).

---

## 12. Existing tests that need migration

| Test / file | Change | Kind |
|---|---|---|
| `tests/invariants/src/remote_companion_contract.rs::test_rmc_s4_remote_is_a_leaf_crate` | remove `"soos-protocol"` from the forbidden list; **add**: the manifest has exactly `soos-protocol = { workspace = true }`, and every dependency key starting with `soos-` is `soos-protocol` or `soos-push-protocol` | **Assertion change, owner-approved (LC-1)** |
| `crates/remote/tests/common/harness.rs`, `server_tests.rs`, `alerts_server_tests.rs`, `push_server_tests.rs` (`RemoteConfig { … }` literals) | add `camera: soos_remote::config::CameraConfig::default(),` | Setup-only (compile) |
| `crates/daemon/tests/preview_authorization_tests.rs:508` (`PreviewConfig { enabled, allowed_uids, max_requests_per_sec: 2 }`) | add `remote_view: false,` | Setup-only (compile) |
| `crates/daemon/tests/preview_authorization_tests.rs:474-478` (`test_preview_frame_allowed_for_configured_uid_serves_frames` session fixture `UID=…\nACTIVE=1\nSTATE=active\n`) | add `REMOTE=0\nSEAT=seat0\nCLASS=user\n` (a realistic seat session; the assertions are unchanged) | Setup-only (fixture) |

Verified **not** to need a change: `test_rmc_s7_*` and `test_rmc_s39_push_is_documented` (S-4), `test_rmc_s50_*`
(S-3), `test_rmc_s5_s11_*`/`s26`/`s35` (unit unchanged), `test_rmc_s8_*` (needles kept, no `innerHTML`/URL),
`test_rmc_s18_*` (one service worker; `window.confirm(` kept), `test_rmc_s2_*` (`tokio::net::UnixStream` only),
`test_rmc_production_code_never_panics_or_prints` (applies to the new files), `test_rmc_logging_never_names_*`
(no `*_id` field), `policy_concurrency_tests` / `session_policy_tests` (`is_active_session` unchanged),
`preview_downscale_tests`, `dispatcher_tests`, `response_timestamp_tests` (enforcement disabled → seat check passes;
test process cgroup is `Local`), `daemon_docs_contract` (passes once `Docs/DAEMON.md` documents `remote_view`),
`dependency_tooling_contract` (`jpeg-encoder` has no required dependency, no new duplicate version).
Any other existing test that fails must be reported, not edited.

---

## 13. Test list for the tester (Phase 2)

### 13.1 `crates/protocol/tests/preview_tests.rs` (new test only)
49. `test_rlc_preview_format_codes` — the six constants equal 0, 1, 2, 3, 4, 255; the daemon's
    `soos_daemon::preview::PREVIEW_FORMAT_EMPTY` equals the protocol one (in a daemon test instead if the protocol
    crate cannot see the daemon: place it in `crates/daemon/tests/preview_remote_view_tests.rs`).

### 13.2 `crates/remote/tests/camera_jpeg_tests.rs` (new, pure)
1. `test_rlc_grey_frame_encodes_to_baseline_jpeg` — 64x48 Grey → `FF D8` … `FF D9`, contains SOF0 `FF C0` with
   height 48, width 64, 1 component; no `FF C2` (progressive).
2. `test_rlc_yuyv_conversion_shuffles_chroma` — `convert_frame` on 2x… YUYV fixture gives `(Y0,U,V,Y1,U,V)`; encoded
   SOF0 has 3 components with sampling 0x22/0x11/0x11.
3. `test_rlc_rgb24_frame_encodes` — 3 components, exact dims.
4. `test_rlc_half_width_box_average` — Grey/RGB/YUYV 640x480 → 320x240 with exact rounded averages on a fixture;
   odd sources (Grey 533x401 → 266x200, RGB24 533x400 → 266x200, YUYV 534x401 → 267x200) drop the last odd
   column/row; `Half` on a 320-wide source leaves it unchanged; `Full` never scales.
5. `test_rlc_frame_geometry_bounds` — YUYV with an odd width → `Geometry`; Grey and RGB24 with odd width and odd
   height (e.g. 533x401) → `Ok`; 15, 641, height 481, length ±1, overflowing dims → `Geometry` (no allocation of the
   output).
6. `test_rlc_unsupported_formats_refused` — 3, 4, 5, 254 → `UnsupportedFormat`; 255 is never passed to the encoder
   (the loop handles it) and also returns `UnsupportedFormat`.
7. `test_rlc_jpeg_sink_never_grows` — writes up to capacity succeed; one byte more fails `WriteZero`; capacity
   unchanged; `encode_preview_jpeg` on a frame whose output cannot fit (test-only smaller sink via a `#[doc(hidden)]`
   `JpegSink::with_capacity_for_tests`) → `TooLarge`.
8. `test_rlc_quality_changes_quantization` — same noisy frame at 50 and 85: different DQT bytes, q50 output shorter.
9. `test_rlc_convert_never_panics` (proptest) — random format (0..=255), dims (0..=700), data length (0..=2 MiB
   bounded) → never panics; `Ok` output length = out_w*out_h*bpp ≤ `CAMERA_MAX_SCRATCH_BYTES`.

### 13.3 `crates/remote/tests/camera_slot_tests.rs` (new, pure)
10. `test_rlc_slot_reserve_begin_stream_end_cycle`
11. `test_rlc_slot_single_global_view` — reserve while Pending/Starting/Streaming → `Busy`.
12. `test_rlc_slot_token_single_use_and_ttl` — second `begin` → `TokenRejected`; `begin` at `expires` → rejected and
    the slot is Idle without cooldown.
13. `test_rlc_slot_token_bound_to_owner` — class mismatch, Funnel session hash mismatch → `TokenRejected`, Pending kept;
    the right owner then succeeds.
14. `test_rlc_slot_cooldown_only_after_shown_view` — `end(shown=true)` → `Cooldown{remaining}` decreasing to 0;
    `end(shown=false)` → immediate reserve OK; `end` with a stale view id is a no-op.
15. `test_rlc_slot_stop` — stop Pending → Idle; stop Streaming → `stop_requested`; stop Idle → false.
16. `test_rlc_view_token_format_and_redaction` — `encode` is 43 chars of the alphabet; `parse` refuses 42/44 chars,
    `=`, `+`, `/`, non-canonical last char; `format!("{:?}", token)` contains `redacted` and no token char run.

### 13.4 `crates/remote/tests/camera_ipc_tests.rs` (new; fake daemon on a tempdir socket answering with `soos-protocol`)
17. `test_rlc_client_sends_preview_request` — decoded request: kind `PreviewFrame`, `uid_hint` = given uid, service
    `soos-remote`, tagged frame, 32-byte random nonce differing between two requests.
18. `test_rlc_client_maps_daemon_replies` — `UidMismatch` → `Refused`; `RateLimited` → `RateLimited`; `Unavailable` →
    `Unavailable`; `Allow` → `Protocol`; nonce mismatch, stale or future-dated stamp → `Protocol`; a valid
    `PreviewResponse` → frame with identical fields and data.
19. `test_rlc_client_bounds_replies` — declared 0, `MAX_PREVIEW_MESSAGE_SIZE + 1` → `Protocol`; truncated payload → `Io`.
20. `test_rlc_client_holds_at_most_one_connection` — the fake counts concurrently open connections (max 1) across
    many frames; daemon-side close → next call reconnects; with paused time, a call after
    `CAMERA_DAEMON_RECONNECT_AFTER_MS` or after `CAMERA_DAEMON_MAX_REQUESTS_PER_CONNECTION` requests opens a new
    connection after closing the old one; a cancelled `next_frame` (dropped future) is followed by a fresh connection.
    **F5**: on a proactive reconnect the fake observes, on the old connection, the client's write shutdown (EOF on
    its read side) **before** it accepts the new connection, and the new connection is only accepted after the fake
    closed the old one (or after `CAMERA_DAEMON_CLOSE_WAIT_MS` when the fake deliberately never closes it).
21. `test_rlc_client_refuses_unexpected_daemon_uid` — default client (expects uid 0) against the test-uid fake → `Io`;
    `with_expected_daemon_uid(test uid)` → works.
22. `test_rlc_client_exchange_is_time_bounded` — fake that never answers → `Io` after `CAMERA_DAEMON_IO_TIMEOUT_MS`
    (paused time).
62. `test_rlc_client_retries_once_after_idle_close` (F12) — the fake closes a served connection right after its
    reply (idle close); the next `next_frame` succeeds on a fresh connection without back-off, the retry carries a new
    nonce, at most one connection is open at any time; when the fresh connection also fails → `Io` (exactly one
    retry); an `Io` on a **fresh** connection is not retried.

### 13.5 `crates/remote/tests/camera_server_tests.rs` (new; existing harness + tester-owned `ScriptedPreviewSource`)
23. `test_rlc_camera_disabled_by_default` — default config: `GET /api/camera` `{"enabled":false,…,"state":"disabled"}`;
    options/start/stop → `403 camera_disabled`; stream → `403 camera_disabled`; the factory is never called.
24. `test_rlc_start_requires_fresh_uv_camera_assertion` — valid camera assertion → `200 view_ready`; an **unlock**
    challenge used on `/api/camera/start` → `403 passkey_rejected`; a **camera** challenge used on `/api/unlock` →
    `403 passkey_rejected`; UV flag cleared → rejected; replayed assertion → rejected; failures count toward the shared
    lockout (6th attempt `429 rate_limited`).
25. `test_rlc_start_gate_order` — CSRF (missing action, wrong action, missing Origin) `403 forbidden` before
    `camera_disabled`; no body byte read before the lockout/slot checks (a stalled body is never awaited when the slot
    is busy); `413` on a too large body.
26. `test_rlc_funnel_is_tailnet_only_by_default` — Funnel session + `camera_view_funnel = false` → `403
    camera_tailnet_only` on options/start/stream, `reachable:false`; with the key → works; anonymous Funnel → `403
    login_required`.
27. `test_rlc_stream_is_multipart_jpeg` — head: `200`, `Content-Type: multipart/x-mixed-replace; boundary=soosframe`,
    the unchanged CSP and every mandatory header, no `Content-Length`; parts parse by `Content-Length`, each body
    starts `FF D8` and ends `FF D9`; trailer on `MaxDuration`.
28. `test_rlc_stream_token_single_use_and_owner_bound` — second stream request with the same token → `403
    view_token_rejected`; token after 10 s → rejected; Funnel session B cannot use session A's token; missing
    `X-Soos-Action: camera-stream` → `403 forbidden`; malformed token path → `404`; `HEAD` → `405` `Allow: GET`.
29. `test_rlc_one_view_and_cooldown` — second start during a view → `409 view_in_progress`; after the end →
    `429 camera_cooldown` with `retry_after_ms` ≤ 10 000; after the cooldown → OK; a failed first frame sets no cooldown.
30. `test_rlc_view_end_conditions` — (paused time, one sub-case each) stop route, `max_view_s`, client close,
    write stall > 2 s (reader never reads), Funnel session revoked, server shutdown, scripted `Refused` mid-view,
    no new frame for 5 s; after each: slot Idle (cooldown), `ended` audit once, the source dropped. **F3 sub-cases**:
    (a) stop issued while a part write is blocked (reader paused, socket buffer full), then the reader resumes → the
    route answered `stopped`, the stream ends within one iteration (no further part after the blocked one completes;
    trailer or close follows) — must fail against a `notify_waiters`-only implementation; (b) stop issued during the
    first-frame phase (scripted source pending) → the connection closes without a head and the slot is Idle without
    cooldown; (c) the read half receiving stray bytes does not end the view (F2b).
31. `test_rlc_frame_rate_and_no_replay` — scripted source with increasing sequences: ≤ `fps × seconds + 1` parts;
    repeated sequence → no extra part; `camera_width = 320` → parts decode SOF0 width 320. **F11**: with paused time
    and one scripted exchange taking 3 × the frame interval, the parts that follow are still ≥ `1000/fps` ms apart
    (no burst of catch-up parts; at most one part right after the slow exchange).
61. `test_rlc_non_terminal_arms_never_cancel_a_frame` (F2) — paused time, Funnel session, `CAMERA_SESSION_CHECK_MS`
    tick and stray read-half bytes fire (i) while a scripted `next_frame` is pending and (ii) while a part write is
    blocked: every part received afterwards parses (`Content-Length` aligned, `FF D8`…`FF D9`), and the scripted
    source records zero cancelled requests (each started `next_frame` completed).
32. `test_rlc_first_frame_failures_are_json_errors` — scripted `Refused` → `403 camera_refused`; `Io` until timeout →
    `503 camera_unavailable`; NV12 frame → `503 camera_format_unsupported`; empty frames then a real one within 5 s →
    `200`.
33. `test_rlc_rate_limited_backs_off_without_ending` — `RateLimited` replies interleaved: the view continues, the next
    request is ≥ 250 ms later.
34. `test_rlc_camera_audit_lines` — captured subscriber: exactly `camera view started`/`camera view ended` for one
    view, `camera view refused` for a rejected token, at most one refused line per 5 s; no event has a field other than
    `message`.
35. `test_rlc_push_on_view_start_is_best_effort` — with push wired (`FakeTransport`): one delivery with topic
    `sooscamera`, decrypted payload `kind: "camera"` and the fixed texts; with a failing/hanging transport the stream
    still delivers frames on time; no push when the first frame failed.

### 13.6 Additions to existing remote test files (new tests only)
36. `config_tests.rs::test_rlc_camera_config_keys` — defaults; each key's accepted bounds and every §4.1 error;
    `camera_width = 480` refused; `camera_fps = 0` refused; wrong types `Syntax`; error texts never contain the value.
37. `routes_tests.rs::test_rlc_camera_route_table` — §8.1 table: methods, `405` + `Allow`, near paths `404`
    (`/api/camera/stream`, `/api/camera/stream/`, 42/44-char tokens, `/api/camera/stream/<token>/x`), `accepts_body`
    true only for `/api/camera/start` among camera routes, `is_funnel_public` false for all five; `format!("{:?}",
    Route::CameraStream)` has no token.
38. `routes_tests.rs::test_rlc_camera_csrf_rules` — `check_camera_csrf` mirrors the lock rules for both actions; any
    other action argument → `MissingActionHeader`.
39. `push_tests.rs::test_rlc_camera_payload` — fixed title/body/kind, `navigate` origin, ≤ 1 KiB, independent of
    `PushPreviews`.
40. `auth_store_tests.rs` (or a new `challenge_tests.rs`) `::test_rlc_camera_view_challenge_pool` — pools
    `(Tailnet, CameraView, None)` and `(Funnel, CameraView, WebSession)` accept 4 each and refuse the 5th; wrong binding
    variant → `Mismatch`; `take` with purpose `Unlock` of a `CameraView` challenge → `Mismatch` (entry removed).

### 13.7 `crates/daemon/tests/preview_remote_view_tests.rs` (new; mock `LogindSource` for cgroups)
41. `test_rlc_classify_preview_peer_cgroup` — every example of §9.2, cgroup v1 `name=systemd`, disagreeing lines,
    other-controller lines ignored, empty content → `Local`; **F11**: `0::/user.slice/user-1000.slice/user@1000.service/
    app.slice/soos-remote.service` with `peer_uid = 1001` → `Malformed` (and with `peer_uid = 1000` →
    `RemoteCompanion`).
42. `test_rlc_remote_view_refused_unless_enabled` — peer cgroup = `soos-remote.service`: `remote_view = false` →
    `ProtocolError`/`UidMismatch`, reply ≤ `MAX_MESSAGE_SIZE`, zero pixel bytes; `true` → `PreviewResponse`.
43. `test_rlc_unreadable_peer_cgroup_refuses_preview` — mock returns `Ok(None)` / `Err` → refused.
44. `test_rlc_preview_requires_local_seat_session` — fixtures: `CLASS=manager` without `SEAT` only → refused;
    `CLASS=user SEAT=seat0 REMOTE=0 ACTIVE=1` → served; `REMOTE=1` → refused; **F1**: `REMOTE` key absent → refused,
    `REMOTE=yes` (malformed) → refused (these two fail against a `REMOTE != 1` implementation); `STATE=active` without
    `ACTIVE` → served; `SessionRecord::is_local_seat_session_of(uid) == check_local_seat_session_of(uid).is_ok()` on
    every fixture; disabled enforcement → served.
45. `test_rlc_remote_first_frame_logged_once_per_connection` — captured tracing: one `info` with the fixed message
    after two non-empty frames on one connection, a new one on a second connection, none for empty frames and none for
    a `Local` peer; no event field named like `bytes|width|height|sequence|data`.
46. `test_rlc_remote_view_config_key` — `daemon.toml` `[preview] remote_view` default `false`, `true` parsed,
    `"yes"` → config error.
47. `test_rlc_local_peer_unaffected_by_remote_view` — `Local` peer with a seat session served with
    `remote_view = false`.
63. `test_rlc_view_connection_leaves_room_for_auth` (F11, ADR item 1) — default `PeerLimitsConfig`
    (`max_connections_per_uid = 2`): while uid U holds one persistent connection that has been served a
    `PreviewFrame`, a second connection of U is admitted and its `Status` request is answered (and an `Auth` request is
    admitted, i.e. not refused at admission — its verdict is irrelevant); a third concurrent connection is refused at
    admission (documents R-3 with `soos-gui` open).

### 13.8 `tests/invariants/src/remote_camera_contract.rs` (new, `mod remote_camera_contract;` in `lib.rs`)
50. `test_rlc_s1_remote_camera_dependencies` — `crates/remote/Cargo.toml` has `soos-protocol = { workspace = true }`,
    `jpeg-encoder = { workspace = true }`, `nix` with feature `time`; workspace line exactly
    `jpeg-encoder = { version = "=0.7.1" }` (no `features`, no `simd`); with `cargo` available, `cargo tree -p
    soos-remote -e normal` contains `jpeg-encoder` and `soos-protocol` and none of `wide`, `bytemuck`, `jpeg-decoder`,
    `zune-jpeg`, `image`, `soos-camera-v4l`, `v4l`, `ort`.
51. `test_rlc_s2_ijg_is_allowed_for_jpeg_encoder_only` — `deny.toml` `[licenses] allow` does not contain `IJG`; exactly
    one `exceptions` entry, `{ crate = "jpeg-encoder", allow = ["IJG"] }`, preceded by a comment naming the ADR.
52. `test_rlc_s3_camera_modules_never_log` — no `TRACING_MACROS` invocation in `camera_jpeg.rs`, `camera_ipc.rs`,
    `camera_slot.rs`; in `camera.rs` every invocation has exactly one string literal argument without `{`; `audit.rs`
    declares the three fixed messages.
53. `test_rlc_s4_no_recording_path` — `camera*.rs` contain none of `std::fs`, `tokio::fs`, `File::`, `OpenOptions`,
    `create_dir`, `std::env::temp_dir`, `tempfile`, `write_to_path`, `Command::new`.
54. `test_rlc_s5_camera_types_redacted` — `ViewToken`, `ViewOwner`: no derived `Debug`, manual `<redacted>` one;
    `PreviewFrame`, `JpegFrame`, `JpegSink`: no `Debug` at all; `ViewToken` implements `Drop`/zeroize.
55. `test_rlc_s6_page_reads_the_stream_into_a_canvas` — `app.js` contains `/api/auth/camera/options`,
    `/api/camera/start`, `/api/camera/stop`, `"/api/camera"`, `"X-Soos-Action": "camera-view"`, `"camera-stream"`,
    `"camera-stop"`, `getReader(`, `createImageBitmap(`, `drawImage(`, `AbortController`, `CAMERA_MAX_PART_BYTES =
    524288`, `Start camera view`, `Stop camera view`, `pagehide`; contains none of `createObjectURL`, `blob:`,
    `new Image(`, `"img"`, `.src =`; `http.rs` `MANDATORY_HEADERS` CSP string is byte-identical to the pre-change one
    (pinned literal); `index.html` unchanged id set (RMC-S50 already covers it).
56. `test_rlc_s7_daemon_remote_view_gate` — `crates/daemon/src/preview_peer.rs` defines `REMOTE_COMPANION_UNIT` =
    `"soos-remote.service"`; `dispatcher.rs` calls `has_local_seat_session` and no longer calls `is_active_session` in
    the preview path; the first-frame message literal is present and its `info!` has only `peer_uid`.
57. `test_rlc_s8_camera_is_documented` — `Docs/REMOTE_COMPANION.md` `## 2f.` section with needles `camera_view`,
    `camera_view_funnel`, `camera_max_view_s`, `camera_fps`, `camera_width`, `camera_quality`, `remote_view`,
    `allowed_uids`, `local seat session`, `Face ID`, `LED`, `no recording`, `multipart/x-mixed-replace`,
    `max_connections_per_uid`, `connection_timeout_ms`, `not a security boundary`, and the IJG attribution sentence
    `This software is based in part on the work of the Independent JPEG Group` (F9); §9 has a line with `live camera`
    and `recording`; `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` names the `jpeg-encoder` IJG exception;
    `Docs/DAEMON.md` documents `remote_view` and "local seat session"; `AI/DECISIONS.md` contains the ADR title.
58. `test_rlc_s9_installer_template` — `scripts/install_remote.sh` contains `# camera_view = false`,
    `# camera_view_funnel = false`, `# camera_max_view_s = 120`, `# camera_fps = 5`, `# camera_width = 640`,
    `# camera_quality = 70`; `remote_view = true` appears only in `echo` lines; no `sudo`, no write under `/etc`.
59. `test_rlc_s10_no_decoder_and_no_device_access_in_remote` — `crates/remote/src` names none of `jpeg_decoder`,
    `zune`, `image::`, `/dev/video`, `v4l`, `Decoder`.

### 13.9 Migrated (§12)
60. `test_rmc_s4_remote_is_a_leaf_crate` (owner-approved assertion migration).

---

## 14. Acceptance criteria → tests → files, and new matrix rows

| Row | Criterion (from ADR / issue) | Tests (§13 numbers → name) | Crate / file |
|---|---|---|---|
| RLC1 | Off by default on both sides; `camera_view` requires `rp_id`; Funnel flag requires `camera_view` and `allow_funnel`; bounded keys fail closed | 36 `test_rlc_camera_config_keys`, 23 `test_rlc_camera_disabled_by_default`, 46 `test_rlc_remote_view_config_key` | remote `config_tests.rs`, `camera_server_tests.rs`; daemon `preview_remote_view_tests.rs` |
| RLC2 | Dependencies: only `soos-protocol` among soos crates; `jpeg-encoder =0.7.1` default features; IJG crate-scoped; no decoder; `cargo deny check` clean | 50, 51, 59, 60 | `tests/invariants/src/remote_camera_contract.rs`, `remote_companion_contract.rs`; `cargo deny --locked check` |
| RLC3 | Daemon: preview needs a local seat session (`check_local_seat_session_of`: `CLASS=user`, `SEAT`, `REMOTE=0`, active; absent/malformed `REMOTE` refused) for every unprivileged peer | 44 `test_rlc_preview_requires_local_seat_session`, 56 | daemon `preview_remote_view_tests.rs`; invariants |
| RLC4 | Daemon: `soos-remote.service` peer refused unless `[preview] remote_view = true`; unreadable or uid-inconsistent cgroup refused; local peers unaffected; a view connection leaves the second per-UID connection for `Auth` | 41, 42, 43, 47, 63 `test_rlc_view_connection_leaves_room_for_auth` | daemon `preview_remote_view_tests.rs` |
| RLC5 | Daemon: one `info` line per remote connection on its first frame, no pixel data | 45 | daemon `preview_remote_view_tests.rs` |
| RLC6 | Every view needs a fresh UV assertion of purpose `CameraView` (distinct from unlock), same-origin CSRF, shared lockout | 24, 25, 38, 40 | remote `camera_server_tests.rs`, `routes_tests.rs`, `auth_store_tests.rs` |
| RLC7 | Tailnet only unless `camera_view_funnel = true`; anonymous Funnel refused | 26 | `camera_server_tests.rs` |
| RLC8 | Stream token: 256-bit, single use, ≤ 10 s, bound to class and Funnel session, path only, redacted | 12, 13, 16, 28, 37 | `camera_slot_tests.rs`, `camera_server_tests.rs`, `routes_tests.rs` |
| RLC9 | One global view, cooldown, `max_view_s`, `fps` without burst catch-up, width 640/320 (odd Grey/RGB24 sources floored) | 10, 11, 14, 29, 30, 31, 4, 5 | `camera_slot_tests.rs`, `camera_server_tests.rs`, `camera_jpeg_tests.rs` |
| RLC10 | Transport: `multipart/x-mixed-replace` JPEG parts with `Content-Length`; page reads with `fetch` + `ReadableStream` into a canvas; CSP unchanged; no object URL | 27, 55 | `camera_server_tests.rs`; invariants |
| RLC11 | Encoder: Grey/YUYV/RGB24 only, bounded geometry and output, baseline JPEG, no panic | 1–9 | `camera_jpeg_tests.rs` |
| RLC12 | Daemon client: ≤ 1 connection with graceful close before reconnect, proactive reconnect, one immediate retry after an idle-closed reused connection, root peer only, bounded replies and time, back-off on `RateLimited`, refusal ends view, failures map to JSON errors before the head | 17–22, 62 `test_rlc_client_retries_once_after_idle_close`, 32, 33 | `camera_ipc_tests.rs`, `camera_server_tests.rs` |
| RLC13 | No recording, no pixel or token logging, zeroized buffers, no Web Push image | 52, 53, 54, 7, 39 | invariants; `camera_jpeg_tests.rs`; `push_tests.rs` |
| RLC14 | Stream ends on stop (durable, never lost), max duration, close, write timeout, session expiry, shutdown, daemon refusal, stall; non-terminal events never cancel an in-flight exchange or a partial part | 30 (F3 sub-cases), 61 `test_rlc_non_terminal_arms_never_cancel_a_frame`, 15 | `camera_server_tests.rs`, `camera_slot_tests.rs` |
| RLC15 | Awareness: push `sooscamera` best effort at start; audit `started`/`ended`/`refused` fixed texts; documentation (incl. IJG attribution) and installer | 34, 35, 39, 57, 58 | `camera_server_tests.rs`, `push_tests.rs`; invariants |
| RLC16 | Hardware (owner): with `[preview] enabled/allowed_uids/remote_view = true` and `camera_view = true`, from the iPhone home-screen app on the tailnet a view starts after Face ID, shows live video within 1 s at ≈ 5 fps for 120 s, the LED is on, face unlock at the lock screen still works during a view (with `soos-gui` closed), the push "camera view started" arrives, Stop ends it at once, a second start within 10 s is refused, after a full logout the view is refused; optionally over Funnel with `camera_view_funnel = true` | Manual (`Docs/REMOTE_COMPANION.md` §2f) | owner only; agents never install, configure or restart |

New matrix rows `RLC1`–`RLC16` go after `RMC88` with status `⬜ Pending (Phase 2)` until evidence exists.

---

## 15. Security invariants (new)

- **RLC-S1 Single camera owner.** `soos-remote` never opens a video device, never depends on a camera, vision or
  inference crate, and obtains frames only through `RequestKind::PreviewFrame` (tests 50, 59).
- **RLC-S2 Double opt-in, fail closed.** No frame reaches `soos-remote` unless `[preview] enabled`, the UID in
  `allowed_uids`, `remote_view = true` and a local seat session; and none leaves `soos-remote` unless `camera_view`
  (and `camera_view_funnel` on the Funnel). Every unknown or failed check refuses (tests 23, 26, 42–44).
- **RLC-S3 Fresh UV assertion per view.** A web session, a cookie, an unlock challenge or a previous assertion never
  starts a view; the challenge purpose `CameraView` is single use and bound to class and session (24, 40).
- **RLC-S4 Token secrecy.** The stream token comes from the crate CSPRNG, is 256 bits, used once, valid ≤ 10 s, bound to
  its owner, compared in constant time, never in a `Route`, log, audit line, `Debug` output or push payload (12, 13,
  16, 28, 37, 52, 54).
- **RLC-S5 One view, one daemon connection.** At most one view globally; the client never holds two daemon
  connections and refuses a non-root daemon peer (11, 20, 21).
- **RLC-S6 Bounded everything.** Duration ≤ 300 s, fps ≤ 10, source ≤ 640x480, output ≤ 512 KiB per JPEG, IPC reply
  ≤ 2 MiB allocated only after the length check, every daemon exchange ≤ 3 s, every part write ≤ 2 s, stall ≤ 5 s,
  first frame ≤ 5 s, page accumulator ≤ 1 MiB (5, 7, 19, 22, 30, 55).
- **RLC-S7 No recording, no pixel logging.** No file I/O in camera modules; IPC reply, converted pixels and JPEG
  output are `Zeroizing`; no pixel, JPEG, dimension or sequence is logged; push carries fixed text only (39, 52–54).
- **RLC-S8 No decoder.** Only Grey, YUYV and RGB24 are converted; any other format ends the view (6, 32, 59).
- **RLC-S9 No stale replay.** A frame is sent only when its sequence differs from the last sent one; no frame is
  buffered between views (31).
- **RLC-S10 Termination.** Every end condition of ADR item 5 ends the stream and frees the slot through a drop guard,
  including cancellation and a panic of the encoding task; a stop is durable (flag + `watch`) and is honoured within
  one iteration; only terminal events may drop an in-flight frame step, and a part is always written whole (30, 61).
- **RLC-S11 Awareness never blocks.** Queuing the camera push is non-blocking and its failure never affects the view;
  audit lines are fixed text, `refused` is rate gated (34, 35).
- **RLC-S12 Page hygiene.** CSP unchanged; no `blob:` URL, no object URL, no `<img>` stream, no storage, text through
  `textContent` only; the view is aborted on hide/pagehide (55; existing RMC-S8/S18/S49/S50).
- **RLC-S13 Daemon classification is administrative.** The cgroup check is fail-closed but documented as **not** a
  security boundary against code running as the owner (57; residual R-1).

Touched existing invariants: RC-1 (still never root, never a network socket — the new socket is `AF_UNIX`), RC-4
(new bounds), RC-5 (no new identity logging; no `*_id` fields), RMC-S2, RMC-S4 (migrated), ADR 2026-09-29 preview
rules (amended by the 2026-10-07 ADR), ARCHITECTURE §2.3 Invariant 3 (stricter for preview).

---

## 16. Documentation drift, ADR precision, open questions

- **D-1 (resolved)**: the amended ADR item (7) states that only buffers owned by `soos-remote` are zeroized and the
  encoder's internal scratch is an accepted residual (§6.4).
- **D-2 (resolved)**: the amended ADR item (4a) requires `REMOTE=0`; the spec reuses `check_local_seat_session_of`
  (S-7, §9.1, F1).
- **D-3 (daemon socket path)**: `/run/soos/daemon.sock` is now defined in a fifth place (`CAMERA_DAEMON_SOCKET_PATH`;
  pam, admin-cli, gui, daemon config already duplicate it). Not fixed here; candidate for a later `soos-protocol`
  constant.
- **D-4 (Docs)**: `Docs/REMOTE_COMPANION.md` §1 ("what it is not") and §9, `Docs/DAEMON.md` §1.5/§3 ("active local
  session" → "active local seat session"; `PreviewFrame` row), `Docs/IPC_PROTOCOL.md` (preview consumers: GUI and
  `soos-remote`; format constants), `Docs/GUI_APPLICATION.md` (the preview now needs a seat session: running
  `soos-gui` from SSH or after logout no longer previews), `AI/ARCHITECTURE.md` §13 (paragraph + privacy row: the
  view token joins the never-logged list). `research_live_camera.md` stays as a research record.
- **D-5 (round 2, F8)**: also update `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` l.119 (licence policy: permissive
  licences plus the crate-scoped `IJG` exception for `jpeg-encoder`, ADR 2026-10-07) and l.352 (frames leave the
  daemon only through the preview stream, to the GUI **and** `soos-remote`, for a peer with a local seat session);
  `.claude/skills/dev-workflow/references/project-facts.md` (crate map: `soos-remote` may depend on `soos-protocol`,
  no longer a soos-free leaf; add the camera constants row and `CAMERA_DAEMON_*`); the `ChallengeStore` doc comment
  and bound in `crates/remote/src/challenge.rs` ("3 × `MAX_PENDING_CHALLENGES`" becomes "5 ×": `(Tailnet, Register)`,
  `(Tailnet, Unlock)`, `(Funnel, Unlock)`, `(Tailnet, CameraView)`, `(Funnel, CameraView)`).
- **D-6 (round 2, F9)**: IJG attribution. `Docs/REMOTE_COMPANION.md` §2f carries the sentence "This software is based
  in part on the work of the Independent JPEG Group." with a note that it comes through `jpeg-encoder`; a README
  third-party note, if one is added later, repeats it. Pinned by test 57.
- **Q-1 (resolved by the orchestrator)**: S-3 — the camera card is built at runtime by `app.js` (`createElement`, no
  `id`); RMC-S50 is not migrated.
- **Q-2 (owner check)**: iOS standalone app behaviour of `fetch` streaming and `createImageBitmap` memory over 120 s
  (research §5.1) is verified only by RLC16.

### Residual risks (for `Docs/REMOTE_COMPANION.md` §8)

- **R-1** Any process of the owner's uid can already read frames through the GUI preview (`allowed_uids`), and can
  start a unit named `soos-remote.service` or run `soos-remote` outside the unit (then it is classified `Local` and
  bypasses `remote_view`). `remote_view` is an administrative opt-in and an audit aid, not a boundary.
- **R-2** iCloud Keychain compromise = passkey compromise (as for unlock); a stolen Funnel cookie alone gives nothing
  without Face ID, but can call `POST /api/camera/stop` (harmless).
- **R-3** A view holds one of the uid's 2 daemon connections and ≤ 10/s of the shared 40/s preview quota: with
  `soos-gui` open as well, a uid-1000 lock screen's face request is refused at admission and falls back to the
  password (fail-safe). Documented; close `soos-gui` during a view. At each proactive reconnect (≤ every 25 s) the
  client half-closes and waits ≤ 200 ms for the daemon's EOF (F5); if the daemon has not released the old permit by
  then, a PAM request arriving in that sub-second window can still be refused at admission (password fallback).
- **R-4** Pixels transit `tailscaled` (and, over Funnel, Tailscale relays, TLS-terminated on the PC); kernel socket
  buffers and `tailscaled`'s Go heap are outside our control; the phone can screen-record.
- **R-5** The camera LED is the only local indicator; there is no on-screen indicator (out of scope).
- **R-6** The cgroup is read by PID after `SO_PEERCRED`; PID reuse within a connection is theoretically possible and
  only affects the administrative classification.

---

## 17. Test hooks for the tester (summary)

- `ServerState::with_camera(settings: CameraSettings, factory: PreviewSourceFactory) -> Self` (ignored unless
  `config.camera.enabled`); `CameraSettings { jpeg: JpegSettings, fps: u32, max_view: Duration }` derived by
  `CameraSettings::from_config(&CameraConfig)`.
- `PreviewSource` trait (scripted source; the tester's `ScriptedPreviewSource` records started, completed and dropped (cancelled) `next_frame` calls for test 61), `DaemonPreviewClient::with_expected_daemon_uid`, `JpegSink`
  test-capacity constructor (`#[doc(hidden)]`), existing `with_random`, `with_unix_clock`, paused tokio time,
  `FakeTransport` for push, captured tracing subscriber for audit.
- Daemon: `ConnectionDispatcher::with_preview_peer_source(Arc<dyn LogindSource>)`, existing
  `with_session_validator`, `with_preview_config`.
- Production wiring (`main.rs`) calls none of the `#[doc(hidden)]` hooks (existing RMC-S22 style check applies).

## 18. Exit criteria

Every ADR item (1)–(9) and every orchestrator default maps to a type, constant or behaviour above and to at least one
test (§14); every new field has a stated bound (§3, §4); every existing test that changes is listed with its kind
(§12); Q-1, D-1 and D-2 are resolved (§16); every round-1 finding is resolved (§19).

## 19. Round 2 — resolution of the plan evaluation (`AI/plan_evaluator_report.md`, round 1)

| Finding | Resolution | Sections | Tests |
|---|---|---|---|
| F1 MAJOR seat predicate | `is_local_seat_session_of(uid)` ≡ `check_local_seat_session_of(uid).is_ok()` (`REMOTE=0`, absent/malformed refuses); D-2 closed | S-7, §9.1, §14 RLC3, §16 | 44 (+ `REMOTE` absent and `REMOTE=yes` fixtures) |
| F2 MAJOR cancellation | frame step pinned and polled by `&mut`, recreated only after completion; part write in the arm body; only terminal arms drop it; read half drained into a 64-byte sink, ends only on `Ok(0)`/`Err`; same for the first-frame phase | §5.3, §8.6 step 5, §8.7 rules 1–2 | 61 (new), 30(c) |
| F3 MAJOR lost stop | durable slot flag + `watch` epoch; check at the top of every iteration and after every arm body; `stopped` ⇒ end within one iteration | §7.2, §8.6 step 5, §8.7 rule 3, §8.8 | 30(a), 30(b) |
| F5 reconnect race | `shutdown(Write)` + bounded wait (`CAMERA_DAEMON_CLOSE_WAIT_MS` 200) for the daemon's EOF before the next connect; residual window in R-3 | §3.2, §5.2 step 1, §15 R-3 | 20 (extended) |
| F12 idle close | one immediate retry with a new nonce after an `Io` on a reused connection before any reply byte; docs recommend `connection_timeout_ms` > `1000/camera_fps`; dangling §7.4 reference fixed | §3.2, §5.2 step 3a | 62 (new), 57 needle |
| F6 geometry | even width for YUYV only; odd Grey/RGB24 accepted; `Half` floors | §6.2, §6.3 | 4, 5 (extended) |
| F9 IJG attribution | sentence in `Docs/REMOTE_COMPANION.md` §2f | §16 D-6 | 57 needle |
| F8 doc drift | `SECURITY_AND_QUALITY_GUIDELINES.md` l.119 and l.352, `project-facts.md`, `ChallengeStore` doc bound | §16 D-5 | 57 needle (guidelines) |
| F10 Funnel permits | connection + `funnel` permits held for the stream lifetime, no body-read guard, no anonymous permit, no SSE slot | §8.6 "Permits held by a stream" | 26, 30 (existing) |
| F11 test power | 41: `user@1000` with `peer_uid` 1001 → `Malformed`; 31: no burst catch-up after a slow exchange; new daemon admission test | §13.4–13.7, §14 RLC4/RLC9 | 41, 31, 63 (new) |
| Wording | §8.5 step 5 "body absent" | §8.5 | — |
| Q-1 / D-1 | confirmed: S-3 runtime DOM; D-1 resolved by the ADR amendment | header, §6.4, §16 | — |

Environment note for Phase 2: `jpeg-encoder` is not in the local cargo cache; run `cargo fetch` once after the
manifest change, before the `cargo tree --offline --locked` invariants (tests 50) and `cargo deny --locked check`.
