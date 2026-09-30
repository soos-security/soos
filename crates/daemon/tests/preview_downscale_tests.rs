//! Contract tests for review finding CAM-14 (GitHub #196): a preview frame whose raw encoding
//! exceeds `MAX_PREVIEW_MESSAGE_SIZE` (2 MiB) must never make the daemon drop the connection.
//!
//! Contract:
//! - Frames at most `MAX_PREVIEW_WIDTH` pixels wide whose payload fits the preview budget are
//!   forwarded unchanged (format, size and bytes).
//! - Larger uncompressed frames are downscaled daemon-side (integer decimation) to at most
//!   `MAX_PREVIEW_WIDTH` pixels wide: colour formats become RGB24 (wire format 0), greyscale
//!   stays greyscale (wire format 1).
//! - A frame that cannot be converted or still does not fit is answered with an explicit empty
//!   `PreviewResponse` (`format = 255`, no data) and the connection stays open.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use soos_biometric_store::{BiometricStore, MasterKey as BioMasterKey};
use soos_camera_v4l::{CameraManager, Frame, PixelFormat};
use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::pipeline::PipelineComponents;
use soos_daemon::preview::{
    preview_image_for_frame, PreviewConfig, MAX_PREVIEW_PIXEL_BYTES, MAX_PREVIEW_WIDTH,
    PREVIEW_FORMAT_EMPTY,
};
use soos_evidence_store::{EvidenceConfig, EvidenceStore, MasterKey as EvMasterKey};
use soos_inference_ort::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_policy::{AuthorizationEngine, RateLimitConfig, RateLimiter, ThresholdConfig};
use soos_protocol::codec::{decode_preview, encode};
use soos_protocol::types::{
    PreviewResponse, Request, RequestKind, CURRENT_VERSION, MAX_PREVIEW_MESSAGE_SIZE,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn frame(width: u32, height: u32, format: PixelFormat, data: Vec<u8>) -> Frame {
    Frame::new(data, width, height, 1_000_000, format, 7)
}

fn yuyv_frame(width: u32, height: u32) -> Frame {
    // Neutral chroma (128) so every decoded pixel is grey = luma.
    let mut data = vec![0u8; (width * height * 2) as usize];
    for (i, px) in data.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        px[0] = (i % 251) as u8;
        px[1] = 128;
    }
    frame(width, height, PixelFormat::Yuyv, data)
}

/// Camera that always serves the same frame.
struct FixedFrameCamera {
    frame: Arc<Frame>,
}

impl CameraManager for FixedFrameCamera {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        Some(self.frame.clone())
    }
    fn is_ready(&self) -> bool {
        true
    }
    fn notify_activity(&self) {}
    fn stop(&self) {}
}

fn pipeline(temp: &std::path::Path, camera: Arc<dyn CameraManager>) -> PipelineComponents {
    let bio_key = BioMasterKey::generate().expect("Generate bio key");
    let bio_store =
        Arc::new(BiometricStore::new(temp.join("biometrics"), bio_key).expect("BioStore init"));
    let ev_dir = temp.join("evidence");
    std::fs::create_dir_all(&ev_dir).expect("Create ev dir");
    let ev_key = EvMasterKey::generate().expect("Generate ev key");
    let evidence_store = Arc::new(EvidenceStore::new(
        EvidenceConfig {
            enabled: false,
            base_dir: ev_dir,
            retention_days: 1,
            daily_cap_per_uid: 1,
            key_path: temp.join("evidence.key"),
        },
        ev_key,
    ));
    let vision = Arc::new(VisionPipeline::new(
        Arc::new(MockFaceDetector::new_centered_face(320, 240, 0.95)),
        Arc::new(MockPadDetector::new_live()),
        Arc::new(MockEmbeddingExtractor::new(512)),
        VisionPipelineConfig::default(),
    ));
    let policy = Arc::new(RwLock::new(AuthorizationEngine::with_rate_limiter(
        ThresholdConfig::default(),
        RateLimiter::new(RateLimitConfig::new(5, 60_000_000_000)),
    )));
    PipelineComponents {
        camera,
        vision,
        biometric_store: bio_store,
        evidence_store,
        policy,
    }
}

fn preview_request(uid_hint: u32, nonce: u8) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::PreviewFrame,
        request_id: [nonce; 32],
        uid_hint,
        service: "soos-gui".to_string(),
        deadline_monotonic_ns: u64::MAX,
    }
}

async fn exchange(client: &mut UnixStream, req: &Request) -> Vec<u8> {
    let framed = encode(req).expect("Encoding failed");
    client.write_all(&framed).await.expect("Write request");
    client.flush().await.expect("Flush client");
    let mut len_bytes = [0u8; 4];
    client
        .read_exact(&mut len_bytes)
        .await
        .expect("The daemon must answer instead of closing the connection");
    let resp_len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + resp_len];
    buf[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut buf[4..]).await.expect("Read body");
    buf
}

/// Serves two preview requests on one connection for a camera streaming `served`.
async fn serve_twice(served: Frame) -> (PreviewResponse, PreviewResponse) {
    let current_uid = nix::unistd::getuid().as_raw();
    let dir = tempdir().expect("tempdir");
    let sock_path = dir.path().join("preview_downscale.sock");
    let camera: Arc<dyn CameraManager> = Arc::new(FixedFrameCamera {
        frame: Arc::new(served),
    });
    let dispatcher = Arc::new(
        ConnectionDispatcher::with_pipeline(
            DispatcherConfig {
                max_concurrent_connections: 4,
                connection_timeout: Duration::from_millis(2000),
                enforce_active_session: false,
                logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
            },
            Arc::new(HealthState::new()),
            pipeline(dir.path(), camera),
        )
        .with_preview_config(PreviewConfig {
            enabled: true,
            allowed_uids: vec![current_uid],
            ..PreviewConfig::default()
        }),
    );
    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let d = dispatcher.clone();
            tokio::spawn(async move {
                let _ = d.handle_connection(stream).await;
            });
        }
    });

    let mut client = UnixStream::connect(&sock_path).await.expect("Connect");
    let first = exchange(&mut client, &preview_request(current_uid, 0x11)).await;
    let second = exchange(&mut client, &preview_request(current_uid, 0x12)).await;
    (
        decode_preview(&first).expect("first reply must be a PreviewResponse"),
        decode_preview(&second).expect("second reply on the same connection must be served"),
    )
}

// ---------------------------------------------------------------------------
// Pure conversion contract
// ---------------------------------------------------------------------------

const _: () = assert!(MAX_PREVIEW_WIDTH == 640);
const _: () = assert!(MAX_PREVIEW_PIXEL_BYTES < MAX_PREVIEW_MESSAGE_SIZE);

#[test]
fn test_ccp_small_frame_is_forwarded_unchanged() {
    let data: Vec<u8> = (0u32..320 * 240 * 3).map(|i| (i % 256) as u8).collect();
    let image = preview_image_for_frame(&frame(320, 240, PixelFormat::Rgb24, data.clone()));
    assert_eq!(image.format, 0);
    assert_eq!((image.width, image.height), (320, 240));
    assert_eq!(image.data, data);
}

#[test]
fn test_ccp_1080p_yuyv_is_downscaled_to_rgb24() {
    let image = preview_image_for_frame(&yuyv_frame(1920, 1080));
    assert_eq!(image.format, 0, "downscaled colour frames are RGB24");
    assert_eq!((image.width, image.height), (640, 360));
    assert_eq!(image.data.len(), 640 * 360 * 3);
    assert!(image.data.len() <= MAX_PREVIEW_PIXEL_BYTES);
    // Output pixel (1, 0) samples source pixel (3, 0): luma 3, neutral chroma -> grey 3.
    assert_eq!(&image.data[3..6], &[3, 3, 3]);
}

#[test]
fn test_ccp_1280x720_yuyv_is_downscaled_to_preview_width() {
    let image = preview_image_for_frame(&yuyv_frame(1280, 720));
    assert_eq!((image.width, image.height), (640, 360));
    assert_eq!(image.format, 0);
}

#[test]
fn test_ccp_large_grey_frame_stays_grey() {
    let data: Vec<u8> = (0u32..1280 * 800).map(|i| (i % 256) as u8).collect();
    let image = preview_image_for_frame(&frame(1280, 800, PixelFormat::Grey, data));
    assert_eq!(image.format, 1);
    assert_eq!((image.width, image.height), (640, 400));
    assert_eq!(image.data.len(), 640 * 400);
    assert_eq!(image.data[1], 2, "output pixel 1 samples source pixel 2");
}

#[test]
fn test_ccp_large_rgb_and_nv12_frames_are_downscaled() {
    let rgb = preview_image_for_frame(&frame(
        1920,
        1080,
        PixelFormat::Rgb24,
        vec![9u8; 1920 * 1080 * 3],
    ));
    assert_eq!((rgb.format, rgb.width, rgb.height), (0, 640, 360));
    assert!(rgb.data.iter().all(|&b| b == 9));

    let mut nv12 = vec![77u8; 1920 * 1080];
    nv12.extend(vec![128u8; 1920 * 1080 / 2]);
    let image = preview_image_for_frame(&frame(1920, 1080, PixelFormat::Nv12, nv12));
    assert_eq!((image.format, image.width, image.height), (0, 640, 360));
    assert!(image.data.iter().all(|&b| b == 77));
}

#[test]
fn test_ccp_truncated_or_undecodable_large_frame_is_empty() {
    let short = preview_image_for_frame(&frame(1920, 1080, PixelFormat::Yuyv, vec![0u8; 1024]));
    assert_eq!(short.format, PREVIEW_FORMAT_EMPTY);
    assert!(short.data.is_empty());
    assert_eq!((short.width, short.height), (0, 0));

    let garbage = preview_image_for_frame(&frame(
        1920,
        1080,
        PixelFormat::Mjpeg,
        vec![0xAB; MAX_PREVIEW_MESSAGE_SIZE + 1],
    ));
    assert_eq!(garbage.format, PREVIEW_FORMAT_EMPTY);
    assert!(garbage.data.is_empty());
}

#[test]
fn test_ccp_wide_mjpeg_within_budget_is_forwarded_compressed() {
    let data = vec![0xFF, 0xD8, 0x00, 0x11, 0xFF, 0xD9];
    let image = preview_image_for_frame(&frame(1920, 1080, PixelFormat::Mjpeg, data.clone()));
    assert_eq!(image.format, 4, "compressed MJPEG that fits is not decoded");
    assert_eq!((image.width, image.height), (1920, 1080));
    assert_eq!(image.data, data);
}

// ---------------------------------------------------------------------------
// Dispatcher contract: no disconnect on oversize frames
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_preview_1080p_yuyv_frame_is_downscaled_not_disconnect() {
    let (first, second) = serve_twice(yuyv_frame(1920, 1080)).await;
    for reply in [&first, &second] {
        assert_eq!(reply.format, 0);
        assert_eq!((reply.width, reply.height), (640, 360));
        assert_eq!(reply.data.len(), 640 * 360 * 3);
        assert_eq!(reply.sequence, 7);
    }
}

#[tokio::test]
async fn test_preview_oversize_frame_returns_empty_response_not_disconnect() {
    let (first, second) = serve_twice(frame(
        1920,
        1080,
        PixelFormat::Mjpeg,
        vec![0xAB; MAX_PREVIEW_MESSAGE_SIZE + 1],
    ))
    .await;
    for reply in [&first, &second] {
        assert_eq!(reply.format, PREVIEW_FORMAT_EMPTY);
        assert!(reply.data.is_empty());
    }
}
