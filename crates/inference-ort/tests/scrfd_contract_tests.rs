//! Contractual tests for the SCRFD score activation contract (review finding VIS-05, GitHub #247)
//! and the startup output-shape validation of `OrtScrfdDetector::new` (VIS-07, GitHub #249).
//!
//! VIS-05: the detector used to pass a raw score through when it lay in `[0, 1]` and apply a
//! sigmoid otherwise, per element. That mapping is non-monotonic across the boundary
//! (`1.0 -> 1.0` but `1.01 -> 0.733`; `0.0 -> 0.0` but `-0.01 -> 0.4975`). The activation is now
//! an explicit property of the detector (`ScoreActivation`, default `Probability` because the
//! attested `scrfd_500m_kps` graph ends in `Sigmoid`), and the production decode path fails
//! closed with `TensorError` on any non-finite or out-of-range probability.
//!
//! VIS-07: `OrtScrfdDetector::new` validated only the output count; it now also validates the
//! per-stride score/bbox/kps shape patterns from the session metadata.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::sync::{Arc, Mutex};

use soos_inference_ort::detector::{OrtScrfdDetector, ScoreActivation};
use soos_inference_ort::error::InferenceError;

const GRID: usize = 10;
const ANCHORS: usize = 2;
const NUM: usize = GRID * GRID * ANCHORS;

/// Tensors for a 10x10 stride-8 grid with every score set to `fill`.
fn tensors(fill: f32) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let scores = vec![fill; NUM];
    let mut bboxes = vec![0.0f32; NUM * 4];
    for b in bboxes.iter_mut() {
        *b = 1.0;
    }
    let kps = vec![0.0f32; NUM * 10];
    (scores, bboxes, kps)
}

fn decode_checked(
    scores: &[f32],
    bboxes: &[f32],
    kps: &[f32],
    threshold: f32,
    activation: ScoreActivation,
) -> Result<Vec<soos_inference_ort::FaceDetection>, InferenceError> {
    OrtScrfdDetector::decode_stride_checked(
        8, GRID, GRID, ANCHORS, scores, bboxes, kps, threshold, 1.0, 0.0, 0.0, 640, 640, activation,
    )
}

/// Decodes with the historical entry point; returns `(x2, score)` pairs (`x2` identifies the
/// grid cell: anchor index `2 * col` of row 0 has `x2 = (col + 1) * 8`).
fn decode_legacy(scores: &[f32], bboxes: &[f32], kps: &[f32], threshold: f32) -> Vec<(f32, f32)> {
    OrtScrfdDetector::decode_stride(
        8, GRID, GRID, ANCHORS, scores, bboxes, kps, threshold, 1.0, 0.0, 0.0, 640, 640,
    )
    .iter()
    .map(|d| (d.box_.x2, d.score))
    .collect()
}

fn score_at(dets: &[(f32, f32)], x2: f32) -> f32 {
    dets.iter()
        .find(|(bx2, _)| (*bx2 - x2).abs() < 1e-3)
        .map(|(_, s)| *s)
        .unwrap_or_else(|| panic!("no detection with x2={x2}: {dets:?}"))
}

// ---------------------------------------------------------------------------
// VIS-05: explicit activation, fail closed
// ---------------------------------------------------------------------------

/// The attested model emits probabilities: the default activation is `Probability`.
#[test]
fn test_scrfd_default_score_activation_is_probability() {
    assert_eq!(ScoreActivation::default(), ScoreActivation::Probability);
}

/// Probability scores outside `[0, 1]` or non-finite fail closed with `TensorError`.
#[test]
fn test_scrfd_rejects_out_of_range_scores() {
    for bad in [
        1.01f32,
        -0.01,
        2.0,
        -5.0,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ] {
        let (mut scores, bboxes, kps) = tensors(0.001);
        scores[7] = bad;
        let err = decode_checked(&scores, &bboxes, &kps, 0.5, ScoreActivation::Probability)
            .expect_err("out-of-range probability must fail closed");
        assert!(
            matches!(err, InferenceError::TensorError(_)),
            "score {bad}: expected TensorError, got {err:?}"
        );
    }
}

/// In-range probabilities, including the exact bounds, pass through unchanged.
#[test]
fn test_scrfd_probability_scores_pass_through_unchanged() {
    let (mut scores, bboxes, kps) = tensors(0.0);
    scores[3] = 1.0;
    scores[11] = 0.7839;
    let dets = decode_checked(&scores, &bboxes, &kps, 0.5, ScoreActivation::Probability)
        .expect("in-range probabilities must decode");
    let mut got: Vec<f32> = dets.iter().map(|d| d.score).collect();
    got.sort_by(|a, b| b.total_cmp(a));
    assert_eq!(got, vec![1.0, 0.7839]);
}

/// `Logit` applies the sigmoid to every score (monotonic), and rejects non-finite values.
#[test]
fn test_scrfd_logit_activation_is_monotonic_and_rejects_non_finite() {
    let (mut scores, bboxes, kps) = tensors(-10.0);
    scores[1] = 0.9;
    scores[2] = 3.0;
    let dets = decode_checked(&scores, &bboxes, &kps, 0.5, ScoreActivation::Logit)
        .expect("finite logits must decode");
    let s1 = dets.iter().find(|d| (d.score - 0.710_949_5).abs() < 1e-5);
    let s2 = dets.iter().find(|d| (d.score - 0.952_574_1).abs() < 1e-5);
    assert!(
        s1.is_some() && s2.is_some(),
        "sigmoid(0.9) and sigmoid(3.0) expected: {dets:?}"
    );

    scores[4] = f32::NAN;
    let err = decode_checked(&scores, &bboxes, &kps, 0.5, ScoreActivation::Logit)
        .expect_err("NaN logit must fail closed");
    assert!(matches!(err, InferenceError::TensorError(_)));
}

/// The historical `decode_stride` entry point infers one activation for the whole tensor, so
/// its mapping is monotonic across the `[0, 1]` boundary: a raw 1.01 can never score below a
/// raw 1.0, and a raw -0.01 can never score above a raw 0.0.
#[test]
fn test_scrfd_decode_stride_is_monotonic_across_unit_boundary() {
    // Row 0: anchor 0 of col 0 (x2 = 8) and anchor 0 of col 1 (x2 = 16).
    let (mut scores, bboxes, kps) = tensors(-10.0);
    scores[0] = 1.0;
    scores[2] = 1.01;
    let got = decode_legacy(&scores, &bboxes, &kps, 0.5);
    let raw_1_0 = score_at(&got, 8.0);
    let raw_1_01 = score_at(&got, 16.0);
    assert!(
        raw_1_01 >= raw_1_0,
        "raw 1.01 scored {raw_1_01} below raw 1.0 ({raw_1_0}): non-monotonic activation"
    );

    let (mut scores, bboxes, kps) = tensors(-10.0);
    scores[0] = 0.0;
    scores[2] = -0.01;
    let got = decode_legacy(&scores, &bboxes, &kps, 0.4);
    let raw_0_0 = score_at(&got, 8.0);
    let raw_neg = score_at(&got, 16.0);
    assert!(
        raw_0_0 >= raw_neg,
        "raw 0.0 scored {raw_0_0} below raw -0.01 ({raw_neg}): non-monotonic activation"
    );
}

/// `ScoreActivation::infer` classifies a whole tensor, never element by element.
#[test]
fn test_scrfd_score_activation_infer_is_tensor_wide() {
    assert_eq!(
        ScoreActivation::infer(&[0.0, 0.5, 1.0]),
        ScoreActivation::Probability
    );
    assert_eq!(
        ScoreActivation::infer(&[0.0, 0.5, 1.01]),
        ScoreActivation::Logit
    );
    assert_eq!(
        ScoreActivation::infer(&[-0.01, 0.5]),
        ScoreActivation::Logit
    );
    assert_eq!(ScoreActivation::infer(&[]), ScoreActivation::Probability);
}

// ---------------------------------------------------------------------------
// VIS-07: OrtScrfdDetector::new validates shape patterns
// ---------------------------------------------------------------------------

fn scrfd_dims(batch: i64, symbolic_anchors: bool) -> Vec<Vec<i64>> {
    let mut dims = Vec::new();
    for k in [1i64, 4, 10] {
        for n in [12800i64, 3200, 800] {
            dims.push(vec![batch, if symbolic_anchors { -1 } else { n }, k]);
        }
    }
    dims
}

/// The attested graph reports `[-1, -1, k]`: symbolic batch and anchor dims are accepted,
/// concrete dims are enforced, and anything the metadata can refute fails closed.
#[test]
fn test_scrfd_validate_output_dims_accepts_symbolic_and_rejects_wrong_layouts() {
    OrtScrfdDetector::validate_output_dims(&scrfd_dims(1, false)).expect("static dims");
    OrtScrfdDetector::validate_output_dims(&scrfd_dims(-1, false)).expect("symbolic batch");
    OrtScrfdDetector::validate_output_dims(&scrfd_dims(-1, true)).expect("attested [-1,-1,k]");

    let reject = |dims: Vec<Vec<i64>>, why: &str| {
        assert!(
            matches!(
                OrtScrfdDetector::validate_output_dims(&dims),
                Err(InferenceError::TensorError(_))
            ),
            "{why} must fail closed: {dims:?}"
        );
    };
    let mut d = scrfd_dims(1, false);
    d[0] = vec![1, 12801, 1];
    reject(d, "unknown anchor count");
    let mut d = scrfd_dims(1, false);
    d[1] = vec![1, 12800, 1];
    reject(d, "duplicate anchor count");
    let mut d = scrfd_dims(1, false);
    d[0] = vec![1, 12800, 2];
    reject(d, "wrong channel dim");
    let mut d = scrfd_dims(-1, true);
    d[8] = vec![-1, -1, -1];
    reject(d, "symbolic channel dim");
    let mut d = scrfd_dims(1, false);
    d[3] = vec![2, 12800, 4];
    reject(d, "batch dim other than 1");
    let mut d = scrfd_dims(1, false);
    d[4] = vec![3200, 4];
    reject(d, "rank-2 output");
    let mut d = scrfd_dims(1, false);
    d.pop();
    reject(d, "8 outputs");
}

// Minimal ONNX encoder for a graph of 9 Identity nodes with [1, 3] tensors: output count 9,
// but none of the SCRFD shape patterns.

fn varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn field_varint(field: u64, value: u64, out: &mut Vec<u8>) {
    varint(field << 3, out);
    varint(value, out);
}

fn field_bytes(field: u64, bytes: &[u8], out: &mut Vec<u8>) {
    varint((field << 3) | 2, out);
    varint(bytes.len() as u64, out);
    out.extend_from_slice(bytes);
}

fn float_value_info(name: &str, shape: &[u64]) -> Vec<u8> {
    let mut dims = Vec::new();
    for &dim_value in shape {
        let mut dim = Vec::new();
        field_varint(1, dim_value, &mut dim);
        field_bytes(1, &dim, &mut dims);
    }
    let mut tensor_type = Vec::new();
    field_varint(1, 1, &mut tensor_type);
    field_bytes(2, &dims, &mut tensor_type);
    let mut type_proto = Vec::new();
    field_bytes(1, &tensor_type, &mut type_proto);
    let mut value_info = Vec::new();
    field_bytes(1, name.as_bytes(), &mut value_info);
    field_bytes(2, &type_proto, &mut value_info);
    value_info
}

fn nine_identity_model(shapes: &[Vec<u64>]) -> Vec<u8> {
    let mut graph = Vec::new();
    for i in 0..9 {
        let mut node = Vec::new();
        field_bytes(1, format!("x{i}").as_bytes(), &mut node);
        field_bytes(2, format!("y{i}").as_bytes(), &mut node);
        field_bytes(4, b"Identity", &mut node);
        field_bytes(1, &node, &mut graph);
    }
    field_bytes(2, b"soos_nine_identity", &mut graph);
    for (i, shape) in shapes.iter().enumerate() {
        field_bytes(11, &float_value_info(&format!("x{i}"), shape), &mut graph);
    }
    for (i, shape) in shapes.iter().enumerate() {
        field_bytes(12, &float_value_info(&format!("y{i}"), shape), &mut graph);
    }
    let mut opset = Vec::new();
    field_varint(2, 13, &mut opset);
    let mut model = Vec::new();
    field_varint(1, 7, &mut model);
    field_bytes(7, &graph, &mut model);
    field_bytes(8, &opset, &mut model);
    model
}

fn session_from(model: &[u8]) -> soos_inference_ort::SharedSession {
    let session = ort::session::Session::builder()
        .expect("builder")
        .commit_from_memory(model)
        .expect("synthetic model must load");
    Arc::new(Mutex::new(session))
}

/// A session with 9 outputs but no SCRFD shape pattern is rejected at construction.
#[test]
fn test_scrfd_new_rejects_nine_outputs_with_wrong_shapes() {
    let shapes = vec![vec![1u64, 3]; 9];
    let session = session_from(&nine_identity_model(&shapes));
    let Err(err) = OrtScrfdDetector::new(session, 0.5, 0.45) else {
        panic!("9 outputs without the SCRFD shape patterns must be rejected");
    };
    assert!(
        matches!(err, InferenceError::TensorError(_)),
        "expected TensorError, got {err:?}"
    );
}

/// A session exposing exactly the SCRFD 500M KPS output shapes is accepted.
#[test]
fn test_scrfd_new_accepts_scrfd_output_shapes() {
    let mut shapes = Vec::new();
    for k in [1u64, 4, 10] {
        for n in [12800u64, 3200, 800] {
            shapes.push(vec![1, n, k]);
        }
    }
    let session = session_from(&nine_identity_model(&shapes));
    let detector = OrtScrfdDetector::new(session, 0.5, 0.45).expect("SCRFD shapes must pass");
    assert_eq!(detector.score_activation, ScoreActivation::Probability);
    let logit = detector.with_score_activation(ScoreActivation::Logit);
    assert_eq!(logit.score_activation, ScoreActivation::Logit);
}
