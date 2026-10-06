# `soos-gui` — Responsiveness, Privileged Operations and Diagnostics

`soos-gui` (crate `crates/gui`) is the eframe/egui desktop application for live model
diagnostics, guided enrollment and profile management. This page documents its threading model,
how it reports failures and its visual design system (§5). Camera device selection and the daemon preview proxy are covered by
`Docs/IPC_PROTOCOL.md` and `Docs/CAMERA_V4L_CRATE.md`.

## 1. Threading Model (GitHub #154, review finding CAM-06)

The UI thread (`SoosApp::ui`) runs once per repaint, and the vision worker requests a repaint for
every camera frame (about 30 per second). The UI thread therefore never spawns a process and never
waits on I/O it does not own.

| Work | Thread | Module |
|---|---|---|
| Camera capture / daemon preview polling | `soos-v4l-capture` or `soos-gui-ipc-cam` | `soos-camera-v4l`, `ipc_camera` |
| Face detection, PAD, embedding | `soos-gui-worker` | `worker` |
| `systemctl show --property=ActiveState --value soos-daemon.service` | `soos-gui-daemon-monitor` | `daemon_control` |
| `pkexec` (pause/resume daemon, list/import/delete templates) | `soos-gui-privileged` | `privileged` |
| Camera-source probing, switching and device release | `soos-gui-camera-source` | `camera_source` |

- **Daemon state polling**: `DaemonMonitor` probes through the `DaemonStatusProbe` trait
  (production: `SystemctlProbe`) at most once per `DAEMON_POLL_INTERVAL` (2 s), gated by the
  clock-injectable `PollThrottle`, and publishes `DaemonState::{Unknown, Active, Inactive}` in an
  atomic. `SystemctlProbe` reads the unit's `ActiveState` (`/usr/bin/systemctl show
  --property=ActiveState --value`) and maps it with `active_state_means_running`: only
  `inactive` and `failed` are `Inactive`; `active`, `reloading`, `refreshing`, `activating`
  (start-up, `auto-restart`) and `deactivating`, an unknown value and a `systemctl` that cannot
  be run all count as `Active`, so the GUI never opens the device directly while the daemon is
  restarting (EBUSY fight; GitHub #314, CAM-NEW-7). The header reads it lock-free. Pause and
  Resume are driven by the daemon toggle switch of the header (§5.3), which submits the same
  `PrivilegedAction::PauseDaemon` / `ResumeDaemon`; after either the UI calls
  `request_refresh()` so the new state appears within one monitor tick (50 ms) instead of the
  next interval.
- **Privileged operations**: the UI submits a `PrivilegedAction` to `TaskRunner`, which runs it on
  a worker thread through the `PrivilegedExecutor` trait (production: `PkexecExecutor`, fixed
  argument vectors, no shell, stdin closed) and returns immediately. Programs are named by
  absolute path, never resolved through the caller's `PATH` (GitHub #314, CAM-NEW-7):
  `PKEXEC_PROGRAM` (`/usr/bin/pkexec`), `SYSTEMCTL_PROGRAM` (`/usr/bin/systemctl`) and
  `SOOS_ENROLL_PROGRAM` (`/usr/bin/soos-enroll`, where `scripts/install.sh` with its default
  prefix, the Debian, Arch and RPM packages install it). Since GitHub #318 the `soos-enroll`
  directory comes from the build-time `SOOS_BINDIR` (unset or `/usr/bin`: the default above;
  `scripts/install.sh --build --prefix <P>` exports `<P>/bin`): `crates/gui/build.rs` validates it
  (normalized absolute directory of `[A-Za-z0-9._+-]` components, otherwise the build fails) and
  enables `cfg(soos_custom_bindir)`, which selects `concat!(env!("SOOS_BINDIR"), "/soos-enroll")`
  (contract `crates/gui/tests/program_path_tests.rs`). Package builders export `/usr/bin`. The template import passes the same
  absolute `SOOS_ENROLL_PROGRAM` from `privileged::import_helper_args` (pinned by the
  `import_privacy_tests` contract, owner-approved 2026-10-02, walkthrough 168). The `PrivilegedOutcome` comes
  back over an `mpsc` channel that `SoosApp::handle_task_outcomes` drains each frame; the worker
  wakes the UI with `request_repaint`. Only one privileged action runs at a time
  (`TaskRunnerError::Busy`), so the user never faces stacked Polkit dialogs; the daemon switch
  is disabled and replaced by a spinner with "Waiting for authorization..." meanwhile ("Daemon
  status unknown" while the daemon state is not yet known).
- **Direct-mode store mutations (GitHub #291)**: with a local store (`GuiStore::System` or
  `GuiStore::Developer`) the UI never calls `BiometricStore::enroll` / `delete` itself: both
  take the store lock and may wait up to `STORE_LOCK_TIMEOUT` (5 s) while `soos-enroll` holds
  it. The UI submits a `store_tasks::StoreTask` (`Enroll(template)` or `Delete { uid }`) to
  `StoreTaskRunner`, which runs it on the `soos-gui-store` thread and returns immediately; the
  `StoreTaskOutcome` is drained each frame by `SoosApp::handle_store_task_outcomes` and the
  worker wakes the UI with `request_repaint`. One store task runs at a time
  (`StoreTaskSubmitError::Busy`). A lock timeout is shown as `StoreTaskError::Busy`, whose
  message is `STORE_BUSY_MESSAGE` ("another soos operation is using the template store; try
  again"); nothing is written or removed. `StoreTask`'s `Debug` output prints only the UID and
  embedding dimension. A separate runner (rather than new `PrivilegedAction` variants) keeps
  local saves independent of a pending Polkit dialog (Pause/Resume) and leaves the
  `PrivilegedExecutor` contract unchanged.
- **Template import (GitHub #156, review findings CAM-08 / STO-12)**: the fused embedding is
  serialized to JSON in a zeroizing buffer and piped to the standard input of
  `pkexec /usr/bin/soos-enroll import --uid <uid> --file -` (`privileged::import_helper_args`,
  `import_template_with`). No file is created, under the temporary directory or anywhere else;
  the child is always reaped, even if it exits before reading its stdin (denied Polkit prompt).
  The fused embedding is held in `Zeroizing<Vec<f32>>` from the enrollment session to the
  pipe. `PrivilegedAction`'s `Debug` output prints only the UID and embedding dimension, never
  embedding values.
- The `soos-enroll list` output accepted from the helper is bounded by
  `MAX_PROFILE_LIST_BYTES` (1 MiB) while it is read (`read_bounded`, `Read::take`); an oversized
  output kills the helper.

- **Frame pacing (walkthrough 184)**: the daemon preview worker keeps a fixed cadence of
  `PREVIEW_POLL_INTERVAL` (33 ms) from one request to the next (`ipc_camera::frame_cadence_delay`
  subtracts the exchange time; a slow exchange polls again at once), which stays below the
  daemon default of 40 preview requests per second. The vision worker polls again immediately
  after it analyzed a frame and waits `WORKER_IDLE_POLL` (5 ms) only when no new frame was
  available (`worker::worker_idle_delay`). Both used to sleep a fixed time after each frame,
  which added to the 25 ms analysis and capped the live view at about 27 analyzed frames per
  second in release builds (30 now, the camera rate).
- **Debug builds**: `[profile.dev]` optimizes the preview hot path (`soos-vision`,
  `soos-inference-ort`, `jpeg-decoder`, `zeroize`). Unoptimized, `cargo run -p soos-gui`
  analyzed about 6 frames per second; it now reaches about 23. Packages and `scripts/install.sh`
  build the release profile.

## 1a. Runtime Camera Source (GitHub #154 / #150)

The camera source is not chosen once at startup. `camera_source::CameraSourceSupervisor` follows
the `DaemonMonitor` state and installs the source in the `SwitchableCamera` read by the vision
worker:

| Daemon state | Source |
|---|---|
| Unknown (no probe yet) | none: the device is never opened (fail-closed) |
| Active, preview authorized | daemon IPC preview (`IpcCameraManager`) |
| Active, preview refused / socket unreachable | blocked notice (`CameraBlockReason`) |
| Inactive (paused) | direct V4L2 through the shared resolver (`resolve_camera_device_from_config`) |

The previous source is always stopped and dropped (device closed) before the next one is opened.
**Resume** first releases a direct V4L2 manager on the privileged worker thread
(`HandoverExecutor`) and keeps direct mode disabled until the daemon is seen active, so the daemon
never meets `EBUSY` because of the GUI; a failed Resume re-enables direct mode. Transient blocks
(rate limit, I/O, daemon starting) are re-probed every second, at most
`MAX_TRANSIENT_PROBE_RETRIES` times; permanent ones (not authorized, not in the `soos` group) only
when the daemon state changes. The decision logic is the pure `CameraSourcePlanner`.

## 1b. Biometric Store Selection (GitHub #156)

`store_mode::resolve_gui_store` picks the store once at startup; there is no implicit fallback
location (the former silent `soos-gui-master.key` / `soos-gui-biometrics` store in the system
temporary directory is gone):

| Mode | When | Templates |
|---|---|---|
| `GuiStore::System` | `--key-file` and `--biometrics-dir` are accessible (root session) | written directly to the system store |
| `GuiStore::Polkit` | the system store is not accessible (unprivileged session) | no local store at all; list / import / delete go through `pkexec /usr/bin/soos-enroll` |
| `GuiStore::Developer` | explicit `--dev-store <DIR>` (absolute path) | `<DIR>/master.key` and `<DIR>/biometrics` (created `0700`); never used by PAM |

The developer mode shows a persistent amber warning banner at the top of the page body, right
under the header band, on every tab ("DEVELOPER STORE:
templates are saved in `<DIR>` and are NOT used by PAM") and logs it at startup; it never
imports into the system store. In Polkit mode the Profiles tab lists the system templates but
the in-process match test needs a readable template (root session or `--dev-store`).

```bash
soos-gui                              # system store (root) or Polkit mode (user)
soos-gui --mock --dev-store "$HOME/.local/share/soos-dev"   # hardware-free development
```

`--mock` requires `--dev-store` (clap `requires`, GitHub #314 CAM-NEW-5): templates computed by
the mock models carry the production model id, so they must never be imported into the system
store; `soos-gui --mock` alone is a usage error.

Verification: `crates/gui/tests/import_privacy_tests.rs` (matrix rows ISE1–ISE4).

## 1c. Guided Enrollment Liveness (GitHub #217 / #218)

The worker feeds every analyzed frame to the guided enrollment session through
`worker::feed_guided_enrollment`, and the session is created by
`worker::new_guided_enrollment_session` with `LivenessPolicy::strict()`:

- a sample needs 3 consecutive live frames (PAD live and score >= `pad_threshold`, NaN rejects);
- a spoof or below-threshold frame discards the current step; the third one aborts the session
  ("Enrollment aborted: repeated spoof detections. Cancel and restart.");
- a frame rejected by the pre-PAD quality gate (`VisionAnalysis::quality_rejection`) shows
  "Face too small or blurred: move closer and hold still." and is never sampled;
- a frame with a face but no PAD verdict, or with no face, breaks the live streak without counting
  as a spoof;
- a spoof PAD verdict is counted even when the frame yields no pose or no embedding
  (`GuidedEnrollmentSession::record_presentation_attack`, GitHub #285);
- only a frame with exactly one face is ever sampled (GitHub #304): `VisionPipeline::analyze_frame`
  reports every detection (`VisionAnalysis::face_count`) but runs PAD, alignment and embedding
  only for a single face, `feed_guided_enrollment` breaks the live streak and returns `None` for
  any other count, and `worker::guided_enrollment_feedback` shows "One face only: make sure
  nobody else is in view of the camera." (`GuiEnrollmentFeedback::OneFaceOnly`) for a frame
  with several faces. This is the rule of the CLI enrollment (`process_frame` rejects more than
  one detection).

## 1d. Live Verification Reference (GitHub #278 / #298)

Selecting a profile in the live-verification panel calls `worker::select_match_reference` with
the stored template (or `None` when the store lookup fails or finds nothing):

- only a template of the loaded embedding model (`template_matches_model`: same id, same
  dimension) becomes the match reference;
- a foreign template (for example a pre-SFace `arcface_w600k_mbf` 512-D one) clears the reference
  and shows "Re-enrollment required: this template was enrolled with model '<id>' (<n>-D)";
- a failed or empty lookup clears the reference and the note;
- in every case the score computed against the previous reference is withdrawn in the same
  critical section that installs the new reference. The worker publishes scores through
  `worker::update_live_match_score`, which holds the reference lock while it writes the score,
  so no stale score is ever displayed for the new selection.
- the reference is a `Zeroizing<Vec<f32>>` cloned straight from the template's zeroized vector,
  so the template copy is wiped when it is replaced, cleared or dropped (row SGF5).

## 2. Camera Error States (GitHub #155, review finding CAM-07)

Every `CameraManager` exposes `status() -> CameraStatus` (see `Docs/CAMERA_V4L_CRATE.md`). When no
analyzed frame is available, the central panel renders `camera_status::camera_status_banner`
instead of a generic spinner (a centered brand status card, `render_status_banner`), and the
header band shows the banner title (with a frame: FPS, latency and resolution). When the band is
too narrow for that text next to the tabs, it is hidden and shown instead as the tooltip of the
wordmark; the status card always carries the title. Each `CameraErrorKind` has a
distinct title and an actionable hint, plus the consecutive failure count and whether the source
retries automatically (`error_is_retried`: every kind except `SourceUnauthorized`, where the IPC
preview worker stops):

| Kind | Title | Typical cause |
|---|---|---|
| `DeviceNotFound` | Camera not found | Wrong `camera_device`, unplugged camera |
| `DeviceBusy` | Camera is busy | `soos-daemon` owns the device (`EBUSY`) |
| `PermissionDenied` | Camera permission denied | User not in `video` group (`EACCES`) |
| `UnsupportedDevice` | Camera format unsupported | No decodable capture format |
| `Starved` | Camera stopped sending frames | Privacy shutter, cable |
| `Io` | Camera I/O error | Other V4L2 failure |
| `SourceUnreachable` | Daemon preview unreachable | Daemon paused or stopped |
| `SourceUnauthorized` | Daemon preview not authorized | UID not in `[preview] allowed_uids` |
| `SourceRateLimited` | Daemon preview rate-limited | `[preview] max_requests_per_sec` |
| `SourceUnavailable` | Daemon camera unavailable | Daemon has no camera frame to serve (`Verdict::Unavailable`, or an empty preview after the first frame) |
| `SourceProtocol` | Daemon preview protocol error | GUI/daemon version mismatch, stale or unstamped daemon `Response` |

`IpcCameraManager` maps each `IpcPreviewError` to the matching `Source*` kind
(`IpcPreviewError::kind`). Every preview reply goes through `ipc_camera::frame_from_preview`
(GitHub #305, #306, #314 S2):

| Reply | Before the first frame | After a frame was shown |
|---|---|---|
| Empty preview (no data, `format = 255`) | `Starting` ("Connecting to camera"), keep polling | frame and overlays withdrawn, not ready, `SourceUnavailable` until frames resume |
| Known format, payload length equal to the geometry (any non-empty length for MJPEG) | frame shown, `Ready` | frame shown, `Ready`, error cleared |
| Unknown format code, `255` with data, zero dimension with data, length mismatch | `SourceProtocol`, reconnect | `SourceProtocol`, reconnect |
| Greyscale (`format = 1`, how the daemon sends every infrared frame) | Monochrome PAD path (IR gate, IR threshold) | same |

Unknown format codes used to be decoded as RGB24 and an empty preview used to leave the last
frame frozen on screen as `Ready`. The reply buffer and the published frame copies
(`LatestFrameData::rgb`, `aligned_crop`) are `Zeroizing`, and `LatestFrameData`'s `Debug`
prints only lengths (GitHub #314, CAM-NEW-6). A daemon refusal (`Response` echoing the request nonce) is trusted
only after `Response::check_freshness` with the shared `MAX_RESPONSE_FUTURE_SKEW_NS` against
CLOCK_MONOTONIC, exactly like `pam_soos.so` and `soos-admin test-pam`: a stale, unstamped or
future-dated `Response` is `IpcPreviewError::Protocol`, and no `Response` is ever an authorized
preview (GitHub #289, `Docs/IPC_PROTOCOL.md` §9). In direct mode the device comes from the shared
`daemon.toml` reader (`Docs/CAMERA_V4L_CRATE.md`); its notes (unusable file, ignored key, never a
value) are logged as warnings. Non-error states render as "Connecting to camera" (`Starting`),
"Camera ready" (`Ready`), "Camera in standby" (`Suspended`) and "Camera stopped" (`Stopped`).

## 3. Logging

`main.rs` calls `soos_gui::logging::init()` before anything else. It installs a
`tracing_subscriber::fmt` subscriber writing to **stderr**, filtered by `RUST_LOG`
(`build_env_filter`; unset, empty or invalid directives fall back to `info`). Initialization is
idempotent and never panics. Logged events carry error kinds, failure counts, device paths and
service state only: camera status transitions (UI thread, logged once per transition), preview
failures and recoveries (`ipc_camera`, once per transition), daemon state changes and privileged
operation failures. Frames, embeddings and credentials are never logged.

Example: `RUST_LOG=soos_gui=debug,soos_camera_v4l=debug soos-gui`.

## 4. Verification

Matrix rows GRE1–GRE6 and ISE1–ISE4 in `AI/VERIFICATION_MATRIX.md`; tests in
`crates/gui/tests/responsiveness_tests.rs`, `crates/gui/tests/camera_status_tests.rs`,
`crates/gui/tests/import_privacy_tests.rs` and `crates/camera-v4l/tests/camera_status_tests.rs`.
Direct-mode store mutations off the UI thread: rows SGU1–SGU2, `crates/gui/tests/store_task_tests.rs`.
Live verification reference selection: rows SGF1–SGF2, `crates/gui/tests/match_reference_selection_tests.rs`;
wipe-on-drop reference: row SGF5, `crates/gui/tests/match_reference_zeroize_tests.rs`.
Failure paths without a daemon or camera (oversized, zero-length and truncated preview replies,
daemon without camera, `EACCES` socket, direct-mode `EACCES` / `EBUSY`) are covered by
`crates/gui/tests/ipc_camera_failure_tests.rs` (matrix CHT5–CHT6, GitHub #198).
Visual design system: rows GUX1–GUX13, `crates/gui/tests/theme_tests.rs` and
`crates/gui/tests/brand_layout_tests.rs`. Frame pacing and stable card ids: rows GFP1–GFP3,
`crates/gui/tests/ipc_preview_cadence_tests.rs`, `crates/gui/tests/worker_pacing_tests.rs` and
`crates/gui/tests/card_id_stability_tests.rs`.

## 5. Visual Design System (brand redesign, walkthrough 183)

The GUI follows the soos design direction (mockup `New_Gui_Interface.png`, 1512x982, Live Model
Diagnostic page). The redesign is presentation only: threading, privileged flows, store tasks,
fail-closed camera logic and every documented message are unchanged. No crate was added; text
uses the egui default fonts (nothing is downloaded or bundled) and the brand marks are drawn
with the egui painter (no image or SVG loader).

### 5.1 Palette

| Token (`theme`) | Hex | Name | Use |
|---|---|---|---|
| `BLUE` | `#0047BB` | Pantone 2728 C | Header band, card borders, primary buttons, stat and star tiles |
| `PALE` | `#EDF1FF` | Brilliant White | Page body, cards, active tab, text on blue |
| `INK` | `#101820` | Pantone Black 6 C | Primary text, video placeholder |
| `PINK` | `#E59BDC` | Pantone 244 C | Brand accent: star mark, overlay chip dots, guidance arrows, rejected-sample reticle |

Every other tone is a tint of these four (`PALE_2..4`, `LINE`, `INK_MUTED`, `INK_WEAK`,
`BLUE_HOVER`, `BLUE_PRESSED`) or a semantic state set (`SUCCESS*`, `DANGER*`, `WARN*`) and
the video overlay colors (`FACE_LIVE`, `FACE_SPOOF`, `EYE`, `NOSE`, `MOUTH`). `theme::apply`
installs the light brand visuals for both system theme preferences (blue selection, rounded
widgets, thin floating scroll bars).

### 5.2 Modules

| Module | Role |
|---|---|
| `theme` | Palette tokens, sizes, `Metrics::for_width` (responsive paddings and the 250–300 pt right column), `split_columns`, `content_rect`, `fit_video`, `apply`, and `paint_faux_bold` (the default font has one weight; passes are snapped to whole physical pixels by `faux_bold_offsets` so headings render with the same weight everywhere) |
| `brand` | SVG path data of the "SOOS" wordmark and the star mark, a small anti-aliased rasterizer (each mark is rasterized once into a texture, re-rasterized only when its size or color changes), `paint_wordmark`, `paint_star` and `window_icon` (the four-quadrant logo used as the window icon) |
| `widgets` | Cards (`card_in_rect`, `card_in_rect_with_footer`), card and section titles, inner tables, stat and star tiles, `column_stack` / `column_stack_card_first`, banners with vector icons, `BrandButton` (primary, secondary, danger, danger outline), `progress_bar`, overlay chips, the enrollment checklist (`step_list`) and the toggle switch |
| `header` | Header band and tabs ("Live Model Diagnostic", "Guided Enrollment", "Biometric Profiles", with painted icons; inactive tabs collapse to their icon on narrow windows) and the daemon panel |

Symbols missing from the default fonts (check, cross, warning, info) are painted as shapes.

### 5.3 Header and Daemon Switch

A 44-point blue band holds the wordmark, the tabs (the active tab is a pale folder shape joined
to the body) and, aligned with the right column, the daemon panel. The panel replaces the old
Pause/Resume buttons with a toggle switch (`header::DaemonSwitch`):

| `DaemonState` | Switch | Click |
|---|---|---|
| `Active` | on (green track) | `PrivilegedAction::PauseDaemon` |
| `Inactive` | off | `PrivilegedAction::ResumeDaemon` |
| `Unknown` | no knob, inert | nothing |
| `Active` / `Inactive`, privileged action pending | spinner, "Waiting for authorization..." | nothing |
| `Unknown`, privileged action pending | spinner, "Daemon status unknown" | nothing |

The action goes through `submit_privileged` exactly as the buttons did, followed by
`request_refresh()`. The result of a daemon action is shown as a success or error banner at
the top of the body, under the developer-store banner when present.

### 5.4 Page Layout

The body below the band is `theme::content_rect` (padding `Metrics::pad`, 40 pt at 1512 pt and
at least 20 pt) and is split by `Metrics::split_columns` into a main area and a right column of
`(259 * k).clamp(250, 300)` points, separated by `Metrics::gap_main`. These formulas replace the
former Live sidebar `(x * 0.32).clamp(280, 380)`, Enrollment sidebar `(x * 0.35).clamp(300, 420)`
and 16-point gap; the CLP4 / GARP3 minimum canvas sizes still hold with the developer banner
(`brand_layout_tests::test_gux12_*`). `crates/gui/tests/layout_tests.rs` is unchanged.

- **Live Model Diagnostic**: the video keeps the frame aspect, centered horizontally at the top
  of the main area with 16-point corners and rule-of-thirds guides. The five overlay toggles are
  pill chips floating over the video (compact size when one row does not fit), so they never
  shrink the canvas. Face and inset captions sit on a translucent ink backing. The right column
  stacks the "Telemetry & Analysis" card (all telemetry rows and the "Live 1-to-1 Match Test"),
  a stat tile (match score while a listed profile is selected, otherwise PAD liveness) and, when
  the height allows, the pink star tile. A selected profile that leaves the list clears the match
  reference.
- **Guided Enrollment**: same video panel with the oval reticle (pale idle, blue capturing, pink
  on a rejected sample, green when complete) and the guidance message in a caption pill. The
  enrollment card holds the target fields, a brand progress bar (no fill at 0 %, label always
  readable), the four-step checklist (compact 30-point rows on short columns, hint in the
  tooltip) and the feedback banner; Start / Cancel / Save stay in a fixed footer.
- **Biometric Profiles**: one main card with the title, Refresh, a store notice (Info for the
  system and Polkit stores, Warning for the developer store) and the table (blue header row,
  40-point rows, red outline Delete). The right column holds the "TEMPLATES" count tile and a
  star tile filling the rest of the height. "Confirm Deletion" is an `egui::Modal`: the backdrop
  blocks every click behind it, and Escape or a backdrop click cancels.

In the right column (`column_stack_card_first`), the card first gets the height its content
needs (measured on the previous frame), the stat tile shrinks toward 140 points, and the tiles
are dropped entirely before the card would have to scroll. Because a second egui pass in the
same frame can read the new measurement and show or hide a tile above the card, each card's
content runs in an explicit id scope (`widgets::card_scope_id`: the parent's stable id and the
card's `id_salt`, never the parent's auto-id counter), so its widget ids never shift between
passes (no "Widget rect changed id between passes" warning, no lost hover or scroll state).
