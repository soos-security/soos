//! ONNX Runtime thread configuration contract (GitHub #252, review finding VIS-10).
//!
//! Single-threaded ORT for SCRFD-640 plus the ResNet34 embedding is likely to exceed the
//! decision budget on low-power laptops. The registry defaults to
//! `min(DEFAULT_MAX_INTRA_THREADS, available_parallelism)` intra-op threads, bounds any
//! override to `1..=MAX_INTRA_THREADS`, and disables ORT spin-waiting so that the extra
//! threads do not burn CPU between authentication attempts.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Test suite utilizes direct assertions"
)]

use soos_inference_ort::registry::{
    default_intra_threads, RegistryConfig, DEFAULT_MAX_INTRA_THREADS, MAX_INTRA_THREADS,
};

#[test]
fn test_default_intra_threads_is_bounded_by_parallelism() {
    let parallelism = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    let expected = parallelism.clamp(1, DEFAULT_MAX_INTRA_THREADS);
    assert_eq!(default_intra_threads(), expected);
    assert_eq!(DEFAULT_MAX_INTRA_THREADS, 4);
    const { assert!(MAX_INTRA_THREADS >= DEFAULT_MAX_INTRA_THREADS) };
}

#[test]
fn test_registry_config_defaults_use_default_intra_threads_without_spinning() {
    for config in [
        RegistryConfig::new("/nonexistent/models"),
        RegistryConfig::with_manifest("/nonexistent/models", "/nonexistent/manifest.toml"),
    ] {
        assert_eq!(config.intra_threads, default_intra_threads());
        assert_eq!(config.inter_threads, 1);
        assert!(
            !config.allow_spinning,
            "ORT spin-waiting must be disabled by default"
        );
    }
}

#[test]
fn test_registry_config_with_intra_threads_is_clamped() {
    let base = RegistryConfig::new("/nonexistent/models");
    assert_eq!(base.clone().with_intra_threads(0).intra_threads, 1);
    assert_eq!(base.clone().with_intra_threads(3).intra_threads, 3);
    assert_eq!(
        base.with_intra_threads(usize::MAX).intra_threads,
        MAX_INTRA_THREADS
    );
}
