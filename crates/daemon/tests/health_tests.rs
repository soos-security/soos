//! Contractual integration tests for health check subsystem.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use soos_daemon::health::{HealthState, HealthStatus};

#[test]
fn test_health_initial_state() {
    let health = HealthState::new();
    let status = health.snapshot();

    assert!(!status.socket_ready);
    assert!(!status.camera_ready);
    assert!(!status.models_verified);
    assert!(!status.is_healthy);
}

#[test]
fn test_health_component_readiness_reporting() {
    // Acceptance D4: Health check exposes socket_ready, camera_ready, models_verified
    let health = HealthState::new();

    health.set_socket_ready(true);
    let s1 = health.snapshot();
    assert!(s1.socket_ready);
    assert!(!s1.camera_ready);
    assert!(!s1.models_verified);
    assert!(!s1.is_healthy);

    health.set_camera_ready(true);
    let s2 = health.snapshot();
    assert!(s2.socket_ready);
    assert!(s2.camera_ready);
    assert!(!s2.models_verified);
    assert!(!s2.is_healthy);

    health.set_models_verified(true);
    let s3 = health.snapshot();
    assert!(s3.socket_ready);
    assert!(s3.camera_ready);
    assert!(s3.models_verified);
    assert!(s3.is_healthy);

    // If one drops, healthy becomes false
    health.set_camera_ready(false);
    let s4 = health.snapshot();
    assert!(!s4.camera_ready);
    assert!(!s4.is_healthy);
}

#[test]
fn test_health_status_display() {
    let status = HealthStatus {
        socket_ready: true,
        camera_ready: false,
        models_verified: true,
        is_healthy: false,
    };

    let display_str = format!("{}", status);
    assert!(display_str.contains("socket_ready: true"));
    assert!(display_str.contains("camera_ready: false"));
    assert!(display_str.contains("models_verified: true"));
    assert!(display_str.contains("is_healthy: false"));
}
