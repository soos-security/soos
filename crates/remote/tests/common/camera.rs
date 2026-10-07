//! Live camera view fixtures of the camera contract tests (ADR 2026-10-07 "Live Camera View
//! in `soos-remote` Through the Daemon Preview Channel", architect spec
//! `AI/architect_spec_remote_live_camera.md` §13.5, §17; GitHub #345).
//!
//! - [`SourceSpy`] / [`ScriptedPreviewSource`]: a tester-owned `PreviewSource` that answers
//!   scripted [`Step`]s (frames, errors, virtual delays, never) and records every factory
//!   call, every started, completed and cancelled (dropped before completion) `next_frame`
//!   call with its virtual start and end time, and every dropped source.
//! - [`start_camera`]: the `harness.rs` service with `camera` settings and the scripted
//!   factory wired through `ServerState::with_camera`, optionally Web Push wired to a
//!   `FakeTransport`.
//! - [`ViewClient`]: a raw multipart reader that parses every part strictly
//!   (`--soosframe`, `Content-Type: image/jpeg`, `Content-Length`, JPEG `FF D8`..`FF D9`,
//!   trailing CRLF), stamps each part with the virtual time it was observed and waits in
//!   real time (never in virtual time) for the blocking-pool JPEG encoder.
//! - Deterministic small frames (Grey 64x48, RGB24) and a 640x480 RGB24 noise frame that
//!   fills the socket buffer in two or three parts (write-stall cases).
//!
//! Nothing here opens a daemon socket or a video device.
//!
//! Include with `#[path = "common/passkey.rs"] mod passkey;`,
//! `#[path = "common/harness.rs"] mod harness;`, `#[path = "common/push.rs"] mod push;`
//! then `#[path = "common/camera.rs"] mod camera;`.

#![allow(
    dead_code,
    unused_imports,
    reason = "Shared test fixtures library used conditionally across test modules"
)]

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;
use tokio::task::yield_now;
use tokio::time::{timeout, Instant};
use zeroize::Zeroizing;

use soos_remote::camera::CameraSettings;
use soos_remote::camera_ipc::{PreviewError, PreviewFrame, PreviewSource, PreviewSourceFactory};
use soos_remote::config::{
    AuthConfig, CameraConfig, PushConfig, PushPreviews, RemoteConfig, TailscaleLogin,
};
use soos_remote::push::PushSettings;
use soos_remote::server::{serve, ServerState};
use soos_remote::{CREDENTIALS_FILE_NAME, DEFAULT_POLL_INTERVAL_MS, PUSH_STORE_FILE_NAME};

use super::harness::*;
use super::passkey::*;
use super::push::FakeTransport;

/// Wire codes of `PreviewResponse::format` (spec §3.1; literal so a wrong constant in the
/// protocol crate cannot hide here).
pub const FMT_RGB24: u8 = 0;
pub const FMT_GREY: u8 = 1;
pub const FMT_YUYV: u8 = 2;
pub const FMT_NV12: u8 = 3;
pub const FMT_MJPEG: u8 = 4;
pub const FMT_EMPTY: u8 = 255;

/// Real-time bound of one wait for the blocking-pool encoder.
pub const ENCODE_WAIT: Duration = Duration::from_millis(1500);
/// Virtual step of the reader loops.
pub const VIEW_STEP_MS: u64 = 20;
/// Real-time bound of one `open_raw` (well below `WALL_CLOCK_FALLBACK`).
pub const OPEN_REAL_BOUND: Duration = Duration::from_secs(12);
/// Scheduler rounds granted to a partially received refusal body before time moves.
pub const PARTIAL_BODY_ROUNDS: u32 = 64;
/// Virtual time granted to a partially received refusal body beyond `max_ms`.
pub const PARTIAL_BODY_GRACE_MS: u64 = 1_000;
/// Real-time budget of one harness: `FrozenClock` releases after `WALL_CLOCK_FALLBACK`
/// (20 s), after which tokio auto-advance could fire the stall, TTL and cooldown timers.
pub const HARNESS_REAL_BUDGET: Duration = Duration::from_secs(17);

/// Real-time budget of one harness (auditor B9): a sub-case that would outlive the
/// `FrozenClock` fails with a clear message instead of silently letting auto-advance run.
pub struct RealBudget {
    start: std::time::Instant,
}

impl RealBudget {
    /// Starts the budget; create it right before `start_camera`.
    pub fn start() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }

    /// Asserts the harness is still inside its frozen-clock window.
    pub fn check(&self, what: &str) {
        let used = self.start.elapsed();
        assert!(
            used < HARNESS_REAL_BUDGET,
            "{what}: {used:?} of real time used, beyond the {HARNESS_REAL_BUDGET:?} budget \
             (the frozen clock releases after 20 s); the result would not be meaningful"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------------------

/// One scripted frame.
#[derive(Clone)]
pub struct FrameSpec {
    pub sequence: u64,
    pub format: u8,
    pub width: u32,
    pub height: u32,
    pub data: Arc<Vec<u8>>,
}

/// A deterministic Grey gradient.
pub fn grey_data(width: u32, height: u32, seed: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            out.push(((u64::from(x) * 3 + u64::from(y) * 5 + seed * 7) % 256) as u8);
        }
    }
    out
}

/// Deterministic pseudo-random bytes (LCG): incompressible pixels.
pub fn noise_data(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        out.push((state >> 33) as u8);
    }
    out
}

/// A Grey 64x48 frame with `sequence`.
pub fn grey_frame(sequence: u64) -> FrameSpec {
    FrameSpec {
        sequence,
        format: FMT_GREY,
        width: 64,
        height: 48,
        data: Arc::new(grey_data(64, 48, sequence)),
    }
}

/// A Grey 640x480 frame with `sequence` (half-width cases).
pub fn grey_vga_frame(sequence: u64) -> FrameSpec {
    FrameSpec {
        sequence,
        format: FMT_GREY,
        width: 640,
        height: 480,
        data: Arc::new(grey_data(640, 480, sequence)),
    }
}

/// An RGB24 64x48 frame with `sequence`.
pub fn rgb_frame(sequence: u64) -> FrameSpec {
    FrameSpec {
        sequence,
        format: FMT_RGB24,
        width: 64,
        height: 48,
        data: Arc::new(noise_data(64 * 48 * 3, sequence)),
    }
}

/// An RGB24 640x480 noise frame: a JPEG part of roughly 100 KiB or more, so two or three
/// unread parts fill a Unix socket buffer (write-stall cases).
pub fn big_noise_frame(sequence: u64) -> FrameSpec {
    FrameSpec {
        sequence,
        format: FMT_RGB24,
        width: 640,
        height: 480,
        data: Arc::new(noise_data(640 * 480 * 3, sequence)),
    }
}

/// An MJPEG frame (never decoded by `soos-remote`).
pub fn mjpeg_frame(sequence: u64) -> FrameSpec {
    FrameSpec {
        sequence,
        format: FMT_MJPEG,
        width: 64,
        height: 48,
        data: Arc::new(vec![0xFF, 0xD8, 0xFF, 0xD9]),
    }
}

/// The explicit empty preview (camera waking).
pub fn empty_frame(sequence: u64) -> FrameSpec {
    FrameSpec {
        sequence,
        format: FMT_EMPTY,
        width: 0,
        height: 0,
        data: Arc::new(Vec::new()),
    }
}

/// An NV12 frame (never decoded by `soos-remote`).
pub fn nv12_frame(sequence: u64) -> FrameSpec {
    FrameSpec {
        sequence,
        format: FMT_NV12,
        width: 64,
        height: 48,
        data: Arc::new(vec![128; 64 * 48 * 3 / 2]),
    }
}

// ---------------------------------------------------------------------------------------
// Scripted preview source
// ---------------------------------------------------------------------------------------

/// What one `next_frame` call does.
#[derive(Clone)]
pub enum Step {
    Frame(FrameSpec),
    Fail(PreviewError),
    /// Never completes.
    Pending,
    /// Waits this many virtual ms, then does the inner step.
    After(u64, Box<Step>),
}

impl Step {
    pub fn after(ms: u64, step: Step) -> Self {
        Self::After(ms, Box::new(step))
    }
}

/// One recorded `next_frame` call.
#[derive(Clone, Debug)]
pub struct CallRecord {
    pub started_at: Instant,
    pub ended_at: Option<Instant>,
    /// `Some(true)` = a non-empty frame, `Some(false)` = an error or an empty frame.
    pub returned_frame: Option<bool>,
    pub error: Option<PreviewError>,
    pub cancelled: bool,
    /// Sequence of a returned non-empty frame.
    pub sequence: Option<u64>,
}

type DefaultStep = Box<dyn FnMut(u64) -> Step + Send>;

pub struct SpyInner {
    pub queue: Mutex<VecDeque<Step>>,
    pub default: Mutex<DefaultStep>,
    pub built: AtomicU64,
    pub dropped: AtomicU64,
    pub calls: Mutex<Vec<CallRecord>>,
}

/// Shared, cloneable view of every scripted source built by the factory.
#[derive(Clone)]
pub struct SourceSpy(pub Arc<SpyInner>);

impl SourceSpy {
    /// Every call answers a fresh Grey 64x48 frame with an increasing sequence (1, 2, ...).
    pub fn increasing() -> Self {
        Self::with_default(|i| Step::Frame(grey_frame(i + 1)))
    }

    pub fn with_default(default: impl FnMut(u64) -> Step + Send + 'static) -> Self {
        Self(Arc::new(SpyInner {
            queue: Mutex::new(VecDeque::new()),
            default: Mutex::new(Box::new(default)),
            built: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            calls: Mutex::new(Vec::new()),
        }))
    }

    /// Replaces the answer of every call once the queue is empty.
    pub fn set_default(&self, default: impl FnMut(u64) -> Step + Send + 'static) {
        *self.0.default.lock().unwrap() = Box::new(default);
    }

    /// Answers handed out (front first) before the default.
    pub fn queue(&self, steps: Vec<Step>) {
        self.0.queue.lock().unwrap().extend(steps);
    }

    /// The factory handed to `ServerState::with_camera`.
    pub fn factory(&self) -> PreviewSourceFactory {
        let spy = self.clone();
        Arc::new(move || -> Box<dyn PreviewSource> {
            spy.0.built.fetch_add(1, Ordering::SeqCst);
            Box::new(ScriptedPreviewSource { spy: spy.clone() })
        })
    }

    pub fn built(&self) -> u64 {
        self.0.built.load(Ordering::SeqCst)
    }

    pub fn dropped(&self) -> u64 {
        self.0.dropped.load(Ordering::SeqCst)
    }

    pub fn calls(&self) -> Vec<CallRecord> {
        self.0.calls.lock().unwrap().clone()
    }

    pub fn started(&self) -> usize {
        self.0.calls.lock().unwrap().len()
    }

    pub fn completed(&self) -> usize {
        self.calls().iter().filter(|c| c.ended_at.is_some()).count()
    }

    pub fn cancelled(&self) -> usize {
        self.calls().iter().filter(|c| c.cancelled).count()
    }

    /// Calls that returned a non-empty frame.
    pub fn frames_returned(&self) -> usize {
        self.calls()
            .iter()
            .filter(|c| c.returned_frame == Some(true))
            .count()
    }

    /// Returned non-empty frames whose sequence differs from the previously returned one:
    /// the frames the view loop actually encodes (a repeated sequence is skipped without
    /// encoding, so the reader never waits real time for it; auditor B9).
    pub fn fresh_frames(&self) -> usize {
        let mut last = None;
        let mut fresh = 0;
        for call in self.calls() {
            if let Some(seq) = call.sequence {
                if last != Some(seq) {
                    fresh += 1;
                    last = Some(seq);
                }
            }
        }
        fresh
    }

    fn next_step(&self, index: u64) -> Step {
        let queued = self.0.queue.lock().unwrap().pop_front();
        match queued {
            Some(step) => step,
            None => (self.0.default.lock().unwrap())(index),
        }
    }

    fn begin_call(&self) -> usize {
        let mut calls = self.0.calls.lock().unwrap();
        calls.push(CallRecord {
            started_at: Instant::now(),
            ended_at: None,
            returned_frame: None,
            error: None,
            cancelled: false,
            sequence: None,
        });
        calls.len() - 1
    }

    fn end_call(&self, index: usize, outcome: &Result<PreviewFrame, PreviewError>) {
        let mut calls = self.0.calls.lock().unwrap();
        let call = &mut calls[index];
        call.ended_at = Some(Instant::now());
        match outcome {
            Ok(frame) => {
                call.returned_frame = Some(!frame.data.is_empty());
                if !frame.data.is_empty() {
                    call.sequence = Some(frame.sequence);
                }
            }
            Err(e) => {
                call.returned_frame = Some(false);
                call.error = Some(*e);
            }
        }
    }

    fn cancel_call(&self, index: usize) {
        self.0.calls.lock().unwrap()[index].cancelled = true;
    }
}

/// Marks a started call cancelled when its future is dropped before completion.
struct CallGuard {
    spy: SourceSpy,
    index: usize,
    done: bool,
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        if !self.done {
            self.spy.cancel_call(self.index);
        }
    }
}

/// The tester-owned `PreviewSource` (spec §17).
pub struct ScriptedPreviewSource {
    spy: SourceSpy,
}

impl Drop for ScriptedPreviewSource {
    fn drop(&mut self) {
        self.spy.0.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

impl PreviewSource for ScriptedPreviewSource {
    fn next_frame(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<PreviewFrame, PreviewError>> + Send + '_>> {
        let spy = self.spy.clone();
        Box::pin(async move {
            let index = spy.begin_call();
            let mut guard = CallGuard {
                spy: spy.clone(),
                index,
                done: false,
            };
            let mut step = spy.next_step(index as u64);
            let outcome = loop {
                match step {
                    Step::After(ms, next) => {
                        tokio::time::sleep(Duration::from_millis(ms)).await;
                        step = *next;
                    }
                    Step::Pending => std::future::pending::<()>().await,
                    Step::Fail(e) => break Err(e),
                    Step::Frame(f) => {
                        break Ok(PreviewFrame {
                            sequence: f.sequence,
                            width: f.width,
                            height: f.height,
                            format: f.format,
                            data: Zeroizing::new(f.data.as_ref().clone()),
                        })
                    }
                }
            };
            guard.done = true;
            spy.end_call(index, &outcome);
            outcome
        })
    }
}

// ---------------------------------------------------------------------------------------
// Service with the camera wired
// ---------------------------------------------------------------------------------------

/// Camera settings of a test: enabled, tailnet only, the default bounds.
pub fn camera_on() -> CameraConfig {
    CameraConfig {
        enabled: true,
        ..CameraConfig::default()
    }
}

/// Camera settings of a test: enabled on the tailnet and on the Funnel.
pub fn camera_on_funnel() -> CameraConfig {
    CameraConfig {
        enabled: true,
        funnel: true,
        ..CameraConfig::default()
    }
}

/// Starts `serve` like `Harness::start_with(options)`, plus `camera` and the scripted
/// factory wired through `ServerState::with_camera`; Web Push wired to `push` when given
/// (`push_notifications = true`, detailed previews).
pub async fn start_camera(
    options: Options,
    camera: CameraConfig,
    spy: &SourceSpy,
    push: Option<&FakeTransport>,
) -> Harness {
    let frozen = FrozenClock::hold();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote.sock");
    let store_path = dir.path().join(CREDENTIALS_FILE_NAME);
    let owner = Authenticator::owner();
    match &options.store {
        StoreSetup::Absent => {}
        StoreSetup::Owner => write_store(&store_path, &[StoredPasskey::of(&owner)]),
        StoreSetup::Passkeys(passkeys) => write_store(&store_path, passkeys),
    }
    let push_config = PushConfig {
        enabled: push.is_some(),
        vapid_subject: None,
        socket_path: None,
        previews: PushPreviews::Detailed,
    };
    let config = RemoteConfig {
        allowed_logins: vec![TailscaleLogin::parse(LOGIN).unwrap()],
        socket_path: path.clone(),
        poll_interval_ms: options.poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS),
        allowed_hosts: options.allowed_hosts.clone(),
        allow_unlock: options.allow_unlock,
        auth: AuthConfig {
            rp_id: options.rp_id.clone(),
            allow_funnel: options.allow_funnel,
            credentials_path: Some(store_path.clone()),
        },
        alerts: soos_remote::config::AlertsConfig::default(),
        push: push_config,
        camera: camera.clone(),
    };
    let source = MockSource::unlocked();
    let clock = TestClock::new();
    let clock_fn = Arc::clone(&clock);
    let mut state = ServerState::new(config, UID, source.clone())
        .with_unix_clock(Arc::new(move || clock_fn.now_ms()))
        .with_credentials_path(store_path.clone())
        .with_file_owner_uid(own_uid())
        .with_camera(CameraSettings::from_config(&camera), spy.factory());
    if let Some(random) = options.random.clone() {
        state = state.with_random(random);
    }
    if let Some(transport) = push {
        state = state.with_push(
            PushSettings {
                store_path: dir.path().join(PUSH_STORE_FILE_NAME),
                subject: ORIGIN.to_string(),
                previews: PushPreviews::Detailed,
                rp_id: HOST.to_string(),
            },
            transport.transport(),
        );
    }
    let listener = UnixListener::bind(&path).unwrap();
    let (tx, rx) = oneshot::channel::<()>();
    let server = tokio::spawn(serve(listener, Arc::new(state), async move {
        let _ = rx.await;
    }));
    settle().await;
    Harness {
        dir: dir.path().to_path_buf(),
        _dir: dir,
        path,
        store_path,
        owner,
        source,
        clock,
        shutdown: Some(tx),
        server: Some(server),
        _frozen: frozen,
        secrets: Mutex::new(Vec::new()),
    }
}

// ---------------------------------------------------------------------------------------
// Camera routes
// ---------------------------------------------------------------------------------------

pub const CAMERA_PATH: &str = "/api/camera";
pub const CAMERA_OPTIONS_PATH: &str = "/api/auth/camera/options";
pub const CAMERA_START_PATH: &str = "/api/camera/start";
pub const CAMERA_STOP_PATH: &str = "/api/camera/stop";
pub const CAMERA_STREAM_PREFIX: &str = "/api/camera/stream/";

/// A well-formed (canonical, 43 base64url chars) token that no view issued.
pub fn unknown_stream_path() -> String {
    format!("{CAMERA_STREAM_PREFIX}{}", "A".repeat(43))
}

/// `GET /api/camera` through `via` (JSON).
pub async fn camera_state(h: &Harness, via: &Via) -> Value {
    let r = h.get_via(via, CAMERA_PATH).await;
    assert_eq!(r.status, 200, "GET /api/camera: {}", r.result_or_body());
    r.assert_mandatory_headers();
    r.assert_json_body();
    r.json()
}

/// `POST /api/auth/camera/options` with `X-Soos-Action: camera-options` and the app origin.
pub async fn camera_options(h: &Harness, via: &Via) -> HttpResponse {
    h.ceremony(via, CAMERA_OPTIONS_PATH, "camera-options", None)
        .await
}

/// `POST /api/camera/start` with `X-Soos-Action: camera-view` and the app origin.
pub async fn camera_start(h: &Harness, via: &Via, body: Option<&str>) -> HttpResponse {
    h.ceremony(via, CAMERA_START_PATH, "camera-view", body)
        .await
}

/// `POST /api/camera/stop` with `X-Soos-Action: camera-stop` (no Origin, no body).
pub async fn camera_stop(h: &Harness, via: &Via) -> HttpResponse {
    h.send(
        via,
        "POST",
        CAMERA_STOP_PATH,
        &[("X-Soos-Action", "camera-stop")],
        None,
    )
    .await
}

/// Camera options, a valid UV assertion of the owner, start: the start response.
pub async fn camera_start_with(h: &Harness, via: &Via, auth: &Authenticator) -> HttpResponse {
    let options = camera_options(h, via).await;
    if options.status != 200 {
        return options;
    }
    let challenge = h.challenge_of(&options);
    let body = h.valid_assertion(auth, &challenge);
    camera_start(h, via, Some(&body)).await
}

/// A started view: the `stream_path` of a `200 view_ready` answer (remembered as a secret).
pub async fn ready_view(h: &Harness, via: &Via) -> String {
    let owner = h.owner.clone();
    let r = camera_start_with(h, via, &owner).await;
    assert_eq!(r.status, 200, "camera start: {}", r.result_or_body());
    assert_eq!(r.result(), "view_ready");
    let path = r.json()["stream_path"]
        .as_str()
        .expect("stream_path")
        .to_string();
    assert!(path.starts_with(CAMERA_STREAM_PREFIX), "{path}");
    h.remember(path.trim_start_matches(CAMERA_STREAM_PREFIX));
    path
}

/// The bytes of a stream request through `via` with `X-Soos-Action: camera-stream`.
pub fn stream_request(via: &Via, path: &str) -> Vec<u8> {
    request_bytes(
        via,
        "GET",
        path,
        &[("X-Soos-Action", "camera-stream")],
        None,
    )
}

// ---------------------------------------------------------------------------------------
// Multipart reader
// ---------------------------------------------------------------------------------------

/// One received part.
#[derive(Clone)]
pub struct Part {
    /// Virtual time at which the whole part was observed.
    pub at: Instant,
    pub jpeg: Vec<u8>,
}

/// The client side of an open view.
pub struct ViewClient {
    pub stream: UnixStream,
    pub head: HttpResponse,
    /// Bytes after the head not yet parsed into parts.
    pub buf: Vec<u8>,
    pub parts: Vec<Part>,
    pub trailer: bool,
    pub eof: bool,
    /// `spy.frames_returned()` already waited for.
    pub seen_frames: usize,
}

/// What a stream request produced before any part.
pub enum Opened {
    View(ViewClient),
    /// A complete non-200 answer.
    Refused(HttpResponse),
    /// Closed without a byte.
    Closed,
    /// No head within the virtual bound.
    Silent,
}

impl Opened {
    pub fn view(self) -> ViewClient {
        match self {
            Self::View(v) => v,
            Self::Refused(r) => panic!("view refused: {}", r.result_or_body()),
            Self::Closed => panic!("view closed without a head"),
            Self::Silent => panic!("no view head within the bound"),
        }
    }

    pub fn refused(self) -> HttpResponse {
        match self {
            Self::Refused(r) => r,
            Self::View(_) => panic!("the view was served"),
            Self::Closed => panic!("closed without a response"),
            Self::Silent => panic!("no response within the bound"),
        }
    }
}

/// Reads everything readable now (no clock movement). `Ok(true)` = EOF or reset.
fn drain(stream: &UnixStream, buf: &mut Vec<u8>) -> bool {
    loop {
        let mut chunk = [0u8; 65536];
        match stream.try_read(&mut chunk) {
            Ok(0) => return true,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return false,
            Err(_) => return true,
        }
    }
}

/// Sends a stream request through `via` and waits (virtual steps of [`VIEW_STEP_MS`], real
/// waits for the encoder) for the head, at most `max_ms` of virtual time.
pub async fn open_view(h: &Harness, spy: &SourceSpy, via: &Via, path: &str, max_ms: u64) -> Opened {
    open_raw(h, spy, &stream_request(via, path), max_ms).await
}

pub async fn open_raw(h: &Harness, spy: &SourceSpy, request: &[u8], max_ms: u64) -> Opened {
    let mut stream = connect(&h.path).await;
    timeout(IO_BOUND, stream.write_all(request))
        .await
        .expect("write within the bound")
        .expect("write");
    let mut buf = Vec::new();
    let mut elapsed = 0;
    let mut seen_frames = 0;
    let mut partial_rounds = 0u32;
    let real_start = std::time::Instant::now();
    loop {
        // Auditor B11: a wrong implementation fails here instead of hanging.
        assert!(
            real_start.elapsed() < OPEN_REAL_BOUND,
            "no complete answer to the stream request within {OPEN_REAL_BOUND:?} of real time"
        );
        for _ in 0..SETTLE_ROUNDS {
            yield_now().await;
        }
        let eof = drain(&stream, &mut buf);
        if let Some((head, start)) = parse_head(&buf) {
            if head.status == 200 && head.header("content-length").is_none() {
                let rest = buf[start..].to_vec();
                let mut client = ViewClient {
                    stream,
                    head,
                    buf: rest,
                    parts: Vec::new(),
                    trailer: false,
                    eof,
                    seen_frames: spy.fresh_frames(),
                };
                client.parse();
                return Opened::View(client);
            }
            let len: usize = head
                .header("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if buf.len() >= start + len || eof {
                return Opened::Refused(parse_response(&buf));
            }
            // Auditor B11: a body that stalls without EOF consumes bounded rounds, then
            // virtual time, then fails.
            partial_rounds += 1;
            if partial_rounds < PARTIAL_BODY_ROUNDS {
                continue;
            }
            assert!(
                elapsed < max_ms + PARTIAL_BODY_GRACE_MS,
                "the response body never completed: {} of {len} bytes after the head",
                buf.len() - start
            );
            h.advance_ms(VIEW_STEP_MS).await;
            elapsed += VIEW_STEP_MS;
            continue;
        }
        if eof {
            assert!(
                buf.is_empty(),
                "partial head {:?}",
                String::from_utf8_lossy(&buf)
            );
            return Opened::Closed;
        }
        // A fresh frame was handed out: give the blocking-pool encoder real time.
        let frames = spy.fresh_frames();
        if frames > seen_frames {
            seen_frames = frames;
            let deadline = std::time::Instant::now() + ENCODE_WAIT;
            while std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
                for _ in 0..SETTLE_ROUNDS {
                    yield_now().await;
                }
                if drain(&stream, &mut buf) || parse_head(&buf).is_some() {
                    break;
                }
            }
            continue;
        }
        if elapsed >= max_ms {
            return Opened::Silent;
        }
        h.advance_ms(VIEW_STEP_MS).await;
        elapsed += VIEW_STEP_MS;
    }
}

impl ViewClient {
    /// Parses every complete part of `buf`; panics on any misalignment (spec §8.6 part
    /// format: a part is always written whole).
    pub fn parse(&mut self) {
        loop {
            if self.buf.is_empty() {
                return;
            }
            let trailer = b"--soosframe--\r\n";
            if self.buf.starts_with(trailer) {
                assert!(!self.trailer, "one trailer");
                self.trailer = true;
                self.buf.drain(..trailer.len());
                assert!(
                    self.buf.is_empty(),
                    "nothing after the trailer: {} bytes",
                    self.buf.len()
                );
                return;
            }
            let opener = b"--soosframe\r\n";
            let probe = self.buf.len().min(opener.len());
            assert_eq!(
                &self.buf[..probe],
                &opener[..probe],
                "a part starts with the boundary (misaligned stream after {} parts)",
                self.parts.len()
            );
            let Some(end) = self.buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                assert!(self.buf.len() < 256, "part head too long");
                return;
            };
            let head = std::str::from_utf8(&self.buf[opener.len()..end]).expect("ASCII part head");
            let mut length = None;
            let mut content_type = None;
            for line in head.split("\r\n") {
                let (name, value) = line
                    .split_once(':')
                    .unwrap_or_else(|| panic!("part header {line:?}"));
                match name.trim().to_ascii_lowercase().as_str() {
                    "content-length" => {
                        let value = value.trim();
                        assert!(
                            !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
                            "{value:?}"
                        );
                        length = Some(value.parse::<usize>().unwrap());
                    }
                    "content-type" => content_type = Some(value.trim().to_string()),
                    other => panic!("unexpected part header {other}"),
                }
            }
            assert_eq!(content_type.as_deref(), Some("image/jpeg"));
            let length = length.expect("Content-Length on every part");
            assert!(length <= soos_remote::MAX_CAMERA_JPEG_BYTES, "{length}");
            let body_start = end + 4;
            if self.buf.len() < body_start + length + 2 {
                return;
            }
            let jpeg = self.buf[body_start..body_start + length].to_vec();
            assert_eq!(
                &self.buf[body_start + length..body_start + length + 2],
                b"\r\n",
                "CRLF after the JPEG bytes"
            );
            assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "SOI");
            assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "EOI");
            self.buf.drain(..body_start + length + 2);
            self.parts.push(Part {
                at: Instant::now(),
                jpeg,
            });
        }
    }

    /// Reads and parses what is readable now; when the source handed out a new frame,
    /// waits in real time (never in virtual time) until a new part, EOF, or
    /// [`ENCODE_WAIT`].
    pub async fn poll(&mut self, spy: &SourceSpy) {
        for _ in 0..SETTLE_ROUNDS {
            yield_now().await;
        }
        if drain(&self.stream, &mut self.buf) {
            self.eof = true;
        }
        self.parse();
        let frames = spy.fresh_frames();
        if frames > self.seen_frames && !self.eof {
            self.seen_frames = frames;
            let before = self.parts.len();
            let deadline = std::time::Instant::now() + ENCODE_WAIT;
            while std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
                for _ in 0..SETTLE_ROUNDS {
                    yield_now().await;
                }
                if drain(&self.stream, &mut self.buf) {
                    self.eof = true;
                }
                self.parse();
                if self.eof || self.parts.len() > before {
                    break;
                }
            }
        }
    }

    /// Advances `total_ms` of virtual time in `step_ms` steps, reading after every step;
    /// stops early at EOF.
    pub async fn run(&mut self, h: &Harness, spy: &SourceSpy, total_ms: u64, step_ms: u64) {
        self.poll(spy).await;
        let mut elapsed = 0;
        while elapsed < total_ms && !self.eof {
            let n = step_ms.min(total_ms - elapsed);
            h.advance_ms(n).await;
            elapsed += n;
            self.poll(spy).await;
        }
    }

    /// Like [`Self::run`] until EOF; returns the virtual time consumed, `None` if the
    /// stream is still open after `max_ms`.
    pub async fn ends_within(
        &mut self,
        h: &Harness,
        spy: &SourceSpy,
        max_ms: u64,
        step_ms: u64,
    ) -> Option<u64> {
        self.poll(spy).await;
        let mut elapsed = 0;
        loop {
            if self.eof {
                return Some(elapsed);
            }
            if elapsed >= max_ms {
                return None;
            }
            h.advance_ms(step_ms).await;
            elapsed += step_ms;
            self.poll(spy).await;
        }
    }

    /// Sends stray bytes on the request side (a client never sends a body here).
    pub async fn send_stray(&mut self, bytes: &[u8]) {
        let _ = timeout(IO_BOUND, self.stream.write_all(bytes)).await;
    }
}

/// SOF0 (`FF C0`) of a baseline JPEG: (height, width, components).
pub fn sof0(jpeg: &[u8]) -> (u16, u16, u8) {
    let pos = jpeg
        .windows(2)
        .position(|w| w == [0xFF, 0xC0])
        .expect("baseline SOF0 marker");
    let s = &jpeg[pos + 2..];
    let height = u16::from_be_bytes([s[3], s[4]]);
    let width = u16::from_be_bytes([s[5], s[6]]);
    (height, width, s[7])
}

/// Waits (virtual steps) until the view loop stops asking for frames: no new
/// `next_frame` call across `quiet_steps` consecutive steps of `step_ms`. Returns the
/// virtual time consumed. Used to detect a part write blocked on a full socket buffer.
pub async fn wait_blocked(
    h: &Harness,
    spy: &SourceSpy,
    step_ms: u64,
    quiet_steps: u32,
    max_ms: u64,
) -> u64 {
    let mut elapsed = 0;
    let mut last = spy.started();
    let mut quiet = 0;
    // Let any frame already handed out reach the encoder and the socket.
    loop {
        let deadline = std::time::Instant::now() + Duration::from_millis(200);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
            for _ in 0..SETTLE_ROUNDS {
                yield_now().await;
            }
        }
        h.advance_ms(step_ms).await;
        elapsed += step_ms;
        let now = spy.started();
        if now == last {
            quiet += 1;
            if quiet >= quiet_steps {
                return elapsed;
            }
        } else {
            quiet = 0;
            last = now;
        }
        assert!(elapsed < max_ms, "the view loop never blocked");
    }
}

/// Like [`wait_blocked`], but every quiet step must stay quiet for a full real
/// [`ENCODE_WAIT`] (auditor B7): a slow debug encode finishes within that window and asks
/// for the next frame, so only a part write blocked on a full socket buffer is reported.
/// Panics if the loop never blocks within `max_ms` of virtual time.
pub async fn wait_write_blocked(
    h: &Harness,
    spy: &SourceSpy,
    step_ms: u64,
    quiet_steps: u32,
    max_ms: u64,
) -> u64 {
    let mut elapsed = 0;
    let mut last = spy.started();
    let mut quiet = 0;
    loop {
        h.advance_ms(step_ms).await;
        elapsed += step_ms;
        let deadline = std::time::Instant::now() + ENCODE_WAIT;
        let mut moved = false;
        while std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
            for _ in 0..SETTLE_ROUNDS {
                yield_now().await;
            }
            if spy.started() != last {
                moved = true;
                break;
            }
        }
        if moved {
            quiet = 0;
            last = spy.started();
        } else {
            quiet += 1;
            if quiet >= quiet_steps {
                return elapsed;
            }
        }
        assert!(elapsed < max_ms, "the part write never blocked");
    }
}
