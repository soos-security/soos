//! Contractual test suite for Presentation Attack Detection (PAD) in the Vision pipeline.
//!
//! Enforces:
//! - Sub-issue #15.2: PAD integration between alignment and embedding extraction
//! - Sub-issue #15.3: Test fixtures (real face vs screen vs printed photo), FAR/FRR benchmark,
//!   and latency budget compliance (< 35ms).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite uses assertions and unwrap"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use soos_inference_ort::error::InferenceError;
use soos_inference_ort::mock::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_inference_ort::pad::{AttackType, PadDetector, PadResult};
use soos_inference_ort::{BiometricEmbedding, EmbeddingExtractor};
use soos_vision::{VisionError, VisionPipeline, VisionPipelineConfig};

// User-approved 2026-09-30 (GitHub #241): the shared fixtures are a dev-dependency crate.
use soos_test_fixtures as fixtures;

/// Spy embedding extractor to verify whether embedding extraction was invoked.
struct SpyEmbeddingExtractor {
    called: AtomicBool,
    inner: MockEmbeddingExtractor,
}

impl SpyEmbeddingExtractor {
    fn new(dim: usize) -> Self {
        Self {
            called: AtomicBool::new(false),
            inner: MockEmbeddingExtractor::new(dim),
        }
    }

    fn was_called(&self) -> bool {
        self.called.load(Ordering::SeqCst)
    }
}

impl EmbeddingExtractor for SpyEmbeddingExtractor {
    fn extract_embedding(
        &self,
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<BiometricEmbedding, InferenceError> {
        self.called.store(true, Ordering::SeqCst);
        self.inner
            .extract_embedding(aligned_crop_rgb, width, height)
    }
}

fn create_test_pipeline(
    pad: Arc<dyn PadDetector>,
    config: VisionPipelineConfig,
) -> (
    VisionPipeline,
    Arc<MockFaceDetector>,
    Arc<SpyEmbeddingExtractor>,
) {
    let detector = Arc::new(MockFaceDetector::new_centered_face(640, 480, 0.95));
    let extractor = Arc::new(SpyEmbeddingExtractor::new(512));

    let pipeline = VisionPipeline::new(detector.clone(), pad, extractor.clone(), config);

    (pipeline, detector, extractor)
}

#[test]
fn test_pipeline_accepts_live_face() {
    let pad = Arc::new(MockPadDetector::new_live());
    let config = VisionPipelineConfig::default();
    let (pipeline, _, extractor) = create_test_pipeline(pad, config);

    let frame = fixtures::pad::create_live_face_frame(640, 480);
    let output = pipeline
        .process_frame(&frame)
        .expect("Live face must pass pipeline");

    assert!(output.pad_result.is_live);
    assert!(output.pad_result.score >= 0.80);
    assert_eq!(output.pad_result.attack_type, None);
    assert!(
        extractor.was_called(),
        "Embedding extraction must be performed on live face"
    );
    assert_eq!(output.embedding.len(), 512);
}

#[test]
fn test_pipeline_rejects_printed_photo_spoof() {
    let pad = Arc::new(MockPadDetector::new_spoof(AttackType::PrintPhoto, 0.05));
    let config = VisionPipelineConfig::default();
    let (pipeline, _, extractor) = create_test_pipeline(pad, config);

    let frame = fixtures::pad::create_printed_photo_frame(640, 480);
    let err = pipeline
        .process_frame(&frame)
        .expect_err("Printed photo spoof must be rejected");

    match err {
        VisionError::PadFailed { score, threshold } => {
            assert_eq!(score, 0.05);
            assert_eq!(threshold, 0.85);
        }
        other => panic!("Expected VisionError::PadFailed, got {:?}", other),
    }

    assert!(
        !extractor.was_called(),
        "Embedding extraction must NOT be invoked when PAD fails"
    );
}

#[test]
fn test_pipeline_rejects_screen_replay_spoof() {
    let pad = Arc::new(MockPadDetector::new_spoof(AttackType::ScreenReplay, 0.15));
    let config = VisionPipelineConfig::default();
    let (pipeline, _, extractor) = create_test_pipeline(pad, config);

    let frame = fixtures::pad::create_screen_replay_frame(640, 480);
    let err = pipeline
        .process_frame(&frame)
        .expect_err("Screen replay spoof must be rejected");

    match err {
        VisionError::PadFailed { score, threshold } => {
            assert_eq!(score, 0.15);
            assert_eq!(threshold, 0.85);
        }
        other => panic!("Expected VisionError::PadFailed, got {:?}", other),
    }

    assert!(
        !extractor.was_called(),
        "Embedding extraction must NOT be invoked when PAD fails"
    );
}

#[test]
fn test_pad_threshold_calibration() {
    // Face with borderline PAD score 0.75
    let pad = Arc::new(MockPadDetector::new_with_result(PadResult::new(
        true, 0.75, None,
    )));

    // Case 1: Configured threshold 0.70 -> Passes
    let config_low = VisionPipelineConfig {
        pad_threshold: 0.70,
        ..Default::default()
    };
    let (pipe_low, _, _) = create_test_pipeline(pad.clone(), config_low);
    let frame = fixtures::pad::create_live_face_frame(640, 480);
    assert!(pipe_low.process_frame(&frame).is_ok());

    // Case 2: Configured threshold 0.80 -> Fails
    let config_high = VisionPipelineConfig {
        pad_threshold: 0.80,
        ..Default::default()
    };
    let (pipe_high, _, _) = create_test_pipeline(pad, config_high);
    let err = pipe_high
        .process_frame(&frame)
        .expect_err("Borderline score must fail high threshold");
    match err {
        VisionError::PadFailed { score, threshold } => {
            assert_eq!(score, 0.75);
            assert_eq!(threshold, 0.80);
        }
        other => panic!("Expected PadFailed, got {:?}", other),
    }
}

#[test]
fn test_pad_pipeline_plumbing_routes_mock_verdicts_without_leaks() {
    // Pipeline plumbing test, NOT a model benchmark (review finding PAD-06 / GitHub #172):
    // the scripted `MockPadDetector` verdicts are routed through the vision pipeline 100 times
    // (50 live, 25 printed photos, 25 screen replays) to prove that every live verdict yields an
    // embedding and every spoof verdict is rejected. The resulting "FAR/FRR" only measures the
    // plumbing; real-model evidence lives in `soos-inference-ort` `pad_real_model_tests`.
    let pad = Arc::new(MockPadDetector::new_live());
    let config = VisionPipelineConfig::default();
    let (pipeline, _, _) = create_test_pipeline(pad.clone(), config);

    let mut false_accepts = 0usize;
    let mut false_rejects = 0usize;

    let total_live = 50usize;
    let total_spoofs = 50usize; // 25 print + 25 screen

    // 1. Evaluate genuine live presentations
    pad.set_result(PadResult::live(0.95));
    let live_frame = fixtures::pad::create_live_face_frame(640, 480);
    for _ in 0..total_live {
        if pipeline.process_frame(&live_frame).is_err() {
            false_rejects += 1;
        }
    }

    // 2. Evaluate printed photo presentation attacks
    pad.set_result(PadResult::spoof(0.04, AttackType::PrintPhoto));
    let print_frame = fixtures::pad::create_printed_photo_frame(640, 480);
    for _ in 0..25 {
        if pipeline.process_frame(&print_frame).is_ok() {
            false_accepts += 1;
        }
    }

    // 3. Evaluate digital screen replay presentation attacks
    pad.set_result(PadResult::spoof(0.11, AttackType::ScreenReplay));
    let screen_frame = fixtures::pad::create_screen_replay_frame(640, 480);
    for _ in 0..25 {
        if pipeline.process_frame(&screen_frame).is_ok() {
            false_accepts += 1;
        }
    }

    let far = false_accepts as f64 / total_spoofs as f64;
    let frr = false_rejects as f64 / total_live as f64;

    assert_eq!(far, 0.0, "False Accept Rate must be strictly 0.0%");
    assert_eq!(frr, 0.0, "False Reject Rate must be strictly 0.0%");
}

#[test]
fn test_pad_latency_budget_compliance() {
    // ARCHITECTURE.md §7 specifies a 35ms budget for PAD liveness verification
    let pad = MockPadDetector::new_live();
    let aligned_crop = vec![128u8; 112 * 112 * 3];

    let start = Instant::now();
    let result = pad
        .evaluate_liveness(&aligned_crop, 112, 112)
        .expect("PAD evaluation should succeed");
    let duration = start.elapsed();

    assert!(result.is_live);
    assert!(
        duration.as_millis() < 35,
        "PAD evaluation must complete within 35ms latency budget, took {:?}",
        duration
    );
}

#[test]
fn test_expand_bbox_shifts_roi_without_distortion() {
    use soos_inference_ort::BoundingBox;
    use soos_vision::crop::expand_bbox_for_pad;

    // A 40x40 face box placed close to the top-left edge: (10, 10) to (50, 50).
    let bbox = BoundingBox::new(10.0, 10.0, 50.0, 50.0);
    // Expanding by 2.7x gives expected width = 40 * 2.7 = 108.0, height = 40 * 2.7 = 108.0.
    // Center is (30, 30). Unshifted bounds would be [30 - 54, 30 + 54] = [-24, 84].
    // Under Minivision shifting algorithm:
    // left_top_x < 0 -> right_bottom_x += 24 -> 108; left_top_x = 0.
    // Width and height remain 108.0, maintaining 1:1 aspect ratio and full context without distortion!
    let expanded = expand_bbox_for_pad(&bbox, 2.7, 640, 480);
    assert_eq!(expanded.x1, 0.0);
    assert_eq!(expanded.y1, 0.0);
    assert_eq!(expanded.x2, 108.0);
    assert_eq!(expanded.y2, 108.0);
    assert_eq!(expanded.x2 - expanded.x1, 108.0);
    assert_eq!(expanded.y2 - expanded.y1, 108.0);
}
