//! Contract tests of the `soos-lockstatus` binary (GitHub #401; matrix LKA6): `--once` always
//! exits 0, writes nothing to stderr and finishes within its budget, whatever the daemon does.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

mod common;

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use common::{runtime_dir, Behavior, MockDaemon};
use soos_lockstatus::format::{IDLE_GLYPH, NOT_RECOGNIZED_GLYPH, SCANNING_STATIC_GLYPH};
use soos_lockstatus::ONCE_EXIT_BUDGET_MS;
use soos_protocol::LockWidgetState;

const BIN: &str = env!("CARGO_BIN_EXE_soos-lockstatus");
/// CI slack on top of the documented budget (process start on a loaded runner).
const SLACK: Duration = Duration::from_millis(900);

fn run(runtime: Option<&Path>, args: &[&str]) -> (Output, Duration) {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("SOOS_REDUCE_MOTION")
        .stdin(Stdio::null());
    if let Some(dir) = runtime {
        cmd.env("XDG_RUNTIME_DIR", dir);
    }
    let start = Instant::now();
    let out = cmd.output().unwrap();
    (out, start.elapsed())
}

fn once(runtime: Option<&Path>, socket: &Path, extra: &[&str]) -> (Output, Duration) {
    let mut args = vec!["--once", "--socket", socket.to_str().unwrap()];
    args.extend_from_slice(extra);
    run(runtime, &args)
}

fn assert_quiet_success(out: &Output) {
    assert!(out.status.success(), "{:?}", out.status);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        out.stderr.is_empty(),
        "stderr must stay empty: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_lka6_once_budget_constant() {
    assert_eq!(ONCE_EXIT_BUDGET_MS, 300);
}

#[test]
fn test_lka6_once_prints_each_state() {
    let dir = runtime_dir();
    for (state, text) in [
        (LockWidgetState::Idle, ""),
        (LockWidgetState::Scanning, "Scanning"),
        (LockWidgetState::NotRecognized, "Not recognized"),
    ] {
        let daemon = MockDaemon::start(Behavior::Answer(state, 2));
        // A fresh runtime directory each time: the shared cache is keyed by socket anyway.
        let (out, _) = once(Some(dir.path()), &daemon.path, &[]);
        assert_quiet_success(&out);
        assert_eq!(String::from_utf8(out.stdout).unwrap(), text);
        let (out, _) = once(
            Some(dir.path()),
            &daemon.path,
            &["--format", "glyph", "--reduce-motion"],
        );
        assert_quiet_success(&out);
        let expected = match state {
            LockWidgetState::Idle => IDLE_GLYPH,
            LockWidgetState::Scanning => SCANNING_STATIC_GLYPH,
            LockWidgetState::NotRecognized => NOT_RECOGNIZED_GLYPH,
        };
        assert_eq!(String::from_utf8(out.stdout).unwrap(), expected);
    }
}

#[test]
fn test_lka6_reduce_motion_env_selects_the_static_glyph() {
    let dir = runtime_dir();
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Scanning, 1));
    let out = Command::new(BIN)
        .args(["--once", "--format", "glyph", "--socket"])
        .arg(&daemon.path)
        .env("XDG_RUNTIME_DIR", dir.path())
        .env("SOOS_REDUCE_MOTION", "1")
        .output()
        .unwrap();
    assert_quiet_success(&out);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        SCANNING_STATIC_GLYPH
    );
}

#[test]
fn test_lka6_once_with_daemon_down_is_idle_quiet_and_fast() {
    let dir = runtime_dir();
    let absent = dir.path().join("absent.sock");
    for format in ["text", "glyph", "label"] {
        let (out, elapsed) = once(Some(dir.path()), &absent, &["--format", format]);
        assert_quiet_success(&out);
        let expected = if format == "text" { "" } else { IDLE_GLYPH };
        assert_eq!(String::from_utf8(out.stdout).unwrap(), expected, "{format}");
        assert!(
            elapsed < Duration::from_millis(ONCE_EXIT_BUDGET_MS) + SLACK,
            "{elapsed:?}"
        );
    }
}

#[test]
fn test_lka6_once_with_stalled_daemon_finishes_within_budget() {
    let dir = runtime_dir();
    let daemon = MockDaemon::start(Behavior::Stall(Duration::from_secs(5)));
    let (out, elapsed) = once(Some(dir.path()), &daemon.path, &[]);
    assert_quiet_success(&out);
    assert_eq!(out.stdout, b"");
    assert!(
        elapsed < Duration::from_millis(ONCE_EXIT_BUDGET_MS) + SLACK,
        "{elapsed:?}"
    );
}

#[test]
fn test_lka6_once_with_refusal_or_old_daemon_renders_idle() {
    let dir = runtime_dir();
    for behavior in [
        Behavior::Close,
        Behavior::Refuse(
            soos_protocol::Verdict::ProtocolError,
            soos_protocol::ReasonClass::RateLimited,
        ),
        Behavior::Raw(vec![0, 0, 0, 0]),
    ] {
        let daemon = MockDaemon::start(behavior);
        let (out, _) = once(Some(dir.path()), &daemon.path, &["--format", "label"]);
        assert_quiet_success(&out);
        assert_eq!(String::from_utf8(out.stdout).unwrap(), IDLE_GLYPH);
    }
}

#[test]
fn test_lka6_once_without_runtime_dir_is_idle_without_daemon_contact() {
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Scanning, 1));
    let (out, _) = once(None, &daemon.path, &[]);
    assert_quiet_success(&out);
    assert_eq!(out.stdout, b"");
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(daemon.connections(), 0);
}

#[test]
fn test_lka6_once_tolerates_a_closed_stdout() {
    let dir = runtime_dir();
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Scanning, 1));
    let mut child = Command::new(BIN)
        .args(["--once", "--socket"])
        .arg(&daemon.path)
        .env("XDG_RUNTIME_DIR", dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Close the read end before the child writes.
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    assert_quiet_success(&out);
}

#[test]
fn test_lka6_once_usage_error_stays_silent() {
    let (out, _) = run(None, &["--once", "--format", "json"]);
    assert_quiet_success(&out);
    assert!(out.stdout.is_empty());
    let (out, _) = run(None, &["--once", "--no-such-flag"]);
    assert_quiet_success(&out);
}

#[test]
fn test_lka6_usage_error_without_once_is_reported() {
    let (out, _) = run(None, &["--no-such-flag"]);
    assert!(!out.status.success());
    assert!(!out.stderr.is_empty());
    let (out, _) = run(None, &[]);
    assert!(!out.status.success(), "a mode is required");
}

#[test]
fn test_lka6_watch_prints_json_lines() {
    let dir = runtime_dir();
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Scanning, 3));
    let mut child = Command::new(BIN)
        .args(["--watch", "--socket"])
        .arg(&daemon.path)
        .env("XDG_RUNTIME_DIR", dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        use std::io::Read;
        if stdout.read(&mut byte).unwrap() == 0 || byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
    }
    child.kill().unwrap();
    let _ = child.wait();
    assert_eq!(
        String::from_utf8(line).unwrap(),
        r#"{"state":"scanning","available":true,"sequence":3}"#
    );
}

#[test]
fn test_lka6_watch_exits_zero_when_stdout_closes() {
    let dir = runtime_dir();
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Idle, 0));
    let mut child = Command::new(BIN)
        .args(["--watch", "--interval-ms", "250", "--socket"])
        .arg(&daemon.path)
        .env("XDG_RUNTIME_DIR", dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    // Make the state change so that a write is attempted.
    std::thread::sleep(Duration::from_millis(300));
    daemon.set_behavior(Behavior::Answer(LockWidgetState::Scanning, 1));
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "watch did not exit on a closed stdout"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(0));
}

#[test]
fn test_lka6_watch_exits_when_stdout_closes_while_the_state_is_stable() {
    let dir = runtime_dir();
    let daemon = MockDaemon::start(Behavior::Answer(LockWidgetState::Idle, 0));
    let mut child = Command::new(BIN)
        .args(["--watch", "--interval-ms", "250", "--socket"])
        .arg(&daemon.path)
        .env("XDG_RUNTIME_DIR", dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Read the first line, then close the pipe; the state never changes afterwards.
    let mut stdout = child.stdout.take().unwrap();
    let mut byte = [0u8; 1];
    loop {
        use std::io::Read;
        if stdout.read(&mut byte).unwrap() == 0 || byte[0] == b'\n' {
            break;
        }
    }
    drop(stdout);
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("watch kept polling after its reader went away");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(0));
    let polls = daemon.requests();
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(daemon.requests(), polls, "no poll after the exit");
}
