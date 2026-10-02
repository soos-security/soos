//! Contract tests for GitHub #312 (review finding STO-NEW-5): `soos-daemon` loads or creates
//! the evidence key only when `[pipeline.evidence] enabled` is set. A disabled store never
//! touches `evidence.key_path`, so a missing evidence key keeps meaning "evidence was never
//! enabled" (`Docs/ENROLLMENT_CLI.md`, `soos-enroll migrate`).
//!
//! `initialize_pipeline` builds both stores before it loads the models; with no models it
//! fails at the model step, after the evidence key decision has been made.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and expect"
)]

use soos_daemon::config::PipelineConfig;
use soos_daemon::error::DaemonError;
use soos_daemon::pipeline::initialize_pipeline;
use tempfile::tempdir;

fn config_in(dir: &std::path::Path, evidence_enabled: bool) -> PipelineConfig {
    let mut config = PipelineConfig {
        use_mock_camera: true,
        ..Default::default()
    };
    config.camera.warmup_frames = 0;
    config.models_dir = dir.join("nonexistent_models");
    config.biometrics_dir = dir.join("biometrics");
    config.master_key_path = dir.join("master.key");
    config.evidence.enabled = evidence_enabled;
    config.evidence.base_dir = dir.join("evidence");
    config.evidence.key_path = dir.join("keys").join("evidence.key");
    config
}

#[tokio::test]
async fn test_312_disabled_evidence_never_creates_the_evidence_key() {
    let dir = tempdir().expect("tempdir");
    let config = config_in(dir.path(), false);

    let res = initialize_pipeline(&config);
    assert!(
        matches!(res, Err(DaemonError::Inference(_))),
        "fails at the model step"
    );
    assert!(
        !config.evidence.key_path.exists(),
        "a disabled evidence store must not create the evidence key"
    );
    assert!(
        !dir.path().join("keys").exists(),
        "a disabled evidence store must not create the evidence key directory"
    );
}

#[tokio::test]
async fn test_312_disabled_evidence_ignores_an_invalid_evidence_key() {
    let dir = tempdir().expect("tempdir");
    let config = config_in(dir.path(), false);
    std::fs::create_dir_all(config.evidence.key_path.parent().unwrap()).unwrap();
    // World-readable and of the wrong length: refused when loaded.
    std::fs::write(&config.evidence.key_path, b"not a key").unwrap();

    let res = initialize_pipeline(&config);
    assert!(
        matches!(res, Err(DaemonError::Inference(_))),
        "a disabled evidence store must not load the evidence key, got {res:?}",
        res = res.err()
    );
}

#[tokio::test]
async fn test_312_enabled_evidence_still_creates_the_evidence_key() {
    let dir = tempdir().expect("tempdir");
    let config = config_in(dir.path(), true);

    let res = initialize_pipeline(&config);
    assert!(
        matches!(res, Err(DaemonError::Inference(_))),
        "fails at the model step"
    );
    assert!(
        config.evidence.key_path.is_file(),
        "an enabled evidence store loads or creates its key"
    );
}
