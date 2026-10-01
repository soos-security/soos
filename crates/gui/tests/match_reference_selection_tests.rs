//! GitHub #298 (rows SGF1, SGF2): selecting a profile in the live-verification panel replaces
//! the match reference and withdraws any score computed against the previous reference in one
//! step, so no stale score is ever displayed for the new selection.
//!
//! - a foreign (pre-SFace) template clears the reference and the score and sets the
//!   re-enrollment note;
//! - a failed or empty template lookup clears the reference, the score and the note;
//! - a worker that keeps publishing scores concurrently never leaves a score behind once a
//!   foreign profile is selected.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses direct assertions and synthetic fixtures"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use soos_biometric_store::BiometricTemplate;
use soos_gui::worker::{select_match_reference, update_live_match_score, WorkerSharedInput};
use soos_inference_ort::SHIPPED_EMBEDDING_MODEL;
use zeroize::Zeroizing;

const LOADED_DIM: Option<usize> = Some(SHIPPED_EMBEDDING_MODEL.dimension);

fn unit_vector(dim: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; dim];
    v[0] = 1.0;
    v
}

fn template(model_id: &str, dim: usize) -> BiometricTemplate {
    BiometricTemplate::new(
        1000,
        model_id.to_string(),
        "2.0.0".to_string(),
        1_726_000_000,
        Zeroizing::new(unit_vector(dim)),
    )
    .expect("synthetic template")
}

fn current_template() -> BiometricTemplate {
    template(
        SHIPPED_EMBEDDING_MODEL.model_id,
        SHIPPED_EMBEDDING_MODEL.dimension,
    )
}

fn foreign_template() -> BiometricTemplate {
    template("arcface_w600k_mbf", 512)
}

fn score(shared: &WorkerSharedInput) -> Option<f32> {
    *shared.live_match_score.lock().unwrap()
}

fn reference(shared: &WorkerSharedInput) -> Option<Vec<f32>> {
    shared
        .match_reference
        .lock()
        .unwrap()
        .as_ref()
        .map(|reference| reference.to_vec())
}

fn select(shared: &WorkerSharedInput, template: Option<&BiometricTemplate>) -> Option<String> {
    select_match_reference(
        shared,
        template,
        SHIPPED_EMBEDDING_MODEL.model_id,
        LOADED_DIM,
    )
}

/// SGF1: a current-model template becomes the reference; a score left over from the previous
/// reference is withdrawn on selection, and no note is shown.
#[test]
fn test_selecting_current_template_sets_reference_and_withdraws_previous_score() {
    let shared = WorkerSharedInput::default();
    *shared.live_match_score.lock().unwrap() = Some(0.93);

    let note = select(&shared, Some(&current_template()));

    assert_eq!(note, None);
    assert_eq!(
        reference(&shared),
        Some(unit_vector(SHIPPED_EMBEDDING_MODEL.dimension))
    );
    assert_eq!(
        score(&shared),
        None,
        "a score computed against the previous reference must not survive a new selection"
    );

    update_live_match_score(&shared, &unit_vector(SHIPPED_EMBEDDING_MODEL.dimension));
    let live = score(&shared).expect("the worker scores against the new reference");
    assert!((live - 1.0).abs() < 1e-5);
}

/// SGF1: switching from a current profile to a foreign one clears the reference and the score
/// immediately and names the foreign model in the note; later frames publish no score.
#[test]
fn test_switching_to_foreign_profile_shows_no_stale_score() {
    let shared = WorkerSharedInput::default();
    let live = unit_vector(SHIPPED_EMBEDDING_MODEL.dimension);
    assert_eq!(select(&shared, Some(&current_template())), None);
    update_live_match_score(&shared, &live);
    assert!(score(&shared).is_some());

    let note = select(&shared, Some(&foreign_template())).expect("re-enrollment note");

    assert!(note.contains("Re-enrollment required"), "{note}");
    assert!(note.contains("arcface_w600k_mbf"), "{note}");
    assert!(note.contains("512-D"), "{note}");
    assert_eq!(reference(&shared), None);
    assert_eq!(score(&shared), None);

    update_live_match_score(&shared, &live);
    assert_eq!(
        score(&shared),
        None,
        "no score may be published once a foreign profile is selected"
    );
}

/// SGF1: a worker thread publishing scores concurrently never leaves a score visible after the
/// foreign selection has returned.
#[test]
fn test_concurrent_worker_leaves_no_score_after_foreign_selection() {
    let live = unit_vector(SHIPPED_EMBEDDING_MODEL.dimension);
    for _ in 0..200 {
        let shared = Arc::new(WorkerSharedInput::default());
        assert_eq!(select(&shared, Some(&current_template())), None);
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            let live = live.clone();
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    update_live_match_score(&shared, &live);
                    thread::yield_now();
                }
            })
        };
        thread::yield_now();

        assert!(select(&shared, Some(&foreign_template())).is_some());
        assert_eq!(score(&shared), None, "stale score right after selection");

        thread::yield_now();
        assert_eq!(
            score(&shared),
            None,
            "stale score published after selection"
        );
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        assert_eq!(score(&shared), None);
    }
}

/// SGF2: a failed or empty template lookup clears the note of the previous (foreign) profile,
/// the reference and the score: nothing of the previous selection stays on screen.
#[test]
fn test_failed_template_lookup_clears_note_reference_and_score() {
    let shared = WorkerSharedInput::default();
    assert!(select(&shared, Some(&foreign_template())).is_some());

    assert_eq!(select(&shared, None), None, "note must be cleared");
    assert_eq!(reference(&shared), None);
    assert_eq!(score(&shared), None);

    assert_eq!(select(&shared, Some(&current_template())), None);
    update_live_match_score(&shared, &unit_vector(SHIPPED_EMBEDDING_MODEL.dimension));
    assert!(score(&shared).is_some());

    assert_eq!(select(&shared, None), None);
    assert_eq!(
        reference(&shared),
        None,
        "a failed lookup must not keep the previous profile's reference"
    );
    assert_eq!(score(&shared), None);
    update_live_match_score(&shared, &unit_vector(SHIPPED_EMBEDDING_MODEL.dimension));
    assert_eq!(score(&shared), None);
}
