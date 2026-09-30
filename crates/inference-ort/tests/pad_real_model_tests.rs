//! Real-model evidence for Presentation Attack Detection (PAD) (review finding PAD-06, GitHub #172).
//!
//! Every other PAD test in the workspace runs against `MockPadDetector` or an in-memory identity
//! ONNX graph, so none of them can observe the behaviour of the shipped MiniFASNetV2 network.
//! This target loads the real `minifasnet_v2_80x80.onnx`, attested against the committed
//! `models/manifest.toml` SHA-256, and drives the production [`OrtPadDetector`] session.
//!
//! Gating (CI stays green on runners without models):
//! - Models directory: `SOOS_MODELS_DIR`, falling back to `/var/lib/soos/models`. When the PAD
//!   model file is absent the model tests print `SKIPPED` and return, unless
//!   `SOOS_REQUIRE_REAL_MODELS=1`, in which case absence is a hard failure.
//! - Corpus: `SOOS_PAD_CORPUS_DIR` (never committed — face crops are biometric data). When unset
//!   the corpus test prints `SKIPPED`. When set, it must hold at least [`MIN_SAMPLES_PER_CLASS`]
//!   crops per class and APCER / BPCER must stay under hard ceilings at the shipped threshold.
//!
//! Only synthetic (non-biometric) inputs and their golden logits are committed in this file.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::print_stdout,
    reason = "Evidence test suite utilizes direct assertions, unwraps and prints a measurement report"
)]

use std::io::Read;
use std::path::{Path, PathBuf};

use soos_inference_ort::pad::{OrtPadDetector, PadDetector, DEFAULT_MINIFASNET_LIVE_CLASS_INDEX};
use soos_inference_ort::registry::{ModelRegistry, RegistryConfig, SharedSession};

/// Manifest identifier of the PAD model.
const PAD_MODEL_ID: &str = "minifasnet_v2_pad";
/// File name of the PAD model inside the models directory.
const PAD_MODEL_FILE: &str = "minifasnet_v2_80x80.onnx";
/// Default installation directory of the models.
const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";

/// Shipped PAD decision threshold. Must equal `soos_policy::ThresholdConfig::DEFAULT_PAD_THRESHOLD`
/// and `soos_vision::pipeline::VisionPipelineConfig::default().pad_threshold` (both 0.85).
const SHIPPED_PAD_THRESHOLD: f32 = 0.85;

/// Hard ceiling for the Attack Presentation Classification Error Rate, per attack species.
const APCER_CEILING: f64 = 0.05;
/// Hard ceiling for the Bona Fide Presentation Classification Error Rate.
const BPCER_CEILING: f64 = 0.10;
/// Minimum number of crops per corpus class for a measurement to be meaningful.
const MIN_SAMPLES_PER_CLASS: usize = 20;
/// Maximum accepted crop edge (pixels) when loading corpus files.
const MAX_CROP_EDGE: usize = 1024;
/// Maximum number of files read per corpus class (bounded I/O).
const MAX_SAMPLES_PER_CLASS: usize = 10_000;

/// Absolute tolerance on golden logits (ORT CPU kernels are deterministic on one host; this
/// absorbs minor cross-CPU SIMD differences while still catching any preprocessing change).
const GOLDEN_LOGIT_TOLERANCE: f32 = 1e-3;

/// Golden raw logits of the real model for the synthetic inputs below, in class order
/// `[PrintPhoto, Live, ScreenReplay]`. Recorded on 2026-09-30 with the attested model installed
/// at `/var/lib/soos/models` (ORT CPU). All three synthetic patterns land on class 2
/// (ScreenReplay) with `p_live` around 0.005-0.006: a non-face is never scored live.
const GOLDEN_UNIFORM_GREY_LOGITS: [f32; 3] = [-3.709_296_7, -0.748_328, 4.458_604_3];
const GOLDEN_GRADIENT_LOGITS: [f32; 3] = [-3.634_143_4, -0.763_562_4, 4.398_681];
const GOLDEN_CHECKERBOARD_160_LOGITS: [f32; 3] = [-3.600_257_9, -0.737_216_3, 4.338_425_6];

// ---------------------------------------------------------------------------
// Gating helpers
// ---------------------------------------------------------------------------

fn models_dir() -> PathBuf {
    std::env::var_os("SOOS_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR))
}

fn repo_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml")
}

fn real_models_required() -> bool {
    std::env::var("SOOS_REQUIRE_REAL_MODELS").is_ok_and(|v| v == "1")
}

/// Loads the real PAD session attested against the committed manifest, or returns `None`
/// (test skipped) when the model is not installed on this host.
fn load_real_pad_session(test_name: &str) -> Option<SharedSession> {
    let dir = models_dir();
    if !dir.join(PAD_MODEL_FILE).is_file() {
        assert!(
            !real_models_required(),
            "SOOS_REQUIRE_REAL_MODELS=1 but {PAD_MODEL_FILE} is missing from {}",
            dir.display()
        );
        println!(
            "SKIPPED {test_name}: {PAD_MODEL_FILE} not found in {} (set SOOS_MODELS_DIR)",
            dir.display()
        );
        return None;
    }
    // A present model that does not match the committed checksum is a hard failure: the
    // evidence must be produced by the attested artifact only.
    let mut registry = ModelRegistry::new(RegistryConfig::with_manifest(&dir, repo_manifest()))
        .expect("committed models/manifest.toml must parse");
    let session = registry
        .get_or_load_session(PAD_MODEL_ID)
        .expect("installed PAD model must match the committed manifest SHA-256 and load");
    Some(session)
}

/// Runs the raw session on an RGB buffer through the production preprocessing and returns
/// the raw logits.
fn raw_logits(session: &SharedSession, rgb: &[u8], width: u32, height: u32) -> Vec<f32> {
    let input = OrtPadDetector::prepare_input(rgb, width, height).expect("prepare_input");
    let tensor = ort::value::TensorRef::from_array_view(([1usize, 3, 80, 80], input.as_slice()))
        .expect("tensor view");
    let mut guard = session.lock().expect("session mutex");
    let outputs = guard.run(ort::inputs![tensor]).expect("session run");
    let (_, value) = outputs.into_iter().next().expect("one output tensor");
    let (_, logits) = value.try_extract_tensor::<f32>().expect("f32 logits");
    logits.to_vec()
}

fn argmax(values: &[f32]) -> usize {
    values
        .iter()
        .enumerate()
        .fold((0usize, f32::NEG_INFINITY), |(bi, bv), (i, &v)| {
            if v > bv {
                (i, v)
            } else {
                (bi, bv)
            }
        })
        .0
}

// ---------------------------------------------------------------------------
// Synthetic, non-biometric inputs
// ---------------------------------------------------------------------------

fn uniform_grey_80() -> Vec<u8> {
    vec![128u8; 80 * 80 * 3]
}

fn gradient_80() -> Vec<u8> {
    let mut buf = Vec::with_capacity(80 * 80 * 3);
    for y in 0..80usize {
        for x in 0..80usize {
            buf.push((x * 3) as u8); // R
            buf.push((y * 3) as u8); // G
            buf.push(((x + y) * 3 / 2) as u8); // B
        }
    }
    buf
}

fn checkerboard_160() -> Vec<u8> {
    let mut buf = Vec::with_capacity(160 * 160 * 3);
    for y in 0..160usize {
        for x in 0..160usize {
            let on = ((x / 10) + (y / 10)) % 2 == 0;
            let v = if on { 220u8 } else { 30u8 };
            buf.extend_from_slice(&[v, v / 2, 255 - v]);
        }
    }
    buf
}

fn assert_golden(name: &str, actual: &[f32], golden: &[f32; 3]) {
    assert_eq!(
        actual.len(),
        3,
        "{name}: real model must emit exactly 3 logits"
    );
    assert_eq!(
        argmax(actual),
        argmax(golden),
        "{name}: argmax class changed; actual logits = {actual:?}"
    );
    for (i, (&a, &g)) in actual.iter().zip(golden.iter()).enumerate() {
        assert!(
            (a - g).abs() <= GOLDEN_LOGIT_TOLERANCE,
            "{name}: logit[{i}] = {a} drifted from golden {g} (tolerance {GOLDEN_LOGIT_TOLERANCE}); \
             actual logits = {actual:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Model-only checks (run locally whenever the model is installed)
// ---------------------------------------------------------------------------

#[test]
fn test_real_pad_model_io_shape_metadata_matches_manifest() {
    let Some(session) = load_real_pad_session("test_real_pad_model_io_shape_metadata") else {
        return;
    };
    let guard = session.lock().expect("session mutex");

    assert_eq!(
        guard.inputs().len(),
        1,
        "MiniFASNetV2 has exactly one input"
    );
    assert_eq!(
        guard.outputs().len(),
        1,
        "MiniFASNetV2 has exactly one output"
    );

    let input_shape: Vec<i64> = guard.inputs()[0]
        .dtype()
        .tensor_shape()
        .expect("input is a tensor")
        .to_vec();
    let output_shape: Vec<i64> = guard.outputs()[0]
        .dtype()
        .tensor_shape()
        .expect("output is a tensor")
        .to_vec();
    println!(
        "REAL PAD MODEL metadata: input '{}' {:?}, output '{}' {:?}",
        guard.inputs()[0].name(),
        input_shape,
        guard.outputs()[0].name(),
        output_shape
    );

    // Batch may be symbolic (-1); channel/spatial/class dimensions must match the manifest.
    assert_eq!(input_shape.len(), 4, "input must be NCHW");
    assert!(
        input_shape[0] == 1 || input_shape[0] == -1,
        "batch 1 or dynamic"
    );
    assert_eq!(
        &input_shape[1..],
        &[3, 80, 80],
        "input must be [N, 3, 80, 80]"
    );
    assert_eq!(output_shape.len(), 2, "output must be [N, classes]");
    assert!(
        output_shape[0] == 1 || output_shape[0] == -1,
        "batch 1 or dynamic"
    );
    assert_eq!(output_shape[1], 3, "output must carry 3 classes");
    assert!(
        DEFAULT_MINIFASNET_LIVE_CLASS_INDEX
            < usize::try_from(output_shape[1]).expect("class count"),
        "live class index must address a real output class"
    );
}

#[test]
fn test_real_pad_model_is_deterministic_and_finite_on_synthetic_input() {
    let Some(session) = load_real_pad_session("test_real_pad_model_deterministic") else {
        return;
    };
    let rgb = gradient_80();
    let first = raw_logits(&session, &rgb, 80, 80);
    let second = raw_logits(&session, &rgb, 80, 80);

    assert_eq!(first.len(), 3, "real model must emit 3 logits");
    assert!(first.iter().all(|v| v.is_finite()), "logits must be finite");
    assert_eq!(first, second, "same input must yield bit-identical logits");

    let probs = OrtPadDetector::softmax(&first);
    let sum: f32 = probs.iter().sum();
    assert!((sum - 1.0).abs() < 1e-5, "softmax must sum to 1, got {sum}");
}

#[test]
fn test_real_pad_model_golden_logits_on_synthetic_inputs() {
    let Some(session) = load_real_pad_session("test_real_pad_model_golden_logits") else {
        return;
    };
    let cases: [(&str, Vec<u8>, u32, &[f32; 3]); 3] = [
        (
            "uniform_grey_80",
            uniform_grey_80(),
            80,
            &GOLDEN_UNIFORM_GREY_LOGITS,
        ),
        ("gradient_80", gradient_80(), 80, &GOLDEN_GRADIENT_LOGITS),
        (
            "checkerboard_160",
            checkerboard_160(),
            160,
            &GOLDEN_CHECKERBOARD_160_LOGITS,
        ),
    ];
    // Print every case before asserting so a drift report shows all new values at once.
    let measured: Vec<(&str, Vec<f32>, &[f32; 3])> = cases
        .iter()
        .map(|(name, rgb, edge, golden)| {
            let logits = raw_logits(&session, rgb, *edge, *edge);
            println!(
                "REAL PAD MODEL golden {name}: logits {logits:?}, argmax {}, softmax {:?}",
                argmax(&logits),
                OrtPadDetector::softmax(&logits)
            );
            (*name, logits, *golden)
        })
        .collect();
    for (name, logits, golden) in measured {
        assert_golden(name, &logits, golden);
    }
}

#[test]
fn test_real_pad_golden_logits_detect_channel_order_swap() {
    let Some(session) = load_real_pad_session("test_real_pad_golden_detect_channel_swap") else {
        return;
    };
    // Feeding the gradient with R and B swapped emulates an RGB/BGR preprocessing regression.
    let mut swapped = gradient_80();
    for px in (0..swapped.len()).step_by(3) {
        swapped.swap(px, px + 2);
    }
    let logits = raw_logits(&session, &swapped, 80, 80);
    let max_delta = logits
        .iter()
        .zip(GOLDEN_GRADIENT_LOGITS.iter())
        .map(|(a, g)| (a - g).abs())
        .fold(0.0f32, f32::max);
    println!("REAL PAD MODEL channel-swapped gradient: logits {logits:?}, max delta {max_delta}");
    assert!(
        max_delta > GOLDEN_LOGIT_TOLERANCE,
        "golden tolerance must be tight enough to detect a channel-order swap (delta {max_delta})"
    );
}

#[test]
fn test_real_ort_pad_detector_score_matches_live_class_softmax() {
    let Some(session) = load_real_pad_session("test_real_ort_pad_detector_score") else {
        return;
    };
    let detector = OrtPadDetector::new(session.clone(), SHIPPED_PAD_THRESHOLD);
    for (name, rgb, edge) in [
        ("uniform_grey_80", uniform_grey_80(), 80u32),
        ("gradient_80", gradient_80(), 80),
        ("checkerboard_160", checkerboard_160(), 160),
    ] {
        let logits = raw_logits(&session, &rgb, edge, edge);
        let p_live = OrtPadDetector::softmax(&logits)[DEFAULT_MINIFASNET_LIVE_CLASS_INDEX];
        let result = detector
            .evaluate_liveness(&rgb, edge, edge)
            .expect("real OrtPadDetector must evaluate");
        println!(
            "REAL OrtPadDetector {name}: is_live={} score={:.6} attack={:?}",
            result.is_live, result.score, result.attack_type
        );
        assert!(
            (result.score - p_live).abs() < 1e-5,
            "{name}: detector score must be softmax(logits)[live class index]"
        );
        assert_eq!(
            result.is_live,
            p_live >= SHIPPED_PAD_THRESHOLD,
            "{name}: live decision must follow the shipped threshold"
        );
        assert!(
            !result.is_live,
            "{name}: a synthetic non-face pattern must never be accepted as live"
        );
    }
}

// ---------------------------------------------------------------------------
// Corpus-driven APCER / BPCER (ISO/IEC 30107-3) at the shipped threshold
// ---------------------------------------------------------------------------

/// Presentation class of a corpus sample (one sub-directory each).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CorpusClass {
    BonaFide,
    Print,
    Screen,
}

impl CorpusClass {
    const ALL: [Self; 3] = [Self::BonaFide, Self::Print, Self::Screen];

    fn dir_name(self) -> &'static str {
        match self {
            Self::BonaFide => "bona_fide",
            Self::Print => "print",
            Self::Screen => "screen",
        }
    }
}

/// Per-class error counts at a given threshold.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct PadErrorCounts {
    bona_fide_total: usize,
    bona_fide_rejected: usize,
    print_total: usize,
    print_accepted: usize,
    screen_total: usize,
    screen_accepted: usize,
}

impl PadErrorCounts {
    fn record(&mut self, class: CorpusClass, accepted_as_live: bool) {
        match class {
            CorpusClass::BonaFide => {
                self.bona_fide_total += 1;
                if !accepted_as_live {
                    self.bona_fide_rejected += 1;
                }
            }
            CorpusClass::Print => {
                self.print_total += 1;
                if accepted_as_live {
                    self.print_accepted += 1;
                }
            }
            CorpusClass::Screen => {
                self.screen_total += 1;
                if accepted_as_live {
                    self.screen_accepted += 1;
                }
            }
        }
    }

    fn rate(errors: usize, total: usize) -> f64 {
        if total == 0 {
            1.0 // No evidence is treated as total failure, never as a perfect score.
        } else {
            errors as f64 / total as f64
        }
    }

    /// APCER per attack species: `(print, screen)`.
    fn apcer(&self) -> (f64, f64) {
        (
            Self::rate(self.print_accepted, self.print_total),
            Self::rate(self.screen_accepted, self.screen_total),
        )
    }

    fn bpcer(&self) -> f64 {
        Self::rate(self.bona_fide_rejected, self.bona_fide_total)
    }
}

#[test]
fn test_pad_error_counts_compute_apcer_bpcer_per_species() {
    let mut c = PadErrorCounts::default();
    for i in 0..20 {
        c.record(CorpusClass::BonaFide, i >= 2); // 2 of 20 rejected
        c.record(CorpusClass::Print, i == 0); // 1 of 20 accepted
        c.record(CorpusClass::Screen, false); // 0 of 20 accepted
    }
    assert_eq!(c.bpcer(), 0.10);
    assert_eq!(c.apcer(), (0.05, 0.0));

    // An empty class is a failure, never a perfect 0% rate.
    let empty = PadErrorCounts::default();
    assert_eq!(empty.bpcer(), 1.0);
    assert_eq!(empty.apcer(), (1.0, 1.0));
}

/// Parses a binary PPM (P6, maxval 255) file into `(rgb, width, height)` with bounded sizes.
fn parse_ppm(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let mut fields: Vec<usize> = Vec::with_capacity(3);
    let mut pos = 0usize;
    if bytes.get(0..2) != Some(b"P6") {
        return Err("not a binary P6 PPM".to_string());
    }
    pos += 2;
    while fields.len() < 3 {
        // Skip whitespace and comments.
        loop {
            match bytes.get(pos) {
                Some(b) if b.is_ascii_whitespace() => pos += 1,
                Some(b'#') => {
                    while bytes.get(pos).is_some_and(|&b| b != b'\n') {
                        pos += 1;
                    }
                }
                Some(_) => break,
                None => return Err("truncated PPM header".to_string()),
            }
        }
        let start = pos;
        while bytes.get(pos).is_some_and(u8::is_ascii_digit) && pos - start < 6 {
            pos += 1;
        }
        let text = std::str::from_utf8(&bytes[start..pos]).map_err(|e| e.to_string())?;
        fields.push(text.parse::<usize>().map_err(|e| e.to_string())?);
    }
    // Exactly one whitespace byte separates the header from the raster.
    if !bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
        return Err("malformed PPM header terminator".to_string());
    }
    pos += 1;
    let (w, h, maxval) = (fields[0], fields[1], fields[2]);
    if maxval != 255 {
        return Err(format!("unsupported maxval {maxval}"));
    }
    if w == 0 || h == 0 || w > MAX_CROP_EDGE || h > MAX_CROP_EDGE {
        return Err(format!("crop {w}x{h} outside 1..={MAX_CROP_EDGE}"));
    }
    let len = w * h * 3;
    let raster = bytes
        .get(pos..pos + len)
        .ok_or_else(|| "truncated PPM raster".to_string())?;
    Ok((raster.to_vec(), w as u32, h as u32))
}

#[test]
fn test_parse_ppm_accepts_valid_and_rejects_malformed() {
    let mut ok = b"P6\n# comment\n2 1\n255\n".to_vec();
    ok.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
    assert_eq!(parse_ppm(&ok), Ok((vec![1, 2, 3, 4, 5, 6], 2, 1)));

    assert!(
        parse_ppm(b"P5\n2 1\n255\n\x00\x00").is_err(),
        "greyscale PGM rejected"
    );
    assert!(parse_ppm(b"P6\n2 1\n65535\n").is_err(), "16-bit rejected");
    assert!(
        parse_ppm(b"P6\n2 1\n255\n\x01\x02").is_err(),
        "truncated raster rejected"
    );
    assert!(
        parse_ppm(b"P6\n4096 4096\n255\n").is_err(),
        "oversized crop rejected"
    );
    assert!(parse_ppm(b"P6\n0 1\n255\n").is_err(), "zero edge rejected");
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let limit = (MAX_CROP_EDGE * MAX_CROP_EDGE * 3 + 256) as u64;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.len() as u64 > limit {
        return Err("file exceeds the bounded crop size".to_string());
    }
    Ok(buf)
}

fn list_crops(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("corpus class directory {} unreadable: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ppm") && p.is_file())
        .take(MAX_SAMPLES_PER_CLASS)
        .collect();
    files.sort();
    files
}

#[test]
fn test_real_pad_corpus_apcer_bpcer_under_ceilings() {
    let Some(corpus) = std::env::var_os("SOOS_PAD_CORPUS_DIR").map(PathBuf::from) else {
        println!("SKIPPED test_real_pad_corpus_apcer_bpcer: SOOS_PAD_CORPUS_DIR not set");
        return;
    };
    let Some(session) = load_real_pad_session("test_real_pad_corpus_apcer_bpcer") else {
        // A corpus without a model cannot be measured: fail instead of skipping silently.
        panic!("SOOS_PAD_CORPUS_DIR is set but the real PAD model is not installed");
    };
    let detector = OrtPadDetector::new(session.clone(), SHIPPED_PAD_THRESHOLD);

    let mut counts = PadErrorCounts::default();
    for class in CorpusClass::ALL {
        let files = list_crops(&corpus.join(class.dir_name()));
        assert!(
            files.len() >= MIN_SAMPLES_PER_CLASS,
            "corpus class '{}' holds {} crops, at least {MIN_SAMPLES_PER_CLASS} required",
            class.dir_name(),
            files.len()
        );
        for path in files {
            let bytes = read_bounded(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let (rgb, w, h) =
                parse_ppm(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let logits = raw_logits(&session, &rgb, w, h);
            let result = detector
                .evaluate_liveness(&rgb, w, h)
                .expect("real OrtPadDetector must evaluate corpus crop");
            let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            // Golden line format: `<class>/<file> argmax=<i> logits=[..] live=<bool>`.
            // Only these derived numbers may be committed, never the crops themselves.
            println!(
                "GOLDEN {}/{} argmax={} logits={logits:?} live={} score={:.6}",
                class.dir_name(),
                file_name.unwrap_or_default(),
                argmax(&logits),
                result.is_live,
                result.score
            );
            counts.record(class, result.is_live);
        }
    }

    let (apcer_print, apcer_screen) = counts.apcer();
    let bpcer = counts.bpcer();
    println!(
        "REAL PAD CORPUS @ threshold {SHIPPED_PAD_THRESHOLD}: {counts:?} \
         APCER(print)={apcer_print:.4} APCER(screen)={apcer_screen:.4} BPCER={bpcer:.4}"
    );
    assert!(
        apcer_print <= APCER_CEILING,
        "APCER(print) {apcer_print:.4} exceeds ceiling {APCER_CEILING}"
    );
    assert!(
        apcer_screen <= APCER_CEILING,
        "APCER(screen) {apcer_screen:.4} exceeds ceiling {APCER_CEILING}"
    );
    assert!(
        bpcer <= BPCER_CEILING,
        "BPCER {bpcer:.4} exceeds ceiling {BPCER_CEILING}"
    );
}
