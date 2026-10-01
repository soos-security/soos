//! Owner-run live camera PAD check (GitHub #212, PAD-07 / PVA11; walkthrough 161).
//!
//! Captures frames from a real camera, runs the real SCRFD detector and the real PAD models
//! (MiniFASNetV2 alone at 2.7x and, when available, the fused V2 + MiniFASNetV1SE at 2.7x /
//! 4.0x, averaged softmax as upstream) and prints aggregate `p_live` statistics only.
//!
//! Privacy: frames, RGB buffers, crops and tensors live in memory only, are wiped on drop
//! (`Zeroizing`) and are never written anywhere; no embedding is computed. Only counts and
//! probability statistics are printed.
//!
//! Run (no root needed when the user can open the camera node):
//!
//! ```text
//! SOOS_PAD_LIVE_CAMERA=/dev/video0 SOOS_PAD_V1SE_DIR=$HOME/.cache/soos-eval/pad/v1se \
//!   cargo test --release --locked --all-features -p soos-daemon --test pad_live_camera_check_tests \
//!   -- --ignored --nocapture
//! ```
//!
//! `SOOS_MODELS_DIR` (default `/var/lib/soos/models`) holds `scrfd_500m_kps.onnx` and
//! `minifasnet_v2_80x80.onnx`; `SOOS_PAD_V1SE_DIR` (optional) holds `minifasnet_v1se_80x80.onnx`
//! attested by `models/optional_models.toml`; `SOOS_PAD_LIVE_FRAMES` (default 60, 1..=600).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::print_stdout,
    reason = "Owner-run evidence test uses assertions, unwrap and prints aggregate measurements"
)]

use std::path::{Path, PathBuf};

use std::time::{Duration, Instant};

use soos_camera_v4l::{CameraConfigBuilder, CameraManager, V4lCameraManager};
use soos_daemon::pipeline::{PAD_MODEL_ID, SECONDARY_PAD_BBOX_SCALE, SECONDARY_PAD_MODEL_ID};
use soos_inference_ort::detector::FaceDetector;
use soos_inference_ort::pad::{OrtPadDetector, DEFAULT_MINIFASNET_LIVE_CLASS_INDEX};
use soos_inference_ort::registry::SharedSession;
use soos_inference_ort::{ModelRegistry, OrtScrfdDetector, RegistryConfig};
use soos_vision::crop::crop_pad_context;
use soos_vision::{convert_to_rgb, VisionPipelineConfig};
use zeroize::Zeroizing;

const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";
const DEFAULT_FRAMES: usize = 60;
const MAX_FRAMES: usize = 600;
const FRAME_TIMEOUT: Duration = Duration::from_secs(3);
const DETECTOR_MODEL_ID: &str = "scrfd_500m_kps";

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn session(dir: &Path, manifest: &str, id: &str) -> SharedSession {
    let mut registry =
        ModelRegistry::new(RegistryConfig::with_manifest(dir, repo_path(manifest))).unwrap();
    registry
        .get_or_load_session(id)
        .unwrap_or_else(|e| panic!("{id} from {} ({manifest}): {e}", dir.display()))
}

/// Softmax of the raw logits of one PAD session on an 80x80 RGB crop.
fn pad_probabilities(session: &SharedSession, crop: &[u8]) -> [f32; 3] {
    let input = OrtPadDetector::prepare_input(crop, 80, 80).unwrap();
    let tensor =
        ort::value::TensorRef::from_array_view(([1usize, 3, 80, 80], input.as_slice())).unwrap();
    let mut guard = session.lock().unwrap();
    let outputs = guard.run(ort::inputs![tensor]).unwrap();
    let (_, value) = outputs.into_iter().next().unwrap();
    let (_, logits) = value.try_extract_tensor::<f32>().unwrap();
    let probs = OrtPadDetector::softmax(logits);
    [probs[0], probs[1], probs[2]]
}

#[derive(Default)]
struct Summary {
    p_live: Vec<f32>,
    argmax: [usize; 3],
}

impl Summary {
    fn push(&mut self, probs: [f32; 3]) {
        self.p_live.push(probs[DEFAULT_MINIFASNET_LIVE_CLASS_INDEX]);
        let best = (0..3).fold(0, |b, i| if probs[i] > probs[b] { i } else { b });
        self.argmax[best] += 1;
    }

    fn print(&self, label: &str, threshold: f32) {
        if self.p_live.is_empty() {
            println!("{label}: no scored face");
            return;
        }
        let mut sorted = self.p_live.clone();
        sorted.sort_by(f32::total_cmp);
        let above = sorted.iter().filter(|&&p| p >= threshold).count();
        println!(
            "{label}: n={} p_live min {:.4} median {:.4} max {:.4}; >= {threshold}: {above}/{} \
             ({:.1} %); argmax [print, live, replay] = {:?}",
            sorted.len(),
            sorted[0],
            sorted[sorted.len() / 2],
            sorted[sorted.len() - 1],
            sorted.len(),
            100.0 * above as f64 / sorted.len() as f64,
            self.argmax
        );
    }
}

#[test]
#[ignore = "owner-run: needs a camera (SOOS_PAD_LIVE_CAMERA) and the real models"]
fn test_pva_live_camera_pad_report() {
    let Some(device) = std::env::var_os("SOOS_PAD_LIVE_CAMERA").map(PathBuf::from) else {
        println!("SKIPPED test_pva_live_camera_pad_report: SOOS_PAD_LIVE_CAMERA not set");
        return;
    };
    let frames: usize = std::env::var("SOOS_PAD_LIVE_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_FRAMES)
        .clamp(1, MAX_FRAMES);
    let models_dir = std::env::var_os("SOOS_MODELS_DIR")
        .map_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR), PathBuf::from);
    let config = VisionPipelineConfig::default();

    let detector = OrtScrfdDetector::new(
        session(&models_dir, "models/manifest.toml", DETECTOR_MODEL_ID),
        config.min_face_confidence,
        config.nms_iou_threshold,
    )
    .unwrap();
    let v2 = session(&models_dir, "models/manifest.toml", PAD_MODEL_ID);
    let v1se = std::env::var_os("SOOS_PAD_V1SE_DIR").map(|dir| {
        session(
            Path::new(&dir),
            "models/optional_models.toml",
            SECONDARY_PAD_MODEL_ID,
        )
    });
    println!(
        "LIVE PAD CHECK: camera {}, {frames} frames, threshold {}, fused member {}",
        device.display(),
        config.pad_threshold,
        if v1se.is_some() {
            "present"
        } else {
            "absent (SOOS_PAD_V1SE_DIR unset)"
        }
    );

    let camera = V4lCameraManager::spawn(CameraConfigBuilder::new().device_path(&device).build())
        .expect("open camera");
    let (mut no_face, mut multi_face, mut timeouts, mut frame_count) = (0usize, 0usize, 0usize, 0);
    let mut single = Summary::default();
    let mut fused = Summary::default();
    let mut last_sequence = None;
    let started = Instant::now();
    while frame_count < frames {
        camera.notify_activity();
        let wait = Instant::now();
        let frame = loop {
            match camera.latest_frame() {
                Some(f) if last_sequence.is_none_or(|s| f.sequence > s) => break Some(f),
                _ if wait.elapsed() >= FRAME_TIMEOUT => break None,
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        };
        let Some(frame) = frame else {
            timeouts += 1;
            if timeouts >= 3 {
                break;
            }
            continue;
        };
        last_sequence = Some(frame.sequence);
        frame_count += 1;

        let rgb = Zeroizing::new(
            convert_to_rgb(&frame.data, frame.width, frame.height, frame.format).unwrap(),
        );
        let faces: Vec<_> = detector
            .detect(&rgb, frame.width, frame.height)
            .unwrap()
            .into_iter()
            .filter(|d| d.score >= config.min_face_confidence)
            .collect();
        match faces.as_slice() {
            [] => no_face += 1,
            [face] => {
                let crop = |scale: f32| {
                    Zeroizing::new(
                        crop_pad_context(
                            &rgb,
                            frame.width,
                            frame.height,
                            &face.box_,
                            scale,
                            80,
                            80,
                        )
                        .unwrap(),
                    )
                };
                let p_v2 = pad_probabilities(&v2, &crop(config.pad_bbox_scale));
                single.push(p_v2);
                if let Some(v1se) = &v1se {
                    let p_v1se = pad_probabilities(v1se, &crop(SECONDARY_PAD_BBOX_SCALE));
                    fused.push([
                        (p_v2[0] + p_v1se[0]) / 2.0,
                        (p_v2[1] + p_v1se[1]) / 2.0,
                        (p_v2[2] + p_v1se[2]) / 2.0,
                    ]);
                }
            }
            _ => multi_face += 1,
        }
    }
    camera.stop();

    println!(
        "LIVE PAD CHECK: {frame_count} frames in {:.1} s ({timeouts} frame timeouts): \
         {} with one face, {no_face} no face, {multi_face} several faces",
        started.elapsed().as_secs_f64(),
        single.p_live.len()
    );
    single.print("LIVE PAD V2 2.7x (single)", config.pad_threshold);
    if v1se.is_some() {
        fused.print(
            "LIVE PAD V2 2.7x + V1SE 4.0x (fused mean)",
            config.pad_threshold,
        );
    }
    assert!(
        frame_count > 0,
        "no frame captured from {}",
        device.display()
    );
}
