//! Contractual tests of the preview placement rule (soos-remote security hardening S-5, ADR
//! "[2026-10-12] soos-remote Security Hardening", amending LC-3a of the 2026-10-07 live camera
//! ADR; spec `AI/architect_spec_remote_security_hardening.md` §6, §11.2; row RSH8).
//!
//! Contract:
//! - `classify_peer_placement` reads the systemd hierarchies of `/proc/<pid>/cgroup`: a
//!   session scope is `Session(id)`, a process below `user@<uid>.service` is
//!   `UserManager(uid)` (a `session-*.scope` nested under the manager is not a session), any
//!   other path is `Other`; a malformed or inconsistent user-manager path is an error.
//! - An unprivileged preview peer in a session scope needs **its own** session record to be a
//!   local seat session of its UID (absent record or unreadable logind: refused).
//! - A peer under its own user manager is served only while its UID owns a local seat session
//!   and no remote (or UID-less) session, exactly the root `Auth` user-manager rule.
//! - `Other` keeps the UID-level seat rule; a disabled validator disables the rule.
//! - The rule is evaluated on every preview request of a connection.
//! - The root `Auth` user-manager rule is unchanged.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use soos_daemon::config::DispatcherConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::peercred::PeerCredentials;
use soos_daemon::preview::PreviewConfig;
use soos_daemon::session::SessionValidator;
use soos_daemon::session_policy::{
    classify_peer_placement, LocalSessionPolicy, LogindError, LogindSource, PeerPlacement,
    SessionDenial, SessionRecord,
};
use soos_protocol::codec::{decode, decode_preview, encode};
use soos_protocol::types::{
    PreviewResponse, ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION,
};

const ALICE: u32 = 1000;
const BOB: u32 = 1001;

// ---------------------------------------------------------------------------
// Mock logind source
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct MockLogind {
    records: HashMap<String, SessionRecord>,
    fail: bool,
}

impl MockLogind {
    fn with_session(mut self, id: &str, record: SessionRecord) -> Self {
        self.records.insert(id.to_string(), record);
        self
    }

    fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }

    fn policy(self) -> LocalSessionPolicy {
        LocalSessionPolicy::new(Arc::new(self))
    }
}

impl LogindSource for MockLogind {
    fn session_id_of_pid(&self, _pid: i32) -> Result<Option<String>, LogindError> {
        if self.fail {
            return Err(LogindError("mock logind unavailable".into()));
        }
        Ok(None)
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

    fn cgroup_of_pid(&self, _pid: i32) -> Result<Option<String>, LogindError> {
        if self.fail {
            return Err(LogindError("mock logind unavailable".into()));
        }
        Ok(None)
    }
}

/// Mock source answering only `cgroup_of_pid` (the dispatcher's preview peer source).
#[derive(Debug)]
struct CgroupSource(String);

impl LogindSource for CgroupSource {
    fn session_id_of_pid(&self, _pid: i32) -> Result<Option<String>, LogindError> {
        Ok(None)
    }
    fn session(&self, _session_id: &str) -> Result<Option<SessionRecord>, LogindError> {
        Ok(None)
    }
    fn sessions(&self) -> Result<Vec<SessionRecord>, LogindError> {
        Ok(Vec::new())
    }
    fn cgroup_of_pid(&self, _pid: i32) -> Result<Option<String>, LogindError> {
        Ok(Some(self.0.clone()))
    }
}

fn record(
    uid: Option<u32>,
    active: bool,
    remote: Option<bool>,
    seat: Option<&str>,
    class: Option<&str>,
) -> SessionRecord {
    SessionRecord {
        uid,
        active,
        remote,
        seat: seat.map(str::to_string),
        class: class.map(str::to_string),
    }
}

fn local_seat(uid: u32) -> SessionRecord {
    record(Some(uid), true, Some(false), Some("seat0"), Some("user"))
}

fn ssh(uid: u32) -> SessionRecord {
    record(Some(uid), true, Some(true), None, Some("user"))
}

fn session_scope(uid: u32, id: &str) -> String {
    format!("0::/user.slice/user-{uid}.slice/session-{id}.scope\n")
}

fn user_unit(uid: u32, unit: &str) -> String {
    format!("0::/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/{unit}\n")
}

// ---------------------------------------------------------------------------
// Pure classification
// ---------------------------------------------------------------------------

#[test]
fn test_rsh_classify_peer_placement() {
    assert_eq!(
        classify_peer_placement(&session_scope(ALICE, "4")),
        Ok(PeerPlacement::Session("4".to_string()))
    );
    assert_eq!(
        classify_peer_placement(&format!(
            "1:name=systemd:/user.slice/user-{ALICE}.slice/session-c2.scope\n"
        )),
        Ok(PeerPlacement::Session("c2".to_string())),
        "cgroup v1 name=systemd hierarchy"
    );
    assert_eq!(
        classify_peer_placement(&user_unit(ALICE, "soos-remote.service")),
        Ok(PeerPlacement::UserManager(ALICE))
    );
    assert_eq!(
        classify_peer_placement(&format!(
            "0::/user.slice/user-{ALICE}.slice/user@{ALICE}.service/session.slice/tmux-spawn-1.scope\n"
        )),
        Ok(PeerPlacement::UserManager(ALICE))
    );
    assert_eq!(
        classify_peer_placement(&format!(
            "0::/user.slice/user-{ALICE}.slice/user@{ALICE}.service/app.slice/session-4.scope\n"
        )),
        Ok(PeerPlacement::UserManager(ALICE)),
        "a session scope nested under the user manager is not a session"
    );
    for other in [
        "0::/system.slice/github-runner.service\n",
        "0::/\n",
        "0::/machine.slice/libpod-abc.scope\n",
        "",
    ] {
        assert_eq!(
            classify_peer_placement(other),
            Ok(PeerPlacement::Other),
            "{other:?}"
        );
    }
    for malformed in [
        format!("0::/user.slice/user-{ALICE}.slice/user@{BOB}.service/app.slice/x.service\n"),
        format!("0::/user.slice/user-{ALICE}.slice/user@{ALICE}.service\n"),
        "0::/user.slice/user-01000.slice/user@01000.service/x.service\n".to_string(),
    ] {
        assert_eq!(
            classify_peer_placement(&malformed),
            Err(SessionDenial::UserManagerCgroupMalformed),
            "{malformed:?}"
        );
    }
    // The cgroup v1 and v2 lines disagree on the user manager.
    let disagreeing = format!(
        "1:name=systemd:/user.slice/user-{ALICE}.slice/user@{ALICE}.service/app.slice/a.service\n\
         0::/user.slice/user-{BOB}.slice/user@{BOB}.service/app.slice/a.service\n"
    );
    assert_eq!(
        classify_peer_placement(&disagreeing),
        Err(SessionDenial::UserManagerCgroupMalformed)
    );
}

// ---------------------------------------------------------------------------
// Pure policy
// ---------------------------------------------------------------------------

#[test]
fn test_rsh_session_peer_needs_its_own_local_seat_session() {
    let own = PeerPlacement::Session("4".to_string());

    let served = MockLogind::default()
        .with_session("4", local_seat(ALICE))
        .policy();
    assert_eq!(served.authorize_preview_placement(ALICE, &own), Ok(()));

    // Its own session is remote while another local seat session of the UID exists.
    let remote_own = MockLogind::default()
        .with_session("4", ssh(ALICE))
        .with_session("2", local_seat(ALICE))
        .policy();
    assert_eq!(
        remote_own.authorize_preview_placement(ALICE, &own),
        Err(SessionDenial::CallerSessionRemote)
    );

    // Its own session has no record while a local seat session exists.
    let absent = MockLogind::default()
        .with_session("2", local_seat(ALICE))
        .policy();
    assert_eq!(
        absent.authorize_preview_placement(ALICE, &own),
        Err(SessionDenial::CallerSessionUnresolved)
    );

    let inactive = MockLogind::default()
        .with_session(
            "4",
            record(Some(ALICE), false, Some(false), Some("seat0"), Some("user")),
        )
        .with_session("2", local_seat(ALICE))
        .policy();
    assert_eq!(
        inactive.authorize_preview_placement(ALICE, &own),
        Err(SessionDenial::CallerSessionInactive)
    );

    let greeter = MockLogind::default()
        .with_session(
            "4",
            record(
                Some(ALICE),
                true,
                Some(false),
                Some("seat0"),
                Some("greeter"),
            ),
        )
        .policy();
    assert_eq!(
        greeter.authorize_preview_placement(ALICE, &own),
        Err(SessionDenial::CallerSessionClass)
    );

    let foreign = MockLogind::default()
        .with_session("4", local_seat(BOB))
        .policy();
    assert_eq!(
        foreign.authorize_preview_placement(ALICE, &own),
        Err(SessionDenial::CallerSessionForeign)
    );

    assert_eq!(
        MockLogind::failing()
            .policy()
            .authorize_preview_placement(ALICE, &own),
        Err(SessionDenial::LogindUnavailable),
        "an unreadable logind state fails closed"
    );
}

#[test]
fn test_rsh_user_manager_peer_refused_while_remote_session_exists() {
    let manager = PeerPlacement::UserManager(ALICE);

    let served = MockLogind::default()
        .with_session("2", local_seat(ALICE))
        .with_session("9", ssh(BOB))
        .policy();
    assert_eq!(
        served.authorize_preview_placement(ALICE, &manager),
        Ok(()),
        "another user's SSH session does not matter"
    );

    let with_ssh = MockLogind::default()
        .with_session("2", local_seat(ALICE))
        .with_session("7", ssh(ALICE))
        .policy();
    assert_eq!(
        with_ssh.authorize_preview_placement(ALICE, &manager),
        Err(SessionDenial::UserManagerCallerRemoteSessionActive)
    );

    // A closing SSH session (process left running from it) still counts.
    let closing = MockLogind::default()
        .with_session("2", local_seat(ALICE))
        .with_session(
            "7",
            record(Some(ALICE), false, Some(true), None, Some("user")),
        )
        .policy();
    assert_eq!(
        closing.authorize_preview_placement(ALICE, &manager),
        Err(SessionDenial::UserManagerCallerRemoteSessionActive)
    );

    let uidless = MockLogind::default()
        .with_session("2", local_seat(ALICE))
        .with_session("8", record(None, true, None, None, Some("user")))
        .policy();
    assert_eq!(
        uidless.authorize_preview_placement(ALICE, &manager),
        Err(SessionDenial::UserManagerCallerRemoteSessionActive),
        "a record without UID counts as possibly remote"
    );

    let no_seat = MockLogind::default().policy();
    assert_eq!(
        no_seat.authorize_preview_placement(ALICE, &manager),
        Err(SessionDenial::UserManagerNoLocalSeatSession)
    );

    assert_eq!(
        served.authorize_preview_placement(ALICE, &PeerPlacement::UserManager(BOB)),
        Err(SessionDenial::UserManagerUidMismatch),
        "a foreign user manager is refused"
    );

    assert_eq!(
        MockLogind::failing()
            .policy()
            .authorize_preview_placement(ALICE, &manager),
        Err(SessionDenial::LogindUnavailable)
    );
}

#[test]
fn test_rsh_other_placement_keeps_seat_rule_policy() {
    // `Other` is decided by the UID-level seat check (6c-2), not by the placement rule.
    for policy in [
        MockLogind::default().policy(),
        MockLogind::default().with_session("7", ssh(ALICE)).policy(),
        MockLogind::failing().policy(),
    ] {
        assert_eq!(
            policy.authorize_preview_placement(ALICE, &PeerPlacement::Other),
            Ok(())
        );
    }
}

#[test]
fn test_rsh_placement_rule_disabled_policy() {
    let disabled = LocalSessionPolicy::disabled();
    for placement in [
        PeerPlacement::Session("4".to_string()),
        PeerPlacement::UserManager(ALICE),
        PeerPlacement::UserManager(BOB),
        PeerPlacement::Other,
    ] {
        assert_eq!(
            disabled.authorize_preview_placement(ALICE, &placement),
            Ok(()),
            "{placement:?}"
        );
    }
}

#[test]
fn test_rsh_root_auth_user_manager_rule_unchanged() {
    let root = PeerCredentials {
        uid: 0,
        gid: 0,
        pid: Some(4242),
    };
    #[derive(Debug)]
    struct ManagerCaller {
        records: Vec<SessionRecord>,
    }
    impl LogindSource for ManagerCaller {
        fn session_id_of_pid(&self, _pid: i32) -> Result<Option<String>, LogindError> {
            Ok(None)
        }
        fn session(&self, _session_id: &str) -> Result<Option<SessionRecord>, LogindError> {
            Ok(None)
        }
        fn sessions(&self) -> Result<Vec<SessionRecord>, LogindError> {
            Ok(self.records.clone())
        }
        fn cgroup_of_pid(&self, _pid: i32) -> Result<Option<String>, LogindError> {
            Ok(Some(user_unit(ALICE, "gnome-terminal-server.service")))
        }
    }
    let policy =
        |records: Vec<SessionRecord>| LocalSessionPolicy::new(Arc::new(ManagerCaller { records }));
    assert_eq!(
        policy(vec![local_seat(ALICE)]).authorize_auth(&root, ALICE),
        Ok(())
    );
    assert_eq!(
        policy(vec![local_seat(ALICE), ssh(ALICE)]).authorize_auth(&root, ALICE),
        Err(SessionDenial::UserManagerCallerRemoteSessionActive)
    );
    assert_eq!(
        policy(vec![
            local_seat(ALICE),
            record(None, true, None, None, Some("user"))
        ])
        .authorize_auth(&root, ALICE),
        Err(SessionDenial::UserManagerCallerRemoteSessionActive)
    );
    assert_eq!(
        policy(Vec::new()).authorize_auth(&root, ALICE),
        Err(SessionDenial::UserManagerNoLocalSeatSession)
    );
    assert_eq!(
        policy(vec![local_seat(BOB)]).authorize_auth(&root, BOB),
        Err(SessionDenial::UserManagerUidMismatch)
    );
}

// ---------------------------------------------------------------------------
// Dispatcher wiring (6c-2a)
// ---------------------------------------------------------------------------

fn current_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

fn dispatcher_config() -> DispatcherConfig {
    DispatcherConfig {
        max_concurrent_connections: 8,
        connection_timeout: Duration::from_millis(2000),
        enforce_active_session: false,
        logind_sessions_dir: PathBuf::from("/run/systemd/sessions"),
    }
}

fn preview_config(uid: u32) -> PreviewConfig {
    PreviewConfig {
        enabled: true,
        allowed_uids: vec![uid],
        remote_view: true,
        ..PreviewConfig::default()
    }
}

fn preview_request(uid: u32, nonce: u8) -> Request {
    Request {
        version: CURRENT_VERSION,
        kind: RequestKind::PreviewFrame,
        request_id: [nonce; 32],
        uid_hint: uid,
        service: "soos-gui".to_string(),
        deadline_monotonic_ns: u64::MAX,
    }
}

fn write_session(dir: &Path, id: &str, content: &str) {
    std::fs::create_dir_all(dir).expect("sessions dir");
    std::fs::write(dir.join(id), content).expect("session file");
}

fn seat_session(uid: u32) -> String {
    format!("UID={uid}\nACTIVE=1\nSTATE=active\nREMOTE=0\nSEAT=seat0\nCLASS=user\n")
}

fn remote_session(uid: u32) -> String {
    format!("UID={uid}\nACTIVE=1\nSTATE=active\nREMOTE=1\nCLASS=user\n")
}

async fn start(
    dir: &Path,
    validator: SessionValidator,
    cgroup: String,
    uid: u32,
) -> std::path::PathBuf {
    let sock_path = dir.join("placement.sock");
    let dispatcher = Arc::new(
        ConnectionDispatcher::new(dispatcher_config(), Arc::new(HealthState::new()))
            .with_session_validator(validator)
            .with_preview_config(preview_config(uid))
            .with_preview_peer_source(Arc::new(CgroupSource(cgroup))),
    );
    let listener = UnixListener::bind(&sock_path).expect("Bind failed");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let d = dispatcher.clone();
            tokio::spawn(async move {
                let _ = d.handle_connection(stream).await;
            });
        }
    });
    sock_path
}

async fn exchange(client: &mut UnixStream, req: &Request) -> Vec<u8> {
    let framed = encode(req).expect("Encoding failed");
    client.write_all(&framed).await.expect("Write request");
    client.flush().await.expect("Flush client");
    let mut len_bytes = [0u8; 4];
    client.read_exact(&mut len_bytes).await.expect("Read len");
    let resp_len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; 4 + resp_len];
    buf[..4].copy_from_slice(&len_bytes);
    client.read_exact(&mut buf[4..]).await.expect("Read body");
    buf
}

fn assert_served(buf: &[u8], what: &str) {
    let preview: Result<PreviewResponse, _> = decode_preview(buf);
    match preview {
        Ok(p) => assert_eq!(p.version, CURRENT_VERSION, "{what}"),
        Err(e) => {
            let refusal: Option<Response> = decode(buf).ok();
            panic!(
                "{what}: expected a PreviewResponse, got {:?} ({e:?})",
                refusal.map(|r| (r.verdict, r.reason_class))
            )
        }
    }
}

fn assert_refused(buf: &[u8], nonce: u8, what: &str) {
    let resp: Response =
        decode(buf).unwrap_or_else(|e| panic!("{what}: a refusal is a standard Response: {e:?}"));
    assert_eq!(resp.request_id, [nonce; 32], "{what}");
    assert_eq!(resp.verdict, Verdict::ProtocolError, "{what}");
    assert_eq!(resp.reason_class, ReasonClass::UidMismatch, "{what}");
}

/// One dispatcher wiring case: name, session records, peer cgroup, expected served.
type WiringCase = (&'static str, Vec<(&'static str, String)>, String, bool);

#[tokio::test]
async fn test_rsh_session_peer_dispatcher_wiring() {
    let uid = current_uid();
    if uid == 0 {
        return;
    }
    // (name, records, peer cgroup, expected served)
    let cases: Vec<WiringCase> = vec![
        (
            "own local seat session",
            vec![("4", seat_session(uid))],
            session_scope(uid, "4"),
            true,
        ),
        (
            "own session remote, another local seat session exists",
            vec![("4", remote_session(uid)), ("2", seat_session(uid))],
            session_scope(uid, "4"),
            false,
        ),
        (
            "own session without record, another local seat session exists",
            vec![("2", seat_session(uid))],
            session_scope(uid, "4"),
            false,
        ),
        (
            "user manager without remote session",
            vec![("2", seat_session(uid))],
            user_unit(uid, "soos-remote.service"),
            true,
        ),
        (
            "user manager while an SSH session of the UID exists",
            vec![("2", seat_session(uid)), ("7", remote_session(uid))],
            user_unit(uid, "soos-remote.service"),
            false,
        ),
        (
            "user manager of another UID",
            vec![("2", seat_session(uid))],
            user_unit(uid.wrapping_add(1), "soos-gui.service"),
            false,
        ),
    ];
    for (i, (name, records, cgroup, expected)) in cases.into_iter().enumerate() {
        let dir = tempdir().expect("tempdir");
        let sessions = dir.path().join("sessions");
        for (id, content) in &records {
            write_session(&sessions, id, content);
        }
        let sock = start(
            dir.path(),
            SessionValidator::with_sessions_dir(sessions),
            cgroup,
            uid,
        )
        .await;
        let mut client = UnixStream::connect(&sock).await.expect("Connect");
        let nonce = 0x40 + i as u8;
        let buf = exchange(&mut client, &preview_request(uid, nonce)).await;
        if expected {
            assert_served(&buf, name);
        } else {
            assert_refused(&buf, nonce, name);
        }
    }
}

#[tokio::test]
async fn test_rsh_other_placement_keeps_seat_rule() {
    let uid = current_uid();
    if uid == 0 {
        return;
    }
    let other = "0::/system.slice/github-runner.service\n".to_string();
    for (name, records, expected) in [
        (
            "seat session of the UID",
            vec![("2", seat_session(uid))],
            true,
        ),
        (
            "seat session and an SSH session of the UID",
            vec![("2", seat_session(uid)), ("7", remote_session(uid))],
            true,
        ),
        (
            "only an SSH session",
            vec![("7", remote_session(uid))],
            false,
        ),
    ] {
        let dir = tempdir().expect("tempdir");
        let sessions = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions).expect("sessions dir");
        for (id, content) in &records {
            write_session(&sessions, id, content);
        }
        let sock = start(
            dir.path(),
            SessionValidator::with_sessions_dir(sessions),
            other.clone(),
            uid,
        )
        .await;
        let mut client = UnixStream::connect(&sock).await.expect("Connect");
        let buf = exchange(&mut client, &preview_request(uid, 0x50)).await;
        if expected {
            assert_served(&buf, name);
        } else {
            assert_refused(&buf, 0x50, name);
        }
    }
}

#[tokio::test]
async fn test_rsh_placement_rule_disabled_with_validator() {
    let uid = current_uid();
    if uid == 0 {
        return;
    }
    for cgroup in [
        session_scope(uid, "4"),
        user_unit(uid, "soos-remote.service"),
    ] {
        let dir = tempdir().expect("tempdir");
        let sock = start(dir.path(), SessionValidator::disabled(), cgroup, uid).await;
        let mut client = UnixStream::connect(&sock).await.expect("Connect");
        let buf = exchange(&mut client, &preview_request(uid, 0x51)).await;
        assert_served(&buf, "disabled validator: no session records needed");
    }
    assert!(!LocalSessionPolicy::from_validator(&SessionValidator::disabled()).is_enforced());
}

#[tokio::test]
async fn test_rsh_placement_rechecked_per_request() {
    let uid = current_uid();
    if uid == 0 {
        return;
    }
    let dir = tempdir().expect("tempdir");
    let sessions = dir.path().join("sessions");
    write_session(&sessions, "4", &seat_session(uid));
    // Another local seat session keeps the UID-level check (6c-2) true throughout.
    write_session(&sessions, "2", &seat_session(uid));
    let sock = start(
        dir.path(),
        SessionValidator::with_sessions_dir(sessions.clone()),
        session_scope(uid, "4"),
        uid,
    )
    .await;
    let mut client = UnixStream::connect(&sock).await.expect("Connect");
    let buf = exchange(&mut client, &preview_request(uid, 0x61)).await;
    assert_served(&buf, "first request in a local seat session");
    // The peer's own session turns remote between two requests on the same connection.
    write_session(&sessions, "4", &remote_session(uid));
    let buf = exchange(&mut client, &preview_request(uid, 0x62)).await;
    assert_refused(
        &buf,
        0x62,
        "the placement rule is evaluated on every request",
    );
}
