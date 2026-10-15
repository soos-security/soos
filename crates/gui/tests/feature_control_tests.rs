//! Contract (GitHub #379, PR 1 of #363, matrix OBT8–OBT14): how the GUI reads and changes the
//! state of each feature.
//!
//! - Root-owned state is changed only through `pkexec` with fixed argument vectors, programs by
//!   absolute path, and a closed set of two flag paths; the Wi-Fi lock through the existing
//!   `soos-admin network-lock install|uninstall` (never `--rfkill`, never `--root`).
//! - The user companion unit through `systemctl --user` (never as root).
//! - `remote.toml` keys are only ever turned off, by an exact line edit verified by a TOML
//!   parse, written atomically with mode 0600, never through a symbolic link; nothing else in
//!   the file changes.
//! - Probes never write and never run `pkexec`.
//! - One action at a time, off the UI thread.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

mod common;

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{wait_until, SETTLE_TIMEOUT};
use soos_gui::feature_control::{
    companion_command, companion_restart_command, disable_remote_keys, network_lock_command,
    parse_daemon_keys, parse_remote_keys, parse_unit_show, probe_snapshot, read_remote_config,
    remote_config_path, system_flag_command, unit_show_command, write_user_file_atomically,
    FeatureAction, FeatureExecutor, FeatureOutcome, FeaturePaths, FeatureRunner,
    FeatureRunnerError, ProgramOutput, ProgramRunner, RemoteEditError, RemoteSwitch,
    SystemFeatureExecutor, SystemFlag, DAEMON_CONFIG_PATH, GLOBAL_DISABLE_FLAG,
    MAX_REMOTE_CONFIG_BYTES, NETWORK_LOCK_RFKILL_PATH, NETWORK_LOCK_RULE_PATH,
    PRESENCE_DISABLE_FLAG, REMOTE_UNIT, SOOS_ETC_DIR, USER_SYSTEMCTL_TIMEOUT,
};
use soos_gui::onboarding::{RemoteConfigState, RemoteKeys, UnitState};
use soos_gui::privileged::{
    PKEXEC_PROGRAM, RM_PROGRAM, SOOS_ADMIN_PROGRAM, SYSTEMCTL_PROGRAM, TOUCH_PROGRAM,
};

fn workspace_file(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------------------------------------
// Programs and argument vectors
// ---------------------------------------------------------------------------------------------

#[test]
fn test_obt_programs_are_absolute() {
    assert_eq!(TOUCH_PROGRAM, "/usr/bin/touch");
    assert_eq!(RM_PROGRAM, "/usr/bin/rm");
    let expected_admin = match option_env!("SOOS_BINDIR") {
        None => "/usr/bin/soos-admin".to_string(),
        Some(dir) => format!("{dir}/soos-admin"),
    };
    assert_eq!(SOOS_ADMIN_PROGRAM, expected_admin);
    let privileged = workspace_file("crates/gui/src/privileged.rs");
    assert!(privileged.contains("pub const SOOS_ADMIN_PROGRAM: &str = \"/usr/bin/soos-admin\";"));
    assert!(privileged.contains(
        "pub const SOOS_ADMIN_PROGRAM: &str = concat!(env!(\"SOOS_BINDIR\"), \"/soos-admin\");"
    ));
}

#[test]
fn test_obt_flag_paths_are_the_closed_daemon_set() {
    assert_eq!(SOOS_ETC_DIR, "/etc/soos");
    assert_eq!(
        SystemFlag::Global.path(),
        PathBuf::from("/etc/soos/disabled")
    );
    assert_eq!(
        SystemFlag::Presence.path(),
        PathBuf::from("/etc/soos/presence.disable")
    );
    // Same names as the daemon (single meaning of the flags, ADR 2026-10-02 presence).
    let daemon = workspace_file("crates/daemon/src/presence/mod.rs");
    assert!(daemon.contains(&format!(
        "pub const GLOBAL_DISABLE_FLAG: &str = \"{GLOBAL_DISABLE_FLAG}\";"
    )));
    assert!(daemon.contains(&format!(
        "pub const PRESENCE_DISABLE_FLAG: &str = \"{PRESENCE_DISABLE_FLAG}\";"
    )));
}

#[test]
fn test_obt_system_flag_commands_are_fixed_pkexec_vectors() {
    let (program, args) = system_flag_command(SystemFlag::Presence, true);
    assert_eq!(program, PKEXEC_PROGRAM);
    assert_eq!(args, vec![TOUCH_PROGRAM, "/etc/soos/presence.disable"]);
    let (program, args) = system_flag_command(SystemFlag::Presence, false);
    assert_eq!(program, PKEXEC_PROGRAM);
    assert_eq!(
        args,
        vec![RM_PROGRAM, "-f", "--", "/etc/soos/presence.disable"]
    );
    let (_, args) = system_flag_command(SystemFlag::Global, true);
    assert_eq!(args, vec![TOUCH_PROGRAM, "/etc/soos/disabled"]);
    let (_, args) = system_flag_command(SystemFlag::Global, false);
    assert_eq!(args, vec![RM_PROGRAM, "-f", "--", "/etc/soos/disabled"]);
}

#[test]
fn test_obt_network_lock_command_uses_the_admin_cli_without_options() {
    let (program, args) = network_lock_command(true);
    assert_eq!(program, PKEXEC_PROGRAM);
    assert_eq!(args, vec![SOOS_ADMIN_PROGRAM, "network-lock", "install"]);
    let (program, args) = network_lock_command(false);
    assert_eq!(program, PKEXEC_PROGRAM);
    assert_eq!(args, vec![SOOS_ADMIN_PROGRAM, "network-lock", "uninstall"]);
    for installed in [true, false] {
        let (_, args) = network_lock_command(installed);
        assert!(!args
            .iter()
            .any(|a| a.contains("rfkill") || a.contains("--root")));
    }
}

#[test]
fn test_obt_network_lock_rule_path_matches_the_admin_cli() {
    let source = workspace_file("crates/admin-cli/src/network_lock.rs");
    assert!(source.contains("pub const NETWORK_LOCK_RULES_DIR: &str = \"etc/polkit-1/rules.d\";"));
    assert!(source
        .contains("pub const NETWORK_LOCK_RULES_FILE: &str = \"49-soos-network-lock.rules\";"));
    assert_eq!(
        NETWORK_LOCK_RULE_PATH,
        "/etc/polkit-1/rules.d/49-soos-network-lock.rules"
    );
    assert!(source.contains("pub const NETWORK_LOCK_RFKILL_DIR: &str = \"etc/udev/rules.d\";"));
    assert!(source.contains(
        "pub const NETWORK_LOCK_RFKILL_FILE: &str = \"72-soos-network-lock-rfkill.rules\";"
    ));
    assert_eq!(
        NETWORK_LOCK_RFKILL_PATH,
        "/etc/udev/rules.d/72-soos-network-lock-rfkill.rules"
    );
    assert_eq!(DAEMON_CONFIG_PATH, "/etc/soos/daemon.toml");
}

#[test]
fn test_obt_parse_daemon_keys_defaults_and_values() {
    // Missing sections: presence on (daemon default), remote view off (preview default).
    assert_eq!(parse_daemon_keys("", 1000), (Some(true), Some(false)));
    assert_eq!(
        parse_daemon_keys("[presence]\nenabled = false\n", 1000),
        (Some(false), Some(false))
    );
    let preview = "[preview]\nenabled = true\nremote_view = true\nallowed_uids = [1000]\n";
    assert_eq!(parse_daemon_keys(preview, 1000), (Some(true), Some(true)));
    assert_eq!(
        parse_daemon_keys(preview, 1001),
        (Some(true), Some(false)),
        "the GUI user's uid must be allowed"
    );
    assert_eq!(
        parse_daemon_keys("[preview]\nenabled = true\nallowed_uids = [1000]\n", 1000),
        (Some(true), Some(false))
    );
    assert_eq!(
        parse_daemon_keys("[presence]\nenabled = \"yes\"\n", 1000),
        (None, Some(false))
    );
    assert_eq!(parse_daemon_keys("[[[", 1000), (None, None));
    // The daemon reads presence with `DEFAULT_PRESENCE_ENABLED = true`.
    let daemon = workspace_file("crates/daemon/src/presence/config.rs");
    assert!(daemon.contains("pub const DEFAULT_PRESENCE_ENABLED: bool = true;"));
}

#[test]
fn test_obt_companion_commands_use_the_user_manager() {
    assert_eq!(REMOTE_UNIT, "soos-remote.service");
    let (program, args) = companion_command(true);
    assert_eq!(program, SYSTEMCTL_PROGRAM);
    assert_eq!(args, vec!["--user", "enable", "--now", REMOTE_UNIT]);
    let (program, args) = companion_command(false);
    assert_eq!(program, SYSTEMCTL_PROGRAM);
    assert_eq!(args, vec!["--user", "disable", "--now", REMOTE_UNIT]);
    let (program, args) = companion_restart_command();
    assert_eq!(program, SYSTEMCTL_PROGRAM);
    assert_eq!(args, vec!["--user", "try-restart", REMOTE_UNIT]);
    let (program, args) = unit_show_command();
    assert_eq!(program, SYSTEMCTL_PROGRAM);
    assert_eq!(
        args,
        vec![
            "--user",
            "show",
            "--property=LoadState",
            "--property=ActiveState",
            REMOTE_UNIT
        ]
    );
    assert!(USER_SYSTEMCTL_TIMEOUT <= Duration::from_secs(5));
    assert!(USER_SYSTEMCTL_TIMEOUT >= Duration::from_secs(1));
}

#[test]
fn test_obt_feature_control_runs_no_shell_and_no_relative_program() {
    let source = workspace_file("crates/gui/src/feature_control.rs");
    for forbidden in [
        "\"sh\"",
        "\"bash\"",
        "\"-c\"",
        "Command::new(\"",
        "\"pkexec\"",
        "\"systemctl\"",
    ] {
        assert!(
            !source.contains(forbidden),
            "feature_control.rs must not contain {forbidden}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------------------------

#[test]
fn test_obt_parse_unit_show() {
    assert_eq!(
        parse_unit_show("LoadState=loaded\nActiveState=active\n"),
        UnitState::Active
    );
    assert_eq!(
        parse_unit_show("ActiveState=inactive\nLoadState=loaded\n"),
        UnitState::Inactive
    );
    assert_eq!(
        parse_unit_show("LoadState=loaded\nActiveState=failed\n"),
        UnitState::Inactive
    );
    for running in ["activating", "reloading", "refreshing", "deactivating"] {
        assert_eq!(
            parse_unit_show(&format!("LoadState=loaded\nActiveState={running}\n")),
            UnitState::Active,
            "{running}"
        );
    }
    assert_eq!(
        parse_unit_show("LoadState=not-found\nActiveState=inactive\n"),
        UnitState::NotInstalled
    );
    assert_eq!(parse_unit_show(""), UnitState::Unknown);
    assert_eq!(parse_unit_show("garbage"), UnitState::Unknown);
    assert_eq!(
        parse_unit_show("LoadState=loaded\nActiveState=weird\n"),
        UnitState::Unknown
    );
    assert_eq!(
        parse_unit_show("LoadState=masked\nActiveState=inactive\n"),
        UnitState::Masked
    );
}

#[test]
fn test_obt_remote_config_path_mirrors_soos_remote() {
    assert_eq!(
        remote_config_path(Some(OsStr::new("/x")), Some(OsStr::new("/h"))),
        Some(PathBuf::from("/x/soos/remote.toml"))
    );
    assert_eq!(
        remote_config_path(Some(OsStr::new("rel")), Some(OsStr::new("/h"))),
        Some(PathBuf::from("/h/.config/soos/remote.toml"))
    );
    assert_eq!(remote_config_path(None, None), None);
}

#[test]
fn test_obt_max_remote_config_bytes_matches_soos_remote() {
    let lib = workspace_file("crates/remote/src/lib.rs");
    assert!(lib.contains("pub const MAX_CONFIG_BYTES: usize = 16_384;"));
    assert_eq!(MAX_REMOTE_CONFIG_BYTES, 16_384);
}

#[test]
fn test_obt_parse_remote_keys() {
    assert_eq!(
        parse_remote_keys("allowed_logins = [\"a@b.c\"]\n"),
        RemoteConfigState::Keys(RemoteKeys::default())
    );
    let text = "allowed_logins = [\"a@b.c\"]\nallow_unlock = true\nallow_funnel = true\n\
                password_alerts = true\npush_notifications = true\ncamera_view = true\n\
                camera_view_funnel = false\n";
    assert_eq!(
        parse_remote_keys(text),
        RemoteConfigState::Keys(RemoteKeys {
            allow_unlock: true,
            allow_funnel: true,
            password_alerts: true,
            push_notifications: true,
            camera_view: true,
            camera_view_funnel: false,
            offline_alerts: false,
        })
    );
    assert_eq!(
        parse_remote_keys("allow_unlock = 1\n"),
        RemoteConfigState::Invalid
    );
    assert_eq!(
        parse_remote_keys("allow_unlock = \n"),
        RemoteConfigState::Invalid
    );
    assert_eq!(parse_remote_keys("= = ="), RemoteConfigState::Invalid);
}

// ---------------------------------------------------------------------------------------------
// remote.toml off-only edit
// ---------------------------------------------------------------------------------------------

const SAMPLE: &str = "# soos-remote configuration\n\
allowed_logins = [\"me@example.com\"]\n\
rp_id = \"pc.tail0000.ts.net\"\n\
allow_funnel = true   # internet\n\
\x20\x20allow_unlock=true\n\
password_alerts = true\n\
push_notifications = true\n\
# camera_view = true\n\
camera_view = true\n\
camera_view_funnel = true\n";

#[test]
fn test_obt_disable_unlock_changes_exactly_one_line() {
    let out = disable_remote_keys(SAMPLE, RemoteSwitch::Unlock.keys_to_clear())
        .unwrap()
        .unwrap();
    let before: Vec<&str> = SAMPLE.lines().collect();
    let after: Vec<&str> = out.lines().collect();
    assert_eq!(before.len(), after.len());
    for (i, (b, a)) in before.iter().zip(&after).enumerate() {
        if b.contains("allow_unlock") {
            assert_eq!(*a, "  allow_unlock = false", "line {i}");
        } else {
            assert_eq!(b, a, "line {i} must be unchanged");
        }
    }
    assert!(out.ends_with('\n'));
    let RemoteConfigState::Keys(keys) = parse_remote_keys(&out) else {
        panic!("edited file must parse");
    };
    assert!(!keys.allow_unlock && keys.allow_funnel && keys.camera_view);
}

#[test]
fn test_obt_disable_keeps_the_trailing_comment() {
    let out = disable_remote_keys(SAMPLE, RemoteSwitch::Funnel.keys_to_clear())
        .unwrap()
        .unwrap();
    assert!(out.contains("allow_funnel = false   # internet\n"), "{out}");
    // Dependent key cleared too: soos-remote refuses camera_view_funnel without allow_funnel.
    assert!(out.contains("camera_view_funnel = false\n"), "{out}");
    assert!(
        out.contains("# camera_view = true\n"),
        "commented line untouched"
    );
    assert!(out.contains("\ncamera_view = true\n"));
}

#[test]
fn test_obt_keys_to_clear_cascade_dependent_keys() {
    assert_eq!(RemoteSwitch::Unlock.keys_to_clear(), ["allow_unlock"]);
    assert_eq!(
        RemoteSwitch::Funnel.keys_to_clear(),
        ["allow_funnel", "camera_view_funnel"]
    );
    // soos-remote refuses `offline_alerts` without `push_notifications` (exit 78).
    assert_eq!(
        RemoteSwitch::Push.keys_to_clear(),
        ["push_notifications", "offline_alerts"]
    );
    assert_eq!(
        RemoteSwitch::OfflineAlerts.keys_to_clear(),
        ["offline_alerts"]
    );
    assert_eq!(
        RemoteSwitch::CameraView.keys_to_clear(),
        ["camera_view", "camera_view_funnel"]
    );
    for switch in [
        RemoteSwitch::Unlock,
        RemoteSwitch::Funnel,
        RemoteSwitch::Push,
        RemoteSwitch::CameraView,
        RemoteSwitch::OfflineAlerts,
    ] {
        assert_eq!(switch.keys_to_clear()[0], switch.key());
    }
}

#[test]
fn test_obt_disable_camera_view_clears_camera_view_funnel() {
    let out = disable_remote_keys(SAMPLE, RemoteSwitch::CameraView.keys_to_clear())
        .unwrap()
        .unwrap();
    let RemoteConfigState::Keys(keys) = parse_remote_keys(&out) else {
        panic!("edited file must parse");
    };
    assert!(!keys.camera_view && !keys.camera_view_funnel);
    assert!(keys.allow_funnel && keys.allow_unlock && keys.push_notifications);
}

#[test]
fn test_obt_disable_absent_or_already_false_key_writes_nothing() {
    let text = "allowed_logins = [\"me@example.com\"]\nallow_unlock = false\n";
    assert_eq!(
        disable_remote_keys(text, RemoteSwitch::Unlock.keys_to_clear()),
        Ok(None)
    );
    assert_eq!(
        disable_remote_keys(text, RemoteSwitch::Push.keys_to_clear()),
        Ok(None),
        "an absent key is already off and is never added"
    );
}

#[test]
fn test_obt_disable_refuses_unsupported_layouts() {
    assert_eq!(
        disable_remote_keys(
            "allow_unlock = true\n[extra]\nx = 1\n",
            RemoteSwitch::Unlock.keys_to_clear()
        ),
        Err(RemoteEditError::UnsupportedLayout)
    );
    assert_eq!(
        disable_remote_keys(
            "allow_unlock = \"true\"\n",
            RemoteSwitch::Unlock.keys_to_clear()
        ),
        Err(RemoteEditError::UnsupportedValue("allow_unlock"))
    );
    assert_eq!(
        disable_remote_keys(
            "allow_unlock = true\nallow_unlock = true\n",
            RemoteSwitch::Unlock.keys_to_clear()
        ),
        Err(RemoteEditError::Invalid),
        "a duplicate key is invalid TOML and is refused before any edit"
    );
    assert_eq!(
        disable_remote_keys("allow_unlock = [", RemoteSwitch::Unlock.keys_to_clear()),
        Err(RemoteEditError::Invalid)
    );
}

#[test]
fn test_obt_disable_refuses_a_value_hidden_in_a_multiline_string() {
    // The only line that looks like the key is inside a multi-line string: the verification
    // step (or the layout check) must refuse rather than corrupt the string.
    let text = "allowed_logins = [\"me@example.com\"]\nvapid_subject = \"\"\"\nallow_unlock = true\n\"\"\"\n";
    let result = disable_remote_keys(text, RemoteSwitch::Unlock.keys_to_clear());
    assert!(
        matches!(result, Ok(None) | Err(_)),
        "must never rewrite text inside a string: {result:?}"
    );
    if let Ok(Some(out)) = result {
        panic!("rewrote a string: {out}");
    }
}

// ---------------------------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------------------------

#[test]
fn test_obt_read_remote_config_bounds_and_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote.toml");
    assert_eq!(read_remote_config(&path), RemoteConfigState::Missing);
    std::fs::write(&path, "allow_unlock = true\n").unwrap();
    assert!(matches!(
        read_remote_config(&path),
        RemoteConfigState::Keys(RemoteKeys {
            allow_unlock: true,
            ..
        })
    ));
    let mut big = "allow_unlock = true\n".to_string();
    big.push_str(&"#".repeat(usize::try_from(MAX_REMOTE_CONFIG_BYTES).unwrap()));
    std::fs::write(&path, &big).unwrap();
    assert_eq!(read_remote_config(&path), RemoteConfigState::Invalid);

    let target = dir.path().join("real.toml");
    std::fs::write(&target, "allow_unlock = true\n").unwrap();
    let link = dir.path().join("link.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(read_remote_config(&link), RemoteConfigState::Invalid);
}

#[test]
fn test_obt_write_user_file_atomically_is_private_and_refuses_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote.toml");
    std::fs::write(&path, "old\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    write_user_file_atomically(&path, "new\n").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "no temporary file left: {names:?}");

    let target = dir.path().join("target.toml");
    std::fs::write(&target, "keep\n").unwrap();
    let link = dir.path().join("link.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(write_user_file_atomically(&link, "evil\n").is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep\n");

    assert!(write_user_file_atomically(&dir.path().join("missing/x.toml"), "x").is_err());
}

// ---------------------------------------------------------------------------------------------
// Executor with a recording program runner
// ---------------------------------------------------------------------------------------------

type Call = (String, Vec<String>, Option<Duration>);

#[derive(Default)]
struct RecordingRunner {
    calls: Mutex<Vec<Call>>,
    fail: bool,
    show_output: String,
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
            success: !self.fail,
            code: Some(i32::from(self.fail)),
            stdout: self.show_output.clone().into_bytes(),
        })
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    paths: FeaturePaths,
    config: PathBuf,
}

fn fixture(running_as_root: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let etc = dir.path().join("etc-soos");
    std::fs::create_dir(&etc).unwrap();
    let cfg_dir = dir.path().join("cfg");
    std::fs::create_dir(&cfg_dir).unwrap();
    let config = cfg_dir.join("remote.toml");
    let paths = FeaturePaths {
        daemon_config: etc.join("daemon.toml"),
        etc_dir: etc,
        network_lock_rule: dir.path().join("49-soos-network-lock.rules"),
        network_lock_rfkill: dir.path().join("72-soos-network-lock-rfkill.rules"),
        remote_config: Some(config.clone()),
        running_as_root,
        uid: 1000,
    };
    Fixture {
        _dir: dir,
        paths,
        config,
    }
}

#[test]
fn test_obt_probe_reads_flags_rule_unit_and_config_without_writing() {
    let fx = fixture(false);
    std::fs::write(fx.paths.etc_dir.join("presence.disable"), "").unwrap();
    std::fs::write(&fx.paths.network_lock_rule, "rule").unwrap();
    std::fs::write(&fx.config, "allow_unlock = true\n").unwrap();
    std::fs::write(
        &fx.paths.daemon_config,
        "[presence]\nenabled = false\n[preview]\nenabled = true\nremote_view = true\nallowed_uids = [1000]\n",
    )
    .unwrap();
    let runner = RecordingRunner {
        show_output: "LoadState=loaded\nActiveState=active\n".to_string(),
        ..RecordingRunner::default()
    };
    let snapshot = probe_snapshot(&fx.paths, &runner);
    assert_eq!(snapshot.network_lock_rfkill_installed, Some(false));
    assert_eq!(snapshot.daemon_presence_enabled, Some(false));
    assert_eq!(snapshot.daemon_remote_view, Some(true));
    assert_eq!(snapshot.global_disabled, Some(false));
    assert_eq!(snapshot.presence_disabled, Some(true));
    assert_eq!(snapshot.network_lock_installed, Some(true));
    assert_eq!(snapshot.remote_unit, UnitState::Active);
    assert!(matches!(
        snapshot.remote_config,
        RemoteConfigState::Keys(RemoteKeys {
            allow_unlock: true,
            ..
        })
    ));
    assert!(!snapshot.running_as_root);
    let calls = runner.calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "only the unit probe runs a program");
    let (program, args) = unit_show_command();
    assert_eq!(calls[0].0, program);
    assert_eq!(calls[0].1, args);
    assert_eq!(calls[0].2, Some(USER_SYSTEMCTL_TIMEOUT));
    assert!(calls.iter().all(|(p, _, _)| p != PKEXEC_PROGRAM));
}

#[test]
fn test_obt_probe_treats_any_flag_entry_as_present() {
    let fx = fixture(false);
    std::fs::create_dir(fx.paths.etc_dir.join("disabled")).unwrap();
    std::os::unix::fs::symlink("/nonexistent", fx.paths.etc_dir.join("presence.disable")).unwrap();
    let snapshot = probe_snapshot(&fx.paths, &RecordingRunner::default());
    assert_eq!(snapshot.global_disabled, Some(true));
    assert_eq!(snapshot.presence_disabled, Some(true));
    assert_eq!(snapshot.network_lock_installed, Some(false));
    assert_eq!(snapshot.remote_config, RemoteConfigState::Missing);
    // A missing daemon.toml means the daemon defaults.
    assert_eq!(snapshot.daemon_presence_enabled, Some(true));
    assert_eq!(snapshot.daemon_remote_view, Some(false));
}

#[test]
fn test_obt_probe_unreadable_daemon_config_is_unknown() {
    let fx = fixture(false);
    std::fs::create_dir(&fx.paths.daemon_config).unwrap();
    let snapshot = probe_snapshot(&fx.paths, &RecordingRunner::default());
    assert_eq!(snapshot.daemon_presence_enabled, None);
    assert_eq!(snapshot.daemon_remote_view, None);
}

#[test]
fn test_obt_user_files_never_block_on_a_fifo() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("remote.toml");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    // Returns at once (no writer would ever unblock a blocking open).
    assert_eq!(read_remote_config(&fifo), RemoteConfigState::Invalid);
    let tour = soos_gui::onboarding::TourStore::new(fifo);
    assert_eq!(tour.load(), soos_gui::onboarding::TourLoad::Unavailable);
}

#[test]
fn test_obt_probe_as_root_skips_the_user_manager() {
    let fx = fixture(true);
    let runner = RecordingRunner::default();
    let snapshot = probe_snapshot(&fx.paths, &runner);
    assert!(snapshot.running_as_root);
    assert_eq!(snapshot.remote_unit, UnitState::Unknown);
    assert!(runner.calls.lock().unwrap().is_empty());
}

#[test]
fn test_obt_probe_failed_unit_query_is_unknown() {
    let fx = fixture(false);
    let runner = RecordingRunner {
        fail: true,
        show_output: "LoadState=loaded\nActiveState=active\n".to_string(),
        ..RecordingRunner::default()
    };
    assert_eq!(
        probe_snapshot(&fx.paths, &runner).remote_unit,
        UnitState::Unknown
    );
}

fn applied(outcome: FeatureOutcome) -> Result<(), String> {
    let FeatureOutcome::Applied { result, .. } = outcome else {
        return Err("expected an applied outcome, got a refresh".to_string());
    };
    result
}

#[test]
fn test_obt_executor_flag_action_runs_one_pkexec_without_deadline() {
    let fx = fixture(false);
    let runner = Arc::new(RecordingRunner::default());
    let executor = SystemFeatureExecutor::new(fx.paths, runner.clone());
    let result = applied(executor.execute(FeatureAction::SetSystemFlag {
        flag: SystemFlag::Presence,
        present: true,
    }));
    assert_eq!(result, Ok(()));
    let calls = runner.calls.lock().unwrap();
    let (program, args) = system_flag_command(SystemFlag::Presence, true);
    assert_eq!(calls.as_slice(), &[(program.to_string(), args, None)]);
}

#[test]
fn test_obt_executor_reports_denied_authorization() {
    let fx = fixture(false);
    let runner = Arc::new(RecordingRunner {
        fail: true,
        ..RecordingRunner::default()
    });
    let executor = SystemFeatureExecutor::new(fx.paths, runner);
    let result = applied(executor.execute(FeatureAction::SetNetworkLock { installed: true }));
    assert!(result.is_err());
}

#[test]
fn test_obt_executor_network_lock_runs_the_admin_cli() {
    let fx = fixture(true);
    let runner = Arc::new(RecordingRunner::default());
    let executor = SystemFeatureExecutor::new(fx.paths, runner.clone());
    assert_eq!(
        applied(executor.execute(FeatureAction::SetNetworkLock { installed: false })),
        Ok(())
    );
    let calls = runner.calls.lock().unwrap();
    let (program, args) = network_lock_command(false);
    assert_eq!(calls.as_slice(), &[(program.to_string(), args, None)]);
}

#[test]
fn test_obt_executor_companion_uses_the_user_manager_with_a_deadline() {
    let fx = fixture(false);
    let runner = Arc::new(RecordingRunner::default());
    let executor = SystemFeatureExecutor::new(fx.paths, runner.clone());
    assert_eq!(
        applied(executor.execute(FeatureAction::SetCompanion { enabled: false })),
        Ok(())
    );
    let calls = runner.calls.lock().unwrap();
    let (program, args) = companion_command(false);
    assert_eq!(
        calls.as_slice(),
        &[(program.to_string(), args, Some(USER_SYSTEMCTL_TIMEOUT))]
    );
}

#[test]
fn test_obt_executor_refuses_user_level_changes_as_root() {
    let fx = fixture(true);
    std::fs::write(&fx.config, "allow_unlock = true\n").unwrap();
    let config = fx.config.clone();
    let runner = Arc::new(RecordingRunner::default());
    let executor = SystemFeatureExecutor::new(fx.paths, runner.clone());
    assert!(applied(executor.execute(FeatureAction::SetCompanion { enabled: true })).is_err());
    assert!(applied(executor.execute(FeatureAction::DisableRemote(RemoteSwitch::Unlock))).is_err());
    assert!(runner.calls.lock().unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(config).unwrap(),
        "allow_unlock = true\n"
    );
}

#[test]
fn test_obt_executor_disable_remote_edits_then_restarts() {
    let fx = fixture(false);
    std::fs::write(&fx.config, SAMPLE).unwrap();
    let config = fx.config.clone();
    let runner = Arc::new(RecordingRunner::default());
    let executor = SystemFeatureExecutor::new(fx.paths, runner.clone());
    assert_eq!(
        applied(executor.execute(FeatureAction::DisableRemote(RemoteSwitch::Push))),
        Ok(())
    );
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.contains("\npush_notifications = false\n"), "{text}");
    assert_eq!(
        std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let calls = runner.calls.lock().unwrap();
    let (program, args) = companion_restart_command();
    assert_eq!(
        calls.as_slice(),
        &[(program.to_string(), args, Some(USER_SYSTEMCTL_TIMEOUT))]
    );
}

#[test]
fn test_obt_executor_disable_remote_noop_writes_and_restarts_nothing() {
    let fx = fixture(false);
    std::fs::write(&fx.config, "allow_unlock = false\n").unwrap();
    let config = fx.config.clone();
    let before = std::fs::metadata(&config).unwrap().modified().unwrap();
    let runner = Arc::new(RecordingRunner::default());
    let executor = SystemFeatureExecutor::new(fx.paths, runner.clone());
    assert_eq!(
        applied(executor.execute(FeatureAction::DisableRemote(RemoteSwitch::Unlock))),
        Ok(())
    );
    assert!(runner.calls.lock().unwrap().is_empty());
    assert_eq!(
        std::fs::metadata(&config).unwrap().modified().unwrap(),
        before
    );
}

#[test]
fn test_obt_executor_disable_remote_refuses_symlinked_config() {
    let fx = fixture(false);
    let target = fx.config.with_file_name("real.toml");
    std::fs::write(&target, "allow_unlock = true\n").unwrap();
    std::os::unix::fs::symlink(&target, &fx.config).unwrap();
    let runner = Arc::new(RecordingRunner::default());
    let executor = SystemFeatureExecutor::new(fx.paths, runner.clone());
    assert!(applied(executor.execute(FeatureAction::DisableRemote(RemoteSwitch::Unlock))).is_err());
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "allow_unlock = true\n"
    );
    assert!(runner.calls.lock().unwrap().is_empty());
}

#[test]
fn test_obt_executor_restart_failure_is_reported_after_saving() {
    let fx = fixture(false);
    std::fs::write(&fx.config, "allow_unlock = true\n").unwrap();
    let config = fx.config.clone();
    let runner = Arc::new(RecordingRunner {
        fail: true,
        ..RecordingRunner::default()
    });
    let executor = SystemFeatureExecutor::new(fx.paths, runner);
    let result = applied(executor.execute(FeatureAction::DisableRemote(RemoteSwitch::Unlock)));
    let err = result.unwrap_err();
    assert!(err.starts_with("Saved"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "allow_unlock = false\n"
    );
}

#[test]
fn test_obt_executor_refresh_returns_a_snapshot() {
    let fx = fixture(true);
    let executor = SystemFeatureExecutor::new(fx.paths, Arc::new(RecordingRunner::default()));
    assert!(matches!(
        executor.execute(FeatureAction::Refresh),
        FeatureOutcome::Refreshed(_)
    ));
}

#[test]
fn test_obt_action_privilege_classification() {
    assert!(!FeatureAction::Refresh.is_privileged());
    assert!(!FeatureAction::SetCompanion { enabled: true }.is_privileged());
    assert!(!FeatureAction::DisableRemote(RemoteSwitch::Funnel).is_privileged());
    assert!(FeatureAction::SetNetworkLock { installed: true }.is_privileged());
    assert!(FeatureAction::SetSystemFlag {
        flag: SystemFlag::Global,
        present: false
    }
    .is_privileged());
}

// ---------------------------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------------------------

struct GatedExecutor {
    gate: Mutex<mpsc::Receiver<()>>,
}

impl FeatureExecutor for GatedExecutor {
    fn execute(&self, action: FeatureAction) -> FeatureOutcome {
        let _ = self.gate.lock().unwrap().recv();
        FeatureOutcome::Applied {
            action,
            result: Ok(()),
        }
    }
}

#[test]
fn test_obt_runner_is_one_at_a_time_and_never_blocks() {
    let (release, gate) = mpsc::channel();
    let executor = Arc::new(GatedExecutor {
        gate: Mutex::new(gate),
    });
    let mut runner = FeatureRunner::new(executor, Arc::new(|| {}));
    assert!(!runner.is_busy());
    let action = FeatureAction::SetSystemFlag {
        flag: SystemFlag::Presence,
        present: true,
    };
    runner.submit(action).unwrap();
    assert!(runner.is_busy());
    assert!(runner.privileged_in_flight());
    assert!(runner.poll().is_empty(), "poll never blocks");
    assert!(matches!(
        runner.submit(FeatureAction::Refresh),
        Err(FeatureRunnerError::Busy)
    ));
    release.send(()).unwrap();
    let mut outcomes = Vec::new();
    assert!(wait_until(SETTLE_TIMEOUT, || {
        outcomes.extend(runner.poll());
        !outcomes.is_empty()
    }));
    assert!(matches!(
        outcomes.as_slice(),
        [FeatureOutcome::Applied { action: a, result: Ok(()) }] if *a == action
    ));
    assert!(!runner.is_busy());
    assert!(!runner.privileged_in_flight());
}

#[test]
fn test_obt_runner_refresh_is_not_privileged_in_flight() {
    let (release, gate) = mpsc::channel();
    let executor = Arc::new(GatedExecutor {
        gate: Mutex::new(gate),
    });
    let mut runner = FeatureRunner::new(executor, Arc::new(|| {}));
    runner.submit(FeatureAction::Refresh).unwrap();
    assert!(runner.is_busy());
    assert!(!runner.privileged_in_flight());
    release.send(()).unwrap();
    assert!(wait_until(SETTLE_TIMEOUT, || {
        let _ = runner.poll();
        !runner.is_busy()
    }));
}

struct PanickingExecutor;

impl FeatureExecutor for PanickingExecutor {
    fn execute(&self, _action: FeatureAction) -> FeatureOutcome {
        panic!("executor failure");
    }
}

#[test]
fn test_obt_runner_recovers_from_a_dead_worker() {
    let mut runner = FeatureRunner::new(Arc::new(PanickingExecutor), Arc::new(|| {}));
    runner.submit(FeatureAction::Refresh).unwrap();
    assert!(wait_until(SETTLE_TIMEOUT, || {
        let _ = runner.poll();
        !runner.is_busy()
    }));
    runner.submit(FeatureAction::Refresh).unwrap();
}

#[test]
fn test_obt_disable_push_also_clears_offline_alerts() {
    // A remote.toml with offline alerts on: turning push off must not leave a file that
    // soos-remote refuses (`offline_alerts` requires `push_notifications`, exit 78).
    let text = format!("{SAMPLE}offline_alerts = true\n");
    let out = disable_remote_keys(&text, RemoteSwitch::Push.keys_to_clear())
        .unwrap()
        .unwrap();
    assert!(out.contains("\npush_notifications = false\n"), "{out}");
    assert!(out.contains("\noffline_alerts = false\n"), "{out}");
    assert_eq!(text.lines().count(), out.lines().count());
    // Without the key, nothing is added.
    let out = disable_remote_keys(SAMPLE, RemoteSwitch::Push.keys_to_clear())
        .unwrap()
        .unwrap();
    assert!(!out.contains("offline_alerts"), "{out}");
}

#[test]
fn test_obt_parse_remote_keys_reads_offline_alerts() {
    let RemoteConfigState::Keys(keys) =
        parse_remote_keys("push_notifications = true\noffline_alerts = true\n")
    else {
        panic!("expected keys");
    };
    assert!(keys.offline_alerts);
    assert_eq!(
        parse_remote_keys("offline_alerts = \"yes\"\n"),
        RemoteConfigState::Invalid
    );
}
