//! GitHub #298 (row SGF5): the GUI live-verification match reference (the copy of the enrolled
//! template embedding) lives in a wipe-on-drop container, so the template is wiped when it is
//! replaced, cleared or dropped.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses direct assertions and synthetic fixtures"
)]

use std::sync::Mutex;

use soos_biometric_store::BiometricTemplate;
use soos_gui::worker::{select_match_reference, update_live_match_score, WorkerSharedInput};
use soos_inference_ort::SHIPPED_EMBEDDING_MODEL;
use zeroize::Zeroizing;

/// Compile-time contract: the reference slot holds `Zeroizing<Vec<f32>>`.
fn assert_reference_slot_is_zeroizing(_slot: &Mutex<Option<Zeroizing<Vec<f32>>>>) {}

fn current_template() -> BiometricTemplate {
    let mut v = vec![0.0f32; SHIPPED_EMBEDDING_MODEL.dimension];
    v[0] = 1.0;
    BiometricTemplate::new(
        1000,
        SHIPPED_EMBEDDING_MODEL.model_id.to_string(),
        "2.0.0".to_string(),
        1_726_000_000,
        Zeroizing::new(v),
    )
    .expect("synthetic template")
}

/// SGF5: the reference field is a `Zeroizing` container and the selection installs the
/// template values into it unchanged; scoring still works against it.
#[test]
fn test_match_reference_is_a_zeroizing_container() {
    let shared = WorkerSharedInput::default();
    assert_reference_slot_is_zeroizing(&shared.match_reference);

    let template = current_template();
    assert_eq!(
        select_match_reference(
            &shared,
            Some(&template),
            SHIPPED_EMBEDDING_MODEL.model_id,
            Some(SHIPPED_EMBEDDING_MODEL.dimension),
        ),
        None
    );
    {
        let guard = shared.match_reference.lock().unwrap();
        let reference: &Zeroizing<Vec<f32>> = guard.as_ref().expect("reference installed");
        assert_eq!(reference.as_slice(), template.embedding.as_slice());
    }

    update_live_match_score(&shared, template.embedding.as_slice());
    let score = shared.live_match_score.lock().unwrap().expect("score");
    assert!((score - 1.0).abs() < 1e-5);
}
