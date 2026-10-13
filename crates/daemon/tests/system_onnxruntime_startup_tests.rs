//! Daemon start-up against the linked ONNX Runtime with the real attested models (GitHub #375,
//! PR 1 of #361; ADR 2026-10-13 "Distribution Builds: System ONNX Runtime"; matrix SORT7).
//!
//! `initialize_pipeline` first runs the CPU execution provider check, then the camera, key,
//! store and model steps (SHA-256 and shape attestation, PAD self-test). In the normal CI jobs
//! the linked library is the downloaded prebuilt archive; in the `system-onnxruntime` CI job
//! (`--no-default-features --features system-onnxruntime`, Arch `onnxruntime-cpu`, network
//! disabled) the same test proves the daemon starts on the distribution library.
//!
//! Gating like the other real-model tests: `SOOS_MODELS_DIR` (default `/var/lib/soos/models`,
//! which must hold the deployed `manifest.toml`), `SKIPPED` without the models unless
//! `SOOS_REQUIRE_REAL_MODELS=1`. Mock camera and temporary stores only.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "Contractual test suite utilizes direct assertions and prints skip notices"
)]

use std::path::PathBuf;

use soos_daemon::config::PipelineConfig;
use soos_daemon::pipeline::initialize_pipeline;
use tempfile::tempdir;

const DEFAULT_MODELS_DIR: &str = "/var/lib/soos/models";

fn models_dir() -> PathBuf {
    std::env::var_os("SOOS_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR))
}

fn real_models_required() -> bool {
    std::env::var("SOOS_REQUIRE_REAL_MODELS").is_ok_and(|v| v == "1")
}

/// SORT7: with the deployed models, the production pipeline (CPU provider check first, then
/// attestation and the PAD self-test) starts on the linked ONNX Runtime.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_sort_daemon_pipeline_starts_on_linked_runtime_with_real_models() {
    let models = models_dir();
    if !models.join("manifest.toml").is_file() {
        assert!(
            !real_models_required(),
            "SOOS_REQUIRE_REAL_MODELS=1 but {} has no manifest.toml",
            models.display()
        );
        println!(
            "SKIPPED test_sort_daemon_pipeline_starts_on_linked_runtime_with_real_models: \
             no manifest.toml in {} (set SOOS_MODELS_DIR)",
            models.display()
        );
        return;
    }

    let dir = tempdir().expect("tempdir");
    let mut config = PipelineConfig {
        use_mock_camera: true,
        ..Default::default()
    };
    config.camera.device_path = PathBuf::from("/dev/nonexistent_mock_cam");
    config.camera.warmup_frames = 0;
    config.models_dir = models;
    config.biometrics_dir = dir.path().join("biometrics");
    config.master_key_path = dir.path().join("master.key");
    config.evidence.base_dir = dir.path().join("evidence");
    config.evidence.key_path = dir.path().join("evidence.key");

    let components = initialize_pipeline(&config)
        .expect("the daemon pipeline must start on the linked ONNX Runtime with the real models");
    components.camera.stop();
}
