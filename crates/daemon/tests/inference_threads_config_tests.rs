//! Daemon `[pipeline] inference_intra_threads` knob (GitHub #252, review finding VIS-10).
//!
//! The ONNX Runtime intra-op thread count is operator-configurable, defaults to
//! `soos_inference_ort::default_intra_threads()` (`min(4, available_parallelism)`), is
//! validated at configuration load (fail-closed on `0` or above `MAX_INTRA_THREADS`), and is
//! the value handed to the model registry by `initialize_pipeline`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Test suite utilizes direct assertions"
)]

use soos_daemon::pipeline::registry_config_for;
use soos_daemon::{DaemonConfig, DaemonError, PipelineConfig};
use soos_inference_ort::registry::{default_intra_threads, MAX_INTRA_THREADS};

#[test]
fn test_pipeline_config_defaults_to_bounded_intra_threads() {
    let config = PipelineConfig::default();
    assert_eq!(config.inference_intra_threads, default_intra_threads());
    let parsed = DaemonConfig::from_toml_str("").expect("empty config");
    assert_eq!(
        parsed.pipeline.inference_intra_threads,
        default_intra_threads()
    );
}

#[test]
fn test_pipeline_config_parses_inference_intra_threads() {
    let parsed = DaemonConfig::from_toml_str("[pipeline]\ninference_intra_threads = 2\n")
        .expect("valid thread count");
    assert_eq!(parsed.pipeline.inference_intra_threads, 2);
}

#[test]
fn test_pipeline_config_rejects_out_of_range_intra_threads() {
    for bad in [0usize, MAX_INTRA_THREADS + 1] {
        let body = format!("[pipeline]\ninference_intra_threads = {bad}\n");
        let result = DaemonConfig::from_toml_str(&body);
        assert!(
            matches!(result, Err(DaemonError::Config(_))),
            "inference_intra_threads = {bad} must be rejected, got {result:?}"
        );
    }
}

#[test]
fn test_registry_config_for_uses_pipeline_threads_and_models_dir() {
    let config = PipelineConfig {
        inference_intra_threads: 3,
        ..Default::default()
    };
    let registry = registry_config_for(&config);
    assert_eq!(registry.intra_threads, 3);
    assert_eq!(registry.inter_threads, 1);
    assert!(!registry.allow_spinning);
    assert_eq!(registry.models_dir, config.models_dir);
    assert_eq!(
        registry.manifest_path,
        config.models_dir.join("manifest.toml")
    );
}
