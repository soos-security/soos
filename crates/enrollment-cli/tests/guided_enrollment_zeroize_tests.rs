//! Contract tests for GitHub #312 (review finding STO-NEW-9): guided enrollment samples are
//! biometric data. They are held in `Zeroizing` buffers, never printed by `Debug` (only the
//! per-step counts are), and the fused composite is returned as `Zeroizing<Vec<f32>>`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses direct assertions and fixtures"
)]

use soos_enrollment_cli::guided_enrollment::{EnrollmentStep, GuidedEnrollmentSession};
use soos_vision::pose::HeadPose;
use zeroize::Zeroizing;

/// Unit vector with two distinctive components (0.28, 0.96).
fn marked_embedding() -> Vec<f32> {
    let mut v = vec![0.0_f32; 128];
    v[0] = 0.28;
    v[1] = 0.96;
    v
}

fn pose(yaw: f32, pitch: f32) -> HeadPose {
    HeadPose {
        yaw,
        pitch,
        roll: 0.0,
    }
}

fn completed_session() -> GuidedEnrollmentSession {
    let mut session = GuidedEnrollmentSession::new(1);
    for p in [
        pose(0.0, 0.0),
        pose(-15.0, 0.0),
        pose(15.0, 0.0),
        pose(0.0, -12.0),
    ] {
        session.process_sample(&p, &marked_embedding(), true, true);
    }
    assert_eq!(session.current_step(), EnrollmentStep::Completed);
    session
}

#[test]
fn test_312_guided_session_debug_never_prints_samples() {
    let session = completed_session();
    let debug = format!("{session:?}");
    assert!(
        !debug.contains("0.96") && !debug.contains("0.28"),
        "Debug must redact embedding samples: {debug}"
    );
    assert!(
        debug.contains("GuidedEnrollmentSession"),
        "Debug still names the type: {debug}"
    );
}

#[test]
fn test_312_guided_composite_is_returned_zeroizing() {
    let session = completed_session();
    let composite: Zeroizing<Vec<f32>> = session.compute_composite_embedding().unwrap();
    assert_eq!(composite.len(), 128);
    assert!((composite[1] - 0.96).abs() < 1e-5);
}
