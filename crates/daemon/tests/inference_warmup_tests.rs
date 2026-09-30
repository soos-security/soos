//! Contractual tests for GitHub #276 (daemon follow-ups): the vision inference estimate is
//! seeded by a warm-up run at daemon start, so the first `Auth` request is admitted against a
//! measured latency instead of the `DEFAULT_INFERENCE_ESTIMATE_MS` guess (walkthrough 96
//! follow-up).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use soos_daemon::inference::{
    InferenceEstimator, InferenceGate, InferenceJobError, DEFAULT_INFERENCE_ESTIMATE_MS,
    MAX_INFERENCE_ESTIMATE_MS, WARMUP_PASSES,
};
use soos_daemon::pipeline::{warm_up_vision_stages, warmed_inference_gate, MAX_WARMUP_DIMENSION};
use soos_inference_ort::{
    BiometricEmbedding, EmbeddingExtractor, FaceDetection, FaceDetector, InferenceError,
    PadDetector, PadResult,
};
use soos_vision::{VisionPipeline, VisionPipelineConfig};

// ---------------------------------------------------------------------------
// Estimator seeding
// ---------------------------------------------------------------------------

#[test]
fn test_estimator_seed_replaces_the_estimate_within_bounds() {
    let estimator = InferenceEstimator::default();
    assert_eq!(
        estimator.estimate(),
        Duration::from_millis(DEFAULT_INFERENCE_ESTIMATE_MS)
    );

    estimator.seed(Duration::from_millis(12));
    assert_eq!(
        estimator.estimate(),
        Duration::from_millis(12),
        "A seed replaces the estimate instead of blending with the default"
    );

    estimator.seed(Duration::from_secs(30));
    assert_eq!(
        estimator.estimate(),
        Duration::from_millis(MAX_INFERENCE_ESTIMATE_MS)
    );

    estimator.seed(Duration::ZERO);
    assert_eq!(estimator.estimate(), Duration::from_millis(1));
}

#[test]
fn test_warmup_runs_at_least_two_passes() {
    const {
        assert!(
            WARMUP_PASSES >= 2,
            "The cold first pass (lazy ONNX Runtime allocations) must be discarded"
        );
    };
}

// ---------------------------------------------------------------------------
// InferenceGate::warm_up
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_gate_warm_up_seeds_estimate_with_measured_latency() {
    let gate = InferenceGate::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let job_calls = calls.clone();
    let measured = gate
        .warm_up(move || {
            job_calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(200));
        })
        .await
        .expect("warm-up succeeds");

    assert_eq!(calls.load(Ordering::SeqCst), WARMUP_PASSES);
    assert!(measured >= Duration::from_millis(200), "{measured:?}");
    // Blending one 200ms sample into the 80ms default would give about 110ms.
    assert!(
        gate.estimate() >= Duration::from_millis(200),
        "The estimate must be seeded with the measurement, got {:?}",
        gate.estimate()
    );
    assert_eq!(gate.available_permits(), gate.max_concurrent());
}

#[tokio::test]
async fn test_gate_warm_up_discards_the_cold_first_pass() {
    let gate = InferenceGate::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let job_calls = calls.clone();
    gate.warm_up(move || {
        let call = job_calls.fetch_add(1, Ordering::SeqCst);
        let delay = if call == 0 { 700 } else { 5 };
        std::thread::sleep(Duration::from_millis(delay));
    })
    .await
    .expect("warm-up succeeds");

    assert!(
        gate.estimate() < Duration::from_millis(400),
        "The cold first pass must not define the estimate, got {:?}",
        gate.estimate()
    );
}

#[tokio::test]
async fn test_gate_warm_up_panic_keeps_default_estimate_and_releases_slot() {
    let gate = InferenceGate::default();
    let result = gate
        .warm_up(|| -> () { panic!("synthetic warm-up failure") })
        .await;
    assert_eq!(result, Err(InferenceJobError::Panicked));
    assert_eq!(
        gate.estimate(),
        Duration::from_millis(DEFAULT_INFERENCE_ESTIMATE_MS),
        "A failed warm-up keeps the conservative default"
    );
    assert_eq!(gate.available_permits(), gate.max_concurrent());
}

// ---------------------------------------------------------------------------
// Vision stage warm-up
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Recorder {
    detector: Mutex<Vec<(usize, u32, u32, bool)>>,
    pad: Mutex<Vec<(usize, u32, u32, bool)>>,
    extractor: Mutex<Vec<(usize, u32, u32, bool)>>,
}

fn all_zero(buf: &[u8]) -> bool {
    buf.iter().all(|b| *b == 0)
}

struct RecordingDetector(Arc<Recorder>, bool);
impl FaceDetector for RecordingDetector {
    fn detect(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<FaceDetection>, InferenceError> {
        self.0
            .detector
            .lock()
            .unwrap()
            .push((rgb.len(), width, height, all_zero(rgb)));
        if self.1 {
            Err(InferenceError::Ort("synthetic".into()))
        } else {
            Ok(Vec::new())
        }
    }
}

struct RecordingPad(Arc<Recorder>, bool);
impl PadDetector for RecordingPad {
    fn evaluate_liveness(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<PadResult, InferenceError> {
        self.0
            .pad
            .lock()
            .unwrap()
            .push((rgb.len(), width, height, all_zero(rgb)));
        if self.1 {
            Err(InferenceError::Ort("synthetic".into()))
        } else {
            Ok(PadResult::live(0.99))
        }
    }
}

struct RecordingExtractor(Arc<Recorder>, bool);
impl EmbeddingExtractor for RecordingExtractor {
    fn extract_embedding(
        &self,
        aligned_crop_rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<BiometricEmbedding, InferenceError> {
        self.0.extractor.lock().unwrap().push((
            aligned_crop_rgb.len(),
            width,
            height,
            all_zero(aligned_crop_rgb),
        ));
        std::thread::sleep(Duration::from_millis(30));
        if self.1 {
            Err(InferenceError::Ort("synthetic".into()))
        } else {
            Ok(BiometricEmbedding::new(vec![0.0; 512]))
        }
    }
}

fn recording_vision(recorder: &Arc<Recorder>, fail: bool) -> VisionPipeline {
    VisionPipeline::new(
        Arc::new(RecordingDetector(recorder.clone(), fail)),
        Arc::new(RecordingPad(recorder.clone(), fail)),
        Arc::new(RecordingExtractor(recorder.clone(), fail)),
        VisionPipelineConfig::default(),
    )
}

#[test]
fn test_vision_warm_up_runs_every_stage_once_on_blank_inputs() {
    let recorder = Arc::new(Recorder::default());
    let vision = recording_vision(&recorder, false);
    warm_up_vision_stages(&vision, 640, 480);

    assert_eq!(
        *recorder.detector.lock().unwrap(),
        vec![(640 * 480 * 3, 640, 480, true)]
    );
    assert_eq!(
        *recorder.pad.lock().unwrap(),
        vec![(80 * 80 * 3, 80, 80, true)]
    );
    assert_eq!(
        *recorder.extractor.lock().unwrap(),
        vec![(112 * 112 * 3, 112, 112, true)]
    );
}

#[test]
fn test_vision_warm_up_continues_after_stage_errors() {
    let recorder = Arc::new(Recorder::default());
    let vision = recording_vision(&recorder, true);
    warm_up_vision_stages(&vision, 320, 240);
    assert_eq!(recorder.detector.lock().unwrap().len(), 1);
    assert_eq!(recorder.pad.lock().unwrap().len(), 1);
    assert_eq!(recorder.extractor.lock().unwrap().len(), 1);
}

#[test]
fn test_vision_warm_up_bounds_the_synthetic_frame() {
    let recorder = Arc::new(Recorder::default());
    let vision = recording_vision(&recorder, false);
    warm_up_vision_stages(&vision, u32::MAX, 0);
    let detector = recorder.detector.lock().unwrap();
    assert_eq!(detector.len(), 1);
    let (len, width, height, _) = detector[0];
    assert!((1..=MAX_WARMUP_DIMENSION).contains(&width), "{width}");
    assert!((1..=MAX_WARMUP_DIMENSION).contains(&height), "{height}");
    assert_eq!(len, (width as usize) * (height as usize) * 3);
}

#[tokio::test]
async fn test_warmed_inference_gate_is_seeded_from_the_vision_stages() {
    let recorder = Arc::new(Recorder::default());
    let vision = Arc::new(recording_vision(&recorder, false));
    let gate = warmed_inference_gate(vision, 320, 240).await;

    assert_eq!(recorder.extractor.lock().unwrap().len(), WARMUP_PASSES);
    // The recording extractor sleeps 30ms per pass; the seeded estimate reflects it.
    assert!(
        gate.estimate() >= Duration::from_millis(30),
        "{:?}",
        gate.estimate()
    );
    assert_ne!(
        gate.estimate(),
        Duration::from_millis(DEFAULT_INFERENCE_ESTIMATE_MS)
    );
}
