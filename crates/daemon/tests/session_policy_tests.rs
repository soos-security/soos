//! Contractual tests for GitHub #160 (DMN-09): face verification is granted only for a
//! local, active, seat-attached logind session of the target user, and a root peer
//! (su, sudo, sshd, display manager) is tied to its own logind session through the
//! kernel `SO_PEERCRED` PID. Every lookup failure denies face (password fallback).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::peercred::PeerCredentials;
use soos_daemon::session::SessionValidator;
use soos_daemon::session_policy::{
    parse_session_id_from_cgroup, LocalSessionPolicy, LogindError, LogindSource, SessionDenial,
    SessionRecord, SystemLogind,
};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION};

const ALICE: u32 = 1000;
const BOB: u32 = 1001;

// ---------------------------------------------------------------------------
// Mock logind source
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct MockLogind {
    pid_sessions: HashMap<i32, String>,
    records: HashMap<String, SessionRecord>,
    fail: bool,
}

impl MockLogind {
    fn with_session(mut self, id: &str, record: SessionRecord) -> Self {
        self.records.insert(id.to_string(), record);
        self
    }

    fn with_pid(mut self, pid: i32, id: &str) -> Self {
        self.pid_sessions.insert(pid, id.to_string());
        self
    }

    fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }
}

impl LogindSource for MockLogind {
    fn session_id_of_pid(&self, pid: i32) -> Result<Option<String>, LogindError> {
        if self.fail {
            return Err(LogindError("mock logind unavailable".into()));
        }
        Ok(self.pid_sessions.get(&pid).cloned())
    }

    fn session(&self, session_id: &str) -> Result<Option<SessionRecord>, LogindError> {
        if self.fail {
            return Err(LogindError("mock logind unavailable".into()));
        }
        Ok(self.records.get(session_id).cloned())
    }

    fn sessions(&self) -> Result<Vec<SessionRecord>, LogindError> {
        if self.fail {
            return Err(LogindError("mock logind unavailable".into()));
        }
        Ok(self.records.values().cloned().collect())
    }
}

fn record(
    uid: u32,
    active: bool,
    remote: Option<bool>,
    seat: Option<&str>,
    class: Option<&str>,
) -> SessionRecord {
    SessionRecord {
        uid: Some(uid),
        active,
        remote,
        seat: seat.map(str::to_string),
        class: class.map(str::to_string),
    }
}

fn local_seat_session(uid: u32) -> SessionRecord {
    record(uid, true, Some(false), Some("seat0"), Some("user"))
}

fn ssh_session(uid: u32) -> SessionRecord {
    record(uid, true, Some(true), None, Some("user"))
}

fn root_peer(pid: Option<i32>) -> PeerCredentials {
    PeerCredentials {
        uid: 0,
        gid: 0,
        pid,
    }
}

fn user_peer(uid: u32, pid: i32) -> PeerCredentials {
    PeerCredentials {
        uid,
        gid: uid,
        pid: Some(pid),
    }
}

fn policy(mock: MockLogind) -> LocalSessionPolicy {
    LocalSessionPolicy::new(Arc::new(mock))
}

// ---------------------------------------------------------------------------
// Session record and cgroup parsing
// ---------------------------------------------------------------------------

#[test]
fn test_session_record_parses_logind_fields() {
    let content = "# This is private data. Do not parse.\nUID=1000\nUSER=alice\nACTIVE=1\n\
                   IS_DISPLAY=1\nSTATE=active\nREMOTE=0\nTYPE=wayland\nCLASS=user\n\
                   SEAT=seat0\nVTNR=2\nSERVICE=gdm-password\n";
    let parsed = SessionRecord::parse(content);
    assert_eq!(parsed, local_seat_session(ALICE));
}

#[test]
fn test_session_record_parses_ssh_session_as_remote_and_seatless() {
    let content = "UID=1001\nUSER=bob\nACTIVE=1\nSTATE=active\nREMOTE=1\nTYPE=tty\n\
                   CLASS=user\nREMOTE_HOST=203.0.113.7\nSERVICE=sshd\n";
    let parsed = SessionRecord::parse(content);
    assert_eq!(parsed.uid, Some(BOB));
    assert!(parsed.active);
    assert_eq!(parsed.remote, Some(true));
    assert_eq!(parsed.seat, None);
}

#[test]
fn test_session_record_unknown_remote_value_is_none() {
    let parsed = SessionRecord::parse("UID=1000\nACTIVE=1\nREMOTE=maybe\nSEAT=\nCLASS=\n");
    assert_eq!(parsed.remote, None, "Malformed REMOTE must stay unknown");
    assert_eq!(parsed.seat, None, "Empty SEAT means no seat");
    assert_eq!(parsed.class, None, "Empty CLASS means no class");
}

#[test]
fn test_cgroup_v2_session_scope_is_resolved() {
    let content = "0::/user.slice/user-1000.slice/session-3.scope\n";
    assert_eq!(parse_session_id_from_cgroup(content), Some("3".to_string()));
}

#[test]
fn test_cgroup_v1_named_systemd_hierarchy_is_resolved() {
    let content =
        "12:cpu,cpuacct:/user.slice\n1:name=systemd:/user.slice/user-1000.slice/session-c2.scope\n";
    assert_eq!(
        parse_session_id_from_cgroup(content),
        Some("c2".to_string())
    );
}

#[test]
fn test_cgroup_outside_session_scope_is_unresolved() {
    // sshd pre-auth, a systemd service and a user-manager process belong to no session.
    for content in [
        "0::/system.slice/sshd.service\n",
        "0::/system.slice/getty@tty1.service\n",
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app.scope\n",
        "",
    ] {
        assert_eq!(parse_session_id_from_cgroup(content), None, "{content:?}");
    }
}

#[test]
fn test_cgroup_ambiguous_or_malformed_session_is_unresolved() {
    for content in [
        "1:name=systemd:/user.slice/user-1000.slice/session-3.scope\n0::/user.slice/user-1001.slice/session-4.scope\n",
        "0::/user.slice/user-1000.slice/session-.scope\n",
        "0::/user.slice/user-1000.slice/session-../../x.scope\n",
        "0::/user.slice/user-1000.slice/session-3;rm.scope\n",
    ] {
        assert_eq!(parse_session_id_from_cgroup(content), None, "{content:?}");
    }
}

// ---------------------------------------------------------------------------
// Root peer: the request is tied to the caller's own session
// ---------------------------------------------------------------------------

#[test]
fn test_root_peer_sudo_in_local_terminal_is_allowed() {
    let p = policy(
        MockLogind::default()
            .with_session("2", local_seat_session(ALICE))
            .with_pid(4242, "2"),
    );
    assert_eq!(p.authorize_auth(&root_peer(Some(4242)), ALICE), Ok(()));
}

#[test]
fn test_root_peer_from_ssh_session_is_denied_even_if_target_is_at_the_desk() {
    // Alice sits at seat0; her SSH session (or an attacker's) runs sudo/su.
    let p = policy(
        MockLogind::default()
            .with_session("2", local_seat_session(ALICE))
            .with_session("7", ssh_session(ALICE))
            .with_pid(5000, "7"),
    );
    assert_eq!(
        p.authorize_auth(&root_peer(Some(5000)), ALICE),
        Err(SessionDenial::CallerSessionRemote)
    );
}

#[test]
fn test_root_peer_su_from_another_users_local_session_is_denied() {
    // Bob runs `su alice` in his own local session while Alice is in front of the camera.
    let p = policy(
        MockLogind::default()
            .with_session("2", local_seat_session(ALICE))
            .with_session("3", local_seat_session(BOB))
            .with_pid(6000, "3"),
    );
    assert_eq!(
        p.authorize_auth(&root_peer(Some(6000)), ALICE),
        Err(SessionDenial::CallerSessionForeign)
    );
}

#[test]
fn test_root_peer_su_from_another_users_ssh_session_is_denied() {
    let p = policy(
        MockLogind::default()
            .with_session("2", local_seat_session(ALICE))
            .with_session("9", ssh_session(BOB))
            .with_pid(6100, "9"),
    );
    assert!(p.authorize_auth(&root_peer(Some(6100)), ALICE).is_err());
}

#[test]
fn test_root_peer_outside_any_session_is_denied() {
    // sshd authenticating a new SSH login: the target is at the desk, but the
    // caller process (sshd) is not part of any logind session.
    let p = policy(MockLogind::default().with_session("2", local_seat_session(ALICE)));
    assert_eq!(
        p.authorize_auth(&root_peer(Some(7000)), ALICE),
        Err(SessionDenial::CallerSessionUnresolved)
    );
}

#[test]
fn test_root_peer_without_kernel_pid_is_denied() {
    let p = policy(
        MockLogind::default()
            .with_session("2", local_seat_session(ALICE))
            .with_pid(0, "2"),
    );
    assert_eq!(
        p.authorize_auth(&root_peer(None), ALICE),
        Err(SessionDenial::MissingPeerPid)
    );
    assert_eq!(
        p.authorize_auth(&root_peer(Some(0)), ALICE),
        Err(SessionDenial::MissingPeerPid)
    );
    assert_eq!(
        p.authorize_auth(&root_peer(Some(-1)), ALICE),
        Err(SessionDenial::MissingPeerPid)
    );
}

#[test]
fn test_root_peer_session_that_vanished_is_denied() {
    let p = policy(MockLogind::default().with_pid(4242, "2"));
    assert_eq!(
        p.authorize_auth(&root_peer(Some(4242)), ALICE),
        Err(SessionDenial::CallerSessionUnresolved)
    );
}

#[test]
fn test_root_peer_session_must_be_active_seated_local_user_class() {
    let cases = [
        (
            record(ALICE, false, Some(false), Some("seat0"), Some("user")),
            SessionDenial::CallerSessionInactive,
        ),
        (
            record(ALICE, true, None, Some("seat0"), Some("user")),
            SessionDenial::CallerSessionRemote,
        ),
        (
            record(ALICE, true, Some(false), None, Some("user")),
            SessionDenial::CallerSessionSeatless,
        ),
        (
            record(ALICE, true, Some(false), Some("seat0"), Some("greeter")),
            SessionDenial::CallerSessionClass,
        ),
        (
            record(ALICE, true, Some(false), Some("seat0"), None),
            SessionDenial::CallerSessionClass,
        ),
    ];
    for (rec, expected) in cases {
        let p = policy(
            MockLogind::default()
                .with_session("2", rec.clone())
                .with_pid(4242, "2"),
        );
        assert_eq!(
            p.authorize_auth(&root_peer(Some(4242)), ALICE),
            Err(expected),
            "{rec:?}"
        );
    }
}

#[test]
fn test_root_peer_session_without_uid_is_foreign() {
    let mut rec = local_seat_session(ALICE);
    rec.uid = None;
    let p = policy(
        MockLogind::default()
            .with_session("2", rec)
            .with_pid(4242, "2"),
    );
    assert_eq!(
        p.authorize_auth(&root_peer(Some(4242)), ALICE),
        Err(SessionDenial::CallerSessionForeign)
    );
}

#[test]
fn test_root_peer_logind_unavailable_fails_closed() {
    let p = policy(MockLogind::failing());
    assert_eq!(
        p.authorize_auth(&root_peer(Some(4242)), ALICE),
        Err(SessionDenial::LogindUnavailable)
    );
}

// ---------------------------------------------------------------------------
// Unprivileged peer (peer UID == target UID)
// ---------------------------------------------------------------------------

#[test]
fn test_same_uid_peer_with_local_active_session_is_allowed() {
    let p = policy(MockLogind::default().with_session("2", local_seat_session(ALICE)));
    assert_eq!(p.authorize_auth(&user_peer(ALICE, 3000), ALICE), Ok(()));
}

#[test]
fn test_same_uid_peer_with_only_remote_sessions_is_denied() {
    let p = policy(MockLogind::default().with_session("7", ssh_session(ALICE)));
    assert_eq!(
        p.authorize_auth(&user_peer(ALICE, 3000), ALICE),
        Err(SessionDenial::TargetNoLocalActiveSession)
    );
}

#[test]
fn test_same_uid_peer_without_active_session_is_denied() {
    let p = policy(
        MockLogind::default()
            .with_session(
                "2",
                record(ALICE, false, Some(false), Some("seat0"), Some("user")),
            )
            .with_session("3", local_seat_session(BOB)),
    );
    assert_eq!(
        p.authorize_auth(&user_peer(ALICE, 3000), ALICE),
        Err(SessionDenial::TargetNoLocalActiveSession)
    );
}

#[test]
fn test_unprivileged_peer_for_another_uid_is_denied() {
    let p = policy(MockLogind::default().with_session("2", local_seat_session(ALICE)));
    assert_eq!(
        p.authorize_auth(&user_peer(BOB, 3000), ALICE),
        Err(SessionDenial::CallerSessionForeign)
    );
}

#[test]
fn test_same_uid_peer_logind_unavailable_fails_closed() {
    let p = policy(MockLogind::failing());
    assert_eq!(
        p.authorize_auth(&user_peer(ALICE, 3000), ALICE),
        Err(SessionDenial::LogindUnavailable)
    );
}

#[test]
fn test_disabled_policy_permits_and_reports_not_enforced() {
    let p = LocalSessionPolicy::disabled();
    assert!(!p.is_enforced());
    assert_eq!(p.authorize_auth(&root_peer(None), ALICE), Ok(()));
    assert!(policy(MockLogind::default()).is_enforced());
}

#[test]
fn test_denial_reason_codes_are_distinct_and_non_empty() {
    let all = [
        SessionDenial::MissingPeerPid,
        SessionDenial::CallerSessionUnresolved,
        SessionDenial::LogindUnavailable,
        SessionDenial::CallerSessionForeign,
        SessionDenial::CallerSessionInactive,
        SessionDenial::CallerSessionRemote,
        SessionDenial::CallerSessionSeatless,
        SessionDenial::CallerSessionClass,
        SessionDenial::TargetNoLocalActiveSession,
    ];
    let mut codes: Vec<&str> = all.iter().map(SessionDenial::as_str).collect();
    assert!(codes.iter().all(|c| !c.is_empty()));
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), all.len());
}

// ---------------------------------------------------------------------------
// SystemLogind over fake /run/systemd/sessions and /proc trees
// ---------------------------------------------------------------------------

fn write_proc_cgroup(proc_root: &Path, pid: i32, content: &str) {
    let dir = proc_root.join(pid.to_string());
    fs::create_dir_all(&dir).expect("proc pid dir");
    fs::write(dir.join("cgroup"), content).expect("cgroup file");
}

const ALICE_LOCAL_FILE: &str = "UID=1000\nUSER=alice\nACTIVE=1\nSTATE=active\nREMOTE=0\n\
                                TYPE=wayland\nCLASS=user\nSEAT=seat0\n";
const ALICE_SSH_FILE: &str =
    "UID=1000\nUSER=alice\nACTIVE=1\nSTATE=active\nREMOTE=1\nTYPE=tty\nCLASS=user\n";

#[test]
fn test_system_logind_ties_root_peer_to_its_cgroup_session() {
    let dir = tempdir().expect("tempdir");
    let sessions = dir.path().join("sessions");
    let proc_root = dir.path().join("proc");
    fs::create_dir_all(&sessions).expect("sessions dir");
    fs::write(sessions.join("2"), ALICE_LOCAL_FILE).expect("local session");
    fs::write(sessions.join("7"), ALICE_SSH_FILE).expect("ssh session");
    fs::write(sessions.join("2.ref"), "").expect("ref fifo stand-in");
    write_proc_cgroup(
        &proc_root,
        100,
        "0::/user.slice/user-1000.slice/session-2.scope\n",
    );
    write_proc_cgroup(
        &proc_root,
        200,
        "0::/user.slice/user-1000.slice/session-7.scope\n",
    );
    write_proc_cgroup(&proc_root, 300, "0::/system.slice/sshd.service\n");

    let source = SystemLogind::with_paths(sessions, proc_root);
    assert_eq!(source.session_id_of_pid(100), Ok(Some("2".to_string())));
    assert_eq!(source.session_id_of_pid(999), Ok(None), "Vanished PID");
    let p = LocalSessionPolicy::new(Arc::new(source));

    assert_eq!(p.authorize_auth(&root_peer(Some(100)), ALICE), Ok(()));
    assert_eq!(
        p.authorize_auth(&root_peer(Some(200)), ALICE),
        Err(SessionDenial::CallerSessionRemote)
    );
    assert_eq!(
        p.authorize_auth(&root_peer(Some(300)), ALICE),
        Err(SessionDenial::CallerSessionUnresolved)
    );
    assert_eq!(
        p.authorize_auth(&root_peer(Some(999)), ALICE),
        Err(SessionDenial::CallerSessionUnresolved)
    );
}

#[test]
fn test_system_logind_rejects_unsafe_session_ids() {
    let dir = tempdir().expect("tempdir");
    let sessions = dir.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions dir");
    fs::write(dir.path().join("secret"), ALICE_LOCAL_FILE).expect("outside file");
    let source = SystemLogind::with_paths(sessions, dir.path().join("proc"));
    assert_eq!(source.session("../secret"), Ok(None));
    assert_eq!(source.session(""), Ok(None));
    assert_eq!(source.session("2/../../secret"), Ok(None));
}

#[test]
fn test_system_logind_oversized_cgroup_file_fails_closed() {
    let dir = tempdir().expect("tempdir");
    let sessions = dir.path().join("sessions");
    let proc_root = dir.path().join("proc");
    fs::create_dir_all(&sessions).expect("sessions dir");
    fs::write(sessions.join("2"), ALICE_LOCAL_FILE).expect("local session");
    let mut huge = "9:blkio:/\n".repeat(4096);
    huge.push_str("0::/user.slice/user-1000.slice/session-2.scope\n");
    write_proc_cgroup(&proc_root, 100, &huge);
    let p = LocalSessionPolicy::new(Arc::new(SystemLogind::with_paths(sessions, proc_root)));
    assert!(p.authorize_auth(&root_peer(Some(100)), ALICE).is_err());
}

#[test]
fn test_system_logind_missing_sessions_dir_fails_closed() {
    let dir = tempdir().expect("tempdir");
    let source = SystemLogind::with_paths(dir.path().join("absent"), dir.path().join("proc"));
    assert!(source.sessions().is_err());
    let p = LocalSessionPolicy::new(Arc::new(source));
    assert_eq!(
        p.authorize_auth(&user_peer(ALICE, 3000), ALICE),
        Err(SessionDenial::LogindUnavailable)
    );
}

#[test]
fn test_policy_from_validator_follows_enforcement_and_directory() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("7"), ALICE_SSH_FILE).expect("ssh session");
    let enforced =
        LocalSessionPolicy::from_validator(&SessionValidator::with_sessions_dir(dir.path().into()));
    assert!(enforced.is_enforced());
    assert_eq!(
        enforced.authorize_auth(&user_peer(ALICE, 3000), ALICE),
        Err(SessionDenial::TargetNoLocalActiveSession)
    );
    let disabled = LocalSessionPolicy::from_validator(&SessionValidator::disabled());
    assert!(!disabled.is_enforced());
}

#[test]
fn test_session_validator_ignores_remote_sessions() {
    // The preview path uses SessionValidator: an SSH-only user must not qualify.
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("7"), ALICE_SSH_FILE).expect("ssh session");
    let validator = SessionValidator::with_sessions_dir(dir.path().into());
    assert!(!validator.is_active_session(ALICE));
    fs::write(dir.path().join("2"), ALICE_LOCAL_FILE).expect("local session");
    assert!(validator.is_active_session(ALICE));
}

// ---------------------------------------------------------------------------
// Dispatcher wiring
// ---------------------------------------------------------------------------

fn auth_request(uid: u32) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Auth,
        request_id: [0x16; 32],
        uid_hint: uid,
        service: "sudo".to_string(),
        deadline_monotonic_ns: u64::MAX,
    }
}

async fn exchange_auth(dispatcher: Arc<ConnectionDispatcher>, sock: &Path, uid: u32) -> Response {
    let listener = UnixListener::bind(sock).expect("bind");
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = dispatcher.handle_connection(stream).await;
        }
    });
    let mut client = UnixStream::connect(sock).await.expect("connect");
    client
        .write_all(&encode(&auth_request(uid)).expect("encode"))
        .await
        .expect("write");
    let mut len = [0u8; 4];
    client.read_exact(&mut len).await.expect("len");
    let body_len = u32::from_be_bytes(len) as usize;
    assert!(body_len <= 4096);
    let mut buf = len.to_vec();
    buf.resize(4 + body_len, 0);
    client.read_exact(&mut buf[4..]).await.expect("body");
    decode(&buf).expect("decode response")
}

fn enforcing_config(sessions_dir: &Path) -> DispatcherConfig {
    DispatcherConfig {
        max_concurrent_connections: 4,
        connection_timeout: Duration::from_millis(500),
        enforce_active_session: true,
        logind_sessions_dir: sessions_dir.to_path_buf(),
    }
}

#[tokio::test]
async fn test_dispatcher_denies_auth_when_target_has_only_remote_session() {
    let uid = nix::unistd::getuid().as_raw();
    let dir = tempdir().expect("tempdir");
    let sessions = dir.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions dir");
    fs::write(
        sessions.join("7"),
        format!("UID={uid}\nACTIVE=1\nSTATE=active\nREMOTE=1\nTYPE=tty\nCLASS=user\n"),
    )
    .expect("ssh session");
    let dispatcher = Arc::new(ConnectionDispatcher::new(
        enforcing_config(&sessions),
        Arc::new(HealthState::new()),
    ));
    let resp = exchange_auth(dispatcher, &dir.path().join("remote.sock"), uid).await;
    assert_eq!(resp.verdict, Verdict::ProtocolError);
    assert_eq!(resp.reason_class, ReasonClass::UidMismatch);
}

#[tokio::test]
async fn test_dispatcher_applies_injected_session_policy() {
    let uid = nix::unistd::getuid().as_raw();
    let pid = i32::try_from(std::process::id()).expect("pid");
    let dir = tempdir().expect("tempdir");

    // Remote-only logind state: denied before the pipeline is consulted.
    let denied = Arc::new(
        ConnectionDispatcher::new(enforcing_config(dir.path()), Arc::new(HealthState::new()))
            .with_session_policy(policy(
                MockLogind::default()
                    .with_session("7", ssh_session(uid))
                    .with_pid(pid, "7"),
            )),
    );
    let resp = exchange_auth(denied, &dir.path().join("denied.sock"), uid).await;
    assert_eq!(resp.verdict, Verdict::ProtocolError);
    assert_eq!(resp.reason_class, ReasonClass::UidMismatch);

    // Local seat session owned by the caller (and tied to this PID for a root runner):
    // passes the session gate and reaches the uninitialized pipeline.
    let allowed = Arc::new(
        ConnectionDispatcher::new(enforcing_config(dir.path()), Arc::new(HealthState::new()))
            .with_session_policy(policy(
                MockLogind::default()
                    .with_session("2", local_seat_session(uid))
                    .with_pid(pid, "2"),
            )),
    );
    let resp = exchange_auth(allowed, &dir.path().join("allowed.sock"), uid).await;
    assert_eq!(resp.verdict, Verdict::Unavailable);
}
