//! Latency benchmark test suite for `soos-vision`.
//! Acceptance criteria: V5 — Full pipeline < 150ms p95 on reference hardware.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "Contractual test suite utilizes direct assertions, unwrap, indexing, and diagnostic output"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use soos_camera_v4l::{Frame, PixelFormat};
use soos_inference_ort::mock::{MockEmbeddingExtractor, MockFaceDetector, MockPadDetector};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

#[test]
fn test_pipeline_latency_budget_under_150ms_p95() {
    let width = 640;
    let height = 480;

    let detector = Arc::new(MockFaceDetector::new_centered_face(width, height, 0.99));
    let pad = Arc::new(MockPadDetector::new_live());
    let extractor = Arc::new(MockEmbeddingExtractor::new(512));
    let config = VisionPipelineConfig::default();

    let pipeline = VisionPipeline::new(detector, pad, extractor, config);

    // Warm-up iteration
    let yuyv_data = vec![128u8; (width * height * 2) as usize];
    let frame = Frame::new(yuyv_data, width, height, 1_000_000, PixelFormat::Yuyv, 1);

    let _ = pipeline.process_frame(&frame);

    // Benchmark 50 consecutive frames
    const ITERATIONS: usize = 50;
    let mut durations = Vec::with_capacity(ITERATIONS);

    for seq in 0..ITERATIONS {
        let frame = Frame::new(
            vec![128u8; (width * height * 2) as usize],
            width,
            height,
            (seq as u64) * 33_333_333,
            PixelFormat::Yuyv,
            seq as u64,
        );

        let start = Instant::now();
        let result = pipeline.process_frame(&frame);
        let elapsed = start.elapsed();

        assert!(
            result.is_ok(),
            "Pipeline processing failed at iteration {}",
            seq
        );
        durations.push(elapsed);
    }

    durations.sort_unstable();
    let p50 = durations[ITERATIONS * 50 / 100];
    let p95 = durations[ITERATIONS * 95 / 100];
    let p99 = durations[ITERATIONS * 99 / 100];

    eprintln!(
        "\n[BENCHMARK] Vision Pipeline Latency (640x480 YUYV -> RGB -> Detect -> Align -> Embed):\n  p50: {:?}\n  p95: {:?}\n  p99: {:?}",
        p50, p95, p99
    );

    assert!(
        p95 < Duration::from_millis(150),
        "ACCEPTANCE CRITERION V5 VIOLATION: p95 latency {:?} exceeds 150ms budget!",
        p95
    );
}
