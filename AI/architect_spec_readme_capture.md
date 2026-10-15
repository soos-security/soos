# Architect Spec — GitHub #381: Reproducible README Screenshot Capture (PR 1 of #356)

- **Date**: 2026-10-13
- **Round**: 2 (resolves every finding of plan evaluation round 1; mapping in §10, which overrides earlier sections where they differ)
- **Branch**: `test/readme-capture` (from `origin/main` 6f32ef8). GitHub-only issue: not registered in
  `scripts/sync_issue.py` `BRANCH_TO_ISSUE`; the commit message carries `Closes #381` and `Refs #356`.
- **Binding inputs**: epic #356 (body, "Planned split" item 1, acceptance criteria that concern capture tooling,
  budget, layout and the asset invariant), sub-issue #381, `AGENTS.md`, `Docs/GUI_APPLICATION.md`,
  `Docs/REMOTE_COMPANION.md` §2e, `AI/MOCK_STRATEGY.md`.
- **Code read**: `crates/gui/src/{app,state,worker,daemon_control,privileged,store_mode,lib,main}.rs`,
  `crates/gui/Cargo.toml`, `crates/gui/tests/{layout_tests.rs,common/}`, `crates/camera-v4l/src/mock.rs`,
  `crates/inference-ort/src/mock.rs`, `tests/fixtures/{Cargo.toml,mod.rs}`, `crates/remote/assets/app.js`,
  `crates/remote/src/{status,alerts,server}.rs` (JSON shapes only), `tests/invariants/{Cargo.toml,src/fixtures_contract.rs}`,
  `deny.toml`, root `Cargo.toml`, eframe 0.35 `epi.rs` (`Frame::_new_kittest`), epaint 0.35 (`Mesh`, `ImageDelta`).
- **Out of scope** (PR 2 / PR 3 of #356): README text, hero, badges, Mermaid diagram, comparison, FAQ, brand SVGs,
  the forbidden-marketing-words check, animations, the visual refresh after the UI redesign.

## 0. Owner Decisions

The epic's open questions have no owner answer yet (`gh issue view 356 --json comments`: none). The run is
authorised; the recommended options are assumed and must be confirmed by the owner:

| ID | Question (#356) | Assumed answer | Status |
|---|---|---|---|
| OD-1 | Q2: ship now or wait for the UI redesign / lock-screen widget | Ship now with the current UI; PR 3 reruns the capture | assumed — owner to confirm |
| OD-2 | Q3: animated WebP/GIF in the repo or uploaded MP4 | Static PNG/WebP only in the repo, within the budget; no animated media in this PR (the invariant refuses animation chunks) | assumed — owner to confirm |
| OD-3 | Web capture toolchain (epic proposes a pinned Playwright container) | No Node toolchain, no container: a std-only Rust fixture server plus an optional script that drives an already-installed Chrome/Chromium headless; never a CI requirement | assumed — owner to confirm |
| OD-4 | Asset budget | 1 MiB per file, 8 MiB for `Docs/assets` in total, at most 32 files; formats PNG, WebP (still), SVG | assumed — owner to confirm (epic default) |

## 1. Design Decisions

- **D-1 (renderer)**: `egui_kittest` is **not** added. Its pixel snapshots need the `wgpu` feature (a GPU or Mesa
  lavapipe, a ~100-crate dependency tree, and a renderer whose output differs across drivers). Instead the
  capture uses the egui harness that already ships with the workspace's egui 0.35: `egui::Context::run_ui` with a
  fixed `RawInput`, `eframe::Frame::_new_kittest()` (the public kittest constructor of eframe 0.35) to call the real
  `eframe::App::ui`, `Context::tessellate`, and a small deterministic CPU rasterizer of the tessellated meshes
  (test code only). Result: no new crate, no GPU, no display server, bit-identical output on every x86_64/aarch64
  runner, so the test runs in normal CI. Recorded in the ADR (§9).
- **D-2 (dev-only)**: the scene builder lives in `soos-gui` behind a non-default feature `readme-capture` (it needs
  private `SoosApp` fields); the rasterizer and the PNG codec live in the test target only. The only new manifest
  entry is `png = "0.18"` in `crates/gui` `[dev-dependencies]` (already in `Cargo.lock` at 0.18.1 through eframe →
  image; same version, MIT OR Apache-2.0). `cargo tree -e normal -p soos-gui` (default features) is unchanged.
- **D-3 (synthetic only)**: the camera image is a new rendered placeholder face from `soos-test-fixtures`
  (`readme::create_placeholder_face_frame`), drawn analytically (no photo, no model output, no real face), aligned
  with the box of `MockFaceDetector::new_centered_face`. Detector, PAD and embedding are the existing mocks. Every
  identity shown is `alice` / uid 1000; no host lookup result is ever rendered.
- **D-4 (determinism)**: no worker thread, no camera thread, no `systemctl`, no `pkexec`, no store I/O in a
  capture scene. Telemetry values (FPS, latency) are fixed constants. The vision pipeline runs once, synchronously,
  on the fixture frame.
- **D-5 (drift)**: the snapshot test compares each rendering with the committed PNG and fails with the
  regeneration command when they differ beyond a tolerance (§4.4). `SOOS_UPDATE_README_ASSETS=1` rewrites them.
- **D-6 (web path)**: the fixture server is an example of `soos-test-fixtures` (std only, loopback only, bounded);
  its routing is a pure function tested in that crate. `soos-remote` gains no flag, route, mode or file.
- **D-7 (manifest)**: `Docs/assets/MANIFEST.toml` lists every asset with its generator and SHA-256. It is written
  by `scripts/capture_readme_assets.sh` and verified by the invariant; nobody edits it by hand.

## 2. Blast Radius

| Path | Change |
|---|---|
| `crates/gui/Cargo.toml` | `[features] readme-capture = []`; `[dev-dependencies] png = "0.18"` and `soos-test-fixtures = { workspace = true }` (both dev-only) |
| `crates/gui/src/app.rs` | `#[cfg(feature = "readme-capture")] pub mod capture;` (child module, file `crates/gui/src/app/capture.rs`); no behaviour change of `SoosApp::new` or `ui` |
| `crates/gui/src/app/capture.rs` | new, feature-gated scene builder (§3.1) |
| `crates/gui/tests/readme_snapshots.rs` | new test target, `#![cfg(feature = "readme-capture")]` |
| `crates/gui/tests/readme_capture/{mod,raster,png_io}.rs` | new shared test code (rasterizer, PNG I/O); not a test target (directory module) |
| `tests/fixtures/mod.rs` | new `pub mod readme` (placeholder face) and `pub mod readme_web` (pure fixture routing) |
| `tests/fixtures/examples/readme_web_fixture_server.rs` | new std-only example binary |
| `tests/fixtures/readme_web/*.json` | none: JSON is built in code (one source of truth) |
| `tests/fixtures/tests/readme_fixture_tests.rs` | new tests |
| `tests/invariants/src/readme_assets_contract.rs` (+ `mod` line in `lib.rs`) | new invariant |
| `Docs/assets/MANIFEST.toml`, `Docs/assets/screenshots/gui-*.png` (and `web-*.png` when generated) | new assets |
| `scripts/capture_readme_assets.sh`, `scripts/capture_readme_web.sh` | new scripts |
| `Docs/README_ASSETS.md` (indexed in `Docs/README.md`), `Docs/GUI_APPLICATION.md`, `Docs/DEVELOPMENT_WORKFLOW.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/207_readme_capture.md` | docs |

Not touched: `README.md` (PR 2), `crates/remote/**`, `packaging/**`, `scripts/install.sh`, `build_deb.sh`,
`soos.spec`, `PKGBUILD`, every runtime crate's normal dependencies.

## 3. Types & Signatures

### 3.1 `soos_gui::app::capture` (feature `readme-capture`)

```rust
/// Username shown by every capture scene (never the host account).
pub const CAPTURE_USERNAME: &str = "alice";
/// Uid shown by every capture scene.
pub const CAPTURE_UID: u32 = 1000;
/// Fixed telemetry shown in the header band.
pub const CAPTURE_FPS: f32 = 30.0;
pub const CAPTURE_LATENCY_MS: f64 = 12.5;
/// Enrollment time of the synthetic profile (2026-10-01T00:00:00Z).
pub const CAPTURE_ENROLLED_AT_UNIX_S: u64 = 1_790_812_800;
/// Logical window size of every capture, in points (the `soos-gui` default window).
pub const CAPTURE_WINDOW_POINTS: [f32; 2] = [1120.0, 780.0];
/// Bound on the wait for the fixed daemon probe to publish its first state.
pub const CAPTURE_DAEMON_STATE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureScene { LiveInspection, GuidedEnrollment, Profiles }

impl CaptureScene {
    pub const ALL: [CaptureScene; 3];
    /// `gui-live-inspection`, `gui-guided-enrollment`, `gui-profiles`.
    pub fn file_stem(self) -> &'static str;
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("capture frame must be RGB24 with non-zero dimensions")]
    UnsupportedFrame,
    #[error("vision pipeline failed on the capture frame: {0}")]
    Pipeline(String),
    #[error("capture thread could not be spawned: {0}")]
    Spawn(std::io::Error),
    #[error("the fixed daemon state was not published in time")]
    DaemonStateTimeout,
}

/// Builds a `SoosApp` showing `scene` from `frame` (a synthetic fixture), with no worker, camera, systemctl,
/// pkexec or store side effect.
pub fn capture_app(cc: &eframe::CreationContext<'_>, scene: CaptureScene, frame: &Frame)
    -> Result<SoosApp, CaptureError>;
```

`capture_app` semantics (each line is testable):
1. `theme::apply(&cc.egui_ctx)` exactly like `SoosApp::new`.
2. Frame must be `PixelFormat::Rgb24`, `width > 0`, `height > 0`, `data.len() == w*h*3` (checked
   multiplication); otherwise `UnsupportedFrame`.
3. Pipeline: `VisionPipeline::new(MockFaceDetector::new_centered_face(w, h, 0.97), MockPadDetector::new_live(),
   MockEmbeddingExtractor::new(EMBEDDING_DIMENSION), VisionPipelineConfig::default())`; one synchronous
   `analyze_frame`; error → `Pipeline(msg)`. `pad_live` from `pipeline.config().pad_passes(p,
   PadInputModality::for_frame(frame))`, as the worker does.
4. `LatestFrameData { fps: CAPTURE_FPS, pipeline_latency_ms: CAPTURE_LATENCY_MS, det_latency_ms:
   CAPTURE_LATENCY_MS * 0.45, pad_latency_ms: CAPTURE_LATENCY_MS * 0.25, sequence: 1, .. }` stored in the slot.
5. Camera: a `MockCameraManager` with the default config (status only; its frames are never read because no
   worker exists). `worker_running` = `false`, `_worker_thread` = `None`.
6. Daemon: `DaemonMonitor::spawn(Arc::new(FixedDaemonProbe { active: true }), DAEMON_POLL_INTERVAL)`; wait up to
   `CAPTURE_DAEMON_STATE_TIMEOUT` (poll 10 ms) for `DaemonState::Active`, else `DaemonStateTimeout`. The probe
   never runs a process.
7. Privileged tasks: `TaskRunner::new(Arc::new(RefusingExecutor), noop)`; `RefusingExecutor::execute` returns a
   failed outcome without running anything (it is never reached: no action is submitted).
8. Store: `GuiStore::Polkit` (no banner, no path), `store_tasks: None`; `refresh_profiles` is **not** called.
9. `enrollment.target_username = CAPTURE_USERNAME`, `target_uid = CAPTURE_UID` (overwriting the host defaults).
10. `profiles.profiles` = `[]` except for `Profiles`: one `EnrolledUserSummary { uid: CAPTURE_UID, username:
    "alice", model_id: SHIPPED_EMBEDDING_MODEL.id, model_version: <manifest version>, enrollment_timestamp:
    CAPTURE_ENROLLED_AT_UNIX_S, embedding_dim: EMBEDDING_DIMENSION }`.
11. `current_tab` from the scene.

### 3.2 Test-only code (`crates/gui/tests/readme_capture/`)

```rust
pub const CAPTURE_PIXELS_PER_POINT: f32 = 1.0;
pub const CAPTURE_PASSES: usize = 3;            // layout settles in pass 2; pass 3 is rendered
pub struct RgbaImage { pub width: u32, pub height: u32, pub pixels: Vec<u8> } // RGBA8, premultiplied → opaque
pub struct Rasterizer { textures: HashMap<TextureId, Texture> }
impl Rasterizer {
    pub fn apply(&mut self, delta: &egui::TexturesDelta);   // set (whole or partial at `pos`) then free
    pub fn render(&self, prims: &[ClippedPrimitive], ppp: f32, size_px: [u32; 2]) -> RgbaImage;
}
pub fn render_scene(scene: CaptureScene) -> (RgbaImage, Vec<String> /* every text shown */);
pub fn encode_png(img: &RgbaImage) -> Vec<u8>;       // RGB8, IHDR/IDAT/IEND only, Best compression, adaptive filter
pub fn decode_png(bytes: &[u8]) -> Result<RgbaImage, String>;
pub fn diff_fraction(a: &RgbaImage, b: &RgbaImage, channel_tolerance: u8) -> Option<f64>; // None if sizes differ
```

Rasterizer contract: triangles in pixel space (`pos * ppp`), clip rect `clip_rect * ppp` rounded to pixel edges,
pixel-centre sampling with the top-left fill rule (shared edges painted once), barycentric interpolation of the
premultiplied vertex colour and UV, bilinear texture fetch with clamp-to-edge, `out = src + dst * (1 - src.a)` in
gamma space (what `egui_glow` does on a non-sRGB framebuffer); the image starts as the opaque
`ctx.style().visuals.panel_fill` (the brand chrome then covers it); final alpha forced to 255. `Callback`
primitives are ignored (`soos-gui` paints none). Bounds: image ≤ 4096×4096 px, texture ≤ 4096×4096; larger
deltas are refused with a test failure.

Every pixel loop uses `usize` indices from bounded ranges and `get`/`get_mut`; test code may use
`#![allow(clippy::indexing_slicing, ...)]` as the other test files do.

### 3.3 `soos_test_fixtures::readme`

```rust
/// A rendered placeholder face (flat brand-coloured illustration: background, shoulders, head ellipse, hair cap,
/// eyes, nose and mouth strokes), no photo and no randomness; head centred on the box of
/// `MockFaceDetector::new_centered_face(width, height, _)` (x 25–75 %, y 20–80 %). Rgb24, sequence 1.
pub fn create_placeholder_face_frame(width: u32, height: u32) -> Frame;
```
Bounds: `width`, `height` in `16..=4096`; outside, a 16×16 frame of the background colour is returned (never a
panic). Pure function of `(width, height)`.

### 3.4 `soos_test_fixtures::readme_web`

```rust
pub const DEMO_HOST: &str = "soos-demo.example";
pub const MAX_FIXTURE_REQUEST_HEAD_BYTES: usize = 8192;
pub const FIXTURE_SSE_HOLD_MS: u64 = 10_000;
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum WebScene { Unlocked, LockedWithAlerts }
impl WebScene { pub const ALL: [WebScene; 2]; pub fn name(self) -> &'static str; pub fn parse(s: &str) -> Option<Self>; }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureResponse {
    Asset(&'static str),                    // file name inside crates/remote/assets (allow-list below)
    Json { status: u16, body: String },
    EventStream(String),                    // complete SSE prelude (status, alerts, battery events)
    NotFound,
}
pub const ASSET_NAMES: [&str; 11];          // the 11 files of crates/remote/assets ("/" → index.html)
pub fn route(scene: WebScene, method: &str, path: &str, now_unix_ms: u64) -> FixtureResponse;
```
Route table: `GET /` and `/index.html` → `Asset("index.html")`; `GET /<name>` for a name in `ASSET_NAMES` → asset;
`GET /api/auth/state` → `{"mode":"tailnet","authenticated":true,"passkeys":true,"unlock_enabled":true,"enrollment":false}`;
`GET /api/status` → status view of the scene (`checked_unix_ms = now`); `GET /api/alerts` → alerts view
(`Unlocked`: active, zero counts; `LockedWithAlerts`: two wrong passwords from `sudo` on `owner`);
`GET /api/push` → `{"state":"disabled"}` (the `PushView::disabled()` state the page tests; the push card stays
hidden); `GET /api/camera` → `{"enabled":true,"reachable":true,"state":"idle","max_view_s":120,"fps":5,"width":640}`
(the camera card shows its start button; no stream route exists, so a tap fails closed);
`GET /api/events` → `EventStream`; anything else (any method other than GET, `..`, query strings, unknown path)
→ `NotFound`. No identity anywhere: no host name, IP, login, passkey id or endpoint URL in any body.

### 3.5 Example `readme_web_fixture_server`

`cargo run -p soos-test-fixtures --example readme_web_fixture_server -- --scene <name> --assets <dir> [--serve-ms N]`.
Binds `127.0.0.1:0` only, prints `PORT=<n>` on stdout, one thread per connection with at most 16 live connections
(extra ones are closed), read/write timeouts 2 s, request head ≤ `MAX_FIXTURE_REQUEST_HEAD_BYTES`, exits after
`--serve-ms` (default 60 000, clamped `1000..=300_000`). Asset bytes are read only from `<dir>/<allow-listed name>`.

## 4. Constants & Budget

| Name | Location | Value | Semantics |
|---|---|---|---|
| `ASSETS_DIR` | `readme_assets_contract.rs` | `Docs/assets` | every README image lives here |
| `MANIFEST_PATH` | idem | `Docs/assets/MANIFEST.toml` | one `[[asset]]` table per file |
| `MAX_ASSET_BYTES` | idem | 1 048 576 | per file, inclusive |
| `MAX_TOTAL_ASSET_BYTES` | idem | 8 388 608 | sum of every file under `Docs/assets` except `MANIFEST.toml` |
| `MAX_ASSET_FILES` | idem | 32 | file count |
| `ALLOWED_EXTENSIONS` | idem | `png`, `webp`, `svg` | anything else refused |
| `ALLOWED_GENERATORS` | idem | `soos-gui readme_snapshots`, `scripts/capture_readme_web.sh` | provenance allow-list |
| `ALLOWED_BADGE_PREFIXES` | idem | `https://img.shields.io/`, `https://github.com/soos-security/soos/actions/workflows/` (+ suffix `/badge.svg`) | only external images |
| PNG chunk allow-list | idem | `IHDR PLTE tRNS gAMA cHRM sRGB pHYs IDAT IEND` | `tEXt zTXt iTXt eXIf tIME iCCP acTL` and anything else refused |
| WebP chunk allow-list | idem | `VP8 `, `VP8L`, `VP8X`, `ALPH` | `EXIF XMP  ICCP ANIM ANMF` refused; VP8X flags EXIF/XMP/ICC/animation bits must be 0 |
| SVG refusals | idem | `<metadata`, `<image`, `sodipodi:`, `inkscape:`, `<script`, `href="http`, `href="data:` | no embedded raster, no editor metadata, no script |
| `CAPTURE_DIFF_CHANNEL_TOLERANCE` | snapshot test | 8 | per-channel difference ignored |
| `CAPTURE_MAX_DIFF_FRACTION` | snapshot test | 0.001 | fraction of differing pixels tolerated (0.1 %) |

Sentinels: a missing `Docs/assets` directory or manifest fails the asset tests (the PR commits them); an empty
README image list passes the README test (the current README has no image); a manifest entry whose file is
missing, a file without entry, a duplicate path, an entry outside `Docs/assets` (`..`, absolute path) all fail.
Manifest grammar (parsed without a TOML crate): repeated `[[asset]]` blocks with exactly the keys `path`
(relative to `Docs/assets`), `generator`, `sha256` (64 lowercase hex), `source` (must be `"synthetic"`); `#`
comments and blank lines allowed; anything else is a parse failure.

## 5. Error Taxonomy

No PAM, IPC, daemon or store path is touched: no verdict, `ReasonClass` or PAM result changes. Capture errors are
test failures (`CaptureError` surfaces through `expect` in test code only). The fixture server exits non-zero on a
bind or argument error and never panics on client input (`route` is total).

## 6. Latency Budget

Not applicable (no auth-path change).

## 7. Invariants Touched

- AGENTS.md "never log frames, biometric embeddings": capture assets are synthetic renderings only; the invariant
  and D-3 keep real imagery out of the repository.
- Dev-only fixtures (`fixtures_contract::test_fil_fixtures_crate_is_never_a_runtime_dependency`): `soos-test-fixtures`
  stays in `[dev-dependencies]` of `soos-gui`.
- `#![forbid(unsafe_code)]` on `soos-gui` (the new module inherits the crate attribute; no `unsafe` anywhere).
- Supply chain: `png` dev-only, already locked, permissive; `cargo deny --locked check` must pass.
- No new surface in `soos-remote`: invariant test RA-S5 below; existing `remote_battery_contract` test 37
  already pins the `[dependencies]` key set of `crates/remote`.

## 8. Test Hooks for the Tester (prefix `test_rac_`, matrix prefix `RAC`)

| ID | Test | Location |
|---|---|---|
| RAC1 | `test_rac_capture_scenes_match_committed_assets` (renders the 3 scenes, compares with `Docs/assets/screenshots/<stem>.png`; update mode rewrites) | `crates/gui/tests/readme_snapshots.rs` |
| RAC2 | `test_rac_capture_render_is_deterministic` (two renders of the live scene are byte-identical) | idem |
| RAC3 | `test_rac_capture_scenes_show_only_the_synthetic_identity` (texts contain `alice` on enrollment/profiles; never the host login unless it is `alice`, never `/tmp`, `DEVELOPER STORE`, `/home/`, `@`) | idem |
| RAC4 | `test_rac_capture_app_rejects_non_rgb_frames` | idem |
| RAC5 | `test_rac_rasterizer_*` (coverage of a triangle, top-left rule on a shared edge, clip rect, premultiplied blend, bilinear texture fetch, partial texture delta) | idem (unit tests on the test module) |
| RAC6 | `test_rac_png_writer_emits_only_critical_chunks` and `test_rac_png_round_trip` | idem |
| RAC7 | `test_rac_placeholder_face_is_deterministic_and_centred` (same bytes twice; head pixels inside the mock box; background outside; out-of-range sizes give 16×16, no panic) | `tests/fixtures/tests/readme_fixture_tests.rs` |
| RAC8 | `test_rac_web_routes_serve_only_allow_listed_assets` (traversal, query, POST, unknown → NotFound) | idem |
| RAC9 | `test_rac_web_fixture_bodies_carry_no_identity` (every JSON/SSE body: no `@`, no `ts.net`, no IPv4 pattern, no `alice`-other logins, no `credential`/`endpoint` values) and `test_rac_web_scenes_differ` | idem |
| RAC10 | `test_rac_readme_images_resolve_under_docs_assets_with_alt_text` (real README) + `test_rac_readme_image_refs_are_checked` (synthetic cases) | `tests/invariants/src/readme_assets_contract.rs` |
| RAC11 | `test_rac_docs_assets_are_listed_in_the_manifest_with_matching_sha256` + `test_rac_manifest_parser_rejects_malformed_entries` | idem |
| RAC12 | `test_rac_docs_assets_stay_within_the_budget` | idem |
| RAC13 | `test_rac_docs_assets_carry_no_metadata_and_no_animation` + `test_rac_png_webp_svg_metadata_detectors` (synthetic byte strings) | idem |
| RAC14 | `test_rac_sha256_known_vectors` (empty string, `abc`, 1 MB of `a`) | idem |
| RAC15 | `test_rac_capture_tooling_is_dev_only` (`readme-capture` not in `default`; `png` and `soos-test-fixtures` only under `[dev-dependencies]` of `crates/gui`; no packaging/install/CI build script names `readme-capture` except the capture script and `ci.yml`-free; the fixture server is under `tests/fixtures/examples`; nothing under `crates/remote` mentions `readme_web`) | idem |
| RAC16 | `test_rac_capture_scripts_are_hardware_free` (both scripts: `set -euo pipefail`; no `/dev/video`, `sudo`, `pkexec`, `systemctl`, `soos-daemon`, `curl`, `wget`, `npm`, `npx`, `docker pull` without digest; web script binds the fixture server only and uses a throw-away browser profile) | idem |

## 9. Documentation Drift / ADR

New ADR, appended at the end of `AI/DECISIONS.md`: "[2026-10-13] README Assets Are Generated From Synthetic Data,
Never Hand-Edited (GitHub #381, PR 1 of #356)": D-1 … D-7, OD-1 … OD-4 marked "assumed — owner to confirm".
Drift recorded: the epic proposes `egui_kittest` + wgpu + a Playwright container and the `chore/readme-redesign`
branch; this PR deliberately deviates (D-1, OD-3) and uses `test/readme-capture` as the epic permits.

## 10. Revision 2 (plan evaluation round 1: F-1 MAJOR, F-2 to F-12 MINOR)

| Finding | Resolution (binding; overrides §1–§9) |
|---|---|
| F-1 (MAJOR) RAC3 substring scan | `capture` exposes `pub fn shown_identities(app: &SoosApp) -> CaptureIdentities { enrollment_username: String, enrollment_uid: u32, profile_usernames: Vec<String> }`. RAC3 asserts `enrollment_username == "alice"`, `enrollment_uid == 1000`, every profile username `== "alice"` (exact state check, independent of the host). The free-text scan of the rendered galleys (collected by walking `FullOutput.shapes` recursively, `Shape::Text` → `galley.text()`, `Shape::Vec` recursed) uses whole-word matching (words split on any character that is not ASCII alphanumeric, `_`, `-` or `.`), and is skipped, by a documented rule, when the host login is `alice` or one of the ordinary words `root`, `user`, `admin`, `test`, `guest`, `soos`. The scan still refuses `/tmp`, `/home/`, `/run/`, `/var/lib/soos`, `@` and `DEVELOPER STORE`. |
| F-2 mock camera thread | `capture` uses a private thread-free `StaticCaptureCamera` (`CameraManager` impl: `latest_frame` → `None`, `is_ready` → `true`, `notify_activity`/`stop` no-ops). D-4 holds: the only capture thread is the `DaemonMonitor` with its fixed probe (no process). |
| F-3 drift tolerance | `CAPTURE_DIFF_CHANNEL_TOLERANCE` = 16 (absorbs ±1 anti-aliasing differences from platform `powf`), `CAPTURE_MAX_DIFF_FRACTION` = 0.00002 (at most 17 of 873 600 pixels); a changed glyph moves far more than 16 levels on many pixels. The claim becomes "identical on the same platform, within tolerance across platforms"; RAC2 still asserts byte identity of two renders in one process. |
| F-4 drift workflow | Update mode refuses to run when `CI` is set (test panics). The failure message names `scripts/capture_readme_assets.sh` (which also rewrites `MANIFEST.toml`). ADR + `Docs/README_ASSETS.md` state: regenerating the golden PNGs through the script after an intended UI change is not test weakening (the PNGs are generated data, not assertions); an unrelated GUI PR that changes a captured screen must rerun the script; each regeneration adds at most the asset budget to git history. |
| F-5 Docker | Deviation recorded in §9/ADR: the GUI capture needs no container because it is a plain `cargo test` (no GPU, no system library beyond the workspace build); the web script runs on the host with an existing browser and is optional. |
| F-6 fixture bodies | Bodies pinned: status = `{"state","active","idle","idle_since_unix_s","checked_unix_ms"}` (`StatusView`); alerts = `{"state","reason","epoch","lock_screen","unacknowledged_wrong_password","unacknowledged_locked_out","last_unix_ms","last_source","through","history"}` (`AlertsView`), history records `{"id","first_unix_ms","last_unix_ms","source","account","kind","count"}` as in `AlertRecord`; battery = `{"state","percent","charge","external_power"}`; push = `PushView::disabled()` = `{"state":"disabled","reason":null,"public_key":null,"subscriptions":0,"devices":[],"last_delivery":null,"sender":"unknown"}`; auth state and camera as §3.4. Test `test_rac_web_fixture_keys_match_the_remote_types` reads `crates/remote/src/{status,alerts,push,battery}.rs` and asserts every pinned key is a `pub <key>:` field of the matching struct. |
| F-7 browser PNG metadata | `soos_test_fixtures::readme_web::strip_png_metadata(&[u8]) -> Result<Vec<u8>, String>` keeps only IHDR, PLTE, tRNS, IDAT, IEND (chunk bytes copied verbatim, CRCs unchanged), refuses APNG (`acTL`) and malformed input; the example exposes it as `--strip-png <in> <out>`; the web script runs it on every screenshot. Viewport pinned at 390×844 CSS px, device scale factor 2, variants `light` and `dark` (`--force-dark-mode`), scenes `unlocked` and `locked-alerts`: 4 files `web-<scene>-<variant>.png`. |
| F-8 loopback | RAC15 adds a static check: `tests/fixtures/mod.rs` and the example bind `127.0.0.1` and never contain `0.0.0.0`, `[::]` or `::/`; `serve` takes an already bound listener and the example binds `127.0.0.1:0`. |
| F-9 facts | Base `6f32ef8`. Profile model id/version: `soos_enrollment_cli::service::{MODEL_ID_EMBEDDING, EMBEDDING_MODEL_VERSION}`. `LatestFrameData` is built with every field listed explicitly (no `Default`). |
| F-10 RAC15 rule | Exact rule: no file under `packaging/`, `scripts/` (except the two capture scripts), `.github/`, `tests/docker/`, `tests/distro/`, nor `Dockerfile`, `save.sh`, `run_tests.sh` contains `readme-capture` or `readme_web_fixture_server`. CI enables the feature only implicitly through `--all-features`. |
| F-11 printing | The example's `main` carries `#[allow(clippy::print_stdout, clippy::print_stderr, reason = "...")]` and prints `PORT=<n>` once. |
| F-12 mock outputs | `Docs/README_ASSETS.md` and the walkthrough state that every detector, liveness and match value in the GUI screenshots comes from the mock models on a drawn placeholder face; PR 2 captions must say so. |
