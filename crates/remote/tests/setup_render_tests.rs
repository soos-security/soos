//! The `remote.toml` written by `scripts/setup.sh` (GitHub #400, PR 2 of #362; ADR 2026-10-15
//! "Setup User Phase: Phone Companion Profile With Default-On Features"; row SET38) is accepted
//! by the real `soos-remote` parser with every owner default on, and with every variant the
//! setup cascade can write; a variant the cascade never writes is refused.
//!
//! The fixture `tests/fixtures/setup/remote.toml` is byte-equal to setup's render (asserted by
//! `soos-invariants::setup_user_phase_contract`). Pure parsing; nothing touches the host.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::path::Path;

use soos_remote::config::{parse_config, parse_config_with_android, ConfigError};

const RUNTIME_DIR: &str = "/run/user/1000";

fn fixture() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/setup/remote.toml");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The fixture with `key = true` turned to `key = false` for each listed key.
fn with_off(keys: &[&str]) -> String {
    let mut text = fixture();
    for key in keys {
        let from = format!("\n{key} = true\n");
        assert!(text.contains(&from), "{key} is on in the fixture");
        text = text.replace(&from, &format!("\n{key} = false\n"));
    }
    text
}

#[test]
fn test_set_render_with_every_default_on_is_accepted() {
    let config = parse_config(&fixture(), Some(Path::new(RUNTIME_DIR))).expect("render accepted");
    assert_eq!(config.allowed_logins.len(), 1);
    assert_eq!(config.allowed_logins[0].as_str(), "owner.login@example.com");
    assert_eq!(config.auth.rp_id.as_deref(), Some("pc.tail1234.ts.net"));
    assert!(config.auth.allow_funnel);
    assert!(config.allow_unlock);
    assert!(config.alerts.enabled);
    assert!(config.push.enabled);
    assert!(config.offline.enabled);
    assert!(config.camera.enabled);
    assert!(config.camera.funnel);
}

#[test]
fn test_set_render_cascade_variants_are_accepted() {
    for keys in [
        &["offline_alerts"][..],
        &["push_notifications", "offline_alerts"],
        &["password_alerts", "push_notifications", "offline_alerts"],
        &["allow_funnel", "camera_view_funnel"],
        &["camera_view", "camera_view_funnel"],
        &["camera_view_funnel"],
        &["allow_unlock"],
        &[
            "allow_funnel",
            "allow_unlock",
            "password_alerts",
            "push_notifications",
            "offline_alerts",
            "camera_view",
            "camera_view_funnel",
        ],
    ] {
        parse_config(&with_off(keys), Some(Path::new(RUNTIME_DIR)))
            .unwrap_or_else(|e| panic!("{keys:?} off must be accepted: {e}"));
    }
}

#[test]
fn test_set_render_never_written_variants_are_refused() {
    // Offline alerts without push, push without password alerts, camera over Funnel without
    // Funnel: setup never writes these (prerequisite cascade), and soos-remote refuses them.
    for keys in [
        &["push_notifications"][..],
        &["password_alerts"],
        &["allow_funnel"],
        &["camera_view"],
    ] {
        let err = parse_config(&with_off(keys), Some(Path::new(RUNTIME_DIR)))
            .expect_err("a broken prerequisite is refused");
        assert!(
            !matches!(err, ConfigError::Syntax),
            "{keys:?}: refused for its prerequisite, not its syntax"
        );
    }
}

/// SDP25 (GitHub #362 PR A, spec §6.3): the managed `remote.toml` with the trailing `[android]`
/// table that setup renders (`tests/fixtures/setup/remote_android.toml`, byte-equal to the
/// render, asserted by `soos-invariants::setup_deps_contract::test_sdp23`) is accepted by the
/// real parser, binds `org.soos.remote` to both fingerprints in file order, and publishes them
/// in the Digital Asset Links body.
#[test]
fn test_sdp25_android_render_is_accepted() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/setup/remote_android.toml");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let loaded = parse_config_with_android(&text, Some(Path::new(RUNTIME_DIR)))
        .expect("the [android] render is accepted");
    assert!(loaded.remote.auth.allow_funnel);
    let android = loaded.android.expect("[android] parsed");
    assert_eq!(android.package(), "org.soos.remote");
    let digests = android.cert_fingerprints();
    assert_eq!(digests.len(), 2);
    assert_eq!(digests[0][0], 0xAA);
    assert_eq!(digests[1][0], 0x01);
    let body = String::from_utf8(android.asset_links_json()).expect("utf-8");
    for fp in [
        "AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99",
        "01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF",
    ] {
        assert!(body.contains(fp), "assetlinks names {fp}: {body}");
    }
    assert!(body.contains("\"org.soos.remote\""), "{body}");
}
