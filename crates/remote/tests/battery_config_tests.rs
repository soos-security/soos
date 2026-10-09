//! Contract test of GitHub #346 for the `battery_status` configuration key of `soos-remote`
//! (ADR 2026-10-07 "Live Battery Level in `soos-remote`" item (4), architect spec
//! `AI/architect_spec_remote_battery.md` §4, §10.2 test 18; matrix RBS1).
//!
//! The spec maps test 18 to `config_tests.rs`; it lives in this new file (spec name
//! unchanged, the #345 precedent) so the existing configuration suite keeps compiling while
//! the new API is missing. Pure parsing over text; nothing touches `/etc` or `~/.config`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::path::Path;

use soos_remote::config::{parse_config, BatteryConfig, ConfigError, RemoteConfig};

const RUNTIME_DIR: &str = "/run/user/1000";
const MINIMAL: &str = "allowed_logins = [\"owner@example.com\"]\n";

fn parse(text: &str) -> Result<RemoteConfig, ConfigError> {
    parse_config(text, Some(Path::new(RUNTIME_DIR)))
}

/// Test 18 (RBS1, §4): `battery_status` is a TOML boolean, absent → on; anything else is a
/// syntax error (exit 78); it depends on no other key.
#[test]
fn test_rbs_battery_config_key() {
    assert!(
        BatteryConfig::default().enabled,
        "on by default (ADR item (4))"
    );
    assert_eq!(BatteryConfig::default(), BatteryConfig { enabled: true });

    let absent = parse(MINIMAL).unwrap();
    assert_eq!(absent.battery, BatteryConfig { enabled: true });

    let off = parse(&format!("{MINIMAL}battery_status = false\n")).unwrap();
    assert_eq!(off.battery, BatteryConfig { enabled: false });
    let on = parse(&format!("{MINIMAL}battery_status = true\n")).unwrap();
    assert_eq!(on.battery, BatteryConfig { enabled: true });

    for bad in ["\"yes\"", "1", "[]", "\"true\"", "0", "{}"] {
        assert_eq!(
            parse(&format!("{MINIMAL}battery_status = {bad}\n")).map(|c| c.battery),
            Err(ConfigError::Syntax),
            "battery_status = {bad}"
        );
    }
    assert_eq!(
        parse(&format!(
            "{MINIMAL}battery_status = false\nbattery_status = true\n"
        ))
        .map(|c| c.battery),
        Err(ConfigError::Syntax),
        "a duplicated key is a TOML error"
    );

    // Independent of rp_id / allow_funnel / alerts / push.
    let funnel = "allowed_logins = [\"owner@example.com\"]\n\
                  rp_id = \"pc.tail1234.ts.net\"\n\
                  allow_funnel = true\n";
    assert_eq!(
        parse(funnel).unwrap().battery,
        BatteryConfig { enabled: true }
    );
    assert_eq!(
        parse(&format!("{funnel}battery_status = false\n"))
            .unwrap()
            .battery,
        BatteryConfig { enabled: false }
    );
    let funnel_config = parse(&format!("{funnel}battery_status = false\n")).unwrap();
    assert!(funnel_config.auth.allow_funnel);
    assert_eq!(
        funnel_config.auth.rp_id.as_deref(),
        Some("pc.tail1234.ts.net")
    );
    // Turning the battery off changes nothing else.
    let mut expected = parse(MINIMAL).unwrap();
    expected.battery = BatteryConfig { enabled: false };
    assert_eq!(off, expected);
}
