//! Shared hermetic test doubles for the presence auto-unlock contract tests (GitHub #323).
//!
//! Nothing here touches D-Bus, a real camera, `/run`, `/etc` or `/var/lib/soos`:
//! - [`MockPresenceLogind`]: scripted logind snapshots, programmable errors, never-resolving
//!   calls and an `unlock_session` call counter (spec §8 hook);
//! - [`TestDisplay`]: a settable `DisplayState` (the spec's `StaticDisplayProbe`);
//! - [`StaticAccountGuard`] / [`ScriptedAccountGuard`]: account guard doubles (spec §8, R3);
//! - [`SpyCamera`]: the mock camera with `notify_activity` / capture counters whose frames are
//!   re-stamped from the test clock so that a shifted test clock keeps them fresh;
//! - [`build_pipeline`]: `PipelineComponents` over the mock camera and mock vision backends;
//! - [`test_clock!`]: a per-test monotonic clock (`fn` pointer) shifted by a static offset.

#![allow(
    dead_code,
    reason = "Shared test fixtures library used conditionally across test modules"
)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::RwLock;
use tracing_subscriber::fmt::MakeWriter;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, Frame, MockCameraManager, PixelFormat};
use soos_daemon::inference::InferenceGate;
use soos_daemon::pipeline::{PipelineComponents, EMBEDDING_MODEL_ID};
use soos_daemon::presence::account::{AccountGuard, AccountState, UserName};
use soos_daemon::presence::config::PresenceConfig;
use soos_daemon::presence::display::{DisplayProbe, DisplayState};
use soos_daemon::presence::logind::{
    LogindSessionState, PresenceLogind, PresenceLogindError, SessionId,
};
use soos_daemon::presence::switch::PresenceSwitch;
use soos_daemon::presence::worker::PresenceWorker;
use soos_daemon::session_policy::SessionRecord;
use soos_daemon::DaemonError;
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{
    BiometricEmbedding, EmbeddingExtractor, InferenceError, MockEmbeddingExtractor,
    MockFaceDetector, MockPadDetector, EMBEDDING_DIMENSION,
};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

pub const MS_NS: u64 = 1_000_000;
pub const SECOND_NS: u64 = 1_000_000_000;

/// Defines a per-test monotonic clock: `$clock()` is `CLOCK_MONOTONIC + $offset`, so a test
/// shifts time forward deterministically (`$offset.fetch_add(..)`) while the clock keeps
/// advancing with real time (deadlines and frame freshness stay meaningful).
#[macro_export]
macro_rules! test_clock {
    ($offset:ident, $clock:ident) => {
        static $offset: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        fn $clock() -> Result<u64, soos_daemon::DaemonError> {
            soos_daemon::pipeline::current_monotonic_nanos()
                .map(|now| now.saturating_add($offset.load(std::sync::atomic::Ordering::SeqCst)))
        }
    };
}

/// A clock that always fails (fault injection).
pub fn failing_clock() -> Result<u64, DaemonError> {
    Err(DaemonError::Clock("injected clock failure".into()))
}

// ---------------------------------------------------------------------------------------
// Log capture
// ---------------------------------------------------------------------------------------

/// In-memory log sink shared by a test subscriber.
#[derive(Clone, Default)]
pub struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Installs a thread-local DEBUG subscriber writing into a buffer (use with a
/// `current_thread` runtime so every event of the awaited worker is captured).
pub fn capture_logs() -> (LogBuffer, tracing::subscriber::DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (buffer, guard)
}

// ---------------------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------------------

pub fn sid(raw: &str) -> SessionId {
    SessionId::parse(raw).expect("valid test session id")
}

pub fn user(raw: &str) -> UserName {
    UserName::parse(raw).expect("valid test user name")
}

/// A session that passes `check_local_seat_session_of(uid)`.
pub fn bound_record(uid: u32) -> SessionRecord {
    SessionRecord {
        uid: Some(uid),
        active: true,
        remote: Some(false),
        seat: Some("seat0".into()),
        class: Some("user".into()),
    }
}

/// A bound session of `uid` owned by `name`, locked or not.
pub fn session(id: &str, uid: u32, name: &str, locked: bool) -> LogindSessionState {
    LogindSessionState {
        id: sid(id),
        record: bound_record(uid),
        locked,
        user_name: Some(user(name)),
    }
}

/// A bound, locked session of `uid` owned by `name`.
pub fn locked_session(id: &str, uid: u32, name: &str) -> LogindSessionState {
    session(id, uid, name, true)
}

// ---------------------------------------------------------------------------------------
// MockPresenceLogind
// ---------------------------------------------------------------------------------------

type Hook = Box<dyn Fn() + Send + Sync>;

/// Shared, scriptable state behind [`MockPresenceLogind`].
pub struct LogindScript {
    /// Snapshot returned by `seat_sessions` when no one-shot result is queued.
    pub sessions: Mutex<Result<Vec<LogindSessionState>, PresenceLogindError>>,
    /// One-shot `seat_sessions` results (consumed first).
    pub seat_queue: Mutex<VecDeque<Result<Vec<LogindSessionState>, PresenceLogindError>>>,
    /// One-shot `session_state` results (consumed first; otherwise looked up in `sessions`).
    pub state_queue: Mutex<VecDeque<Result<Option<LogindSessionState>, PresenceLogindError>>>,
    pub lid: Mutex<Result<bool, PresenceLogindError>>,
    pub unlock_result: Mutex<Result<(), PresenceLogindError>>,
    pub hang_seat: AtomicBool,
    pub hang_state: AtomicBool,
    pub hang_lid: AtomicBool,
    pub hang_unlock: AtomicBool,
    pub seat_calls: AtomicUsize,
    pub state_calls: AtomicUsize,
    pub lid_calls: AtomicUsize,
    pub unlock_calls: AtomicUsize,
    pub unlocked_ids: Mutex<Vec<SessionId>>,
    /// Session IDs requested by every `session_state` call, in order (T3).
    pub state_requests: Mutex<Vec<SessionId>>,
    /// Runs at the start of every `session_state` call (e.g. to shift the test clock).
    pub on_session_state: Mutex<Option<Hook>>,
    /// Runs at the start of every `unlock_session` call.
    pub on_unlock: Mutex<Option<Hook>>,
}

impl Default for LogindScript {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(Ok(Vec::new())),
            seat_queue: Mutex::new(VecDeque::new()),
            state_queue: Mutex::new(VecDeque::new()),
            lid: Mutex::new(Ok(false)),
            unlock_result: Mutex::new(Ok(())),
            hang_seat: AtomicBool::new(false),
            hang_state: AtomicBool::new(false),
            hang_lid: AtomicBool::new(false),
            hang_unlock: AtomicBool::new(false),
            seat_calls: AtomicUsize::new(0),
            state_calls: AtomicUsize::new(0),
            lid_calls: AtomicUsize::new(0),
            unlock_calls: AtomicUsize::new(0),
            unlocked_ids: Mutex::new(Vec::new()),
            state_requests: Mutex::new(Vec::new()),
            on_session_state: Mutex::new(None),
            on_unlock: Mutex::new(None),
        }
    }
}

/// Cloneable handle: the worker owns one clone, the test inspects another.
#[derive(Clone, Default)]
pub struct MockPresenceLogind(pub Arc<LogindScript>);

impl MockPresenceLogind {
    pub fn set_sessions(&self, sessions: Vec<LogindSessionState>) {
        *self.0.sessions.lock().unwrap() = Ok(sessions);
    }
    pub fn set_sessions_error(&self, err: PresenceLogindError) {
        *self.0.sessions.lock().unwrap() = Err(err);
    }
    pub fn queue_session_state(
        &self,
        result: Result<Option<LogindSessionState>, PresenceLogindError>,
    ) {
        self.0.state_queue.lock().unwrap().push_back(result);
    }
    pub fn set_lid(&self, result: Result<bool, PresenceLogindError>) {
        *self.0.lid.lock().unwrap() = result;
    }
    pub fn set_unlock_result(&self, result: Result<(), PresenceLogindError>) {
        *self.0.unlock_result.lock().unwrap() = result;
    }
    pub fn on_session_state(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self.0.on_session_state.lock().unwrap() = Some(Box::new(hook));
    }
    pub fn on_unlock(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self.0.on_unlock.lock().unwrap() = Some(Box::new(hook));
    }
    pub fn seat_calls(&self) -> usize {
        self.0.seat_calls.load(Ordering::SeqCst)
    }
    pub fn state_calls(&self) -> usize {
        self.0.state_calls.load(Ordering::SeqCst)
    }
    pub fn lid_calls(&self) -> usize {
        self.0.lid_calls.load(Ordering::SeqCst)
    }
    pub fn unlock_calls(&self) -> usize {
        self.0.unlock_calls.load(Ordering::SeqCst)
    }
    pub fn unlocked_ids(&self) -> Vec<SessionId> {
        self.0.unlocked_ids.lock().unwrap().clone()
    }
    pub fn state_requests(&self) -> Vec<SessionId> {
        self.0.state_requests.lock().unwrap().clone()
    }

    fn next_seat_sessions(&self) -> Result<Vec<LogindSessionState>, PresenceLogindError> {
        if let Some(queued) = self.0.seat_queue.lock().unwrap().pop_front() {
            return queued;
        }
        self.0.sessions.lock().unwrap().clone()
    }

    fn next_session_state(
        &self,
        id: &SessionId,
    ) -> Result<Option<LogindSessionState>, PresenceLogindError> {
        if let Some(queued) = self.0.state_queue.lock().unwrap().pop_front() {
            return queued;
        }
        match &*self.0.sessions.lock().unwrap() {
            Ok(sessions) => Ok(sessions.iter().find(|s| &s.id == id).cloned()),
            Err(err) => Err(err.clone()),
        }
    }

    fn run_hook(slot: &Mutex<Option<Hook>>) {
        if let Some(hook) = slot.lock().unwrap().as_ref() {
            hook();
        }
    }
}

impl PresenceLogind for MockPresenceLogind {
    async fn seat_sessions(&self) -> Result<Vec<LogindSessionState>, PresenceLogindError> {
        self.0.seat_calls.fetch_add(1, Ordering::SeqCst);
        let result = self.next_seat_sessions();
        if self.0.hang_seat.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        result
    }

    async fn session_state(
        &self,
        id: &SessionId,
    ) -> Result<Option<LogindSessionState>, PresenceLogindError> {
        self.0.state_calls.fetch_add(1, Ordering::SeqCst);
        self.0.state_requests.lock().unwrap().push(id.clone());
        Self::run_hook(&self.0.on_session_state);
        let result = self.next_session_state(id);
        if self.0.hang_state.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        result
    }

    async fn lid_closed(&self) -> Result<bool, PresenceLogindError> {
        self.0.lid_calls.fetch_add(1, Ordering::SeqCst);
        let result = self.0.lid.lock().unwrap().clone();
        if self.0.hang_lid.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        result
    }

    async fn unlock_session(&self, id: &SessionId) -> Result<(), PresenceLogindError> {
        self.0.unlock_calls.fetch_add(1, Ordering::SeqCst);
        Self::run_hook(&self.0.on_unlock);
        self.0.unlocked_ids.lock().unwrap().push(id.clone());
        let result = self.0.unlock_result.lock().unwrap().clone();
        if self.0.hang_unlock.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        result
    }
}

/// Every `PresenceLogindError` variant (fault injection loops).
pub fn all_logind_errors() -> Vec<PresenceLogindError> {
    vec![
        PresenceLogindError::BusUnavailable,
        PresenceLogindError::Timeout,
        PresenceLogindError::Call("org.freedesktop.DBus.Error.AccessDenied".into()),
        PresenceLogindError::Malformed,
        PresenceLogindError::TooManySessions,
    ]
}

// ---------------------------------------------------------------------------------------
// Display probe
// ---------------------------------------------------------------------------------------

/// Settable display state (the spec's `StaticDisplayProbe`).
#[derive(Clone)]
pub struct TestDisplay(pub Arc<Mutex<DisplayState>>);

impl TestDisplay {
    pub fn new(state: DisplayState) -> Self {
        Self(Arc::new(Mutex::new(state)))
    }
    pub fn set(&self, state: DisplayState) {
        *self.0.lock().unwrap() = state;
    }
}

impl DisplayProbe for TestDisplay {
    fn display_state(&self) -> DisplayState {
        *self.0.lock().unwrap()
    }
}

// ---------------------------------------------------------------------------------------
// Account guards
// ---------------------------------------------------------------------------------------

/// Always answers the same state.
pub struct StaticAccountGuard(pub AccountState);

impl AccountGuard for StaticAccountGuard {
    fn check(&self, _user: &UserName, _uid: u32) -> AccountState {
        self.0
    }
}

/// One scripted answer: optional blocking delay, then the state.
#[derive(Clone, Copy)]
pub struct ScriptedAnswer {
    pub delay: Duration,
    pub state: AccountState,
}

pub struct GuardScript {
    pub answers: Mutex<VecDeque<ScriptedAnswer>>,
    /// Answer once the script is exhausted.
    pub fallback: Mutex<ScriptedAnswer>,
    pub calls: AtomicUsize,
    pub checked: Mutex<Vec<(String, u32)>>,
}

/// Per-call scripted guard with a call counter (cloneable handle).
#[derive(Clone)]
pub struct ScriptedAccountGuard(pub Arc<GuardScript>);

impl ScriptedAccountGuard {
    pub fn always(state: AccountState) -> Self {
        Self::script(Vec::new(), state)
    }
    pub fn script(answers: Vec<ScriptedAnswer>, fallback: AccountState) -> Self {
        Self(Arc::new(GuardScript {
            answers: Mutex::new(answers.into_iter().collect()),
            fallback: Mutex::new(ScriptedAnswer {
                delay: Duration::ZERO,
                state: fallback,
            }),
            calls: AtomicUsize::new(0),
            checked: Mutex::new(Vec::new()),
        }))
    }
    pub fn calls(&self) -> usize {
        self.0.calls.load(Ordering::SeqCst)
    }
    pub fn checked(&self) -> Vec<(String, u32)> {
        self.0.checked.lock().unwrap().clone()
    }
}

impl AccountGuard for ScriptedAccountGuard {
    fn check(&self, user: &UserName, uid: u32) -> AccountState {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0
            .checked
            .lock()
            .unwrap()
            .push((user.as_str().to_string(), uid));
        let answer = self
            .0
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| *self.0.fallback.lock().unwrap());
        if !answer.delay.is_zero() {
            std::thread::sleep(answer.delay);
        }
        answer.state
    }
}

// ---------------------------------------------------------------------------------------
// Camera and vision
// ---------------------------------------------------------------------------------------

/// Spy camera over the mock camera: counts wakes and capture reads, can be held not ready,
/// and re-stamps every frame with the test clock.
pub struct SpyCamera {
    pub inner: Arc<MockCameraManager>,
    pub clock: fn() -> Result<u64, DaemonError>,
    pub never_ready: AtomicBool,
    pub wakes: AtomicUsize,
    pub captures: AtomicUsize,
    /// Runs inside every `notify_activity` (e.g. to observe the inference gate).
    pub on_wake: Mutex<Option<Hook>>,
    /// Stream start stamp in the test-clock domain returned by `stream_started_mono_ns`
    /// (0 = `None`, the trait default; GitHub #331).
    pub stream_started_ns: AtomicU64,
}

impl SpyCamera {
    pub fn wakes(&self) -> usize {
        self.wakes.load(Ordering::SeqCst)
    }
    pub fn captures(&self) -> usize {
        self.captures.load(Ordering::SeqCst)
    }
}

impl CameraManager for SpyCamera {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        self.captures.fetch_add(1, Ordering::SeqCst);
        if self.never_ready.load(Ordering::SeqCst) {
            return None;
        }
        let frame = self.inner.latest_frame()?;
        let mut restamped = (*frame).clone();
        restamped.timestamp_mono_ns = (self.clock)().unwrap_or(frame.timestamp_mono_ns);
        Some(Arc::new(restamped))
    }

    fn is_ready(&self) -> bool {
        !self.never_ready.load(Ordering::SeqCst) && self.inner.is_ready()
    }

    fn notify_activity(&self) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.on_wake.lock().unwrap().as_ref() {
            hook();
        }
        self.inner.notify_activity();
    }

    fn stop(&self) {
        self.inner.stop();
    }

    fn stream_started_mono_ns(&self) -> Option<u64> {
        match self.stream_started_ns.load(Ordering::SeqCst) {
            0 => None,
            ns => Some(ns),
        }
    }
}

/// Mock extractor of the shipped dimension that counts its inferences and runs an optional
/// hook on each one (e.g. register an interactive demand, or panic).
pub struct CountingExtractor {
    pub inner: MockEmbeddingExtractor,
    pub calls: Arc<AtomicUsize>,
    pub hook: Arc<Mutex<Option<Hook>>>,
}

impl EmbeddingExtractor for CountingExtractor {
    fn extract_embedding(
        &self,
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<BiometricEmbedding, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.hook.lock().unwrap().as_ref() {
            hook();
        }
        self.inner
            .extract_embedding(aligned_crop_rgb, width, height)
    }

    fn output_dimension(&self) -> Option<usize> {
        self.inner.output_dimension()
    }
}

/// What the store holds for one UID.
#[derive(Clone, Copy)]
pub enum Enrollment {
    /// The embedding of the mock camera frame (cosine similarity 1.0).
    LiveIdentity,
    /// A current-model template whose cosine similarity with the mock frame is exactly this.
    Similarity(f32),
    /// A retired ArcFace template (`arcface_w600k_mbf`, 512-D): `Foreign`.
    RetiredArcFace,
    /// A current template whose encrypted file is overwritten with garbage (store error).
    Corrupt,
}

pub struct PipelineOptions {
    pub clock: fn() -> Result<u64, DaemonError>,
    pub max_attempts: u32,
    pub thresholds: ThresholdConfig,
    pub enrolled: Vec<(u32, Enrollment)>,
}

impl PipelineOptions {
    pub fn new(clock: fn() -> Result<u64, DaemonError>) -> Self {
        Self {
            clock,
            max_attempts: 40,
            thresholds: ThresholdConfig::default(),
            enrolled: vec![(1000, Enrollment::LiveIdentity)],
        }
    }
}

pub struct PipelineParts {
    pub components: PipelineComponents,
    pub camera: Arc<SpyCamera>,
    pub policy: Arc<RwLock<AuthorizationEngine>>,
    pub vision: Arc<VisionPipeline>,
    pub detector: Arc<MockFaceDetector>,
    pub pad: Arc<MockPadDetector>,
    pub inferences: Arc<AtomicUsize>,
    pub extractor_hook: Arc<Mutex<Option<Hook>>>,
    pub store: Arc<BiometricStore>,
    pub frame_embedding: Vec<f32>,
    pub clock: fn() -> Result<u64, DaemonError>,
    pub temp: tempfile::TempDir,
}

impl Drop for PipelineParts {
    fn drop(&mut self) {
        self.camera.stop();
    }
}

impl PipelineParts {
    pub fn remaining_attempts(&self, uid: u32) -> u32 {
        let engine = self.policy.try_read().expect("policy lock free");
        let limiter = engine.rate_limiter().expect("rate limiter configured");
        limiter.remaining_attempts(uid, (self.clock)().expect("clock"))
    }

    pub fn tracked_uids(&self) -> usize {
        let engine = self.policy.try_read().expect("policy lock free");
        engine
            .rate_limiter()
            .expect("rate limiter configured")
            .tracked_uids()
    }

    pub fn inferences(&self) -> usize {
        self.inferences.load(Ordering::SeqCst)
    }

    pub fn set_extractor_hook(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self.extractor_hook.lock().unwrap() = Some(Box::new(hook));
    }

    pub fn enroll(&self, uid: u32, enrollment: Enrollment) {
        enroll(&self.store, &self.frame_embedding, uid, enrollment);
    }
}

fn unit_orthogonal_to(e: &[f32]) -> Vec<f32> {
    // Gram-Schmidt on the basis vector where `e` is smallest.
    let (j, _) = e
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap();
    let mut u = vec![0.0f32; e.len()];
    u[j] = 1.0;
    let dot: f32 = e[j];
    for (ui, ei) in u.iter_mut().zip(e) {
        *ui -= dot * ei;
    }
    let norm = u.iter().map(|x| x * x).sum::<f32>().sqrt();
    u.iter().map(|x| x / norm).collect()
}

fn normalized(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / norm).collect()
}

/// A unit vector whose cosine similarity with `frame_embedding` is exactly `s`.
pub fn template_with_similarity(frame_embedding: &[f32], s: f32) -> Vec<f32> {
    let e = normalized(frame_embedding);
    let u = unit_orthogonal_to(&e);
    let t = (1.0 - s * s).max(0.0).sqrt();
    e.iter().zip(&u).map(|(a, b)| s * a + t * b).collect()
}

fn enroll(store: &BiometricStore, frame_embedding: &[f32], uid: u32, enrollment: Enrollment) {
    let unit_vector = |len: usize| {
        let mut v = vec![0.0f32; len];
        v[0] = 1.0;
        v
    };
    let (model_id, vector) = match enrollment {
        Enrollment::LiveIdentity | Enrollment::Corrupt => {
            (EMBEDDING_MODEL_ID, frame_embedding.to_vec())
        }
        Enrollment::Similarity(s) => (
            EMBEDDING_MODEL_ID,
            template_with_similarity(frame_embedding, s),
        ),
        Enrollment::RetiredArcFace => ("arcface_w600k_mbf", unit_vector(512)),
    };
    let template = BiometricTemplate::new(
        uid,
        model_id.into(),
        "2.0.0".into(),
        1,
        zeroize::Zeroizing::new(vector),
    )
    .unwrap();
    store.enroll(&template).unwrap();
    if matches!(enrollment, Enrollment::Corrupt) {
        let path = store.template_path(uid).unwrap();
        let len = std::fs::metadata(&path).unwrap().len();
        std::fs::write(&path, vec![0x5A_u8; usize::try_from(len).unwrap()]).unwrap();
    }
}

/// Builds `PipelineComponents` over the mock camera (320x240 RGB24, 30 fps) and the mock
/// vision backends (centered face, live PAD, counting extractor).
pub async fn build_pipeline(options: PipelineOptions) -> PipelineParts {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        BiometricStore::new(
            temp.path().join("biometrics"),
            BioMasterKey::generate().unwrap(),
        )
        .unwrap(),
    );
    let ev_dir = temp.path().join("evidence");
    std::fs::create_dir_all(&ev_dir).unwrap();
    let evidence_store = Arc::new(EvidenceStore::new(
        EvidenceConfig {
            enabled: false,
            base_dir: ev_dir,
            retention_days: 7,
            daily_cap_per_uid: 3,
            key_path: temp.path().join("evidence.key"),
        },
        EvMasterKey::generate().unwrap(),
    ));

    let mock = Arc::new(MockCameraManager::new(
        CameraConfigBuilder::new()
            .device_path("/dev/null")
            .resolution(320, 240)
            .fps(30)
            .idle_timeout(Duration::from_secs(60))
            .format(PixelFormat::Rgb24)
            .warmup_frames(0)
            .build(),
    ));
    let mut frame = None;
    for _ in 0..200 {
        if let Some(f) = mock.latest_frame() {
            frame = Some(f);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let frame = frame.expect("mock camera frame");

    let inferences = Arc::new(AtomicUsize::new(0));
    let extractor_hook: Arc<Mutex<Option<Hook>>> = Arc::new(Mutex::new(None));
    let detector = Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95));
    let pad = Arc::new(MockPadDetector::new_live());
    let vision = Arc::new(VisionPipeline::new(
        detector.clone(),
        pad.clone(),
        Arc::new(CountingExtractor {
            inner: MockEmbeddingExtractor::new(EMBEDDING_DIMENSION),
            calls: Arc::clone(&inferences),
            hook: Arc::clone(&extractor_hook),
        }),
        VisionPipelineConfig {
            min_face_confidence: 0.70,
            match_threshold: 0.45,
            pad_threshold: 0.80,
            target_width: 112,
            target_height: 112,
            ..Default::default()
        },
    ));
    let frame_embedding = vision
        .process_frame(&frame)
        .unwrap()
        .embedding
        .as_slice()
        .to_vec();
    for (uid, enrollment) in &options.enrolled {
        enroll(&store, &frame_embedding, *uid, *enrollment);
    }
    inferences.store(0, Ordering::SeqCst);

    let policy = Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
        options.thresholds,
        RateLimiter::new(RateLimitConfig::new(options.max_attempts, 60 * SECOND_NS)),
    )));
    let camera = Arc::new(SpyCamera {
        inner: mock,
        clock: options.clock,
        never_ready: AtomicBool::new(false),
        wakes: AtomicUsize::new(0),
        captures: AtomicUsize::new(0),
        on_wake: Mutex::new(None),
        stream_started_ns: AtomicU64::new(0),
    });
    let components = PipelineComponents::new(
        camera.clone(),
        vision.clone(),
        store.clone(),
        evidence_store,
        Arc::clone(&policy),
    );
    PipelineParts {
        components,
        camera,
        policy,
        vision,
        detector,
        pad,
        inferences,
        extractor_hook,
        store,
        frame_embedding,
        clock: options.clock,
        temp,
    }
}

// ---------------------------------------------------------------------------------------
// Presence worker fixture
// ---------------------------------------------------------------------------------------

/// Presence configuration used by the worker tests: 1 s grace, 1 s scan interval (minimums).
pub fn fast_presence_config() -> PresenceConfig {
    PresenceConfig {
        enabled: true,
        scan_interval: Duration::from_millis(1000),
        lock_grace: Duration::from_millis(1000),
    }
}

pub struct WorkerFixture<A: AccountGuard> {
    pub worker: PresenceWorker<MockPresenceLogind, TestDisplay, A>,
    pub logind: MockPresenceLogind,
    pub display: TestDisplay,
    pub gate: InferenceGate,
    pub switch_dir: PathBuf,
    pub parts: PipelineParts,
}

/// Builds a presence worker over the mocks. The kill-switch directory is an empty
/// sub-directory of the pipeline tempdir; the inference gate is shared with the test.
pub async fn build_worker<A: AccountGuard>(
    options: PipelineOptions,
    config: PresenceConfig,
    guard: A,
) -> WorkerFixture<A> {
    let clock = options.clock;
    let parts = build_pipeline(options).await;
    let switch_dir = parts.temp.path().join("etc-soos");
    std::fs::create_dir_all(&switch_dir).unwrap();
    let logind = MockPresenceLogind::default();
    let display = TestDisplay::new(DisplayState::On);
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    let worker = PresenceWorker::new(
        config,
        logind.clone(),
        display.clone(),
        PresenceSwitch::new(switch_dir.clone()),
        guard,
        parts.components.clone(),
        gate.clone(),
    )
    .with_expected_embedding_model(EMBEDDING_MODEL_ID)
    .with_clock_fn(clock);
    WorkerFixture {
        worker,
        logind,
        display,
        gate,
        switch_dir,
        parts,
    }
}
