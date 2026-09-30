//! Contractual tests for GitHub #276 (session policy hardening from the P1 batch review):
//! - an unreadable sessions-directory entry denies instead of being skipped, because the
//!   user-manager path relies on the full scan to prove the target owns no remote session;
//! - the session-scope parser reads only the hierarchies systemd manages (`0::` and
//!   `name=systemd`), exactly like the user-manager parser.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use tempfile::tempdir;

use soos_daemon::peercred::PeerCredentials;
use soos_daemon::session_policy::{
    collect_session_records, parse_session_id_from_cgroup, LocalSessionPolicy, SessionDenial,
    SystemLogind, MAX_SCANNED_SESSIONS,
};

const ALICE_LOCAL_FILE: &str = "UID=1000\nUSER=alice\nACTIVE=1\nSTATE=active\nREMOTE=0\n\
                                TYPE=wayland\nCLASS=user\nSEAT=seat0\n";
const ALICE_SSH_FILE: &str =
    "UID=1000\nUSER=alice\nACTIVE=1\nSTATE=active\nREMOTE=1\nTYPE=tty\nCLASS=user\n";

fn entry(name: &str, path: PathBuf) -> io::Result<(OsString, PathBuf)> {
    Ok((OsString::from(name), path))
}

#[test]
fn test_session_scan_entry_error_fails_closed() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("2"), ALICE_LOCAL_FILE).expect("local session");
    fs::write(dir.path().join("7"), ALICE_SSH_FILE).expect("ssh session");

    // The remote session "7" hides behind an entry that failed to read: skipping it would
    // let the user-manager path conclude that the target owns no remote session.
    let entries = vec![
        entry("2", dir.path().join("2")),
        Err(io::Error::other("synthetic readdir failure")),
        entry("7", dir.path().join("7")),
    ];
    assert!(collect_session_records(entries).is_err());
}

#[test]
fn test_session_scan_keeps_every_valid_record_and_skips_non_sessions() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("2"), ALICE_LOCAL_FILE).expect("local session");
    fs::write(dir.path().join("7"), ALICE_SSH_FILE).expect("ssh session");
    fs::write(dir.path().join("2.ref"), "").expect("ref stand-in");
    let entries = vec![
        entry("2", dir.path().join("2")),
        entry("2.ref", dir.path().join("2.ref")),
        entry("7", dir.path().join("7")),
        entry("9", dir.path().join("9")),
    ];
    let records = collect_session_records(entries).expect("scan succeeds");
    assert_eq!(records.len(), 2, "Vanished '9' and '2.ref' are skipped");
    assert!(records.iter().any(|r| r.remote == Some(true)));
    assert!(records.iter().any(|r| r.remote == Some(false)));
}

#[test]
fn test_session_scan_is_bounded() {
    let entries =
        (0..=MAX_SCANNED_SESSIONS).map(|i| entry(&format!("{i}"), PathBuf::from("/nonexistent")));
    assert!(collect_session_records(entries).is_err());
}

#[test]
fn test_cgroup_session_parser_ignores_non_systemd_controller_lines() {
    // A cgroup v1 controller hierarchy can be arbitrarily placed by whoever controls it;
    // only the systemd-managed hierarchies define the logind session.
    assert_eq!(
        parse_session_id_from_cgroup(
            "4:cpu:/user.slice/user-1000.slice/session-3.scope\n0::/system.slice/sshd.service\n"
        ),
        None
    );
    assert_eq!(
        parse_session_id_from_cgroup(
            "5:memory:/user.slice/user-1001.slice/session-9.scope\n\
             0::/user.slice/user-1000.slice/session-3.scope\n"
        ),
        Some("3".to_string()),
        "A disagreeing controller line must not mask the systemd session"
    );
    assert_eq!(
        parse_session_id_from_cgroup(
            "3:cpu,name=systemd:/user.slice/user-1000.slice/session-3.scope\n"
        ),
        None,
        "Only an exact name=systemd controller list counts"
    );
}

#[test]
fn test_cgroup_session_parser_still_reads_both_systemd_hierarchies() {
    assert_eq!(
        parse_session_id_from_cgroup("0::/user.slice/user-1000.slice/session-3.scope\n"),
        Some("3".to_string())
    );
    assert_eq!(
        parse_session_id_from_cgroup(
            "1:name=systemd:/user.slice/user-1000.slice/session-c2.scope\n"
        ),
        Some("c2".to_string())
    );
}

#[test]
fn test_polkit_126_agent_helper_in_system_slice_falls_back_to_password() {
    // polkit >= 126 runs `polkit-agent-helper-1` as a socket-activated
    // `polkit-agent-helper@.service` instance in `system.slice`: it is in no session scope
    // and not under a user manager, so face is refused and polkit prompts for the password.
    let dir = tempdir().expect("tempdir");
    let sessions = dir.path().join("sessions");
    let proc_root = dir.path().join("proc");
    fs::create_dir_all(&sessions).expect("sessions dir");
    fs::write(sessions.join("2"), ALICE_LOCAL_FILE).expect("local session");
    let pid_dir = proc_root.join("4242");
    fs::create_dir_all(&pid_dir).expect("pid dir");
    fs::write(
        pid_dir.join("cgroup"),
        "0::/system.slice/system-polkit\\x2dagent\\x2dhelper.slice/polkit-agent-helper@3-4242-0.service\n",
    )
    .expect("cgroup");

    let policy = LocalSessionPolicy::new(Arc::new(SystemLogind::with_paths(sessions, proc_root)));
    let peer = PeerCredentials {
        uid: 0,
        gid: 0,
        pid: Some(4242),
    };
    assert_eq!(
        policy.authorize_auth(&peer, 1000),
        Err(SessionDenial::CallerSessionUnresolved)
    );
}
