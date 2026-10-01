//! Contract tests for spoof evidence capture and final-reason priority
//! (review findings PAD-14 — GitHub #261, PAD-16 — GitHub #262).
//!
//! - #261: a request vetoed by a presentation attack stores exactly one encrypted
//!   `PadFailed` evidence snapshot of the spoof capture, only when `[pipeline.evidence]`
//!   is enabled (opt-in), and within `daily_cap_per_uid`.
//! - #262: once a spoof has been detected, the final reason stays `PadFailed` even when
//!   the following captures contain no face.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract test suite uses direct assertions, unwrap and indexing"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use soos_biometric_store::{BiometricStore, BiometricTemplate, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager, PixelFormat};
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::{ConnectionDispatcher, PAD_FAILED_EVIDENCE_REASON};
use soos_daemon::health::HealthState;
use soos_daemon::pipeline::PipelineComponents;
use soos_evidence_store::{
    EvidenceConfig, EvidenceRecord, EvidenceStore, MasterKey as EvMasterKey,
};
use soos_inference_ort::{
    AttackType, MockEmbeddingExtractor, MockFaceDetector, MockPadDetector, PadResult,
};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

/// How long a positive test polls for the asynchronous evidence write.
const EVIDENCE_POLL_BUDGET: Duration = Duration::from_secs(5);

/// How long a negative test waits before asserting that nothing was written.
const EVIDENCE_QUIET_PERIOD: Duration = Duration::from_millis(600);

struct Fixture {
    dispatcher: Arc<ConnectionDispatcher>,
    evidence_store: Arc<EvidenceStore>,
    camera: Arc<MockCameraManager>,
    detector: Arc<MockFaceDetector>,
    pad: Arc<MockPadDetector>,
    sock_path: PathBuf,
    current_uid: u32,
    _temp_dir: tempfile::TempDir,
}

impl Fixture {
    async fn new(evidence_enabled: bool, daily_cap_per_uid: u32) -> Self {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        let temp_dir = tempdir().expect("tempdir");
        let sock_path = temp_dir.path().join("pad_evidence.sock");
        let bio_dir = temp_dir.path().join("biometrics");
        let ev_dir = temp_dir.path().join("evidence");
        std::fs::create_dir_all(&ev_dir).expect("create evidence dir");

        let bio_store = Arc::new(
            BiometricStore::new(&bio_dir, BioMasterKey::generate().expect("bio key"))
                .expect("biometric store"),
        );
        let evidence_store = Arc::new(EvidenceStore::new(
            EvidenceConfig {
                enabled: evidence_enabled,
                base_dir: ev_dir,
                retention_days: 7,
                daily_cap_per_uid,
                key_path: temp_dir.path().join("evidence.key"),
            },
            EvMasterKey::generate().expect("evidence key"),
        ));

        let camera_config = CameraConfigBuilder::new()
            .device_path("/dev/null")
            .resolution(320, 240)
            .fps(30)
            .idle_timeout(Duration::from_secs(60))
            .format(PixelFormat::Rgb24)
            .warmup_frames(0)
            .build();
        let camera = Arc::new(MockCameraManager::new(camera_config));
        let mut first = None;
        for _ in 0..100 {
            if let Some(frame) = camera.latest_frame() {
                first = Some(frame);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let frame = first.expect("mock camera frame");

        let detector = Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95));
        let pad = Arc::new(MockPadDetector::new_live());
        let extractor = Arc::new(MockEmbeddingExtractor::new(512));
        let vision = Arc::new(VisionPipeline::new(
            detector.clone(),
            pad.clone(),
            extractor,
            VisionPipelineConfig {
                min_face_confidence: 0.70,
                match_threshold: 0.45,
                pad_threshold: 0.80,
                target_width: 112,
                target_height: 112,
                ..Default::default()
            },
        ));
        let enrolled = vision
            .process_frame(&frame)
            .expect("process frame")
            .embedding
            .as_slice()
            .to_vec();

        let current_uid = nix::unistd::getuid().as_raw();
        let template = BiometricTemplate::new(
            current_uid,
            "mock-model".into(),
            "1.0".into(),
            1,
            zeroize::Zeroizing::new(enrolled),
        )
        .expect("template");
        bio_store.enroll(&template).expect("enroll");

        let policy = Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
            ThresholdConfig::default(),
            RateLimiter::new(RateLimitConfig::new(10, 60_000_000_000)),
        )));
        let components = PipelineComponents {
            camera: camera.clone(),
            vision,
            biometric_store: bio_store,
            evidence_store: evidence_store.clone(),
            policy,
        };

        let health = Arc::new(HealthState::new());
        health.set_socket_ready(true);
        health.set_camera_ready(camera.is_ready());
        health.set_models_verified(true);

        let dispatcher = Arc::new(ConnectionDispatcher::with_pipeline(
            DispatcherConfig {
                max_concurrent_connections: 8,
                connection_timeout: Duration::from_secs(5),
                enforce_active_session: false,
                logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
            },
            health,
            components,
        ));

        Self {
            dispatcher,
            evidence_store,
            camera,
            detector,
            pad,
            sock_path,
            current_uid,
            _temp_dir: temp_dir,
        }
    }

    /// Serves `connections` sequential connections on the fixture socket.
    fn serve(&self, connections: usize) {
        let listener = UnixListener::bind(&self.sock_path).expect("bind");
        let dispatcher = self.dispatcher.clone();
        tokio::spawn(async move {
            for _ in 0..connections {
                if let Ok((stream, _)) = listener.accept().await {
                    let _ = dispatcher.handle_connection(stream).await;
                }
            }
        });
    }

    async fn auth(&self, tag: u8) -> Response {
        let req = Request {
            version: CURRENT_VERSION,
            kind: RequestKind::Auth,
            request_id: [tag; 32],
            uid_hint: self.current_uid,
            service: "sudo".into(),
            deadline_monotonic_ns: u64::MAX,
        };
        let mut client = UnixStream::connect(&self.sock_path).await.expect("connect");
        client
            .write_all(&encode(&req).expect("encode"))
            .await
            .expect("write");
        client.flush().await.expect("flush");
        let mut len_bytes = [0u8; 4];
        client.read_exact(&mut len_bytes).await.expect("read len");
        let len = usize::try_from(u32::from_be_bytes(len_bytes)).expect("len");
        let mut buf = vec![0u8; 4 + len];
        buf[..4].copy_from_slice(&len_bytes);
        client.read_exact(&mut buf[4..]).await.expect("read body");
        decode::<Response>(&buf).expect("decode")
    }

    /// Decrypts every snapshot stored under the evidence base directory.
    fn records(&self) -> Vec<EvidenceRecord> {
        let base = self.evidence_store.config().base_dir.clone();
        let mut records = Vec::new();
        let Ok(entries) = std::fs::read_dir(&base) else {
            return records;
        };
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let date = entry.file_name().to_string_lossy().to_string();
                for file in self
                    .evidence_store
                    .list_snapshots_for_date(&date)
                    .expect("list snapshots")
                {
                    records.push(self.evidence_store.load_snapshot(&file).expect("load"));
                }
            }
        }
        records
    }

    /// Polls until at least `count` snapshots exist or the budget elapses.
    async fn wait_for_records(&self, count: usize) -> Vec<EvidenceRecord> {
        let started = Instant::now();
        loop {
            let records = self.records();
            if records.len() >= count || started.elapsed() >= EVIDENCE_POLL_BUDGET {
                return records;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.camera.stop();
    }
}

fn dir_entry_count(path: &Path) -> usize {
    std::fs::read_dir(path).map_or(0, Iterator::count)
}

// ---------------------------------------------------------------------------
// GitHub #261 (PAD-14): spoof attempts are captured as opt-in evidence
// ---------------------------------------------------------------------------

#[test]
fn test_261_pad_failed_evidence_reason_is_stable() {
    assert_eq!(PAD_FAILED_EVIDENCE_REASON, "PadFailed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_261_spoof_veto_stores_one_pad_failed_evidence_snapshot() {
    let fixture = Fixture::new(true, 3).await;
    fixture
        .pad
        .set_result(PadResult::spoof(0.04, AttackType::PrintPhoto));
    fixture.serve(1);

    let resp = fixture.auth(61).await;
    assert_eq!(resp.verdict, Verdict::Deny);
    assert_eq!(resp.reason_class, ReasonClass::PadFailed);

    let records = fixture.wait_for_records(1).await;
    // Leave time for any duplicate write before asserting the exact count.
    tokio::time::sleep(EVIDENCE_QUIET_PERIOD).await;
    let records = if records.len() == 1 {
        fixture.records()
    } else {
        records
    };
    assert_eq!(
        records.len(),
        1,
        "a spoof-vetoed request must store exactly one evidence snapshot"
    );
    let record = &records[0];
    assert_eq!(record.reason, PAD_FAILED_EVIDENCE_REASON);
    assert_eq!(record.uid, fixture.current_uid);
    let meta = record
        .frame
        .as_ref()
        .expect("spoof evidence must be a self-describing frame snapshot");
    assert_eq!((meta.width, meta.height), (320, 240));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_261_spoof_veto_without_opt_in_stores_no_evidence() {
    let fixture = Fixture::new(false, 3).await;
    fixture
        .pad
        .set_result(PadResult::spoof(0.04, AttackType::ScreenReplay));
    fixture.serve(1);

    let resp = fixture.auth(62).await;
    assert_eq!(resp.verdict, Verdict::Deny);
    assert_eq!(resp.reason_class, ReasonClass::PadFailed);

    tokio::time::sleep(EVIDENCE_QUIET_PERIOD).await;
    assert_eq!(
        dir_entry_count(&fixture.evidence_store.config().base_dir),
        0,
        "evidence capture is opt-in: nothing may be written when disabled"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_261_spoof_evidence_respects_daily_cap_per_uid() {
    let fixture = Fixture::new(true, 1).await;
    fixture
        .pad
        .set_result(PadResult::spoof(0.04, AttackType::PrintPhoto));
    fixture.serve(3);

    for tag in [63u8, 64, 65] {
        let resp = fixture.auth(tag).await;
        assert_eq!(resp.verdict, Verdict::Deny);
        assert_eq!(resp.reason_class, ReasonClass::PadFailed);
    }

    let _ = fixture.wait_for_records(1).await;
    tokio::time::sleep(EVIDENCE_QUIET_PERIOD).await;
    let records = fixture.records();
    assert_eq!(
        records.len(),
        1,
        "daily_cap_per_uid = 1 must bound spoof evidence to one snapshot"
    );
    assert_eq!(records[0].reason, PAD_FAILED_EVIDENCE_REASON);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_261_allowed_request_stores_no_evidence() {
    let fixture = Fixture::new(true, 3).await;
    fixture.serve(1);

    let resp = fixture.auth(66).await;
    assert_eq!(resp.verdict, Verdict::Allow);

    tokio::time::sleep(EVIDENCE_QUIET_PERIOD).await;
    assert!(
        fixture.records().is_empty(),
        "only a presentation attack may produce PAD evidence"
    );
}

// ---------------------------------------------------------------------------
// GitHub #262 (PAD-16): a detected spoof outranks later no-face captures
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_262_spoof_then_no_face_captures_reports_pad_failed() {
    let fixture = Fixture::new(false, 3).await;
    // The first evaluated capture is a spoof, then the attacker withdraws the device:
    // every later capture has no face at all.
    fixture
        .pad
        .set_result(PadResult::spoof(0.04, AttackType::PrintPhoto));
    let detector = fixture.detector.clone();
    let pad = fixture.pad.clone();
    // The fixture already ran one enrollment capture through the PAD mock.
    let baseline = pad.call_count();
    tokio::spawn(async move {
        // Withdraw the face as soon as the first capture reached the PAD model.
        let started = Instant::now();
        while pad.call_count() <= baseline && started.elapsed() < EVIDENCE_POLL_BUDGET {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        detector.set_detections(Vec::new());
    });
    fixture.serve(1);

    let resp = fixture.auth(67).await;
    assert_eq!(resp.verdict, Verdict::Deny);
    assert_eq!(
        resp.reason_class,
        ReasonClass::PadFailed,
        "a detected spoof must stay the final reason after no-face captures"
    );
}
