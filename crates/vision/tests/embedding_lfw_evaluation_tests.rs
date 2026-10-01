//! Real-face evaluation harness of the shipped verification pipeline on the public LFW
//! benchmark (GitHub #278, fourth item; walkthrough 160; matrix EVR1-EVR5).
//!
//! The ignored test [`test_lfw_real_face_evaluation_report`] runs the production path of
//! `soos-vision` / `soos-inference-ort` on every image referenced by the official LFW
//! `pairs.txt` (6000 pairs, 10 folds):
//!
//! 1. bounded read and production MJPEG decode (`convert_to_rgb`, `PixelFormat::Mjpeg`);
//! 2. SCRFD detection through `OrtScrfdDetector` with the production confidence and NMS
//!    defaults (`DEFAULT_MIN_FACE_CONFIDENCE`, `DEFAULT_NMS_IOU_THRESHOLD`);
//! 3. 5-point alignment through `align_face_112`;
//! 4. one embedding per pre-processing variant: the production `OrtEmbeddingExtractor` on the
//!    shipped SFace model (R, G, B planes, raw 0..255, the OpenCV `FaceRecognizerSF` recipe) and
//!    a raw BGR run of the same session; when the retired ArcFace ResNet34 file is installed
//!    next to the models, its four variants (B, G, R or R, G, B with `(x - 127.5) / 127.5` or
//!    `/ 128`) are loaded through `models/retired_models.toml` (GitHub #278, walkthrough 162).
//!    An optional, separately attested candidate model is evaluated with the SFace recipe.
//!
//! Every model is loaded through `ModelRegistry` (SHA-256 and I/O shape attestation). Images,
//! crops, embeddings and per-image or per-pair scores stay in memory and are never written or
//! printed: the report holds aggregate metrics only. Wipe-on-drop containers hold the crops
//! and embeddings.
//!
//! The data never enters the repository. Reproduce with:
//!
//! ```text
//! scripts/fetch_lfw_eval.sh                 # bounded, SHA-256 verified, ~/.cache/soos-eval
//! SOOS_EVAL_LFW_DIR=$HOME/.cache/soos-eval/lfw \
//! SOOS_EVAL_LFW_PAIRS=$HOME/.cache/soos-eval/pairs.txt \
//! SOOS_MODELS_DIR=/var/lib/soos/models \
//! SOOS_EVAL_ORT_THREADS=4 \
//! cargo test --release --locked -p soos-vision --test embedding_lfw_evaluation_tests \
//!     -- --ignored --nocapture test_lfw_real_face_evaluation_report
//! ```
//!
//! Optional: `SOOS_EVAL_CANDIDATE_DIR` (a directory holding a candidate `manifest.toml` with
//! exactly one entry and its ONNX file) and `SOOS_EVAL_MAX_PAIRS` (smoke runs).
//! Without its variables the harness refuses to run; the non-ignored tests of this file pin
//! that refusal and the metric arithmetic, without network or data.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::print_stdout,
    clippy::too_many_lines,
    reason = "Evaluation harness uses assertions, unwraps, indexing and prints an aggregate report"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use soos_camera_v4l::PixelFormat;
use soos_inference_ort::embedding::{EmbeddingExtractor, OrtEmbeddingExtractor};
use soos_inference_ort::registry::{ModelRegistry, RegistryConfig, SharedSession};
use soos_inference_ort::{FaceDetection, FaceDetector, OrtScrfdDetector, ZeroizingOutputs};
use soos_vision::{
    align_face_112, convert_to_rgb, DEFAULT_MATCH_THRESHOLD, DEFAULT_MIN_FACE_CONFIDENCE,
    DEFAULT_MIN_FACE_WIDTH_PX, DEFAULT_NMS_IOU_THRESHOLD,
};

/// Manifest id of the retired ArcFace ResNet34 (`models/retired_models.toml`).
const RETIRED_MODEL_ID: &str = "arcface_w600k_mbf";
use zeroize::Zeroizing;

/// Manifest id of the production detector.
const DETECTOR_MODEL_ID: &str = "scrfd_500m_kps";
/// Manifest id of the production embedding model (OpenCV Zoo SFace, GitHub #278).
const EMBEDDING_MODEL_ID: &str = "sface_2021dec";

/// Upper bound of `pairs.txt` (the official file is about 160 KiB).
const MAX_PAIRS_FILE_BYTES: u64 = 1024 * 1024;
/// Upper bound of one LFW JPEG (the largest is well below 64 KiB).
const MAX_IMAGE_BYTES: u64 = 1024 * 1024;
/// Upper bound of the folds x pairs-per-fold declared by the `pairs.txt` header.
const MAX_PAIRS: usize = 20_000;
/// Upper bound of an LFW person name.
const MAX_NAME_LEN: usize = 64;
/// Aligned crop side.
const SIDE: usize = 112;
/// Default ORT intra-op threads of the evaluation sessions (the host is shared).
const DEFAULT_EVAL_ORT_THREADS: usize = 2;

/// Score given to a pair whose image produced no embedding: a failed detection never matches
/// (fail closed, as in production).
const FAILED_PAIR_SCORE: f32 = -1.0;

/// FAR operating points reported (the project target is FAR <= 1e-3, `Docs/POLICY_CRATE.md`).
const TARGET_FARS: [f64; 4] = [1e-2, 1e-3, 1e-4, 1e-5];

/// Fixed thresholds reported with their FAR / TAR: the policy floor
/// (`ThresholdConfig::MIN_MATCH_THRESHOLD = 0.40`), the production default
/// `DEFAULT_MATCH_THRESHOLD = 0.50` and the retired default 0.70.
const REPORTED_THRESHOLDS: [f32; 7] = [0.40, 0.45, 0.50, 0.55, 0.60, 0.65, 0.70];

// ---------------------------------------------------------------------------
// Configuration: the harness refuses to run without its environment
// ---------------------------------------------------------------------------

/// Why the harness refuses to run.
#[derive(Debug, PartialEq, Eq)]
enum EvalConfigError {
    /// A required environment variable is unset or empty.
    Missing(&'static str),
    /// A variable is set but unusable (reason attached).
    Invalid(&'static str, String),
}

/// Validated harness configuration.
#[derive(Debug)]
struct EvalConfig {
    lfw_dir: PathBuf,
    pairs_path: PathBuf,
    models_dir: PathBuf,
    candidate_dir: Option<PathBuf>,
    ort_threads: usize,
    max_pairs: Option<usize>,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves")
}

fn required(
    lookup: &dyn Fn(&str) -> Option<String>,
    name: &'static str,
) -> Result<String, EvalConfigError> {
    lookup(name)
        .filter(|value| !value.is_empty())
        .ok_or(EvalConfigError::Missing(name))
}

/// Canonical path of an existing entry that lies outside the repository.
fn outside_repo(
    name: &'static str,
    value: &str,
    want_dir: bool,
) -> Result<PathBuf, EvalConfigError> {
    let path = Path::new(value)
        .canonicalize()
        .map_err(|e| EvalConfigError::Invalid(name, format!("cannot resolve {value}: {e}")))?;
    let kind_ok = if want_dir {
        path.is_dir()
    } else {
        path.is_file()
    };
    if !kind_ok {
        return Err(EvalConfigError::Invalid(
            name,
            format!(
                "{} is not a {}",
                path.display(),
                if want_dir { "directory" } else { "file" }
            ),
        ));
    }
    if path.starts_with(workspace_root()) {
        return Err(EvalConfigError::Invalid(
            name,
            "evaluation data must live outside the repository".to_string(),
        ));
    }
    Ok(path)
}

impl EvalConfig {
    /// Reads the configuration through `lookup` (the process environment in the real run).
    fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, EvalConfigError> {
        let lfw = required(lookup, "SOOS_EVAL_LFW_DIR")?;
        let pairs = required(lookup, "SOOS_EVAL_LFW_PAIRS")?;
        let models = required(lookup, "SOOS_MODELS_DIR")?;
        let lfw_dir = outside_repo("SOOS_EVAL_LFW_DIR", &lfw, true)?;
        let pairs_path = outside_repo("SOOS_EVAL_LFW_PAIRS", &pairs, false)?;
        let models_dir = Path::new(&models).canonicalize().map_err(|e| {
            EvalConfigError::Invalid("SOOS_MODELS_DIR", format!("cannot resolve {models}: {e}"))
        })?;
        let candidate_dir = match lookup("SOOS_EVAL_CANDIDATE_DIR").filter(|v| !v.is_empty()) {
            Some(dir) => Some(outside_repo("SOOS_EVAL_CANDIDATE_DIR", &dir, true)?),
            None => None,
        };
        let ort_threads = match lookup("SOOS_EVAL_ORT_THREADS").filter(|v| !v.is_empty()) {
            Some(v) => v
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=16).contains(n))
                .ok_or_else(|| {
                    EvalConfigError::Invalid("SOOS_EVAL_ORT_THREADS", format!("{v} not in 1..=16"))
                })?,
            None => DEFAULT_EVAL_ORT_THREADS,
        };
        let max_pairs = match lookup("SOOS_EVAL_MAX_PAIRS").filter(|v| !v.is_empty()) {
            Some(v) => Some(v.parse::<usize>().ok().filter(|n| *n > 0).ok_or_else(|| {
                EvalConfigError::Invalid("SOOS_EVAL_MAX_PAIRS", format!("{v} is not positive"))
            })?),
            None => None,
        };
        Ok(Self {
            lfw_dir,
            pairs_path,
            models_dir,
            candidate_dir,
            ort_threads,
            max_pairs,
        })
    }
}

// ---------------------------------------------------------------------------
// pairs.txt (official LFW View 2 protocol)
// ---------------------------------------------------------------------------

/// One LFW image: person name and 1-based image number.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ImageRef {
    name: String,
    number: u16,
}

impl ImageRef {
    fn path(&self, lfw_dir: &Path) -> PathBuf {
        lfw_dir
            .join(&self.name)
            .join(format!("{}_{:04}.jpg", self.name, self.number))
    }
}

/// One evaluation pair of the official protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pair {
    a: ImageRef,
    b: ImageRef,
    genuine: bool,
    fold: usize,
}

fn parse_name(raw: &str) -> Result<String, String> {
    let valid = !raw.is_empty()
        && raw.len() <= MAX_NAME_LEN
        && !raw.starts_with('.')
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '\'' | '.'));
    if valid {
        Ok(raw.to_string())
    } else {
        Err(format!("invalid LFW person name {raw:?}"))
    }
}

fn parse_number(raw: &str) -> Result<u16, String> {
    raw.parse::<u16>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("invalid LFW image number {raw:?}"))
}

/// Parses `pairs.txt`: a `folds<TAB>n` header, then per fold `n` genuine lines
/// (`name i j`) followed by `n` impostor lines (`name1 i name2 j`).
fn parse_pairs(text: &str) -> Result<Vec<Pair>, String> {
    let mut lines = text.lines();
    let header: Vec<&str> = lines
        .next()
        .ok_or("empty pairs file")?
        .split_whitespace()
        .collect();
    let [folds, per_fold] = header.as_slice() else {
        return Err("pairs header must be `folds n`".to_string());
    };
    let folds: usize = folds.parse().map_err(|_| "invalid fold count")?;
    let per_fold: usize = per_fold.parse().map_err(|_| "invalid pairs per fold")?;
    let total = folds
        .checked_mul(per_fold)
        .and_then(|n| n.checked_mul(2))
        .filter(|n| *n > 0 && *n <= MAX_PAIRS)
        .ok_or("pairs header out of bounds")?;

    let mut pairs = Vec::with_capacity(total);
    for index in 0..total {
        let line = lines.next().ok_or("pairs file is truncated")?;
        let fields: Vec<&str> = line.split_whitespace().collect();
        let fold = index / (2 * per_fold);
        let genuine = index % (2 * per_fold) < per_fold;
        let pair = match (genuine, fields.as_slice()) {
            (true, [name, i, j]) => {
                let name = parse_name(name)?;
                Pair {
                    a: ImageRef {
                        name: name.clone(),
                        number: parse_number(i)?,
                    },
                    b: ImageRef {
                        name,
                        number: parse_number(j)?,
                    },
                    genuine,
                    fold,
                }
            }
            (false, [n1, i, n2, j]) => Pair {
                a: ImageRef {
                    name: parse_name(n1)?,
                    number: parse_number(i)?,
                },
                b: ImageRef {
                    name: parse_name(n2)?,
                    number: parse_number(j)?,
                },
                genuine,
                fold,
            },
            _ => return Err(format!("malformed pairs line {}", index + 2)),
        };
        pairs.push(pair);
    }
    if lines.any(|l| !l.trim().is_empty()) {
        return Err("trailing content after the declared pairs".to_string());
    }
    Ok(pairs)
}

fn read_bounded(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    file.take(max + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > max {
        return Err(std::io::Error::other("file exceeds the harness bound"));
    }
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Metrics (pure; covered by the non-ignored tests below)
// ---------------------------------------------------------------------------

/// Count of `scores` at or above `t` (production accepts `score >= match_threshold`).
fn count_at_or_above(sorted_asc: &[f32], t: f32) -> usize {
    sorted_asc.len() - sorted_asc.partition_point(|s| *s < t)
}

/// Smallest threshold `t` whose false accept rate `#{impostor >= t} / n` is at most `far`,
/// with the achieved FAR. `None` when the impostor set is too small to resolve `far` (fewer
/// than `1 / far` impostor scores).
fn threshold_at_far(impostor_sorted_asc: &[f32], far: f64) -> Option<(f32, f64)> {
    let n = impostor_sorted_asc.len();
    if n == 0 || (n as f64) * far < 1.0 {
        return None;
    }
    let allowed = ((n as f64) * far).floor() as usize;
    // The `allowed + 1`-th highest impostor score must be rejected: t is just above it.
    let pivot = impostor_sorted_asc[n - 1 - allowed];
    let t = pivot.next_up();
    let achieved = count_at_or_above(impostor_sorted_asc, t) as f64 / n as f64;
    Some((t, achieved))
}

/// Accuracy of the threshold `t` over `(score, genuine)` samples.
fn accuracy_at(samples: &[(f32, bool)], t: f32) -> f64 {
    let correct = samples
        .iter()
        .filter(|(score, genuine)| (*score >= t) == *genuine)
        .count();
    correct as f64 / samples.len() as f64
}

/// Threshold maximizing the accuracy over `samples` (midpoint between adjacent scores).
fn best_threshold(samples: &[(f32, bool)]) -> f32 {
    let mut scores: Vec<f32> = samples.iter().map(|(s, _)| *s).collect();
    scores.sort_by(f32::total_cmp);
    scores.dedup();
    let mut best = (f64::MIN, 0.0f32);
    let mut previous = f32::NEG_INFINITY;
    for &s in scores.iter().chain(std::iter::once(&f32::INFINITY)) {
        let t = if previous.is_finite() && s.is_finite() {
            (previous + s) / 2.0
        } else {
            s
        };
        let acc = accuracy_at(samples, t);
        if acc > best.0 {
            best = (acc, t);
        }
        previous = s;
    }
    best.1
}

/// Standard LFW 10-fold protocol: for every fold, the threshold is the best one on the other
/// folds and the accuracy is measured on the held-out fold. Returns
/// `(mean accuracy, standard deviation, mean threshold)`.
fn ten_fold_accuracy(scored: &[(f32, bool, usize)]) -> (f64, f64, f64) {
    let folds: BTreeSet<usize> = scored.iter().map(|(_, _, f)| *f).collect();
    let mut accs = Vec::new();
    let mut thresholds = Vec::new();
    for &fold in &folds {
        let train: Vec<(f32, bool)> = scored
            .iter()
            .filter(|(_, _, f)| *f != fold)
            .map(|(s, g, _)| (*s, *g))
            .collect();
        let test: Vec<(f32, bool)> = scored
            .iter()
            .filter(|(_, _, f)| *f == fold)
            .map(|(s, g, _)| (*s, *g))
            .collect();
        let t = if train.is_empty() {
            best_threshold(&test)
        } else {
            best_threshold(&train)
        };
        accs.push(accuracy_at(&test, t));
        thresholds.push(f64::from(t));
    }
    let n = accs.len() as f64;
    let mean = accs.iter().sum::<f64>() / n;
    let var = accs.iter().map(|a| (a - mean).powi(2)).sum::<f64>() / n;
    (mean, var.sqrt(), thresholds.iter().sum::<f64>() / n)
}

/// Percentile (nearest rank) of an ascending slice.
fn percentile(sorted_asc: &[f32], p: f64) -> f32 {
    let rank = ((p / 100.0) * (sorted_asc.len() as f64 - 1.0)).round() as usize;
    sorted_asc[rank.min(sorted_asc.len() - 1)]
}

/// One-line aggregate summary of a score distribution (no individual score is printed).
fn summary(sorted_asc: &[f32]) -> String {
    if sorted_asc.is_empty() {
        return "n=0".to_string();
    }
    let n = sorted_asc.len() as f64;
    let mean = sorted_asc.iter().map(|s| f64::from(*s)).sum::<f64>() / n;
    let std = (sorted_asc
        .iter()
        .map(|s| (f64::from(*s) - mean).powi(2))
        .sum::<f64>()
        / n)
        .sqrt();
    format!(
        "n={} mean={mean:.4} std={std:.4} p1={:.4} p5={:.4} p50={:.4} p95={:.4} p99={:.4}",
        sorted_asc.len(),
        percentile(sorted_asc, 1.0),
        percentile(sorted_asc, 5.0),
        percentile(sorted_asc, 50.0),
        percentile(sorted_asc, 95.0),
        percentile(sorted_asc, 99.0),
    )
}

/// Prints TAR/threshold at every FAR target and FAR/TAR at the production threshold.
fn report_operating_points(label: &str, genuine_asc: &[f32], impostor_asc: &[f32]) {
    for far in TARGET_FARS {
        match threshold_at_far(impostor_asc, far) {
            Some((t, achieved)) => {
                let tar = count_at_or_above(genuine_asc, t) as f64 / genuine_asc.len() as f64;
                println!(
                    "LFW {label}: FAR<={far:.0e}: threshold={t:.4} TAR={tar:.4} \
                     (achieved FAR={achieved:.2e})"
                );
            }
            None => println!(
                "LFW {label}: FAR<={far:.0e}: not resolvable with {} impostor scores",
                impostor_asc.len()
            ),
        }
    }
    for t in REPORTED_THRESHOLDS {
        let far = count_at_or_above(impostor_asc, t) as f64 / impostor_asc.len() as f64;
        let tar = count_at_or_above(genuine_asc, t) as f64 / genuine_asc.len() as f64;
        let tag = if t == DEFAULT_MATCH_THRESHOLD {
            " (production match_threshold)"
        } else {
            ""
        };
        println!(
            "LFW {label}: at threshold={t:.2}{tag}: FAR={far:.2e} ({} of {}) TAR={tar:.4}",
            count_at_or_above(impostor_asc, t),
            impostor_asc.len()
        );
    }
}

// ---------------------------------------------------------------------------
// Embedding variants
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChannelOrder {
    Bgr,
    Rgb,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    Nhwc,
    Nchw,
}

/// Which attested session a variant runs on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    /// The shipped embedding model (committed manifest).
    Production,
    /// The retired ArcFace ResNet34 (`models/retired_models.toml`), when installed.
    Retired,
    /// The optional out-of-repository candidate.
    Candidate,
}

/// How a variant turns an aligned 112x112 RGB crop into an embedding.
#[derive(Clone, Copy)]
enum Arm {
    /// The production `OrtEmbeddingExtractor` on the crop as produced by `align_face_112`.
    Production,
    /// A raw session run with explicit channel order and `(x - mean) / std`.
    Raw {
        order: ChannelOrder,
        mean: f32,
        std: f32,
        layout: Layout,
    },
}

struct Variant {
    label: &'static str,
    source: Source,
    arm: Arm,
}

const VARIANTS: [Variant; 7] = [
    Variant {
        label: "sface RGB x (0..255) [production, OpenCV FaceRecognizerSF]",
        source: Source::Production,
        arm: Arm::Production,
    },
    Variant {
        label: "sface BGR x (0..255)",
        source: Source::Production,
        arm: Arm::Raw {
            order: ChannelOrder::Bgr,
            mean: 0.0,
            std: 1.0,
            layout: Layout::Nchw,
        },
    },
    Variant {
        label: "retired arcface BGR (x-127.5)/127.5 [former production]",
        source: Source::Retired,
        arm: Arm::Raw {
            order: ChannelOrder::Bgr,
            mean: 127.5,
            std: 127.5,
            layout: Layout::Nhwc,
        },
    },
    Variant {
        label: "retired arcface RGB (x-127.5)/127.5 [insightface arcface_onnx.py]",
        source: Source::Retired,
        arm: Arm::Raw {
            order: ChannelOrder::Rgb,
            mean: 127.5,
            std: 127.5,
            layout: Layout::Nhwc,
        },
    },
    Variant {
        label: "retired arcface RGB (x-127.5)/128 [model card]",
        source: Source::Retired,
        arm: Arm::Raw {
            order: ChannelOrder::Rgb,
            mean: 127.5,
            std: 128.0,
            layout: Layout::Nhwc,
        },
    },
    Variant {
        label: "retired arcface BGR (x-127.5)/128",
        source: Source::Retired,
        arm: Arm::Raw {
            order: ChannelOrder::Bgr,
            mean: 127.5,
            std: 128.0,
            layout: Layout::Nhwc,
        },
    },
    Variant {
        label: "candidate RGB x (0..255) NCHW [OpenCV FaceRecognizerSF recipe]",
        source: Source::Candidate,
        arm: Arm::Raw {
            order: ChannelOrder::Rgb,
            mean: 0.0,
            std: 1.0,
            layout: Layout::Nchw,
        },
    },
];

fn l2_normalized(raw: &[f32]) -> Option<Zeroizing<Vec<f32>>> {
    let norm = raw.iter().map(|v| v * v).sum::<f32>().sqrt();
    (norm.is_finite() && norm > 1e-12)
        .then(|| Zeroizing::new(raw.iter().map(|v| v / norm).collect()))
}

/// Raw session run on an aligned crop (output wiped in place by `ZeroizingOutputs`).
fn embed_raw(
    session: &SharedSession,
    crop_rgb: &[u8],
    order: ChannelOrder,
    mean: f32,
    std: f32,
    layout: Layout,
) -> Option<Zeroizing<Vec<f32>>> {
    let plane = SIDE * SIDE;
    let mut input = Zeroizing::new(vec![0.0f32; 3 * plane]);
    for (i, px) in crop_rgb.as_chunks::<3>().0.iter().enumerate() {
        let ordered = match order {
            ChannelOrder::Bgr => [px[2], px[1], px[0]],
            ChannelOrder::Rgb => [px[0], px[1], px[2]],
        };
        for (c, value) in ordered.into_iter().enumerate() {
            let v = (f32::from(value) - mean) / std;
            match layout {
                Layout::Nhwc => input[i * 3 + c] = v,
                Layout::Nchw => input[c * plane + i] = v,
            }
        }
    }
    let shape = match layout {
        Layout::Nhwc => [1usize, SIDE, SIDE, 3],
        Layout::Nchw => [1usize, 3, SIDE, SIDE],
    };
    let tensor = ort::value::TensorRef::from_array_view((shape, input.as_slice())).ok()?;
    let mut guard = session.lock().ok()?;
    let outputs = ZeroizingOutputs::new(guard.run(ort::inputs![tensor]).ok()?);
    let value = outputs.values().next()?;
    let (_, data) = value.try_extract_tensor::<f32>().ok()?;
    l2_normalized(data)
}

fn embed(
    arm: Arm,
    extractor: &OrtEmbeddingExtractor,
    session: &SharedSession,
    crop: &[u8],
) -> Option<Zeroizing<Vec<f32>>> {
    match arm {
        Arm::Production => extractor
            .extract_embedding(crop, SIDE as u32, SIDE as u32)
            .ok()
            .map(|e| e.to_vec()),
        Arm::Raw {
            order,
            mean,
            std,
            layout,
        } => embed_raw(session, crop, order, mean, std, layout),
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

// ---------------------------------------------------------------------------
// Face selection
// ---------------------------------------------------------------------------

/// Per-run detection counters (aggregates only).
#[derive(Default)]
struct DetectionStats {
    images: usize,
    read_or_decode_failed: usize,
    no_face: usize,
    missing_landmarks: usize,
    align_failed: usize,
    multiple_faces: usize,
    below_min_face_width: usize,
}

/// LFW protocol face selection: the confident detection closest to the image centre (LFW
/// crops are centred on the labelled person; production instead rejects multi-face frames,
/// which is counted separately).
fn central_face(detections: &[FaceDetection], width: u32, height: u32) -> Option<&FaceDetection> {
    let (cx, cy) = (width as f32 / 2.0, height as f32 / 2.0);
    detections
        .iter()
        .filter(|d| d.score >= DEFAULT_MIN_FACE_CONFIDENCE)
        .min_by(|a, b| {
            let da = ((a.box_.x1 + a.box_.x2) / 2.0 - cx).powi(2)
                + ((a.box_.y1 + a.box_.y2) / 2.0 - cy).powi(2);
            let db = ((b.box_.x1 + b.box_.x2) / 2.0 - cx).powi(2)
                + ((b.box_.y1 + b.box_.y2) / 2.0 - cy).powi(2);
            da.total_cmp(&db)
        })
}

fn latency_line(label: &str, samples: &mut [Duration]) -> String {
    if samples.is_empty() {
        return format!("{label}: n=0");
    }
    samples.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let mean = samples.iter().map(|d| ms(*d)).sum::<f64>() / samples.len() as f64;
    let at = |p: f64| ms(samples[((p / 100.0) * (samples.len() as f64 - 1.0)).round() as usize]);
    format!(
        "{label}: n={} mean={mean:.1} ms p50={:.1} ms p95={:.1} ms",
        samples.len(),
        at(50.0),
        at(95.0)
    )
}

fn load_session(registry: &mut ModelRegistry, id: &str) -> SharedSession {
    registry
        .get_or_load_session(id)
        .unwrap_or_else(|e| panic!("model {id} must match its manifest attestation: {e}"))
}

// ---------------------------------------------------------------------------
// The evaluation (ignored: needs LFW, the real models and tens of CPU minutes)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real-face LFW evaluation: needs SOOS_EVAL_LFW_DIR, SOOS_EVAL_LFW_PAIRS, SOOS_MODELS_DIR"]
fn test_lfw_real_face_evaluation_report() {
    // Unset variables skip the report (like the other env-gated real-hardware tests, so a plain
    // `--include-ignored` run stays green); a set but unusable variable (for example data
    // inside the repository) is still a hard refusal.
    let config = match EvalConfig::from_lookup(&|k| std::env::var(k).ok()) {
        Ok(config) => config,
        Err(EvalConfigError::Missing(var)) => {
            println!("SKIPPED test_lfw_real_face_evaluation_report: {var} not set");
            return;
        }
        Err(e) => panic!("refusing to run the LFW evaluation: {e:?}"),
    };

    let pairs_text = String::from_utf8(
        read_bounded(&config.pairs_path, MAX_PAIRS_FILE_BYTES).expect("read pairs.txt"),
    )
    .expect("pairs.txt is UTF-8");
    let mut pairs = parse_pairs(&pairs_text).expect("official pairs.txt parses");
    if let Some(max) = config.max_pairs {
        // Keep the fold structure: the first `max / 2` genuine and impostor pairs per fold
        // are not representative, so smoke runs only truncate uniformly.
        pairs.truncate(max.min(pairs.len()));
    }
    let images: Vec<ImageRef> = pairs
        .iter()
        .flat_map(|p| [p.a.clone(), p.b.clone()])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let index_of: BTreeMap<&ImageRef, usize> =
        images.iter().enumerate().map(|(i, r)| (r, i)).collect();
    println!(
        "LFW: {} pairs ({} genuine), {} distinct images, {} identities; ORT intra threads {}",
        pairs.len(),
        pairs.iter().filter(|p| p.genuine).count(),
        images.len(),
        images
            .iter()
            .map(|r| &r.name)
            .collect::<BTreeSet<_>>()
            .len(),
        config.ort_threads
    );

    // Production models through the attested registry, against the committed manifest.
    let manifest = workspace_root().join("models/manifest.toml");
    let mut registry = ModelRegistry::new(
        RegistryConfig::with_manifest(&config.models_dir, manifest)
            .with_intra_threads(config.ort_threads),
    )
    .expect("committed manifest parses");
    registry
        .verify_integrity()
        .expect("installed models match the committed SHA-256 attestations");
    let detector: Arc<dyn FaceDetector> = Arc::new(
        OrtScrfdDetector::new(
            load_session(&mut registry, DETECTOR_MODEL_ID),
            DEFAULT_MIN_FACE_CONFIDENCE,
            DEFAULT_NMS_IOU_THRESHOLD,
        )
        .expect("production SCRFD detector"),
    );
    let production = load_session(&mut registry, EMBEDDING_MODEL_ID);
    let extractor = OrtEmbeddingExtractor::new(production.clone());

    // The retired ArcFace ResNet34, attested by models/retired_models.toml, when installed.
    let mut retired_registry = ModelRegistry::new(
        RegistryConfig::with_manifest(
            &config.models_dir,
            workspace_root().join("models/retired_models.toml"),
        )
        .with_intra_threads(config.ort_threads),
    )
    .expect("committed retired manifest parses");
    let retired_file = retired_registry
        .manifest()
        .get_model(RETIRED_MODEL_ID)
        .expect("retired ArcFace entry")
        .filename
        .clone();
    let retired = config
        .models_dir
        .join(&retired_file)
        .is_file()
        .then(|| load_session(&mut retired_registry, RETIRED_MODEL_ID));
    if retired.is_none() {
        println!("LFW: retired {retired_file} not installed; its variants are skipped");
    }

    let candidate = config.candidate_dir.as_ref().map(|dir| {
        let mut reg =
            ModelRegistry::new(RegistryConfig::new(dir).with_intra_threads(config.ort_threads))
                .expect("candidate manifest parses");
        reg.verify_integrity()
            .expect("candidate model matches its SHA-256 attestation");
        let ids: Vec<String> = reg.manifest().models.keys().cloned().collect();
        assert_eq!(
            ids.len(),
            1,
            "the candidate manifest holds exactly one model"
        );
        load_session(&mut reg, &ids[0])
    });
    let active: Vec<&Variant> = VARIANTS
        .iter()
        .filter(|v| match v.source {
            Source::Production => true,
            Source::Retired => retired.is_some(),
            Source::Candidate => candidate.is_some(),
        })
        .collect();

    // Per variant: flat embeddings and a validity mask, in wipe-on-drop containers.
    let mut embeddings: Vec<Vec<Option<Zeroizing<Vec<f32>>>>> = active
        .iter()
        .map(|_| Vec::with_capacity(images.len()))
        .collect();
    let mut stats = DetectionStats::default();
    let (mut t_decode, mut t_detect, mut t_align) = (Vec::new(), Vec::new(), Vec::new());
    let mut t_embed: Vec<Vec<Duration>> = active.iter().map(|_| Vec::new()).collect();
    let started = Instant::now();

    for (n, image) in images.iter().enumerate() {
        stats.images += 1;
        if n % 500 == 0 {
            println!(
                "LFW progress: {n}/{} images ({:.0} s)",
                images.len(),
                started.elapsed().as_secs_f64()
            );
        }
        let push_none = |embeddings: &mut Vec<Vec<Option<Zeroizing<Vec<f32>>>>>| {
            for per_variant in embeddings.iter_mut() {
                per_variant.push(None);
            }
        };

        let t0 = Instant::now();
        let rgb = read_bounded(&image.path(&config.lfw_dir), MAX_IMAGE_BYTES)
            .ok()
            .and_then(|jpeg| {
                let mut decoder = jpeg_decoder::Decoder::new(jpeg.as_slice());
                decoder.read_info().ok()?;
                let info = decoder.info()?;
                let (w, h) = (u32::from(info.width), u32::from(info.height));
                convert_to_rgb(&jpeg, w, h, PixelFormat::Mjpeg)
                    .ok()
                    .map(|rgb| (Zeroizing::new(rgb), w, h))
            });
        let Some((rgb, width, height)) = rgb else {
            stats.read_or_decode_failed += 1;
            push_none(&mut embeddings);
            continue;
        };
        t_decode.push(t0.elapsed());

        let t1 = Instant::now();
        let detections = detector.detect(&rgb, width, height).unwrap_or_default();
        t_detect.push(t1.elapsed());
        let confident = detections
            .iter()
            .filter(|d| d.score >= DEFAULT_MIN_FACE_CONFIDENCE)
            .count();
        if confident > 1 {
            stats.multiple_faces += 1;
        }
        let Some(face) = central_face(&detections, width, height) else {
            stats.no_face += 1;
            push_none(&mut embeddings);
            continue;
        };
        if face.box_.width().min(face.box_.height()) < DEFAULT_MIN_FACE_WIDTH_PX {
            stats.below_min_face_width += 1;
        }
        let Some(landmarks) = &face.landmarks else {
            stats.missing_landmarks += 1;
            push_none(&mut embeddings);
            continue;
        };

        let t2 = Instant::now();
        let Ok(crop) = align_face_112(&rgb, width, height, landmarks).map(Zeroizing::new) else {
            stats.align_failed += 1;
            push_none(&mut embeddings);
            continue;
        };
        t_align.push(t2.elapsed());

        for (v, variant) in active.iter().enumerate() {
            let session = match variant.source {
                Source::Production => &production,
                Source::Retired => retired.as_ref().expect("retired session is loaded"),
                Source::Candidate => candidate.as_ref().expect("candidate session is loaded"),
            };
            let t3 = Instant::now();
            let emb = embed(variant.arm, &extractor, session, &crop);
            t_embed[v].push(t3.elapsed());
            embeddings[v].push(emb);
        }
    }

    println!(
        "LFW detection: images={} decode_failed={} no_face(FTD)={} missing_landmarks={} \
         align_failed={} multi_face(production would reject)={} below_min_face_width={} \
         ({} s total)",
        stats.images,
        stats.read_or_decode_failed,
        stats.no_face,
        stats.missing_landmarks,
        stats.align_failed,
        stats.multiple_faces,
        stats.below_min_face_width,
        started.elapsed().as_secs_f64().round()
    );
    println!("LFW latency {}", latency_line("decode", &mut t_decode));
    println!(
        "LFW latency {}",
        latency_line("scrfd detect", &mut t_detect)
    );
    println!("LFW latency {}", latency_line("align", &mut t_align));
    for (v, variant) in active.iter().enumerate() {
        println!(
            "LFW latency {}",
            latency_line(&format!("embed [{}]", variant.label), &mut t_embed[v])
        );
    }

    let failed_images =
        stats.read_or_decode_failed + stats.no_face + stats.missing_landmarks + stats.align_failed;
    assert!(
        (failed_images as f64) < 0.05 * images.len() as f64,
        "more than 5% of the LFW images produced no face: the harness is broken"
    );

    for (v, variant) in active.iter().enumerate() {
        let embs = &embeddings[v];
        let failed_variant = embs.iter().filter(|e| e.is_none()).count();
        println!("--- {} (no embedding: {failed_variant}) ---", variant.label);

        // Official protocol: 6000 pairs; a pair with a failed image never matches.
        let scored: Vec<(f32, bool, usize)> = pairs
            .iter()
            .map(|p| {
                let score = match (&embs[index_of[&p.a]], &embs[index_of[&p.b]]) {
                    (Some(a), Some(b)) => dot(a, b),
                    _ => FAILED_PAIR_SCORE,
                };
                (score, p.genuine, p.fold)
            })
            .collect();
        let (acc, acc_std, acc_t) = ten_fold_accuracy(&scored);
        println!(
            "LFW official: 10-fold accuracy={acc:.4} +/- {acc_std:.4} (mean best threshold \
             {acc_t:.4})"
        );
        let mut genuine: Vec<f32> = scored.iter().filter(|s| s.1).map(|s| s.0).collect();
        let mut impostor: Vec<f32> = scored.iter().filter(|s| !s.1).map(|s| s.0).collect();
        genuine.sort_by(f32::total_cmp);
        impostor.sort_by(f32::total_cmp);
        println!("LFW official genuine: {}", summary(&genuine));
        println!("LFW official impostor: {}", summary(&impostor));
        report_operating_points("official", &genuine, &impostor);
        drop((genuine, impostor, scored));

        // Extended protocol: every pair of successfully embedded images of the pairs set,
        // labelled by LFW identity (resolves FAR down to about 1e-7).
        let valid: Vec<usize> = (0..images.len()).filter(|i| embs[*i].is_some()).collect();
        let mut genuine = Vec::new();
        let mut impostor = Vec::with_capacity(valid.len() * valid.len() / 2);
        for (k, &i) in valid.iter().enumerate() {
            let a = embs[i].as_ref().expect("valid");
            for &j in &valid[k + 1..] {
                let score = dot(a, embs[j].as_ref().expect("valid"));
                if images[i].name == images[j].name {
                    genuine.push(score);
                } else {
                    impostor.push(score);
                }
            }
        }
        genuine.sort_by(f32::total_cmp);
        impostor.sort_by(f32::total_cmp);
        println!("LFW extended genuine: {}", summary(&genuine));
        println!("LFW extended impostor: {}", summary(&impostor));
        report_operating_points("extended", &genuine, &impostor);
    }
}

// ---------------------------------------------------------------------------
// Regular tests: no data, no network, no model
// ---------------------------------------------------------------------------

fn no_env(_: &str) -> Option<String> {
    None
}

#[test]
fn test_lfw_harness_refuses_to_run_without_env_vars() {
    assert_eq!(
        EvalConfig::from_lookup(&no_env).unwrap_err(),
        EvalConfigError::Missing("SOOS_EVAL_LFW_DIR")
    );
    let tmp = tempfile::tempdir().expect("temp dir");
    let lfw = tmp.path().to_string_lossy().into_owned();
    let only_lfw = move |k: &str| (k == "SOOS_EVAL_LFW_DIR").then(|| lfw.clone());
    assert_eq!(
        EvalConfig::from_lookup(&only_lfw).unwrap_err(),
        EvalConfigError::Missing("SOOS_EVAL_LFW_PAIRS")
    );
}

#[test]
fn test_lfw_harness_refuses_data_inside_the_repository() {
    let repo = workspace_root();
    let pairs = repo.join("Cargo.toml").to_string_lossy().into_owned();
    let lfw = repo.join("crates").to_string_lossy().into_owned();
    let lookup = move |k: &str| match k {
        "SOOS_EVAL_LFW_DIR" => Some(lfw.clone()),
        "SOOS_EVAL_LFW_PAIRS" => Some(pairs.clone()),
        "SOOS_MODELS_DIR" => Some("/".to_string()),
        _ => None,
    };
    assert!(matches!(
        EvalConfig::from_lookup(&lookup).unwrap_err(),
        EvalConfigError::Invalid("SOOS_EVAL_LFW_DIR", _)
    ));
}

#[test]
fn test_lfw_pairs_parser_reads_the_official_layout_and_rejects_traversal() {
    let text = "2\t1\nAlice_A\t1\t2\nBob_B\t1\tCarol_C\t3\nDan_D\t2\t4\nEve_E\t1\tFay_F\t1\n";
    let pairs = parse_pairs(text).expect("well-formed pairs");
    assert_eq!(pairs.len(), 4);
    assert!(pairs[0].genuine && !pairs[1].genuine && pairs[2].genuine && !pairs[3].genuine);
    assert_eq!(
        pairs.iter().map(|p| p.fold).collect::<Vec<_>>(),
        [0, 0, 1, 1]
    );
    assert_eq!(
        pairs[1].b.path(Path::new("/lfw")),
        Path::new("/lfw/Carol_C/Carol_C_0003.jpg")
    );
    assert!(parse_pairs("1\t1\n../etc\t1\t2\nA\t1\tB\t1\n").is_err());
    assert!(parse_pairs("1\t1\nA/B\t1\t2\nA\t1\tB\t1\n").is_err());
    assert!(parse_pairs("1\t1\nA\t0\t2\nA\t1\tB\t1\n").is_err());
    assert!(parse_pairs("1\t2\nA\t1\t2\n").is_err(), "truncated file");
    assert!(parse_pairs("100000\t300\n").is_err(), "unbounded header");
}

#[test]
fn test_lfw_metrics_threshold_at_far_and_ten_fold_accuracy() {
    // 1000 impostor scores 0.000..0.999, genuine scores all above them.
    let impostor: Vec<f32> = (0..1000).map(|i| i as f32 / 1000.0).collect();
    let genuine: Vec<f32> = (0..100).map(|i| 1.5 + i as f32 / 1000.0).collect();
    let (t, achieved) = threshold_at_far(&impostor, 1e-2).expect("resolvable");
    assert_eq!(count_at_or_above(&impostor, t), 10, "exactly 1% accepted");
    assert!((achieved - 1e-2).abs() < 1e-12);
    assert!(t > 0.989 && t <= 0.990);
    assert!(
        threshold_at_far(&impostor, 1e-4).is_none(),
        "1000 scores cannot resolve 1e-4"
    );

    let mut scored = Vec::new();
    for fold in 0..10 {
        for i in 0..30 {
            scored.push((0.8 + i as f32 / 1000.0, true, fold));
            scored.push((0.1 + i as f32 / 1000.0, false, fold));
        }
    }
    let (acc, std, t) = ten_fold_accuracy(&scored);
    assert_eq!(acc, 1.0);
    assert_eq!(std, 0.0);
    assert!(t > 0.129 && t < 0.8, "separating threshold, got {t}");
    assert_eq!(accuracy_at(&[(0.5, true), (0.5, false)], 0.5), 0.5);
    assert!(summary(&genuine).starts_with("n=100 "));
}
