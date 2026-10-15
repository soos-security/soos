//! Dispatcher contract of `RequestKind::LockState` and of the PAM-side publication of the
//! lock-state snapshot (GitHub #380, PR 1 of #352; ADR 2026-10-13 "Read-Only Lock-Screen State
//! Channel"; matrix LKS7-LKS11).
//!
//! Power rules: "no attempt" is asserted through the shared rate limiter, "no camera" through
//! the `notify_activity` counter, "no inference" through the extractor counter, and the state
//! mapping through the board the dispatcher publishes into.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use common::{build_pipeline, Enrollment, PipelineOptions, PipelineParts};
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::inference::InferenceGate;
use soos_daemon::lock_state::{
    LockStateBoard, LockStateView, ScanEnd, LOCK_STATE_MAX_REQUESTS_PER_SEC,
    NOT_RECOGNIZED_REVEAL_FLOOR_MS,
};
use soos_daemon::pipeline::current_monotonic_nanos;
use soos_daemon::session::SessionValidator;
use soos_inference_ort::{AttackType, BoundingBox, FaceDetection, MockFaceDetector, PadResult};
use soos_protocol::codec::{decode, encode};
use soos_protocol::message::encode_request;
use soos_protocol::types::{
    LockStateResponse, LockWidgetState, ReasonClass, Request, RequestKind, Response, Verdict,
    CURRENT_VERSION,
};

const MS: u64 = 1_000_000;
const FLOOR_NS: u64 = NOT_RECOGNIZED_REVEAL_FLOOR_MS * MS;

#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Lock(LockStateResponse),
    Refused(Verdict, ReasonClass),
}

fn peer_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

fn lock_state_request(uid: u32, nonce: u8) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::LockState,
        request_id: [nonce; 32],
        uid_hint: uid,
        service: "soos-lockstatus".into(),
        deadline_monotonic_ns: u64::MAX,
    }
}

fn auth_request(uid: u32, nonce: u8) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [nonce; 32],
        uid_hint: uid,
        service: "swaylock".into(),
        deadline_monotonic_ns: u64::MAX,
    }
}

fn config(sessions_dir: &Path, enforce: bool) -> DispatcherConfig {
    DispatcherConfig {
        max_concurrent_connections: 8,
        connection_timeout: Duration::from_secs(5),
        enforce_active_session: enforce,
        logind_sessions_dir: sessions_dir.to_path_buf(),
    }
}

/// A local seat session record of `uid` in `dir` (the predicate of the read path).
fn write_local_seat_session(dir: &Path, uid: u32) {
    std::fs::write(
        dir.join("7"),
        format!("UID={uid}\nACTIVE=1\nREMOTE=0\nSEAT=seat0\nCLASS=user\n"),
    )
    .unwrap();
}

struct Harness {
    parts: PipelineParts,
    board: LockStateBoard,
    gate: InferenceGate,
    sock: PathBuf,
    _sock_dir: tempfile::TempDir,
}

/// Dispatcher over the shared mock pipeline (peer UID enrolled with `enrollment`) and a board
/// the test keeps a clone of. `sessions`: `None` = session enforcement disabled (mock
/// harness), `Some(with_session)` = a validator over an empty or populated sessions directory.
async fn harness(enrollment: Enrollment, sessions: Option<bool>) -> Harness {
    let mut options = PipelineOptions::new(current_monotonic_nanos);
    options.enrolled = vec![(peer_uid(), enrollment)];
    let parts = build_pipeline(options).await;
    let board = LockStateBoard::new();
    let gate = InferenceGate::new(1, Duration::from_millis(80));
    let sock_dir = tempfile::tempdir().unwrap();
    let sessions_dir = sock_dir.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let health = Arc::new(HealthState::new());
    let mut dispatcher = ConnectionDispatcher::with_pipeline(
        config(&sessions_dir, false),
        health,
        parts.components.clone(),
    )
    .with_inference_gate(gate.clone())
    .with_lock_state_board(board.clone());
    if let Some(with_session) = sessions {
        if with_session {
            write_local_seat_session(&sessions_dir, peer_uid());
        }
        dispatcher =
            dispatcher.with_session_validator(SessionValidator::with_sessions_dir(sessions_dir));
    }
    let sock = sock_dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let dispatcher = Arc::new(dispatcher);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let disp = Arc::clone(&dispatcher);
            tokio::spawn(async move {
                let _ = disp.handle_connection(stream).await;
            });
        }
    });
    Harness {
        parts,
        board,
        gate,
        sock,
        _sock_dir: sock_dir,
    }
}

async fn read_frame(stream: &mut UnixStream) -> Vec<u8> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await.expect("read length");
    let size = u32::from_be_bytes(len) as usize;
    let mut frame = vec![0u8; 4 + size];
    frame[..4].copy_from_slice(&len);
    stream.read_exact(&mut frame[4..]).await.expect("read body");
    frame
}

fn classify(frame: &[u8]) -> Reply {
    if let Ok(lock) = decode::<LockStateResponse>(frame) {
        assert!(
            decode::<Response>(frame).is_err(),
            "exactly one interpretation"
        );
        return Reply::Lock(lock);
    }
    let resp = decode::<Response>(frame).expect("either a LockStateResponse or a Response");
    Reply::Refused(resp.verdict, resp.reason_class)
}

async fn ask_on(stream: &mut UnixStream, req: &Request) -> Reply {
    stream
        .write_all(&encode_request(req).expect("encode"))
        .await
        .expect("write");
    classify(&read_frame(stream).await)
}

async fn ask(sock: &Path, req: &Request) -> Reply {
    let mut stream = UnixStream::connect(sock).await.expect("connect");
    ask_on(&mut stream, req).await
}

async fn send_auth(sock: &Path, req: &Request) -> Response {
    let mut stream = UnixStream::connect(sock).await.expect("connect");
    stream
        .write_all(&encode(req).expect("encode"))
        .await
        .expect("write");
    decode::<Response>(&read_frame(&mut stream).await).expect("auth response")
}

fn lock(state: LockWidgetState, sequence: u64) -> Reply {
    Reply::Lock(LockStateResponse {
        version: CURRENT_VERSION,
        state,
        sequence,
    })
}

fn now() -> u64 {
    current_monotonic_nanos().unwrap()
}

/// Publishes a visible `NotRecognized` for `uid` (scan begun just over the floor ago).
fn publish_visible_not_recognized(board: &LockStateBoard, uid: u32) {
    let begin = now() - FLOOR_NS - 100 * MS;
    board
        .begin_scan(uid, begin)
        .finish(ScanEnd::NotRecognized, Some(begin + 50 * MS));
    assert_eq!(
        board.view(uid, now()).state,
        LockWidgetState::NotRecognized,
        "precondition: the state is visible"
    );
}

/// Records every board view taken when the camera is woken (mid-scan observation).
fn record_views_on_wake(h: &Harness) -> Arc<Mutex<Vec<LockStateView>>> {
    let seen: Arc<Mutex<Vec<LockStateView>>> = Arc::new(Mutex::new(Vec::new()));
    let board = h.board.clone();
    let sink = Arc::clone(&seen);
    let uid = peer_uid();
    *h.parts.camera.on_wake.lock().unwrap() = Some(Box::new(move || {
        sink.lock().unwrap().push(board.view(uid, now()));
    }));
    seen
}

// ---------------------------------------------------------------------------------------
// Read path: authorization
// ---------------------------------------------------------------------------------------

/// LKS7: a peer asking for its own UID reads `Idle`, sequence 0, before any scan.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_own_uid_reads_idle_before_any_scan() {
    let h = harness(Enrollment::LiveIdentity, None).await;
    assert_eq!(
        ask(&h.sock, &lock_state_request(peer_uid(), 1)).await,
        lock(LockWidgetState::Idle, 0)
    );
}

/// LKS7: an unprivileged peer asking for another UID is refused `ProtocolError`/`UidMismatch`
/// and receives no `LockStateResponse`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_other_uid_is_refused_uid_mismatch() {
    let uid = peer_uid();
    if uid == 0 {
        // Root is the diagnostic exception by design: it may ask for any UID.
        return;
    }
    let h = harness(Enrollment::LiveIdentity, None).await;
    publish_visible_not_recognized(&h.board, uid + 1);
    assert_eq!(
        ask(&h.sock, &lock_state_request(uid + 1, 2)).await,
        Reply::Refused(Verdict::ProtocolError, ReasonClass::UidMismatch)
    );
}

/// LKS8: a UID without a local seat session reads `Idle`/0 even when the board holds a
/// visible state for it; the same UID with a local seat session reads the real state.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_uid_without_local_seat_session_gets_no_data() {
    let without = harness(Enrollment::LiveIdentity, Some(false)).await;
    publish_visible_not_recognized(&without.board, peer_uid());
    assert_eq!(
        ask(&without.sock, &lock_state_request(peer_uid(), 3)).await,
        lock(LockWidgetState::Idle, 0)
    );

    let with = harness(Enrollment::LiveIdentity, Some(true)).await;
    publish_visible_not_recognized(&with.board, peer_uid());
    assert_eq!(
        ask(&with.sock, &lock_state_request(peer_uid(), 4)).await,
        lock(LockWidgetState::NotRecognized, 2)
    );
}

/// LKS8: a malformed `LockState` request (oversize service) is refused like any request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_malformed_request_is_refused() {
    let h = harness(Enrollment::LiveIdentity, None).await;
    let mut req = lock_state_request(peer_uid(), 5);
    req.service = "x".repeat(65);
    assert_eq!(
        ask(&h.sock, &req).await,
        Reply::Refused(Verdict::ProtocolError, ReasonClass::MalformedRequest)
    );
}

// ---------------------------------------------------------------------------------------
// Read path: side-effect freedom and quota
// ---------------------------------------------------------------------------------------

/// LKS9: `LockState` never records an authentication attempt, never wakes the camera, never
/// runs an inference, never registers an interactive demand, and keeps the connection open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_lock_state_has_no_side_effect() {
    let h = harness(Enrollment::LiveIdentity, None).await;
    let mut stream = UnixStream::connect(&h.sock).await.expect("connect");
    for nonce in 0..LOCK_STATE_MAX_REQUESTS_PER_SEC as u8 {
        assert_eq!(
            ask_on(&mut stream, &lock_state_request(peer_uid(), nonce)).await,
            lock(LockWidgetState::Idle, 0),
            "persistent connection, request {nonce}"
        );
        assert_eq!(h.gate.interactive_demand(), 0);
    }
    assert_eq!(h.parts.tracked_uids(), 0, "no attempt recorded");
    assert_eq!(h.parts.remaining_attempts(peer_uid()), 40);
    assert_eq!(h.parts.camera.wakes(), 0, "the camera is never woken");
    assert_eq!(h.parts.inferences(), 0, "no inference");
    assert_eq!(
        h.board.view(peer_uid(), now()),
        LockStateView {
            state: LockWidgetState::Idle,
            sequence: 0,
        }
    );
}

/// LKS9: the per-peer-UID quota refuses the request beyond `LOCK_STATE_MAX_REQUESTS_PER_SEC`
/// within one second with `ProtocolError`/`RateLimited`, still without any attempt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_quota_refuses_excess_requests() {
    let h = harness(Enrollment::LiveIdentity, None).await;
    let mut stream = UnixStream::connect(&h.sock).await.expect("connect");
    let mut refused = 0;
    for nonce in 0..=LOCK_STATE_MAX_REQUESTS_PER_SEC as u8 {
        if ask_on(&mut stream, &lock_state_request(peer_uid(), nonce)).await
            == Reply::Refused(Verdict::ProtocolError, ReasonClass::RateLimited)
        {
            refused += 1;
        }
    }
    assert!(refused >= 1, "the request beyond the quota is refused");
    assert_eq!(
        h.parts.tracked_uids(),
        0,
        "the quota is not the auth limiter"
    );
    assert_eq!(h.parts.camera.wakes(), 0);
}

// ---------------------------------------------------------------------------------------
// PAM publication: state mapping
// ---------------------------------------------------------------------------------------

/// Sends one `Auth` and returns its response, the views taken at camera wake, and the view
/// just after the response and after the reveal floor.
async fn auth_and_observe(
    h: &Harness,
) -> (Response, Vec<LockStateView>, LockStateView, LockStateView) {
    let seen = record_views_on_wake(h);
    let resp = send_auth(&h.sock, &auth_request(peer_uid(), 0x77)).await;
    let after = now();
    let immediate = h.board.view(peer_uid(), after);
    let revealed = h.board.view(peer_uid(), after + FLOOR_NS);
    let seen = seen.lock().unwrap().clone();
    (resp, seen, immediate, revealed)
}

fn assert_not_recognized(
    label: &str,
    observed: (Response, Vec<LockStateView>, LockStateView, LockStateView),
) {
    let (resp, seen, immediate, revealed) = observed;
    assert_ne!(resp.verdict, Verdict::Allow, "{label}: never authorizes");
    assert!(!seen.is_empty(), "{label}: the camera was woken");
    assert!(
        seen.iter().all(|v| *v
            == LockStateView {
                state: LockWidgetState::Scanning,
                sequence: 1
            }),
        "{label}: Scanning while the camera wakes, got {seen:?}"
    );
    assert_eq!(
        immediate,
        LockStateView {
            state: LockWidgetState::Scanning,
            sequence: 1
        },
        "{label}: the end is hidden before the reveal floor"
    );
    assert_eq!(
        revealed,
        LockStateView {
            state: LockWidgetState::NotRecognized,
            sequence: 2
        },
        "{label}: every failure reason collapses into NotRecognized"
    );
}

/// LKS10: a matching face (`Allow`) publishes `Scanning` then, after the floor, `Idle`: no
/// success state, and the end is hidden like every other end.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_pam_allow_publishes_scanning_then_idle() {
    let h = harness(Enrollment::LiveIdentity, None).await;
    let (resp, seen, immediate, revealed) = auth_and_observe(&h).await;
    assert_eq!(resp.verdict, Verdict::Allow);
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|v| v.state == LockWidgetState::Scanning));
    assert_eq!(
        immediate,
        LockStateView {
            state: LockWidgetState::Scanning,
            sequence: 1
        }
    );
    assert_eq!(
        revealed,
        LockStateView {
            state: LockWidgetState::Idle,
            sequence: 2
        }
    );
}

/// LKS10: low similarity (`ScoreBelowThreshold` family) reads `NotRecognized`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_pam_low_score_maps_to_not_recognized() {
    let h = harness(Enrollment::Similarity(0.10), None).await;
    assert_not_recognized("low score", auth_and_observe(&h).await);
}

/// LKS10: a presentation attack (`PadFailed`) reads exactly like a non-match.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_pam_pad_reject_maps_to_not_recognized() {
    let h = harness(Enrollment::LiveIdentity, None).await;
    h.parts
        .pad
        .set_result(PadResult::spoof(0.04, AttackType::PrintPhoto));
    let observed = auth_and_observe(&h).await;
    assert_eq!(
        (observed.0.verdict, observed.0.reason_class),
        (Verdict::Deny, ReasonClass::PadFailed),
        "precondition: the PAD veto"
    );
    assert_not_recognized("pad reject", observed);
}

/// LKS10: no face in the frame reads `NotRecognized`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_pam_no_face_maps_to_not_recognized() {
    let h = harness(Enrollment::LiveIdentity, None).await;
    h.parts.detector.set_detections(Vec::new());
    assert_not_recognized("no face", auth_and_observe(&h).await);
}

/// LKS10: several faces in the frame read `NotRecognized`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_pam_multiple_faces_maps_to_not_recognized() {
    let h = harness(Enrollment::LiveIdentity, None).await;
    let face = |x0: f32| {
        let bbox = BoundingBox::new(x0, 120.0, x0 + 200.0, 360.0);
        let landmarks = MockFaceDetector::canonical_landmarks_for_box(&bbox);
        FaceDetection::with_landmarks(bbox, 0.95, landmarks)
    };
    h.parts
        .detector
        .set_detections(vec![face(40.0), face(380.0)]);
    assert_not_recognized("multiple faces", auth_and_observe(&h).await);
}

/// LKS10: a camera that never becomes ready (`Unavailable`) never reads as a success.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_pam_camera_unavailable_maps_to_not_recognized() {
    let h = harness(Enrollment::LiveIdentity, None).await;
    h.parts.camera.never_ready.store(true, Ordering::SeqCst);
    let observed = auth_and_observe(&h).await;
    assert_eq!(observed.0.verdict, Verdict::Unavailable, "precondition");
    assert_not_recognized("camera unavailable", observed);
}

/// LKS10: a vision failure (`ConsensusRun::Aborted`) never reads as a success.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_pam_aborted_consensus_maps_to_not_recognized() {
    let h = harness(Enrollment::LiveIdentity, None).await;
    h.parts.detector.set_fail_next(true);
    let observed = auth_and_observe(&h).await;
    assert_eq!(
        observed.0.verdict,
        Verdict::Unavailable,
        "precondition: aborted"
    );
    assert_not_recognized("aborted consensus", observed);
}

/// LKS10: a short client deadline with no face (timeout family) reads `NotRecognized`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_pam_short_deadline_maps_to_not_recognized() {
    let h = harness(Enrollment::LiveIdentity, None).await;
    h.parts.detector.set_detections(Vec::new());
    let seen = record_views_on_wake(&h);
    let mut req = auth_request(peer_uid(), 0x78);
    req.deadline_monotonic_ns = now() + 400 * MS;
    let resp = send_auth(&h.sock, &req).await;
    let after = now();
    let observed = (
        resp,
        seen.lock().unwrap().clone(),
        h.board.view(peer_uid(), after),
        h.board.view(peer_uid(), after + FLOOR_NS),
    );
    assert_not_recognized("short deadline", observed);
}

/// LKS11: requests refused before the scan (UID not enrolled, rate limited) publish nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lks_pam_refused_before_the_scan_publishes_nothing() {
    let mut options = PipelineOptions::new(current_monotonic_nanos);
    options.enrolled = Vec::new();
    options.max_attempts = 1;
    let parts = build_pipeline(options).await;
    let board = LockStateBoard::new();
    let sock_dir = tempfile::tempdir().unwrap();
    let dispatcher = Arc::new(
        ConnectionDispatcher::with_pipeline(
            config(sock_dir.path(), false),
            Arc::new(HealthState::new()),
            parts.components.clone(),
        )
        .with_lock_state_board(board.clone()),
    );
    let sock = sock_dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let disp = Arc::clone(&dispatcher);
            tokio::spawn(async move {
                let _ = disp.handle_connection(stream).await;
            });
        }
    });
    // Not enrolled (attempt recorded, no scan), then rate limited.
    let first = send_auth(&sock, &auth_request(peer_uid(), 1)).await;
    assert_eq!(first.verdict, Verdict::Unavailable);
    let second = send_auth(&sock, &auth_request(peer_uid(), 2)).await;
    assert_eq!(second.reason_class, ReasonClass::RateLimited);
    for at in [now(), now() + FLOOR_NS] {
        assert_eq!(
            board.view(peer_uid(), at),
            LockStateView {
                state: LockWidgetState::Idle,
                sequence: 0
            }
        );
    }
    assert_eq!(parts.camera.wakes(), 0);
}
