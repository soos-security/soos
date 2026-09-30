//! Contract (candid review 2026-09-30, Finding 4): the JSON piped to `soos-enroll import`
//! is serialized into a zeroizing buffer reserved at its exact size, so no reallocation
//! leaves an unzeroized copy of the embedding. Observable effect: capacity equals length.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    reason = "Contractual test suite utilizes direct assertions"
)]

use soos_gui::privileged::embedding_json;

#[test]
fn test_embedding_json_is_presized_exactly_and_roundtrips() {
    for dim in [0_usize, 1, 3, 128, 512, 513] {
        let embedding: Vec<f32> = (0..dim)
            .map(|i| (i as f32).mul_add(-0.013_7, 0.5) / 3.0)
            .collect();
        let json = embedding_json(&embedding).unwrap();
        assert_eq!(
            json.capacity(),
            json.len(),
            "dim {dim}: buffer must be pre-sized exactly"
        );
        let back: Vec<f32> = serde_json::from_slice(&json).unwrap();
        assert_eq!(back, embedding);
    }
}
