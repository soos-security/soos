//! Contractual tests for GitHub #276 (DMN-03 remainder): the global daily evidence snapshot
//! cap is configurable as `[pipeline.evidence] daily_cap_total` and defaults to
//! `soos_evidence_store::DEFAULT_DAILY_CAP_TOTAL`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use soos_daemon::config::{DaemonConfig, PipelineConfig};
use soos_evidence_store::DEFAULT_DAILY_CAP_TOTAL;

#[test]
fn test_global_evidence_cap_defaults_to_the_store_default() {
    assert_eq!(
        PipelineConfig::default().evidence_daily_cap_total,
        DEFAULT_DAILY_CAP_TOTAL
    );
    let config = DaemonConfig::from_toml_str("[pipeline.evidence]\nenabled = true\n").unwrap();
    assert_eq!(
        config.pipeline.evidence_daily_cap_total,
        DEFAULT_DAILY_CAP_TOTAL
    );
}

#[test]
fn test_global_evidence_cap_is_read_from_toml() {
    let config = DaemonConfig::from_toml_str(
        "[pipeline.evidence]\nenabled = true\ndaily_cap_per_uid = 4\ndaily_cap_total = 25\n",
    )
    .unwrap();
    assert_eq!(config.pipeline.evidence_daily_cap_total, 25);
    assert_eq!(config.pipeline.evidence.daily_cap_per_uid, 4);
}
