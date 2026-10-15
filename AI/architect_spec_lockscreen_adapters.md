# Architect Spec — GitHub #401: `soos-lockstatus` bridge and swaylock-plugin / hyprlock adapters

- **Date**: 2026-10-15
- **Issue**: GitHub #401 (PR 2 of epic #352; follows #380 merged in PR #388). GitHub-only issue: no
  backlog id, branch not registered in `scripts/sync_issue.py`; the commit carries `Closes #401`.
- **Branch**: `feat/lockscreen-adapters` (from `origin/main` 2025738)
- **Owner decisions applied** (#352, 2026-10-12 and 2026-10-13): Q1 swaylock-plugin + hyprlock
  first (owner host: driftwm + swaylock-plugin); Q2 the locked screen shows only Scanning / Idle /
  Not recognized, every failure collapsed; Q3 animate for PAM face requests and presence scans alike
  (the daemon already publishes both, #380); brand + motion system of #358 (ADR 2026-10-13 "Motion
  System", `Docs/UI_MOTION.md`).
- **Walkthrough**: 214 (reserved).

## 1. Scope & Blast Radius

The epic split planned PR 2 = bridge and PR 3 = swaylock-plugin renderer + hyprlock snippet. The
owner's instruction for this run merges them: the client side for swaylock-plugin **and** hyprlock.

New, user-level only (never root, never in the daemon or PAM):

| Crate dir | Package | Kind | Purpose |
|---|---|---|---|
| `crates/lockstatus` | `soos-lockstatus` | lib + bin `soos-lockstatus` | bounded blocking `LockState` client; `--once` (hyprlock `cmd[update:N]`) and `--watch` (JSON lines for later GNOME/KDE adapters); presentation policy; lock-screen motion tokens |
| `crates/lock-widget` | `soos-lock-widget` | lib + bin `soos-lock-widget` | wlr-layer-shell background program for `swaylock-plugin --command`; pure scene / layout / raster modules + a thin Wayland loop |

Both are business crates with `#![forbid(unsafe_code)]` (no `unsafe` is needed: smithay-client-toolkit
offers a safe shm pool and layer-shell API).

Touched existing files (no behavior change of any existing component):

- Root `Cargo.toml`: two `members`, `soos-lockstatus` / `soos-lock-widget` in
  `[workspace.dependencies]`, `smithay-client-toolkit = { version = "0.20", default-features = false,
  features = ["calloop"] }` in `[workspace.dependencies]` (already locked at 0.20.0 with exactly these
  features through `smithay-clipboard` 0.7.3; **no new crate enters `Cargo.lock`** besides the two
  workspace members). `Cargo.lock` gains only the two path packages.
- `tests/invariants/src/lib.rs`: `test_business_crates_forbid_unsafe_code` list gains `"lockstatus"`
  and `"lock-widget"` (strengthening only; no existing assertion changes) and a new module
  `lockscreen_adapters_contract`.
- `scripts/candid_review.sh` `BUSINESS_CRATES`: same two names.
- Docs: new `Docs/LOCKSCREEN_WIDGET.md`; `Docs/IPC_PROTOCOL.md` §14 client contract (implemented);
  `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.3/§5.5 pointer; `AI/ARCHITECTURE.md` and `AGENTS.md` workspace
  layout; `AI/DECISIONS.md` new ADR; `.claude/skills/dev-workflow/references/project-facts.md` crate
  map; `AI/VERIFICATION_MATRIX.md` new component; walkthrough 214.
- `packaging/lockscreen/hyprlock-soos.conf` (new, documentation snippet, installed by nobody in this
  PR; the installer opt-in is PR 6 of #352).

Not touched: `crates/pam` (byte-for-byte), `crates/daemon`, `crates/protocol` (consumed as is),
socket path/permissions, systemd units, `scripts/install.sh` / `uninstall.sh` / packages (PR 6).

Consumers of new public items: only the two new crates and the invariants. `soos-lock-widget`
depends on `soos-lockstatus` (client, view, presentation, motion tokens). No other crate depends on
either.

## 2. Types & Signatures

### 2.1 `soos-lockstatus` (`crates/lockstatus/src/`)

Dependencies: `soos-protocol`, `nix` (workspace + `time`; `socket` and `user` come from the workspace
features), `getrandom`, `thiserror`, `clap`. **Never** `tokio`, `zbus`, a network crate, `serde_json`,
`tracing`.

`lib.rs` — `#![forbid(unsafe_code)]`, modules `client`, `view`, `format`, `motion`, `poll`, `args`.

Constants (`lib.rs`, single source):

| Constant | Value | Meaning / sentinel |
|---|---|---|
| `DEFAULT_SOCKET_PATH` | `"/run/soos/daemon.sock"` | `--socket` overrides (tests, diagnostics) |
| `CLIENT_SERVICE` | `"soos-lockstatus"` | `Request::service` (≤ `MAX_SERVICE_LEN`) |
| `EXCHANGE_BUDGET_MS` | `250` | one cumulative deadline for connect + write + read of one poll |
| `ONCE_EXIT_BUDGET_MS` | `300` | documented upper bound of a `--once` run (budget + start-up/print margin) |
| `DEFAULT_POLL_INTERVAL_MS` | `500` | `--watch` and the widget |
| `MIN_POLL_INTERVAL_MS` | `250` | 4 Hz cap; `--interval-ms` below (including `0`) is clamped up |
| `MAX_POLL_INTERVAL_MS` | `2000` | `--interval-ms` above is clamped down |
| `ERROR_BACKOFF_MS` | `[1000, 2000, 4000]` | delay after the 1st, 2nd, ≥3rd consecutive failed poll (never below the interval); reset by a successful poll |
| `MAX_JSON_LINE_BYTES` | `96` | upper bound of one JSON line including `\n` (the longest line is 76 bytes) |

`client.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum LockStatusError {
    #[error("daemon socket unreachable")] Connect(#[source] std::io::Error),
    #[error("daemon socket I/O failed")]  Io(#[source] std::io::Error),
    #[error("lock state exchange timed out")] Timeout,
    #[error("invalid response frame length {0}")] FrameLength(usize), // 0 or > MAX_MESSAGE_SIZE
    #[error("undecodable daemon response")] Decode,
    #[error("daemon refused the lock state request")] Refused,      // a standard Response
    #[error("unsupported lock state version {0}")] Version(u8),
    #[error("monotonic clock unavailable")] Clock,
    #[error("random request id unavailable")] Random,
}

/// One bounded `LockState` exchange on a fresh connection, closed afterwards.
pub fn query_lock_state(socket: &Path, uid: u32, budget: Duration)
    -> Result<LockStateResponse, LockStatusError>;

/// `query_lock_state` mapped to a view: every error is `LockView::UNAVAILABLE`.
pub fn fetch_view(socket: &Path, uid: u32, budget: Duration) -> LockView;
```

Exchange algorithm (mirrors `crates/admin-cli/src/test_pam.rs`, the reviewed pattern):

1. `ExchangeDeadline::start(budget)`; `budget == 0` → `Timeout` without any socket call.
2. `deadline_monotonic_ns = CLOCK_MONOTONIC + budget` (`Clock` on failure); 32 random request-id
   bytes from `getrandom` (`Random`); `Request { version: CURRENT_VERSION, kind: LockState,
   request_id, uid_hint: uid, service: CLIENT_SERVICE, deadline_monotonic_ns }` encoded with
   `encode_request` (tagged client frame).
3. Non-blocking `socket(AF_UNIX, SOCK_STREAM, NONBLOCK|CLOEXEC)` + `connect` retried every 5 ms on
   `EAGAIN/EINTR/EINPROGRESS/EALREADY` until the deadline; switched back to blocking; every `write` /
   `read` re-arms `SO_SNDTIMEO` / `SO_RCVTIMEO` with the budget left (never `Duration::ZERO`);
   `WouldBlock | TimedOut` → `Timeout`; `Interrupted` retried; `Ok(0)` read → `Io(UnexpectedEof)`
   (an old daemon closing on `kind = 3`).
4. Read the 4-byte big-endian length; `0` or `> MAX_MESSAGE_SIZE` → `FrameLength` **before**
   allocating; read exactly that many bytes (byte-counted).
5. Strict decode of `len ‖ body`: first as `LockStateResponse`; on failure as `Response` →
   `Refused` (any verdict, the verdict is never used); otherwise `Decode`. A decoded
   `LockStateResponse` with `!is_current()` → `Version`.
6. The stream is dropped (closed) in every path: **one request per connection**.

The request carries no secret; nothing of the answer is logged (the crate has no logger).

`view.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewState { Unavailable, Idle, Scanning, NotRecognized }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LockView { pub state: ViewState, pub sequence: u64 }

impl LockView {
    pub const UNAVAILABLE: Self = Self { state: ViewState::Unavailable, sequence: 0 };
    pub fn from_response(r: &LockStateResponse) -> Self; // !is_current() → UNAVAILABLE
    pub fn presented(&self) -> Presented;               // Unavailable → Presented::Idle
    pub fn available(&self) -> bool;                    // state != Unavailable
}

/// The only states a locked screen may show (owner decision Q2 of 2026-10-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Presented { Idle, Scanning, NotRecognized }
```

`LockWidgetState::{Idle, Scanning, NotRecognized}` map one to one to the `ViewState` of the same
name; there is no success value and none is invented.

`format.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat { Text, Glyph, Label, Json }

pub const SCANNING_TEXT: &str = "Scanning";
pub const NOT_RECOGNIZED_TEXT: &str = "Not recognized";
pub const IDLE_TEXT: &str = "";                       // Idle (and Unavailable) shows no words
pub const IDLE_GLYPH: &str = "\u{2726}";              // ✦ four-pointed star (brand mark)
pub const SCANNING_GLYPHS: [&str; 4] = ["\u{25DC}", "\u{25DD}", "\u{25DE}", "\u{25DF}"]; // ◜◝◞◟
pub const SCANNING_STATIC_GLYPH: &str = "\u{25CC}";   // ◌ (reduced motion)
pub const NOT_RECOGNIZED_GLYPH: &str = "\u{2715}";    // ✕

pub fn text(p: Presented) -> &'static str;
/// Scanning steps one quarter per `T_PENDING_MS` of `now_ms` (CLOCK_MONOTONIC milliseconds);
/// reduced motion → `SCANNING_STATIC_GLYPH`.
pub fn glyph(p: Presented, now_ms: u64, prefs: MotionPrefs) -> &'static str;
/// `glyph` alone when the text is empty, else `"{glyph}  {text}"` (two spaces).
pub fn label(p: Presented, now_ms: u64, prefs: MotionPrefs) -> String;
/// `{"state":"<idle|scanning|not_recognized>","available":<bool>,"sequence":<u64>}` without `\n`;
/// `state` is the **presented** state; `available` is false only for transport errors.
pub fn json(view: &LockView) -> String;
/// The `--once` stdout bytes: text/glyph/label without a trailing newline (hyprlock renders stdout
/// verbatim as Pango markup), JSON with one `\n`.
pub fn once_output(format: OutputFormat, view: &LockView, now_ms: u64, prefs: MotionPrefs) -> String;
```

No output contains `<`, `>` or `&` (Pango markup safe) and none contains a reason, score, frame,
embedding, template, session or timestamp field.

`motion.rs` — lock-screen mirror of `Docs/UI_MOTION.md` §2 (the GUI `theme.rs` tokens land with
`feat/gui-motion`; the invariant compares these values with the normative table):

```rust
pub const T_SHORT_MS: u64 = 180;      // --t-short   (ring colour cross-fade)
pub const T_MEDIUM_MS: u64 = 260;     // --t-medium  (Not recognized shake)
pub const T_PENDING_MS: u64 = 1200;   // --t-pending (one quarter turn of the pending star)
pub const AMBIENT_REPAINT_MS: u64 = 33; // UI_MOTION §9 ambient loop cap (≤ 30 Hz)
pub const SLIDE_PX: f32 = 8.0;        // --slide     (shake amplitude)
pub const EASE_STANDARD: [f32; 4] = [0.2, 0.0, 0.0, 1.0];
pub const REDUCE_MOTION_ENV: &str = "SOOS_REDUCE_MOTION";
pub const MAX_REDUCE_MOTION_ENV_BYTES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MotionPrefs { pub reduced: bool }

/// `--reduce-motion` → reduced; else `SOOS_REDUCE_MOTION` `1` → reduced, `0` → full motion,
/// anything else (or more than 8 bytes, or not UTF-8) ignored; default full motion.
pub fn resolve_motion_prefs(flag: bool, env: Option<&OsStr>) -> MotionPrefs;

/// Cubic Bézier easing `y(x)` with control points `(x1, y1, x2, y2)`; `t` clamped to `[0, 1]`,
/// non-finite `t` → 0; f(0) = 0, f(1) = 1; result always finite.
pub fn cubic_bezier(points: [f32; 4], t: f32) -> f32;
```

No desktop portal is read (that would need zbus on the session bus, which the lock-state ADR item 7
excludes from this path); see §8 "Owner to confirm".

`poll.rs`:

```rust
pub fn clamp_interval_ms(requested: u64) -> u64;           // into [MIN, MAX]

#[derive(Debug, Clone, Default)]
pub struct Backoff { failures: u32 }
impl Backoff {
    /// Delay before the next poll: `interval` after a success (resets), else
    /// `max(interval, ERROR_BACKOFF_MS[min(failures, 3) - 1])`.
    pub fn next_delay(&mut self, interval: Duration, ok: bool) -> Duration;
}

pub struct Poller { /* socket, uid, interval, backoff */ }
impl Poller {
    pub fn new(socket: PathBuf, uid: u32, interval_ms: u64) -> Self; // interval clamped
    /// One poll: `(view, delay before the next poll)`.
    pub fn poll(&mut self) -> (LockView, Duration);
}

/// `--watch`: writes `json(view)` + `\n` for the first view and for every change of
/// `(presented, available, sequence)`, sleeping the poller delay in between; returns `Ok(())`
/// when `keep_running()` turns false, `Err` as soon as a write fails (closed pipe).
pub fn watch<W: Write>(poller: &mut Poller, out: &mut W, keep_running: &dyn Fn() -> bool)
    -> std::io::Result<()>;
```

`args.rs` (clap derive): `soos-lockstatus (--once | --watch) [--format text|glyph|label|json]
[--socket PATH] [--interval-ms N] [--reduce-motion]`; `--once` and `--watch` are a required,
mutually exclusive group; `--format` defaults to `text` and is accepted with `--once` only
(`--watch` is always JSON); `--interval-ms` with `--watch` only. Usage errors are clap's (stderr,
exit 2): they are configuration mistakes, not daemon failures.

`main.rs`: installs a silent panic hook, runs inside `catch_unwind`; `--once` prints
`once_output` (a caught panic prints the Idle rendering of the format), ignores a stdout write error,
never writes to stderr, and **always exits 0**; `--watch` exits 0 when stdout closes, 1 on a caught
panic (stderr one line, no state).

### 2.2 `soos-lock-widget` (`crates/lock-widget/src/`)

Dependencies: `soos-lockstatus`, `smithay-client-toolkit` (workspace: 0.20, `calloop` only; its
`reexports::{client, calloop, calloop_wayland_source}` are used, no direct `wayland-client` /
`calloop` dependency), `clap`. Never `tokio`, `zbus`, a network crate, a font or image crate.

`lib.rs` — `#![forbid(unsafe_code)]`, modules `palette`, `scene`, `layout`, `raster`, `font`,
`render`, `wayland`, `args`.

`palette.rs` — mirrors `crates/gui/src/theme.rs` by name and value (brand ADR 2026-10-06 item 1):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);
pub const BLUE: Rgb = Rgb(0x00, 0x47, 0xBB);
pub const PALE: Rgb = Rgb(0xED, 0xF1, 0xFF);
pub const PALE_4: Rgb = Rgb(0xC9, 0xD6, 0xFA);
pub const PINK: Rgb = Rgb(0xE5, 0x9B, 0xDC);
pub const WARN_BORDER: Rgb = Rgb(0xF5, 0xB5, 0x47);
// Roles
pub const BACKGROUND: Rgb = BLUE;
pub const RING_IDLE: Rgb = PALE_4;
pub const RING_SCANNING: Rgb = PALE;
pub const RING_NOT_RECOGNIZED: Rgb = WARN_BORDER;
pub const STAR: Rgb = PINK;
pub const LABEL: Rgb = PALE;
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb;   // t clamped [0,1], non-finite → 0; rounded per channel
pub fn ring_color(p: Presented) -> Rgb;
```

Flat colours only: no gradient, shadow, blur or glow (motion ADR item 1).

`scene.rs` — pure animation state machine (milliseconds of a monotonic clock supplied by the
caller; never reads a clock itself):

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSpec {
    pub ring: Rgb,
    pub star_angle_deg: f32,   // in [0, 90)
    pub offset_x: f32,         // in [-SLIDE_PX, SLIDE_PX]
    pub label: &'static str,   // format::text(presented)
    pub animating: bool,       // another frame is needed
}

pub struct Scene { /* prefs, key, entered_ms, from_ring, pending_anchor_ms, pending_stop_ms */ }
impl Scene {
    pub fn new(prefs: MotionPrefs, now_ms: u64) -> Self;          // Idle, settled
    /// Applies a polled view. The scene key is `(presented, sequence)`, with every Idle view
    /// (Unavailable included) keyed `(Idle, 0)`; an unchanged key is a no-op.
    pub fn update(&mut self, view: LockView, now_ms: u64);
    pub fn frame(&self, now_ms: u64) -> FrameSpec;
}
```

Rules (full motion):

- **Label first**: the label of the new state is in the first frame after `update` (motion ADR
  "security truth first"); no fade on text.
- **Ring colour**: cross-fades from the colour shown at `update` time to `ring_color(new)` over
  `T_SHORT_MS` with `EASE_STANDARD`.
- **Star** (signature move): while Scanning it turns one quarter turn per `T_PENDING_MS` with
  `EASE_STANDARD` per quarter (angle = 90 × ease(frac), anchored at the scan entry). Leaving
  Scanning, it finishes the current quarter (stops at the next multiple of `T_PENDING_MS` after the
  anchor; a 90° turn of the four-fold mark is identical to 0°) and then rests at 0°. Never 45°: no
  lock-screen state lowers protection (motion ADR item 5).
- **Not recognized**: one horizontal shake of the whole group, `offset = SLIDE_PX · sin(4π·t) ·
  (1 − t)`, `t = elapsed / T_MEDIUM_MS`, then 0.
- **Idle**: nothing moves; `animating` is false once the colour fade and any quarter turn end.

Reduced motion: colours change instantly, the star stays at 0°, no shake; `animating` is false on the
first frame after `update`. Out-of-order clocks (`now < entered`) saturate to 0 elapsed; never panics.

`layout.rs`:

```rust
pub const MAX_OUTPUTS: usize = 8;                 // surfaces beyond are not created
pub const MAX_SURFACE_DIM: u32 = 8192;            // per axis, buffer pixels
pub const MAX_SURFACE_PIXELS: u64 = 33_554_432;   // per surface: 8K (7680 x 4320) fits; 128 MiB per buffer
pub const MAX_TOTAL_PIXELS: u64 = 67_108_864;     // across all surfaces (x 2 buffers x 4 B = 512 MiB worst case)
pub const BUFFERS_PER_SURFACE: usize = 2;
pub const WIDGET_CENTER_Y_FRACTION: f32 = 0.78;   // below swaylock's centred indicator

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect { pub x: u32, pub y: u32, pub w: u32, pub h: u32 }

/// Bytes of one ARGB8888 buffer, `None` when 0, above a bound, or overflowing.
pub fn buffer_len(width: u32, height: u32) -> Option<usize>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout { pub width: u32, pub height: u32, pub cx: f32, pub cy: f32, pub radius: f32,
                    pub stroke: f32, pub star_half: f32, pub label_scale: u32, pub label_top: f32,
                    pub widget: Rect }
impl Layout {
    /// `None` for a zero-sized surface. `radius = clamp(round(min(w,h) · 0.045), 20, 72)`,
    /// `stroke = max(2, round(radius / 12))`, `star_half = 0.55 · radius`,
    /// `label_scale = clamp(radius / 12, 2, 6)`, the label 0.5 · radius below the ring.
    /// `widget` encloses ring, star and the widest label at every shake offset (+2 px), clipped
    /// to the surface.
    pub fn for_surface(width: u32, height: u32) -> Option<Self>;
}
```

`raster.rs` — CPU drawing into one ARGB8888 little-endian buffer (bytes `B, G, R, A`, alpha 0xFF):

```rust
pub struct Canvas<'a> { /* data, width, height */ }
impl<'a> Canvas<'a> {
    /// `None` unless `data.len() >= buffer_len(width, height)`.
    pub fn new(data: &'a mut [u8], width: u32, height: u32) -> Option<Self>;
    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgb>;
    pub fn fill_rect(&mut self, rect: Rect, color: Rgb);                      // clipped
    /// Anti-aliased ring (coverage from the pixel-centre distance), blended over the canvas,
    /// limited to `clip`. Non-finite or non-positive geometry draws nothing.
    pub fn ring(&mut self, cx: f32, cy: f32, radius: f32, stroke: f32, color: Rgb, clip: Rect);
    /// The brand star (`STAR_SPIKES` of `crates/gui/src/brand.rs`: the two concave spikes of the
    /// 508-unit view box, i.e. the top-right and bottom-left quadrants minus the quarter discs of
    /// radius 0.5 centred on the outer corners), half-size `half`, rotated by `angle_deg`
    /// clockwise, 4 × 4 supersampled, limited to `clip`.
    pub fn star(&mut self, cx: f32, cy: f32, half: f32, angle_deg: f32, color: Rgb, clip: Rect);
    /// Bitmap text (`font.rs`, 5 × 7 cells, 1 cell gap), centred on `cx`, integer `scale`.
    pub fn text(&mut self, cx: f32, top: f32, scale: u32, text: &str, color: Rgb, clip: Rect);
}
/// Normalised star predicate in the unit square (v pointing down).
pub fn star_contains(u: f32, v: f32) -> bool;
```

`font.rs`: `pub fn glyph(c: char) -> Option<[u8; 7]>` for exactly the characters of the two labels
(`S c a n i g N o t r e z d` and space; any other character is a blank cell);
`pub fn text_width(text: &str, scale: u32) -> u32` (saturating).

`render.rs`:

```rust
/// Fills the whole canvas with BACKGROUND, then `paint_widget`.
pub fn paint_full(canvas: &mut Canvas<'_>, layout: &Layout, frame: &FrameSpec);
/// Repaints only `layout.widget`: background, ring, star, label shifted by `offset_x`.
pub fn paint_widget(canvas: &mut Canvas<'_>, layout: &Layout, frame: &FrameSpec);
```

Pixels outside `layout.widget` are never written by `paint_widget` (damage tracking relies on it).

`wayland.rs` — the only module touching Wayland (not unit-testable without a compositor; owner rows):

- Connects with `Connection::connect_to_env` (swaylock-plugin hands the program its private
  connection through `WAYLAND_SOCKET`, which `connect_to_env` honours; `WAYLAND_DISPLAY` otherwise); binds `wl_compositor`, `wl_shm`, `zwlr_layer_shell_v1`
  (any missing global → error, exit 1); binds **no seat** (no pointer, keyboard or touch object).
- One layer surface per advertised `wl_output` (at most `MAX_OUTPUTS`): `Layer::Background`, anchored
  to all four edges, size `(0, 0)`, exclusive zone `-1`, `KeyboardInteractivity::None`, namespace
  `soos-lock-widget`; a removed output drops its surface and buffers.
- Per surface `BUFFERS_PER_SURFACE` buffers allocated for the configured size (`buffer_len`; a
  surface over the per-surface or total pixel budget is not drawn). A buffer is fully painted the
  first time it is used, then only `layout.widget` is repainted and damaged.
- One `calloop` loop: the Wayland source; a channel source fed by one poller thread
  (`std::thread`, `Poller` with `DEFAULT_POLL_INTERVAL_MS` or `--interval-ms`; sends only changed
  views, `sync_channel(4)` + `try_send`, keeping the newest unsent view); a timer that redraws every
  `AMBIENT_REPAINT_MS` only while `FrameSpec::animating`, and is not re-armed once settled (idle = 0
  frames).
- Exits 0 when every layer surface is `closed` or the compositor disconnects, 1 on a setup error
  (one stderr line, never the state). swaylock-plugin restarts the program whenever it closes its
  connection (`swaylock-plugin(1)`, `--command`), so every exit 1 is preceded by
  `SETUP_FAILURE_EXIT_DELAY_MS` (1000 ms, `wayland.rs`): a broken setup restarts at most about once
  per second instead of spinning.
- swaylock-plugin requires the program to "promptly provide a full-size background layer surface for
  each Wayland output" (`--command`); with `--command-each` each instance sees one output. Both work;
  `--command` (one process, one poller) is the documented default.

`args.rs`: `soos-lock-widget [--socket PATH] [--interval-ms N] [--reduce-motion]`.

`main.rs`: silent panic hook + `catch_unwind`; exit codes above.

## 3. Constants & Config

All in §2 with locations. No new config file, no new config key of any existing component, no
environment variable besides `SOOS_REDUCE_MOTION` (already defined by the motion ADR for the GUI;
same semantics). Sentinels: `--interval-ms 0` → 250 ms (clamped, never a busy loop); a budget of
`0` → immediate `Timeout` → Unavailable; an empty `SOOS_REDUCE_MOTION` → ignored.

## 4. Error Taxonomy

| Source | Result |
|---|---|
| Socket missing / permission denied / connect refused / old daemon EOF / stall / bad length / undecodable / refusal `Response` (UidMismatch, RateLimited, …) / version / clock / random | `LockView::UNAVAILABLE` → presented `Idle` (no words, idle glyph, idle widget); `--once` exit 0, empty stderr |
| `LockStateResponse { Idle | Scanning | NotRecognized }` | the same presented state |
| Widget Wayland setup failure | exit 1, one stderr line; swaylock-plugin keeps its own background and the password prompt |
| Panic in either binary | caught; `--once` prints the Idle rendering and exits 0 |

No path produces a success state: there is no `Recognized`/"unlocked" rendering at all.
No component here can unlock, start a scan, wake the camera or send input.

## 5. Latency Budget

Not on the auth path. `--once`: ≤ `EXCHANGE_BUDGET_MS` (250) blocking + process start/print, target
`ONCE_EXIT_BUDGET_MS` (300). The daemon answers from a snapshot after a sessions scan bounded by
`LOCK_STATE_SESSION_CHECK_TIMEOUT_MS` (250): an answer delayed past our 250 ms budget renders Idle
(fail quiet). Quota: the daemon allows 8 `LockState` requests per second per UID; the widget polls at
2 Hz per process, a hyprlock label at `update:500` 2 Hz per label instance (documented arithmetic,
§`Docs/LOCKSCREEN_WIDGET.md`). Connection slots: one short connection per poll, so the per-UID
regular cap (2) is held for at most one exchange at a time per process and the auth-reserved slot
of `pam_soos.so` is never needed by these clients.

## 6. Invariants Touched

- ARCHITECTURE §2: IPC Unix socket only (unchanged socket, 0660 `root:soos`); no biometric data in
  IPC (the client reads 3 coarse states + a counter); PAM untouched.
- ADR 2026-10-13 "Read-Only Lock-Screen State Channel": client contract of §14 (strict decode order,
  ≤ 1 connection, ≤ 4 Hz, back-off on refusal/EOF); no zbus in this path.
- ADR 2026-10-13 "Motion System": tokens mirrored, reduced motion mandatory, idle = no repaint,
  ambient ≤ 30 Hz, only transform-like motion (rotation, translation) and colour cross-fades.
- ADR 2026-10-06 brand: palette mirrored by name and value, flat colours.
- Matrix: LKS17 (owner) becomes checkable with `soos-lockstatus`; new component rows LKA1–LKA16.

## 7. Test Hooks for Tester

- `client::query_lock_state` / `fetch_view` take the socket path and budget → a std `UnixListener`
  mock daemon in a temp dir (answers a `LockStateResponse`, a refusal `Response`, garbage, a zero or
  oversize length, stalls, or closes).
- `format::*`, `view::*`, `motion::*`, `poll::{clamp_interval_ms, Backoff, Poller, watch}` are pure
  or path-injected; `watch` takes any `Write` and a `keep_running` closure.
- Binaries via `CARGO_BIN_EXE_soos-lockstatus` / `CARGO_BIN_EXE_soos-lock-widget` with `--socket`
  and, for the widget, `WAYLAND_DISPLAY` / `XDG_RUNTIME_DIR` pointing into a temp dir with no
  compositor.
- `scene::Scene` takes explicit `now_ms`; `layout`, `raster`, `font`, `render` are pure over a
  `Vec<u8>` canvas.
- Static invariants (`tests/invariants/src/lockscreen_adapters_contract.rs`): palette and motion
  mirrors, dependency allow-lists, forbid-unsafe, no seat binding, no stderr in the `--once` path,
  docs and snippet presence, `pam_soos.so` untouched by the diff.

## 8. Documentation Drift / ADR

New ADR **[2026-10-15] Lock-Screen Adapters for swaylock-plugin and hyprlock** (GitHub #401, PR 2 of
#352), recording: (1) the merged PR 2 + PR 3 scope; (2) two user-level crates, no privileged
component; (3) one short connection per poll (the §14 contract allows a persistent one; a short one
never holds a connection slot between polls, needs no lifetime/request-cap bookkeeping, and an old
daemon's EOF is just one failed poll); (4) presentation: transport errors render exactly like Idle,
JSON keeps `available` for diagnostics only; Idle shows no words; (5) rendering dependencies:
`smithay-client-toolkit` 0.20 (MIT) with `calloop` only, already locked, no new crate, own 5 × 7
bitmap font and analytic star (no font, image or 2D-graphics crate; the epic's `tiny-skia` proposal
is not adopted); (6) motion and reduced motion (flag, then `SOOS_REDUCE_MOTION`, no portal read);
(7) bounds (§2); (8) no seat, no input, no unlock path; (9) owner-to-confirm list below.

Drift recorded: the epic draft listed `--once` "prints the Unavailable text"; owner decision Q2 (later)
allows only three visible states, so Unavailable renders as Idle (assumption below).

**Owner to confirm** (recorded in the ADR and the PR body):

1. A transport error (daemon down, old daemon, refusal) renders exactly like Idle on the locked
   screen (Q2 allows three visible states); only `--format json` says `"available":false`.
2. Idle shows no words (hyprlock text is empty; the widget shows the ring and the star only).
3. Visual design: brand-blue background, pink star (pending quarter turn while scanning), pale ring,
   amber (`WARN_BORDER`) ring and one 260 ms shake for Not recognized, placed at 78 % of the screen
   height below swaylock's indicator.
4. Reduced motion comes from `--reduce-motion` or `SOOS_REDUCE_MOTION` only (no portal / GNOME
   setting read on the lock screen; that needs zbus, excluded by the lock-state ADR).
5. PR 2 and PR 3 of the epic split are merged into this PR; the installer opt-in stays PR 6.
6. hyprlock glyphs (`✦ ◜◝◞◟ ◌ ✕`) rely on the font fallback of the hyprlock host.

## 9. Revision 1 (plan evaluation of 2026-10-15, findings 1–14)

This section **supersedes** the conflicting statements above.

- **R1 (F1, MAJOR) — one exchange in flight per user, shared cache.** The claim "the auth-reserved
  slot is never needed" is withdrawn: a UID at its regular cap (2) falls back to the auth-reserved
  permit (`ConnectionDispatcher::handle_connection`), so concurrent `--once` processes (one hyprlock
  label per monitor) or several widget instances could hold it while `pam_soos.so` connects. New
  module `crates/lockstatus/src/shared.rs`:
  - `pub const LOCK_FILE_NAME: &str = "soos-lockstatus.lock"`, in `$XDG_RUNTIME_DIR`.
  - `pub const CACHE_FRESH_MS: u64 = 250` (= `MIN_POLL_INTERVAL_MS`), `pub const MAX_CACHE_BYTES:
    usize = 64`.
  - `pub fn shared_fetch(runtime_dir: Option<&Path>, socket: &Path, uid: u32, budget: Duration,
    now_ms: &dyn Fn() -> Option<u64>) -> LockView`:
    1. `runtime_dir` must be absolute, a real directory (`symlink_metadata`, no symlink), owned by
       `uid`, without group/other write bits; otherwise `UNAVAILABLE` **without contacting the
       daemon** (fail quiet; an uncoordinated client could take PAM's slot).
    2. Open `runtime_dir/LOCK_FILE_NAME` with `O_RDWR | O_CREAT | O_NOFOLLOW | O_CLOEXEC`, mode
       0600; `fstat` must show a regular file owned by `uid` with one link; else `UNAVAILABLE`.
    3. `flock(LOCK_EX | LOCK_NB)` retried every 5 ms until `budget`; never acquired → `UNAVAILABLE`.
    4. Under the lock: read at most `MAX_CACHE_BYTES`; a record
       `v1 <idle|scanning|not_recognized|unavailable> <sequence> <monotonic_ms>\n` whose stamp is
       not in the future and is at most `CACHE_FRESH_MS` old is returned without contacting the
       daemon. Otherwise `fetch_view` with the budget left, the record rewritten
       (`set_len(0)` + positioned write), and the lock released by closing.
    5. Any parse or I/O error of the cache is a miss (never a decision); the cache holds only the
       coarse view of the caller's own UID (mode 0600 in the user's runtime directory).
  - Effect: across every `soos-lockstatus --once`, `--watch` and widget process of one user, at most
    one daemon connection exists at a time and at most about 4 daemon requests per second are made
    (each query refreshes a cache that serves everyone for 250 ms), so the user's regular slots are
    never both taken by these clients and the 8/s quota is never reached by them.
    `Poller::poll` and `--once` use `shared_fetch`; `fetch_view` stays the inner exchange.
  - Test: N concurrent `shared_fetch` callers against a mock daemon that records concurrent
    connections: the maximum concurrency is 1, and N callers within 250 ms cause one request.
  - The hyprlock snippet uses one label at `update:500` (any monitor count is safe with the cache);
    an invariant pins `--once` and `update:` ≥ 500 in `packaging/lockscreen/hyprlock-soos.conf`.
- **R2 (F2, MAJOR) — star symmetry.** `STAR_SPIKES` has two spikes (top-right, bottom-left): it
  repeats every 180°, not 90°. `FrameSpec::star_angle_deg` is in `[0, 180)`: while Scanning the
  angle is `90 · (q mod 2) + 90 · ease(frac)` (`q` = completed quarters since the anchor); leaving
  Scanning, the turn continues to the next multiple of 180° (two quarters) and rests at 0°. Tests:
  `star` at 90° renders different pixels from 0°, and 180° the same pixels as 0°.
- **R3 (F3, MAJOR) — acceptance coverage.** New invariants: no `unwrap(`/`expect(`/`panic!`/
  `todo!`/`unimplemented!`/`unreachable!`/`print!`/`println!`/`eprint`/`dbg!` in the production
  code of either crate (the `print` family: `soos-lockstatus` writes stdout only through
  `std::io::Write`); the JSON key set is exactly `state`, `available`, `sequence`. Matrix rows
  (component `lockscreen-adapters`):
  | Row | Acceptance line |
  |---|---|
  | LKA1 | bounded client: request fields, one request per connection, strict decode order, refusal (any verdict, `Allow` included) → Unavailable, lengths 0 / 4097 refused before allocation, stall / missing socket / EOF → Unavailable within the budget |
  | LKA2 | shared coordination: one connection at a time per user, shared 250 ms cache, unsafe runtime dir → no daemon contact |
  | LKA3 | presentation: three visible states only; Unavailable byte-identical to Idle in text / glyph / label and in the widget frame; no success rendering |
  | LKA4 | formats: exact text, glyph, label; glyph quarter step per `T_PENDING`, static under reduced motion; Pango-safe |
  | LKA5 | `--watch` JSON: exact key set, bounded line (≤ 96 bytes, `u64::MAX` included), emits on change only, ≤ 4 Hz, back-off |
  | LKA6 | `--once` binary: exit 0, empty stderr, finishes within `ONCE_EXIT_BUDGET_MS` (+ CI slack) with daemon down / stalled; closed stdout tolerated |
  | LKA7 | reduced motion resolution (flag, env `1`/`0`, junk ignored, >8 bytes ignored) |
  | LKA8 | scene: label first, colour cross-fade `T_SHORT`, pending star 180° domain and finish rule, bounded pending loop, single shake per sequence, idle settles (no frame), reduced motion settles at once |
  | LKA9 | layout and memory bounds: `buffer_len` limits, widget box encloses the shake, tiny surfaces safe |
  | LKA10 | raster: ring / star / text geometry, clipping, non-finite inputs, `paint_widget` writes nothing outside the widget box |
  | LKA11 | brand and motion mirrors equal `theme.rs` and `Docs/UI_MOTION.md` |
  | LKA12 | static: forbid unsafe (lib and main), no panic / print path, dependency allow-lists (no tokio, zbus, network, font, image crate), no seat / keyboard / pointer binding, PAM untouched, docs and snippet present |
  | LKA13 | widget binary: no compositor → exit 1 after `SETUP_FAILURE_EXIT_DELAY_MS`, one stderr line, no panic |
  | LKA14 | owner: driftwm + swaylock-plugin `--command soos-lock-widget` renders on every output, Scanning during PAM and presence scans, Not recognized after a failure, idle CPU ~0 when stable, reduced motion honoured; `--command-each` also renders |
  | LKA15 | owner: killing `soos-lock-widget` never affects the locker (swaylock-plugin restarts it), password and presence unlock unchanged with the widget and the hyprlock snippet installed |
  | LKA16 | owner: hyprlock snippet shows Scanning / Not recognized / nothing on hardware |
  | LKA17 | CI: Dockerized PAM matrix unchanged and green |
- **R4 (F4) — `--once` formats.** `--once` accepts `text`, `glyph`, `label` only; JSON exists only
  in `--watch` (for desktop adapters, never displayed verbatim). `OutputFormat` has no `Json`
  variant; `format::json` stays the `--watch` line.
- **R5 (F5)** — the longest JSON line (`not_recognized`, `available:true`, `u64::MAX`) is 76 bytes
  with `\n`; `MAX_JSON_LINE_BYTES` = 96 holds; tested with `u64::MAX`.
- **R6 (F6) — `--once` silence.** Output through `std::io::Write::write_all` + `flush`, errors
  ignored; no print macro. Arguments are parsed with `try_parse_from`; on a parse error the clap
  message goes to stderr **only if** `--once` is absent from the raw arguments; with `--once`
  nothing is written and the exit code is 0 (help/version still print to stdout).
- **R7 (F7)** — both `main.rs` files start with `#![forbid(unsafe_code)]` (invariant).
- **R8 (F8)** — also edited: `Docs/README.md` index, `deny.toml` getrandom comment (+
  `soos-lockstatus`), manifests with `version`, `edition`, `rust-version`, `license`, `repository`,
  `publish` from the workspace and `[lints] workspace = true`; the manifests never contain the
  strings `zbus` or `soos-remote`. Walkthrough number 214 stays (reserved by the orchestrator; 213
  is reserved for a parallel branch).
- **R9 (F9) — bounded pending loop.** `PENDING_LOOP_MAX_MS = 4800` (4 quarter turns, a multiple
  of 180°, below the 5 s of WCAG 2.2.2 / `Docs/UI_MOTION.md` §5): a scan turns the star for at most
  4.8 s from its entry, then the star rests while the ring and words still say Scanning. Owner to
  confirm: a new scan (every ~4.24 s while presence fails) restarts the loop.
- **R10 (F10) — widget robustness.** The poller thread ending (panic or exit) → the scene shows Idle
  and the widget exits 1 after `SETUP_FAILURE_EXIT_DELAY_MS` (swaylock-plugin restarts it). A caught
  panic in the widget → exit 1 after the same delay. One `SlotPool` per surface, sized for
  `BUFFERS_PER_SURFACE` buffers. Buffer scale 1 (buffers at the logical size; a HiDPI compositor
  upscales; documented). Outputs beyond `MAX_OUTPUTS` or over the pixel budgets get no surface
  (documented limit: swaylock-plugin shows its own colour there). The shake plays once per
  `NotRecognized` sequence (`Scene` remembers the last shaken sequence), so a transient poll error
  never replays it.
- **R11 (F11)** — token names keep the `UI_MOTION.md` names with an `_MS` suffix (integer
  milliseconds suit the scene's integer clock); the invariant compares each value with the normative
  table of `Docs/UI_MOTION.md` §2/§9; `theme.rs` is not compared until `feat/gui-motion` adds them
  (follow-up noted in the walkthrough).
- **R12 (F12)** — test power items adopted (LKA1, LKA3, LKA6).
- **R13 (F13)** — `wayland-sys` is built with `dlopen` (`dlib`): a missing `libwayland-client.so`
  is a connection error → exit 1 after the delay; documented for PR 6 packaging.
- **R14 (F14)** — `/etc/soos/lockwidget.disable` is deferred to the GNOME PR (where an extension can
  break the unlock dialog); here removing the `--command` / label line is the switch. Recorded in
  the ADR.

## 10. Revision 2 (plan evaluation round 2 of 2026-10-15, findings R2-1 to R2-8)

This section **supersedes** the conflicting statements of §2 and §9. (The untracked
`crates/lockstatus` / `crates/lock-widget` scaffolding seen by the evaluator is the tester's
signature stubs for Phase 2; no behaviour is implemented before this gate.)

- **R2-1 (MAJOR) — continuous star phase.** `Scene` keeps `turn_anchor_ms: Option<u64>` and
  `turn_stop_ms: Option<u64>`; with `H = 2 · T_PENDING_MS` (a half turn, the star's symmetry) and
  `up(x) = anchor + ceil((x − anchor) / H) · H`:
  - entering Scanning at `t`: when a turn is running (`anchor` set and `t < stop`), the anchor is
    **kept** (the angle stays continuous), else `anchor = t`; then `stop = up(t + PENDING_LOOP_MAX_MS)`
    (never earlier than the current stop);
  - leaving Scanning at `t`: `stop = min(stop, up(t))` (the turn ends at the next half-turn
    boundary, at most two quarters later);
  - `frame(now)`: while `anchor ≤ now < stop`, `q = (now − anchor) / T_PENDING_MS`, `frac` the
    remainder fraction, `angle = (90 · (q mod 2) + 90 · ease_standard(frac)) mod 180`; else 0. At
    `stop` the angle is 0 ≡ 180, so the end is continuous too.
  - Test: replay the presence cadence (scan entries every 4240 ms, `NotRecognized` after 3240 ms)
    for 30 s at 33 ms frames and assert no frame-to-frame jump above the largest single-step easing
    increment (circular distance on `[0, 180)`), and that the star rests at 0° once the scans stop.
- **R2-2 — cache keyed by socket.** A record carries `fnv1a64(socket path bytes)` as 16 hex digits;
  another socket's record is a miss (and is overwritten). `MAX_CACHE_BYTES` = 128.
- **R2-3 — private cache, no check-then-open hole that matters.** The runtime directory must be
  absolute, not a symlink (`symlink_metadata`), a directory owned by `uid` with `mode & 0o077 == 0`.
  The lock file is opened with `O_RDWR | O_CREAT | O_NOFOLLOW | O_CLOEXEC`, mode 0600, and then
  `fstat` of the **opened descriptor** must show a regular file owned by `uid`, one link and
  `mode & 0o077 == 0`; any failure → unavailable, no daemon contact. `openat` is not used: nix 0.29
  returns raw descriptors that cannot be owned without `unsafe` (the crate forbids it), and the
  remaining window (between the directory check and the open) can only be used by a process that can
  rename entries of the user's `0700` runtime directory, i.e. the same user (trust boundary below);
  the post-open `fstat` is the authoritative check. Tests: a 0644 lock file, a 0755 runtime
  directory, a symlinked runtime directory, a hard-linked lock file, a symlinked lock file → each
  unavailable without contacting the daemon.
- **R2-4 — cache clock.** Records are stamped with `CLOCK_BOOTTIME` milliseconds (it keeps counting
  across suspend, like the presence worker's lock timestamps), taken when the exchange completed.
  `shared::boottime_now_ms()` is the production clock; an injected clock returning `None` disables
  the cache read and write for that call (the coordinated exchange still runs under the lock). A
  stamp in the future is a miss. Tested.
- **R2-5 — accepted residual (ADR + docs).** When two other connections of the same user (for
  example a `soos-gui` camera preview and a `soos-remote` camera view) already hold both regular
  slots, the single coordinated lock-status connection still falls back to the auth-reserved permit
  for its short exchange (≤ 250 ms, at most every 250 ms); the daemon then refuses `LockState` there
  (`warn!`) and the client backs off. Pre-existing per-UID design (#157); recorded, not changed here.
- **R2-6 — back-off shared through the cache.** A record also carries the consecutive daemon failure
  count `failures` (0 after a success). A record with `failures = n ≥ 1` (state `unavailable`) stays
  fresh for `ERROR_BACKOFF_MS[min(n, 3) − 1]` instead of `CACHE_FRESH_MS`, so every client of the user
  together contacts a failing or old daemon at most once per back-off step. `shared_fetch` returns
  `SharedOutcome { view: LockView, source: Source }` with `Source::{Daemon, Cache, LockBusy,
  NoRuntimeDir}`; `Poller` feeds its own `Backoff` with failures of `Source::Daemon` only (a busy lock
  or a cached answer is never a daemon failure). Record:
  `v1 <idle|scanning|not_recognized|unavailable> <sequence> <failures> <stamp_ms> <socket_hash>\n`
  (at most 95 bytes). When the runtime directory is absent or unsafe, `soos-lock-widget` writes one
  stderr line at start (never the state); `--once` stays silent.
- **R2-7 — motion wording.** Owner-to-confirm item 7 (ADR and PR body): while presence keeps failing
  (a new scan about every 4.24 s), each new scan extends the turn by up to 4.8 s, so the star turns
  almost continuously as long as scans repeat; the 4.8 s cap bounds one scan, not the whole
  sequence. Reduced motion stops it entirely.
- **R2-8 — swaylock-plugin uncovered outputs.** That swaylock-plugin shows its own colour on an
  output without a background surface (beyond `MAX_OUTPUTS` or the pixel budgets) is an
  **assumption**, added to owner row LKA14.

Trust boundary (ADR): the cache and lock file are trusted exactly as much as the user's own
processes; a same-user process can make the widget show a wrong coarse state or nothing, which it
could already do by drawing over its own session, and can never produce a success state (none
exists) or reach any other user.

## 11. Revision 3 (plan evaluation round 3: APPROVED, minor findings adopted)

- **R3-1** — `Record::failures` is a `u8` saturating at 3 (the last back-off step); the longest
  record (`not_recognized`, `u64::MAX` sequence and stamp, failures 0, hash) is 79 bytes (newline included; the longest `unavailable` record is 57) ≤
  `MAX_CACHE_BYTES` (128); tested with every field at its maximum.
- **R3-2** — `MIN_EXCHANGE_BUDGET_MS` = 50 (`shared.rs`): when less than that is left of the budget
  after the lock was taken and no reusable record exists, the call returns `Source::LockBusy`
  (unavailable view) without contacting the daemon, incrementing `failures` or writing a record.
  Tested with the lock held by another descriptor for 240 ms of the 250 ms budget.
- **R3-3** — not adopted: `rustix` would become a direct dependency for a race that only the same
  user can exploit (the evaluator accepted the residual); the dependency allow-list stays minimal.

## 12. Revision 4 (candid review of 2026-10-16: APPROVED with minor findings, adopted)

- **C1** — `poll::output_closed(fd)` (`poll(2)` with a zero timeout, `POLLERR | POLLHUP | POLLNVAL`
  or a failed `poll` = closed): `--watch` checks stdout before every poll, so an orphaned watcher
  stops even while the state is stable (`nix` gains its `poll` feature; no new crate). Test
  `test_lka6_watch_exits_when_stdout_closes_while_the_state_is_stable`.
- **C2** — supersedes R13: built alone the widget uses the pure-Rust `wayland-backend` (no Wayland C
  library); only a whole-workspace build, unified with `soos-gui`, loads `libwayland-client.so`.
  Documentation corrected; the shipped variant is decided by the installer PR.
- **C3** — `configure_surface` returns whether the surface can be drawn; the first skipped output
  (budget or shared-memory failure) writes one stderr line (size only, never the state).
- **C4, C5** — follow-ups, not changed here: the daemon could refuse to admit `LockState` onto the
  auth-reserved permit at accept time (removes the R2-5 residual); the worst-case shared memory is
  512 MiB (two 8K outputs) and could shrink with one full buffer plus small widget-box buffers.

## 13. Revision 5 (candid review round 2 of 2026-10-16: APPROVED with minor findings, adopted)

- **D1** — a scan joining a running turn rounded its stop **up** and could turn the star up to
  7.2 s after its own start. The extension is now rounded **down** to the half-turn grid
  (`down_to_half_turn(anchor, start + PENDING_LOOP_MAX_MS)`), so no scan turns the star later than
  its start plus 4.8 s; a fresh scan still turns exactly 4.8 s. Test
  `test_lka8_scan_joining_a_running_turn_stops_within_the_loop_limit` (red against the round-up).
- **D2** — the longest cache record is 79 bytes (`not_recognized`, largest sequence and stamp), not
  76; `MAX_CACHE_BYTES` (128) unchanged.
- **D3** — every widget exit except help and version waits `SETUP_FAILURE_EXIT_DELAY_MS`: a usage
  error (exit 2) and a closed surface set or connection (`Exit::Closed`, `Exit::Disconnected`,
  exit 0) too, so swaylock-plugin can never restart the widget in a tight loop. Test
  `test_lka13_usage_error_waits_the_restart_delay`.
- **D4** — documentation: the widget's start-up stderr line covers only a missing or non-private
  `XDG_RUNTIME_DIR`; the ADR cites §9–§13.

## 14. Revision 6 (candid review round 4 of 2026-10-16: APPROVED with minor findings, adopted)

- **E1** — `soos-lockstatus` comments: a usage error with `--once` prints nothing (exit 0); daemon
  and transport failures print the Idle rendering.
- **E2** — the skipped-output stderr line names the zero-size case; outputs beyond `MAX_OUTPUTS`
  are reported once too (never the state).
- **E3** — `Scene` keeps the last answered sequence (`epoch_sequence`); an available view with a
  lower sequence (daemon restart, new epoch) clears the last shaken sequence, so a new failure
  numbered like an old one still shakes. Test `test_lka8_daemon_restart_epoch_shakes_a_reused_sequence`.
- Not adopted (follow-up): keeping the last view in the widget poller while the coordination lock
  is busy.

## 15. Revision 7 (candid review round 5 of 2026-10-16: APPROVED with minor findings, adopted)

- **F1** — the widget poller thread replaces any older unsent view with the newest poll
  (a view is sent only when it differs from the last delivered one; a full channel is retried with the next, newest poll), so a full channel never delivers a stale view.
- **F2** — separate once-per-run flags for "more than `MAX_OUTPUTS` outputs" and "an output cannot
  be drawn", so each kind of skip is reported once.
- **F3, F4** — wording: `SOOS_REDUCE_MOTION` is defined by `Docs/UI_MOTION.md` §8 (the GUI reads it
  from `feat/gui-motion` on); matrix LKA13 qualifies "one stderr line".

## 16. Revision 8 (candid review round 6 of 2026-10-16: APPROVED with one minor finding, adopted)

- **G1** — `Scene` remembers `(sequence, first-seen instant)` of the last Scanning view
  (`scan_start`, cleared on a new epoch); the loop limit is `down_to_half_turn(anchor, scan_start +
  PENDING_LOOP_MAX_MS)`, so a scan seen again after a transient poll error (Scanning s → Idle →
  Scanning s) never turns the star later than 4.8 s after it was first seen. Test
  `test_lka8_scan_seen_again_after_a_transient_error_keeps_its_start`.
- **G2** — `Poller::poll` keeps its last view when the outcome is `Source::LockBusy` (no answer at
  all; `UNAVAILABLE` before the first answer); `--once` is stateless and still reads Idle. Test
  `test_lka5_poller_keeps_the_last_view_while_the_lock_is_busy`.

## 17. Revision 9 (candid review round 7 of 2026-10-16: APPROVED with one minor finding, adopted)

- **H1** — sequence 0 is also the daemon's neutral answer (clock error, no local seat session,
  session-check timeout), so only a lower **non-zero** sequence starts a new epoch, and sequence 0
  never overwrites `epoch_sequence`; the restarted daemon is recognized at its first scan
  (sequence 1). Residual (cosmetic, less motion only): if the last sequence of the old epoch was 1,
  a restart is not noticed until a higher sequence. Test `test_lka8_neutral_idle_answer_is_not_a_new_epoch`.
- Matrix LKA2 wording: a record above 128 bytes is a miss (at most 129 bytes read).
