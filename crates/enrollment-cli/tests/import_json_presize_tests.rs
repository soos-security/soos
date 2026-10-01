//! Contract (candid review 2026-09-30, Finding 4): an `import` JSON payload is decoded into
//! a zeroizing buffer reserved once at `IMPORT_EMBEDDING_DIM` values; it never grows (no
//! reallocation leaving unzeroized copies), extra values are counted but not stored, and the
//! total count is reported for the dimension check.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    reason = "Contractual test suite utilizes direct assertions"
)]

use soos_enrollment_cli::service::{decode_json_embedding, IMPORT_EMBEDDING_DIM};

fn json_array(n: usize) -> (Vec<f32>, Vec<u8>) {
    let values: Vec<f32> = (0..n).map(|i| (i as f32) * 0.001 - 0.25).collect();
    let json = serde_json::to_vec(&values).unwrap();
    (values, json)
}

#[test]
fn test_import_json_decodes_into_a_buffer_that_never_grows() {
    for n in [
        0_usize,
        1,
        3,
        IMPORT_EMBEDDING_DIM / 2 + 1,
        IMPORT_EMBEDDING_DIM - 1,
        IMPORT_EMBEDDING_DIM,
    ] {
        let (values, json) = json_array(n);
        let (embedding, count) = decode_json_embedding(&json).expect("valid JSON array");
        assert_eq!(count, n);
        assert_eq!(embedding.as_slice(), values.as_slice());
        assert_eq!(
            embedding.capacity(),
            IMPORT_EMBEDDING_DIM,
            "{n} values: reserved once, never grown"
        );
    }
}

#[test]
fn test_import_json_counts_but_never_stores_values_beyond_the_dimension() {
    for n in [IMPORT_EMBEDDING_DIM + 1, 600, 4 * IMPORT_EMBEDDING_DIM] {
        let (values, json) = json_array(n);
        let (embedding, count) = decode_json_embedding(&json).expect("valid JSON array");
        assert_eq!(count, n, "the full count is reported");
        assert_eq!(embedding.len(), IMPORT_EMBEDDING_DIM);
        assert_eq!(embedding.as_slice(), &values[..IMPORT_EMBEDDING_DIM]);
        assert_eq!(embedding.capacity(), IMPORT_EMBEDDING_DIM, "{n} values");
    }
}

#[test]
fn test_import_json_rejects_non_float_arrays() {
    assert!(decode_json_embedding(b"{\"a\": 1}").is_none());
    assert!(decode_json_embedding(b"[1.0, \"x\"]").is_none());
    assert!(decode_json_embedding(b"not json").is_none());
    assert!(decode_json_embedding(b"[1.0] trailing").is_none());
}
