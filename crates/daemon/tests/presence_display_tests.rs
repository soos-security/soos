//! Contract tests of GitHub #323 for the presence gating inputs that do not need logind:
//! the DRM display classifier / sysfs probe (matrix PAU9) and the kill switch (PAU4).
//!
//! Every filesystem input is a tempdir tree (no real `/sys` or `/etc`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use soos_daemon::presence::display::{
    classify_connectors, DisplayProbe, DisplayState, SysfsDisplayProbe,
};
use soos_daemon::presence::switch::PresenceSwitch;
use soos_daemon::presence::{
    GLOBAL_DISABLE_FLAG, MAX_DRM_CONNECTORS, MAX_SYSFS_ATTR_BYTES, PRESENCE_DISABLE_FLAG,
};

// ---------------------------------------------------------------------------------------
// PAU9 — pure classifier
// ---------------------------------------------------------------------------------------

/// PAU9: `On` iff some connected connector is `On`; `Off` iff at least one is connected and
/// none is `On`; `Unknown` without a connected connector.
#[test]
fn test_pau_classify_connectors_table() {
    let cases: [(&[(&str, &str)], DisplayState); 9] = [
        (&[], DisplayState::Unknown),
        (&[("connected", "On")], DisplayState::On),
        (&[("connected", "Off")], DisplayState::Off),
        (&[("connected", "Standby")], DisplayState::Off),
        (&[("connected", "Suspend")], DisplayState::Off),
        (&[("disconnected", "On")], DisplayState::Unknown),
        (&[("unknown", "On")], DisplayState::Unknown),
        (
            &[("connected", "Off"), ("connected", "On")],
            DisplayState::On,
        ),
        (
            &[("connected", "Off"), ("disconnected", "On")],
            DisplayState::Off,
        ),
    ];
    for (connectors, expected) in cases {
        assert_eq!(
            classify_connectors(connectors),
            expected,
            "connectors {connectors:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// PAU9 — sysfs probe
// ---------------------------------------------------------------------------------------

fn connector(dir: &Path, name: &str, status: &str, dpms: &str) {
    let path = dir.join(name);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("status"), status).unwrap();
    fs::write(path.join("dpms"), dpms).unwrap();
}

/// PAU9: a connected, DPMS-on eDP panel reads `On` (attributes carry a trailing newline).
#[test]
fn test_pau_sysfs_probe_reads_a_connected_panel() {
    let temp = tempfile::tempdir().unwrap();
    connector(temp.path(), "card1-eDP-1", "connected\n", "On\n");
    connector(temp.path(), "card1-HDMI-A-1", "disconnected\n", "Off\n");
    fs::write(temp.path().join("version"), "drm 1.1.0 20060810\n").unwrap();
    let probe = SysfsDisplayProbe::new(temp.path().to_path_buf());
    assert_eq!(probe.display_state(), DisplayState::On);
}

/// PAU9: every connected connector DPMS-off reads `Off` (gates the scan).
#[test]
fn test_pau_sysfs_probe_reads_a_blanked_screen_as_off() {
    let temp = tempfile::tempdir().unwrap();
    connector(temp.path(), "card0-eDP-1", "connected\n", "Off\n");
    connector(temp.path(), "card0-DP-1", "connected\n", "Off\n");
    let probe = SysfsDisplayProbe::new(temp.path().to_path_buf());
    assert_eq!(probe.display_state(), DisplayState::Off);
}

/// PAU9: `renderD*`, bare `cardN`, `version` and names outside `card[0-9]+-[A-Za-z0-9-]+`
/// are ignored even when they look connected and on.
#[test]
fn test_pau_sysfs_probe_ignores_non_connector_entries() {
    let temp = tempfile::tempdir().unwrap();
    connector(temp.path(), "card0-eDP-1", "connected\n", "Off\n");
    connector(temp.path(), "renderD128", "connected\n", "On\n");
    connector(temp.path(), "card0", "connected\n", "On\n");
    connector(temp.path(), "cardX-eDP-1", "connected\n", "On\n");
    connector(temp.path(), "card0-", "connected\n", "On\n");
    connector(temp.path(), "card0-eDP 2", "connected\n", "On\n");
    connector(temp.path(), "card0-eDP_3", "connected\n", "On\n");
    connector(temp.path(), "xcard0-DP-1", "connected\n", "On\n");
    fs::write(temp.path().join("version"), "connected\n").unwrap();
    let probe = SysfsDisplayProbe::new(temp.path().to_path_buf());
    assert_eq!(
        probe.display_state(),
        DisplayState::Off,
        "only card0-eDP-1 is a connector; it is off"
    );
}

/// PAU9: an attribute longer than `MAX_SYSFS_ATTR_BYTES` makes that connector ignored.
#[test]
fn test_pau_sysfs_probe_ignores_oversized_attributes() {
    let temp = tempfile::tempdir().unwrap();
    connector(temp.path(), "card0-eDP-1", "connected\n", "Off\n");
    let padded_status = format!("connected{}\n", " ".repeat(MAX_SYSFS_ATTR_BYTES));
    assert!(padded_status.len() > MAX_SYSFS_ATTR_BYTES);
    connector(temp.path(), "card0-DP-1", &padded_status, "On\n");
    let padded_dpms = format!("On{}\n", " ".repeat(MAX_SYSFS_ATTR_BYTES));
    connector(temp.path(), "card0-DP-2", "connected\n", &padded_dpms);
    let probe = SysfsDisplayProbe::new(temp.path().to_path_buf());
    assert_eq!(probe.display_state(), DisplayState::Off);
}

/// PAU9: sysfs class entries are symlinks; they are followed.
#[test]
fn test_pau_sysfs_probe_follows_class_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let devices = temp.path().join("devices");
    let class = temp.path().join("class-drm");
    fs::create_dir_all(&class).unwrap();
    connector(&devices, "card1-eDP-1", "connected\n", "On\n");
    symlink(devices.join("card1-eDP-1"), class.join("card1-eDP-1")).unwrap();
    let probe = SysfsDisplayProbe::new(class);
    assert_eq!(probe.display_state(), DisplayState::On);
}

/// PAU9: unreadable / missing directory, no connected connector, or more than
/// `MAX_DRM_CONNECTORS` entries ⇒ `Unknown` (which never gates).
#[test]
fn test_pau_sysfs_probe_unknown_cases() {
    let temp = tempfile::tempdir().unwrap();
    let missing = SysfsDisplayProbe::new(temp.path().join("absent"));
    assert_eq!(missing.display_state(), DisplayState::Unknown);

    let not_a_dir = temp.path().join("file");
    fs::write(&not_a_dir, "x").unwrap();
    assert_eq!(
        SysfsDisplayProbe::new(not_a_dir).display_state(),
        DisplayState::Unknown
    );

    let empty = temp.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    assert_eq!(
        SysfsDisplayProbe::new(empty.clone()).display_state(),
        DisplayState::Unknown
    );
    connector(&empty, "card0-HDMI-A-1", "disconnected\n", "Off\n");
    assert_eq!(
        SysfsDisplayProbe::new(empty).display_state(),
        DisplayState::Unknown
    );

    let crowded = temp.path().join("crowded");
    fs::create_dir_all(&crowded).unwrap();
    for i in 0..=MAX_DRM_CONNECTORS {
        connector(&crowded, &format!("card0-DP-{i}"), "connected\n", "On\n");
    }
    assert_eq!(
        SysfsDisplayProbe::new(crowded).display_state(),
        DisplayState::Unknown,
        "more than MAX_DRM_CONNECTORS entries is Unknown, never On"
    );
}

/// PAU9: exactly `MAX_DRM_CONNECTORS` entries are still examined.
#[test]
fn test_pau_sysfs_probe_examines_up_to_the_entry_bound() {
    let temp = tempfile::tempdir().unwrap();
    for i in 0..MAX_DRM_CONNECTORS {
        connector(
            temp.path(),
            &format!("card0-DP-{i}"),
            "connected\n",
            "Off\n",
        );
    }
    assert_eq!(
        SysfsDisplayProbe::new(temp.path().to_path_buf()).display_state(),
        DisplayState::Off
    );
}

// ---------------------------------------------------------------------------------------
// PAU4 — kill switch
// ---------------------------------------------------------------------------------------

/// PAU4: no flag ⇒ not engaged; a missing directory (`NotFound`) ⇒ not engaged.
#[test]
fn test_pau_kill_switch_is_off_without_flags() {
    let temp = tempfile::tempdir().unwrap();
    assert!(!PresenceSwitch::new(temp.path().to_path_buf()).is_engaged());
    assert!(!PresenceSwitch::new(temp.path().join("absent")).is_engaged());
}

/// PAU4: either flag as a regular file, a directory or a dangling symlink engages it.
#[test]
fn test_pau_kill_switch_engages_on_any_entry_kind() {
    for flag in [GLOBAL_DISABLE_FLAG, PRESENCE_DISABLE_FLAG] {
        for kind in ["file", "dir", "dangling-symlink", "symlink-to-file"] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join(flag);
            match kind {
                "file" => fs::write(&path, "").unwrap(),
                "dir" => fs::create_dir(&path).unwrap(),
                "dangling-symlink" => symlink(temp.path().join("nowhere"), &path).unwrap(),
                _ => {
                    let target = temp.path().join("target");
                    fs::write(&target, "").unwrap();
                    symlink(&target, &path).unwrap();
                }
            }
            assert!(
                PresenceSwitch::new(temp.path().to_path_buf()).is_engaged(),
                "{flag} as {kind} must engage the kill switch"
            );
        }
    }
}

/// PAU4: a stat error other than `NotFound` fails closed toward "disabled" (here
/// `ENOTDIR`: the flag directory is a regular file).
#[test]
fn test_pau_kill_switch_fails_closed_on_stat_errors() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("not-a-dir");
    fs::write(&file, "").unwrap();
    assert!(
        PresenceSwitch::new(file).is_engaged(),
        "a stat error other than NotFound must engage the kill switch"
    );
}

/// D9 (owner answer Q3): `gdm.disable` and per-service flags do not stop presence.
#[test]
fn test_pau_gdm_disable_does_not_engage_the_kill_switch() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("gdm.disable"), "").unwrap();
    fs::write(temp.path().join("sudo.disable"), "").unwrap();
    fs::write(temp.path().join("presence.disabled"), "").unwrap();
    assert!(!PresenceSwitch::new(temp.path().to_path_buf()).is_engaged());
}

/// PAU4: the switch is re-evaluated on every call (removal takes effect, no restart).
#[test]
fn test_pau_kill_switch_is_re_evaluated_on_every_call() {
    let temp = tempfile::tempdir().unwrap();
    let switch = PresenceSwitch::new(temp.path().to_path_buf());
    let flag = temp.path().join(PRESENCE_DISABLE_FLAG);
    assert!(!switch.is_engaged());
    fs::write(&flag, "").unwrap();
    assert!(switch.is_engaged());
    fs::remove_file(&flag).unwrap();
    assert!(!switch.is_engaged());
}
