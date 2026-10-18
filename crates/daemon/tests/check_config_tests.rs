//! Contractual tests: `soos-daemon --check-config` validates `daemon.toml` with the loader the
//! daemon uses at start, and opens nothing (GitHub #441, part of #362; spec
//! `AI/architect_spec_daemon_check_config.md`; matrix DCC1-DCC7).
//!
//! The library part (`config::DaemonConfig::load_classified`, `check_config`) runs with an
//! injected system path, so no test depends on the host's `/etc/soos/daemon.toml`. The binary
//! part runs the real `soos-daemon` with an explicit `--config` and checks the exit code
//! (sysexits: 0, 66 `EX_NOINPUT`, 78 `EX_CONFIG`), the single bounded diagnostic line and that no
//! socket was created.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use soos_daemon::check_config::{
    bounded_diagnostic, check_config, CheckStream, EXIT_CONFIG_INVALID, EXIT_CONFIG_UNAVAILABLE,
    EXIT_CONFIG_VALID, MAX_CHECK_DIAGNOSTIC_BYTES,
};
use soos_daemon::config::{ConfigLoadFailure, ConfigOrigin, DaemonConfig};

/// A key no table of `daemon.toml` knows; never echoed in a diagnostic.
const CANARY: &str = "canary_value_7f3a";

fn daemon_bin() -> &'static str {
    env!("CARGO_BIN_EXE_soos-daemon")
}

fn setup_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/setup/daemon.toml")
}

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, content).unwrap();
    path
}

fn run_check(args: &[&str]) -> Output {
    Command::new(daemon_bin())
        .args(args)
        .env_remove("RUST_LOG")
        .stdin(Stdio::null())
        .output()
        .expect("run soos-daemon")
}

fn run_check_file(path: &Path) -> Output {
    run_check(&["--check-config", "--config", path.to_str().unwrap()])
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Exactly one non-empty line of at most `MAX_CHECK_DIAGNOSTIC_BYTES` bytes (plus `\n`).
fn assert_one_bounded_line(text: &str, what: &str) {
    assert!(
        text.ends_with('\n'),
        "{what}: line must end with a newline: {text:?}"
    );
    let body = text.strip_suffix('\n').unwrap_or(text);
    assert!(!body.is_empty(), "{what}: empty diagnostic");
    assert!(!body.contains('\n'), "{what}: more than one line: {text:?}");
    assert!(
        body.len() <= MAX_CHECK_DIAGNOSTIC_BYTES,
        "{what}: {} bytes > {MAX_CHECK_DIAGNOSTIC_BYTES}",
        body.len()
    );
    assert!(
        !body.chars().any(char::is_control),
        "{what}: control character in {body:?}"
    );
}

fn assert_valid_run(out: &Output, what: &str) {
    assert_eq!(
        out.status.code(),
        Some(i32::from(EXIT_CONFIG_VALID)),
        "{what}: stdout {:?} stderr {:?}",
        stdout(out),
        stderr(out)
    );
    assert_one_bounded_line(&stdout(out), what);
    assert!(
        stdout(out).starts_with("soos-daemon: "),
        "{what}: {}",
        stdout(out)
    );
    assert!(stdout(out).contains("is valid"), "{what}: {}", stdout(out));
    assert_eq!(stderr(out), "", "{what}: nothing on stderr");
}

fn assert_refused_run(out: &Output, code: u8, what: &str) {
    assert_eq!(
        out.status.code(),
        Some(i32::from(code)),
        "{what}: stdout {:?} stderr {:?}",
        stdout(out),
        stderr(out)
    );
    assert_eq!(stdout(out), "", "{what}: nothing on stdout");
    assert_one_bounded_line(&stderr(out), what);
    assert!(
        stderr(out).starts_with("soos-daemon: configuration refused: "),
        "{what}: {}",
        stderr(out)
    );
}

/// DCC1: the classified loader the start path is built on reports where the configuration came
/// from and the class of every refusal, with the start path's own error text.
#[test]
fn test_dcc1_classified_loader_matches_the_start_path() {
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("absent.toml");

    // No --config and no system file: the runtime defaults.
    let (config, origin) = DaemonConfig::load_classified(None, &absent).unwrap();
    assert_eq!(origin, ConfigOrigin::RuntimeDefault(absent.clone()));
    assert_eq!(
        format!("{config:?}"),
        format!("{:?}", DaemonConfig::runtime_default())
    );

    // A system file that is present is read.
    let valid = write(dir.path(), "valid.toml", "log_level = \"debug\"\n");
    let (config, origin) = DaemonConfig::load_classified(None, &valid).unwrap();
    assert_eq!(origin, ConfigOrigin::File(valid.clone()));
    assert_eq!(config.log_level, "debug");
    let (_, origin) = DaemonConfig::load_classified(Some(&valid), &absent).unwrap();
    assert_eq!(origin, ConfigOrigin::File(valid.clone()));

    // Unavailable: missing explicit file, a directory, a dangling symbolic link.
    let dangling = dir.path().join("dangling.toml");
    std::os::unix::fs::symlink(dir.path().join("nowhere.toml"), &dangling).unwrap();
    let subdir = dir.path().join("subdir");
    fs::create_dir(&subdir).unwrap();
    for path in [&absent, &subdir, &dangling] {
        let failure = DaemonConfig::load_classified(Some(path), &absent).unwrap_err();
        assert!(
            matches!(failure, ConfigLoadFailure::Unavailable(_)),
            "{}: {failure:?}",
            path.display()
        );
        assert_eq!(
            failure.error().to_string(),
            DaemonConfig::load_or_default_with_system_path(Some(path), &absent)
                .unwrap_err()
                .to_string(),
            "same text as the start path"
        );
    }
    // A system path that is a directory is not "absent": refused, never the defaults.
    assert!(matches!(
        DaemonConfig::load_classified(None, &subdir).unwrap_err(),
        ConfigLoadFailure::Unavailable(_)
    ));

    // Invalid: oversized, not UTF-8, bad TOML, wrong type, validation error.
    let limit = usize::try_from(soos_camera_v4l::daemon_config::MAX_DAEMON_CONFIG_BYTES).unwrap();
    let oversized = write(dir.path(), "big.toml", &"#".repeat(limit + 1));
    let non_utf8 = dir.path().join("latin1.toml");
    fs::write(&non_utf8, b"log_level = \"\xe9\"\n").unwrap();
    let bad = write(dir.path(), "bad.toml", "[preview\nenabled = true\n");
    let mistyped = write(dir.path(), "type.toml", "[socket]\nsocket_mode = \"x\"\n");
    let invalid = write(dir.path(), "mode.toml", "[socket]\nsocket_mode = 438\n");
    for path in [&oversized, &non_utf8, &bad, &mistyped, &invalid] {
        let failure = DaemonConfig::load_classified(Some(path), &absent).unwrap_err();
        assert!(
            matches!(failure, ConfigLoadFailure::Invalid(_)),
            "{}: {failure:?}",
            path.display()
        );
        let start = DaemonConfig::load_or_default_with_system_path(Some(path), &absent)
            .unwrap_err()
            .to_string();
        assert_eq!(failure.into_error().to_string(), start);
    }
}

/// DCC2: the library check reports each class with its sysexits code, stream and line, counts
/// warnings, and validates the defaults when the system file is absent.
#[test]
fn test_dcc2_check_config_reports_each_class() {
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("daemon.toml");

    let report = check_config(None, &absent);
    assert_eq!(report.exit_code, EXIT_CONFIG_VALID);
    assert_eq!(report.stream, CheckStream::Stdout);
    assert_eq!(
        report.line,
        format!(
            "soos-daemon: no configuration file at '{}'; the built-in defaults are valid",
            absent.display()
        )
    );

    let valid = write(dir.path(), "valid.toml", "[preview]\nenabled = false\n");
    let report = check_config(Some(&valid), &absent);
    assert_eq!(report.exit_code, EXIT_CONFIG_VALID);
    assert_eq!(report.stream, CheckStream::Stdout);
    assert_eq!(
        report.line,
        format!("soos-daemon: configuration '{}' is valid", valid.display())
    );
    // The system path is used when no --config is given.
    assert_eq!(check_config(None, &valid).line, report.line);

    // Non-fatal problems are counted, never listed (the daemon logs them at start).
    let warned = write(dir.path(), "warn.toml", "[camera]\nunknown_key = 1\n");
    let report = check_config(Some(&warned), &absent);
    assert_eq!(report.exit_code, EXIT_CONFIG_VALID);
    assert_eq!(
        report.line,
        format!(
            "soos-daemon: configuration '{}' is valid (1 warning(s), logged at daemon start)",
            warned.display()
        )
    );

    let report = check_config(Some(&absent), &absent);
    assert_eq!(report.exit_code, EXIT_CONFIG_UNAVAILABLE);
    assert_eq!(report.stream, CheckStream::Stderr);
    assert!(
        report
            .line
            .starts_with("soos-daemon: configuration refused: ")
            && report.line.contains("not found"),
        "{}",
        report.line
    );

    let invalid = write(dir.path(), "mode.toml", "[socket]\nsocket_mode = 438\n");
    let report = check_config(Some(&invalid), &absent);
    assert_eq!(report.exit_code, EXIT_CONFIG_INVALID);
    assert_eq!(report.stream, CheckStream::Stderr);
    assert!(
        report
            .line
            .starts_with("soos-daemon: configuration refused: ")
            && report.line.contains("socket_mode"),
        "{}",
        report.line
    );
}

/// DCC3: a diagnostic is the first line only, without control characters, at most
/// `MAX_CHECK_DIAGNOSTIC_BYTES` bytes cut on a character boundary.
#[test]
fn test_dcc3_diagnostic_is_one_bounded_line() {
    assert_eq!(bounded_diagnostic("plain"), "plain");
    assert_eq!(
        bounded_diagnostic("TOML parse error at line 2, column 9\n  |\n2 | secret = 1\n"),
        "TOML parse error at line 2, column 9"
    );
    assert_eq!(bounded_diagnostic("a\tb\u{1b}[31mc\rd"), "a?b?[31mc?d");
    let long = "é".repeat(MAX_CHECK_DIAGNOSTIC_BYTES);
    let cut = bounded_diagnostic(&long);
    assert!(cut.len() <= MAX_CHECK_DIAGNOSTIC_BYTES, "{}", cut.len());
    assert!(cut.ends_with("..."), "{cut}");
    assert!(cut.trim_end_matches("...").chars().all(|c| c == 'é'));
    let exact = "x".repeat(MAX_CHECK_DIAGNOSTIC_BYTES);
    assert_eq!(bounded_diagnostic(&exact), exact);
}

/// DCC4: a valid file exits 0 with one stdout line, nothing on stderr, no daemon log line and
/// no socket (the check never binds, never starts the runtime, the camera or the models).
#[test]
fn test_dcc4_valid_file_exits_zero_and_opens_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let sockets = dir.path().join("run");
    let config = write(
        dir.path(),
        "daemon.toml",
        &format!(
            "log_level = \"trace\"\n[socket]\nsocket_dir = \"{}\"\nsocket_path = \"{}\"\n\
             enforce_root_owner = false\n[pipeline]\nuse_mock_camera = true\n\
             models_dir = \"{}\"\n",
            sockets.display(),
            sockets.join("daemon.sock").display(),
            dir.path().join("no-models").display()
        ),
    );
    let out = run_check_file(&config);
    assert_valid_run(&out, "valid file");
    assert_eq!(
        stdout(&out),
        format!(
            "soos-daemon: configuration '{}' is valid\n",
            config.display()
        )
    );
    assert!(
        !sockets.exists(),
        "the check must not create the socket directory"
    );
    assert!(
        !stdout(&out).contains("Starting soos-daemon") && !stderr(&out).contains("INFO"),
        "no daemon log line"
    );
}

/// DCC5: the exact `[preview]` file `scripts/setup.sh` writes (fixture, byte-equal to the setup
/// render by `soos-invariants::daemon_check_config_contract`) is valid and loads as intended.
#[test]
fn test_dcc5_setup_preview_render_is_valid() {
    let fixture = setup_fixture();
    assert_valid_run(&run_check_file(&fixture), "setup render");
    let config = DaemonConfig::load_from_path(&fixture).unwrap();
    assert!(config.preview.enabled);
    assert_eq!(config.preview.allowed_uids, vec![1000]);
    assert!(config.preview.remote_view);
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
}

/// DCC6: every refusal class of the start path, through the binary, with start parity; the
/// unknown-key policy is the start policy (ignored, accepted); symbolic links are followed like
/// the daemon follows them; file content is never echoed.
#[test]
fn test_dcc6_refusals_and_start_parity() {
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("absent.toml");

    // Oversized: EX_CONFIG.
    let limit = usize::try_from(soos_camera_v4l::daemon_config::MAX_DAEMON_CONFIG_BYTES).unwrap();
    let oversized = write(dir.path(), "big.toml", &"#".repeat(limit + 1));
    let out = run_check_file(&oversized);
    assert_refused_run(&out, EXIT_CONFIG_INVALID, "oversized");
    assert!(stderr(&out).contains("larger than"), "{}", stderr(&out));

    // Bad TOML: EX_CONFIG, location only, the source line is never quoted.
    let bad = write(
        dir.path(),
        "bad.toml",
        &format!("[preview]\n{CANARY} = = 1\n"),
    );
    let out = run_check_file(&bad);
    assert_refused_run(&out, EXIT_CONFIG_INVALID, "bad TOML");
    assert!(stderr(&out).contains("line 2"), "{}", stderr(&out));
    assert!(
        !stderr(&out).contains(CANARY),
        "content echoed: {}",
        stderr(&out)
    );

    // Wrong type: EX_CONFIG without the value.
    let mistyped = write(
        dir.path(),
        "type.toml",
        &format!("[socket]\nsocket_mode = \"{CANARY}\"\n"),
    );
    let out = run_check_file(&mistyped);
    assert_refused_run(&out, EXIT_CONFIG_INVALID, "wrong type");
    assert!(
        !stderr(&out).contains(CANARY),
        "value echoed: {}",
        stderr(&out)
    );

    // Validation error: EX_CONFIG, the start path refuses it too.
    let invalid = write(dir.path(), "mode.toml", "[socket]\nsocket_mode = 438\n");
    let out = run_check_file(&invalid);
    assert_refused_run(&out, EXIT_CONFIG_INVALID, "world-writable socket mode");
    assert!(DaemonConfig::load_or_default_with_system_path(Some(&invalid), &absent).is_err());

    // Unknown keys: the start policy ignores them, so the check accepts them.
    let unknown = write(
        dir.path(),
        "unknown.toml",
        &format!("{CANARY} = 1\n[preview]\nenabled = true\n{CANARY}_too = true\n[unknown_table]\nx = 1\n"),
    );
    assert_valid_run(&run_check_file(&unknown), "unknown keys");
    assert!(DaemonConfig::load_or_default_with_system_path(Some(&unknown), &absent).is_ok());

    // Missing explicit file, directory, FIFO, dangling link: EX_NOINPUT.
    let subdir = dir.path().join("subdir");
    fs::create_dir(&subdir).unwrap();
    let fifo = dir.path().join("fifo.toml");
    let status = Command::new("mkfifo").arg(&fifo).status().expect("mkfifo");
    assert!(status.success());
    let dangling = dir.path().join("dangling.toml");
    std::os::unix::fs::symlink(dir.path().join("nowhere.toml"), &dangling).unwrap();
    for (path, what) in [
        (&absent, "missing"),
        (&subdir, "directory"),
        (&fifo, "fifo"),
        (&dangling, "dangling link"),
    ] {
        assert_refused_run(&run_check_file(path), EXIT_CONFIG_UNAVAILABLE, what);
    }

    // A symbolic link to a valid file is followed (the daemon reads the same file).
    let link = dir.path().join("link.toml");
    std::os::unix::fs::symlink(setup_fixture(), &link).unwrap();
    assert_valid_run(&run_check_file(&link), "link to a valid file");

    // Permission denied: EX_NOINPUT (asserted only when the mode really binds: not as root).
    let locked = write(dir.path(), "locked.toml", "[preview]\nenabled = true\n");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&locked).is_err() {
        let out = run_check_file(&locked);
        assert_refused_run(&out, EXIT_CONFIG_UNAVAILABLE, "permission denied");
        assert!(stderr(&out).contains("cannot be read"), "{}", stderr(&out));
    }
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o600)).unwrap();
}

/// DCC7: `--check-config` never combines with `--mock-camera` (clap usage error, exit 2), and
/// is documented in `--help`.
#[test]
fn test_dcc7_usage() {
    let out = run_check(&[
        "--check-config",
        "--mock-camera",
        "--config",
        setup_fixture().to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    let help = run_check(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(
        stdout(&help).contains("--check-config"),
        "{}",
        stdout(&help)
    );
}
