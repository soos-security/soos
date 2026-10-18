//! Contract (GitHub #429, part of #363 / #379, matrix GAK1–GAK6): `soos-gui` turns off the safe
//! top-level `remote.toml` keys of a file that ends with an `[android]` table.
//!
//! - Remote unlock, push alerts, offline alerts and the camera view (with their dependent keys)
//!   are edited line by line above `[android]`; the table and every other line stay
//!   byte-identical, and the re-parsed result keeps `[android]` intact and satisfies the
//!   cross-key rules of `soos-remote` (whose loader is not reachable: it is a leaf crate, RMC-S4).
//! - Internet access (Funnel) stays refused with a specific error, because `soos-remote`
//!   requires `allow_funnel` while `[android]` is configured (exit 78).
//! - Any other table, an array of tables, a sub-table, or a target key inside a table is
//!   refused (`RemoteEditError::UnsupportedLayout`), as before.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use soos_gui::feature_control::{
    disable_remote_keys, parse_remote_keys, FeatureAction, FeatureExecutor, FeatureOutcome,
    FeaturePaths, ProgramOutput, ProgramRunner, RemoteEditError, RemoteSwitch,
    SystemFeatureExecutor,
};
use soos_gui::onboarding::{
    toggle_route, Feature, FeatureSnapshot, RemoteConfigState, ToggleRoute, UnitState,
};
/// The keys `soos-remote` accepts at the top level of `remote.toml` (`deny_unknown_fields`,
/// `crates/remote/src/config.rs::FileConfig`, `Docs/REMOTE_COMPANION.md` §5).
const REMOTE_TOP_LEVEL_KEYS: [&str; 22] = [
    "allowed_logins",
    "socket_path",
    "poll_interval_ms",
    "allowed_hosts",
    "allow_unlock",
    "rp_id",
    "allow_funnel",
    "credentials_path",
    "password_alerts",
    "lock_screen_programs",
    "push_notifications",
    "vapid_subject",
    "push_socket_path",
    "push_previews",
    "camera_view",
    "camera_view_funnel",
    "camera_max_view_s",
    "camera_fps",
    "camera_width",
    "camera_quality",
    "battery_status",
    "offline_alerts",
];

/// What the re-parse found.
struct Loaded {
    android: Option<toml::Table>,
}

/// Re-parses `text` and checks the cross-key rules `soos-remote` enforces with exit 78
/// (`parse_config_with_android`). `soos-remote` is a leaf crate (RMC-S4), so its loader is not
/// reachable from `soos-gui` tests; these rules mirror it for the keys this edit can touch.
fn load(text: &str) -> Result<Loaded, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("not TOML: {e}"))?;
    for key in table.keys() {
        if key != "android" && !REMOTE_TOP_LEVEL_KEYS.contains(&key.as_str()) {
            return Err(format!("unknown key {key}"));
        }
    }
    let on = |key: &str| -> Result<bool, String> {
        match table.get(key) {
            None => Ok(false),
            Some(toml::Value::Boolean(b)) => Ok(*b),
            Some(_) => Err(format!("{key} is not a boolean")),
        }
    };
    let logins = table
        .get("allowed_logins")
        .and_then(toml::Value::as_array)
        .ok_or("no allowed_logins")?;
    if logins.is_empty() {
        return Err("empty allowed_logins".to_string());
    }
    let rp_id = table.get("rp_id").and_then(toml::Value::as_str).is_some();
    let funnel = on("allow_funnel")?;
    if funnel && !rp_id {
        return Err("allow_funnel needs rp_id".to_string());
    }
    let push = on("push_notifications")?;
    if push && !(on("password_alerts")? && rp_id) {
        return Err("push_notifications needs password_alerts and rp_id".to_string());
    }
    let view = on("camera_view")?;
    if view && !rp_id {
        return Err("camera_view needs rp_id".to_string());
    }
    if on("camera_view_funnel")? && !(view && funnel) {
        return Err("camera_view_funnel needs camera_view and allow_funnel".to_string());
    }
    if on("offline_alerts")? && !push {
        return Err("offline_alerts needs push_notifications".to_string());
    }
    on("allow_unlock")?;
    let android = match table.get("android") {
        None => None,
        Some(toml::Value::Table(android)) => {
            if !rp_id || !funnel {
                return Err("[android] requires rp_id and allow_funnel".to_string());
            }
            if android
                .keys()
                .any(|k| k != "package" && k != "sha256_cert_fingerprints")
            {
                return Err("unknown key in [android]".to_string());
            }
            let package = android.get("package").and_then(toml::Value::as_str);
            let prints = android
                .get("sha256_cert_fingerprints")
                .and_then(toml::Value::as_array);
            if package.is_none() || prints.is_none_or(|p| p.is_empty() || p.len() > 2) {
                return Err("[android] package or fingerprints missing".to_string());
            }
            Some(android.clone())
        }
        Some(_) => return Err("android is not a table".to_string()),
    };
    Ok(Loaded { android })
}

/// Signing-certificate digest of the fixture (`AA:AA:..`, 32 bytes).
fn fingerprint() -> String {
    vec!["AA"; 32].join(":")
}

/// The `[android]` part of the fixture, comments and blank lines included.
fn android_table() -> String {
    format!(
        "\n# Native Android app binding.\n[android]  # added by the owner\n\
         package = \"org.soos.remote\"\n\
         sha256_cert_fingerprints = [\n  \"{}\",\n]\n",
        fingerprint()
    )
}

/// The top-level keys of the fixture: every companion feature on (setup profile).
const TOP: &str = "# Managed by hand.\n\
allowed_logins = [\"owner.login@example.com\"]\n\
socket_path = \"/run/user/1000/soos-remote/remote.sock\"\n\
rp_id = \"pc.tail1234.ts.net\"\n\
allow_funnel = true\n\
allow_unlock = true   # phone unlock\n\
password_alerts = true\n\
push_notifications = true\n\
offline_alerts = true\n\
camera_view = true\n\
camera_view_funnel = true\n";

fn sample() -> String {
    format!("{TOP}{}", android_table())
}

/// The fixture satisfies the `soos-remote` rules, with its `[android]` binding.
#[test]
fn test_gak1_fixture_is_a_valid_android_configuration() {
    let loaded = load(&sample()).unwrap();
    assert!(loaded.android.is_some());
    // The rule checker rejects what soos-remote rejects with exit 78: Funnel off under
    // [android], and a dangling dependent key.
    for broken in [
        sample().replace("allow_funnel = true", "allow_funnel = false"),
        sample().replace("push_notifications = true", "push_notifications = false"),
        sample().replace("camera_view = true", "camera_view = false"),
        format!("{}extra = 1\n", sample()),
    ] {
        assert!(load(&broken).is_err(), "{broken}");
    }
}

#[test]
fn test_gak2_safe_switches_edit_keys_above_android() {
    let original = sample();
    let table = android_table();
    for switch in [
        RemoteSwitch::Unlock,
        RemoteSwitch::Push,
        RemoteSwitch::OfflineAlerts,
        RemoteSwitch::CameraView,
    ] {
        let keys = switch.keys_to_clear();
        let out = disable_remote_keys(&original, keys)
            .unwrap_or_else(|e| panic!("{switch:?}: {e}"))
            .unwrap_or_else(|| panic!("{switch:?}: the key was on, the file must change"));

        // The `[android]` table and everything after it are byte-identical.
        assert!(
            out.ends_with(&table),
            "{switch:?}: [android] changed:\n{out}"
        );

        // Only the target `key = true` lines change, to `key = false` (comment kept).
        let before: Vec<&str> = original.lines().collect();
        let after: Vec<&str> = out.lines().collect();
        assert_eq!(before.len(), after.len(), "{switch:?}");
        for (b, a) in before.iter().zip(&after) {
            let key = b.split('=').next().unwrap_or("").trim();
            if keys.contains(&key) && b.contains("true") {
                assert_eq!(*a, b.replacen("true", "false", 1), "{switch:?}");
            } else {
                assert_eq!(b, a, "{switch:?}: unrelated line changed");
            }
        }

        // The re-parsed result keeps the binding and every soos-remote cross-key rule.
        let loaded = load(&out)
            .unwrap_or_else(|e| panic!("{switch:?}: the edit breaks a soos-remote rule: {e}"));
        assert!(loaded.android.is_some(), "{switch:?}");
        let RemoteConfigState::Keys(state) = parse_remote_keys(&out) else {
            panic!("{switch:?}: edited file must parse");
        };
        assert!(state.allow_funnel, "{switch:?}: allow_funnel must stay on");
        match switch {
            RemoteSwitch::Unlock => assert!(!state.allow_unlock && state.push_notifications),
            RemoteSwitch::Push => assert!(!state.push_notifications && !state.offline_alerts),
            RemoteSwitch::OfflineAlerts => {
                assert!(!state.offline_alerts && state.push_notifications);
            }
            RemoteSwitch::CameraView => {
                assert!(!state.camera_view && !state.camera_view_funnel && state.allow_unlock);
            }
            RemoteSwitch::Funnel => panic!("Funnel is not a safe switch"),
        }
        let edited: toml::Table = out.parse().unwrap();
        let source: toml::Table = original.parse().unwrap();
        assert_eq!(edited.get("android"), source.get("android"), "{switch:?}");
    }
}

#[test]
fn test_gak2_already_off_above_android_writes_nothing() {
    let text = sample().replace("allow_unlock = true", "allow_unlock = false");
    assert_eq!(
        disable_remote_keys(&text, RemoteSwitch::Unlock.keys_to_clear()),
        Ok(None)
    );
    let text = format!(
        "allowed_logins = [\"me@example.com\"]\nrp_id = \"pc.tail1234.ts.net\"\n\
         allow_funnel = true\n{}",
        android_table()
    );
    assert_eq!(
        disable_remote_keys(&text, RemoteSwitch::CameraView.keys_to_clear()),
        Ok(None),
        "an absent key is already off and is never added"
    );
}

#[test]
fn test_gak3_funnel_is_refused_while_android_is_configured() {
    let original = sample();
    let result = disable_remote_keys(&original, RemoteSwitch::Funnel.keys_to_clear());
    assert_eq!(result, Err(RemoteEditError::FunnelRequiredByAndroid));
    let message = RemoteEditError::FunnelRequiredByAndroid.to_string();
    for needle in ["[android]", "allow_funnel", "Internet access"] {
        assert!(message.contains(needle), "missing {needle:?} in {message}");
    }
    // Already off or absent: still refused (soos-remote refuses that file anyway).
    let off = original.replace("allow_funnel = true", "allow_funnel = false");
    assert_eq!(
        disable_remote_keys(&off, RemoteSwitch::Funnel.keys_to_clear()),
        Err(RemoteEditError::FunnelRequiredByAndroid)
    );
    // Without `[android]`, Funnel is still turned off as before.
    let out = disable_remote_keys(TOP, RemoteSwitch::Funnel.keys_to_clear())
        .unwrap()
        .unwrap();
    assert!(out.contains("\nallow_funnel = false\n"), "{out}");
}

#[test]
fn test_gak4_other_tables_and_keys_inside_tables_are_refused() {
    let table = android_table();
    let refused = [
        // Another table, after or before `[android]`.
        format!("{TOP}{table}\n[extra]\nx = 1\n"),
        format!("{TOP}[extra]\nx = 1\n{table}"),
        // A sub-table of `android`, or an array of tables.
        format!("{TOP}{table}\n[android.extra]\nx = 1\n"),
        format!("{TOP}\n[[extra]]\nx = 1\n"),
        // A target key inside `[android]` (on or off), even with the top-level key on.
        format!("{TOP}{table}allow_unlock = true\n"),
        format!("{TOP}{table}allow_unlock = false\n"),
    ];
    for text in &refused {
        assert_eq!(
            disable_remote_keys(text, RemoteSwitch::Unlock.keys_to_clear()),
            Err(RemoteEditError::UnsupportedLayout),
            "{text}"
        );
    }
    // The same rule for every switch whose key sits in the table.
    let text = format!("{TOP}{table}camera_view = true\n");
    assert_eq!(
        disable_remote_keys(&text, RemoteSwitch::CameraView.keys_to_clear()),
        Err(RemoteEditError::UnsupportedLayout)
    );
}

#[test]
fn test_gak4_android_header_spelling_variants() {
    let body = format!(
        "package = \"org.soos.remote\"\nsha256_cert_fingerprints = [\"{}\"]\n",
        fingerprint()
    );
    for header in ["[android]", "[ android ]", "  [android]\t# app"] {
        let text = format!("{TOP}\n{header}\n{body}");
        let out = disable_remote_keys(&text, RemoteSwitch::Unlock.keys_to_clear())
            .unwrap_or_else(|e| panic!("{header:?}: {e}"))
            .unwrap();
        assert!(out.ends_with(&format!("\n{header}\n{body}")), "{header:?}");
        assert!(load(&out).unwrap().android.is_some());
    }
    // A quoted header names the same table, but the GUI does not edit that spelling.
    let text = format!("{TOP}\n[\"android\"]\n{body}");
    assert_eq!(
        disable_remote_keys(&text, RemoteSwitch::Unlock.keys_to_clear()),
        Err(RemoteEditError::UnsupportedLayout)
    );
}

// ---------------------------------------------------------------------------------------------
// Executor and routing
// ---------------------------------------------------------------------------------------------

type Call = (String, Vec<String>, Option<Duration>);

#[derive(Default)]
struct RecordingRunner {
    calls: Mutex<Vec<Call>>,
}

impl ProgramRunner for RecordingRunner {
    fn run(
        &self,
        program: &str,
        args: &[String],
        deadline: Option<Duration>,
        _max_stdout: usize,
    ) -> Result<ProgramOutput, String> {
        self.calls
            .lock()
            .unwrap()
            .push((program.to_string(), args.to_vec(), deadline));
        Ok(ProgramOutput {
            success: true,
            code: Some(0),
            stdout: Vec::new(),
        })
    }
}

fn executor(dir: &tempfile::TempDir) -> (SystemFeatureExecutor, PathBuf, Arc<RecordingRunner>) {
    let etc = dir.path().join("etc-soos");
    std::fs::create_dir(&etc).unwrap();
    let config = dir.path().join("remote.toml");
    let paths = FeaturePaths {
        daemon_config: etc.join("daemon.toml"),
        etc_dir: etc,
        network_lock_rule: dir.path().join("49-soos-network-lock.rules"),
        network_lock_rfkill: dir.path().join("72-soos-network-lock-rfkill.rules"),
        remote_config: Some(config.clone()),
        running_as_root: false,
        uid: 1000,
    };
    let runner = Arc::new(RecordingRunner::default());
    (
        SystemFeatureExecutor::new(paths, runner.clone()),
        config,
        runner,
    )
}

fn applied(outcome: FeatureOutcome) -> Result<(), String> {
    let FeatureOutcome::Applied { result, .. } = outcome else {
        return Err("expected an applied outcome, got a refresh".to_string());
    };
    result
}

#[test]
fn test_gak5_executor_edits_above_android_and_refuses_funnel() {
    let dir = tempfile::tempdir().unwrap();
    let (executor, config, runner) = executor(&dir);
    std::fs::write(&config, sample()).unwrap();

    assert_eq!(
        applied(executor.execute(FeatureAction::DisableRemote(RemoteSwitch::Unlock))),
        Ok(())
    );
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(
        text.contains("\nallow_unlock = false   # phone unlock\n"),
        "{text}"
    );
    assert!(text.ends_with(&android_table()), "{text}");
    assert!(load(&text).unwrap().android.is_some());
    assert_eq!(
        std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(runner.calls.lock().unwrap().len(), 1, "one try-restart");

    let error =
        applied(executor.execute(FeatureAction::DisableRemote(RemoteSwitch::Funnel))).unwrap_err();
    assert!(error.contains("[android]"), "{error}");
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        text,
        "nothing written"
    );
    assert_eq!(
        runner.calls.lock().unwrap().len(),
        1,
        "no restart after a refusal"
    );
}

#[test]
fn test_gak6_funnel_off_command_names_the_android_requirement() {
    let snapshot = FeatureSnapshot {
        remote_unit: UnitState::Active,
        remote_config: RemoteConfigState::Missing,
        running_as_root: true,
        ..FeatureSnapshot::unknown()
    };
    let ToggleRoute::Command(text) = toggle_route(&snapshot, Feature::InternetAccess, false) else {
        panic!("expected a command");
    };
    assert!(text.contains("allow_funnel = false"), "{text}");
    assert!(text.contains("[android]"), "{text}");
    // Other switches keep their plain command.
    let ToggleRoute::Command(text) = toggle_route(&snapshot, Feature::RemoteUnlock, false) else {
        panic!("expected a command");
    };
    assert!(!text.contains("[android]"), "{text}");
}
