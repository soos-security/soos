//! Contract of the push sender socket candidates of `soos-remote` (ADR "[2026-10-23] Push
//! Sender as a Sandboxed System Service"; spec `AI/architect_spec_push_sender_system_service.md`
//! §5; matrix PSS10).
//!
//! - Without an explicit `push_socket_path` (the configured path equals the derived default
//!   `$XDG_RUNTIME_DIR/soos-push/push.sock`), the candidates are the system socket
//!   `/run/soos-push/<uid>/push.sock` (peer uid 0: PID 1 listens for the socket-activated
//!   system sender) and then the legacy per-user socket (peer uid = owner).
//! - An explicit path is the only candidate (peer uid = owner).
//! - Per exchange the first candidate whose path exists is used, and only it: a refused
//!   socket never falls through to the next candidate.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::path::{Path, PathBuf};

use soos_push_protocol::{
    encode_reply, system_push_socket_path, DeliveryReply, DeliveryRequest, Outcome, PushEndpoint,
    Urgency,
};
use soos_remote::push::{
    push_socket_candidates, PushSocketCandidate, PushTransport, TransportError, UnixPushTransport,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

fn own_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

fn request() -> DeliveryRequest {
    DeliveryRequest {
        endpoint: PushEndpoint::parse(
            "https://web.push.apple.com/QGuQyavXutnMH8l2ce1fmbpFBjgLv9SQK3-ASsYlvuDH_tok1",
        )
        .unwrap(),
        authorization: zeroize::Zeroizing::new("vapid t=a.b.c, k=BAAA".to_string()),
        ttl_s: 60,
        urgency: Urgency::High,
        topic: None,
        body: zeroize::Zeroizing::new(vec![0x42; 32]),
    }
}

fn answer() -> DeliveryReply {
    DeliveryReply {
        outcome: Outcome::Delivered,
        status: Some(201),
        retry_after_s: None,
    }
}

/// Accepts one connection, reads one frame and answers `answer()`.
fn fake_sender(listener: UnixListener) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut prefix = [0u8; 4];
        stream.read_exact(&mut prefix).await.unwrap();
        let mut payload = vec![0u8; u32::from_be_bytes(prefix) as usize];
        stream.read_exact(&mut payload).await.unwrap();
        stream
            .write_all(&encode_reply(&answer()).unwrap())
            .await
            .unwrap();
    })
}

#[test]
fn test_pss10_candidates_for_the_default_path() {
    let derived = PathBuf::from("/run/user/1000/soos-push/push.sock");
    assert_eq!(
        push_socket_candidates(&derived, Some(&derived), 1000),
        vec![
            PushSocketCandidate {
                path: PathBuf::from("/run/soos-push/1000/push.sock"),
                peer_uid: 0,
            },
            PushSocketCandidate {
                path: derived.clone(),
                peer_uid: 1000,
            },
        ]
    );
    assert_eq!(
        push_socket_candidates(&derived, Some(&derived), 1000)[0].path,
        system_push_socket_path(1000)
    );
}

#[test]
fn test_pss10_candidates_for_an_explicit_path() {
    let explicit = PathBuf::from("/srv/push/push.sock");
    let derived = PathBuf::from("/run/user/1000/soos-push/push.sock");
    for derived in [Some(derived.as_path()), None] {
        assert_eq!(
            push_socket_candidates(&explicit, derived, 1000),
            vec![PushSocketCandidate {
                path: explicit.clone(),
                peer_uid: 1000,
            }]
        );
    }
    // Without a derivable default, the configured path is the only candidate.
    let configured = Path::new("/run/user/1000/soos-push/push.sock");
    assert_eq!(
        push_socket_candidates(configured, None, 1000),
        vec![PushSocketCandidate {
            path: configured.to_path_buf(),
            peer_uid: 1000,
        }]
    );
}

/// The system socket is absent (no system unit on this host): the legacy socket is used.
#[tokio::test]
async fn test_pss10_absent_first_candidate_falls_back() {
    let uid = own_uid();
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("system").join("push.sock");
    let legacy = dir.path().join("push.sock");
    let sender = fake_sender(UnixListener::bind(&legacy).unwrap());
    let transport = UnixPushTransport::with_candidates(
        vec![
            PushSocketCandidate {
                path: absent,
                peer_uid: 0,
            },
            PushSocketCandidate {
                path: legacy,
                peer_uid: uid,
            },
        ],
        uid,
    );
    assert_eq!(transport.deliver(request()).await, Ok(answer()));
    sender.await.unwrap();
}

/// A present first candidate whose peer is not the expected uid is refused, and the next
/// (live) candidate is never tried: no downgrade.
#[tokio::test]
async fn test_pss10_refused_first_candidate_never_falls_through() {
    let uid = own_uid();
    if uid == 0 {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.sock");
    let second = dir.path().join("second.sock");
    // Our own listener at the "system" position: its peer uid is ours, not 0.
    let first_listener = std::os::unix::net::UnixListener::bind(&first).unwrap();
    first_listener.set_nonblocking(true).unwrap();
    let second_listener = std::os::unix::net::UnixListener::bind(&second).unwrap();
    second_listener.set_nonblocking(true).unwrap();
    let transport = UnixPushTransport::with_candidates(
        vec![
            PushSocketCandidate {
                path: first,
                peer_uid: 0,
            },
            PushSocketCandidate {
                path: second,
                peer_uid: uid,
            },
        ],
        uid,
    );
    assert_eq!(
        transport.deliver(request()).await,
        Err(TransportError::Unavailable)
    );
    assert!(
        second_listener.accept().is_err(),
        "the second candidate was never connected"
    );
    // A regular file at the first position is refused the same way.
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("plain.sock");
    std::fs::write(&plain, b"x").unwrap();
    let live = dir.path().join("live.sock");
    let live_listener = std::os::unix::net::UnixListener::bind(&live).unwrap();
    live_listener.set_nonblocking(true).unwrap();
    let transport = UnixPushTransport::with_candidates(
        vec![
            PushSocketCandidate {
                path: plain,
                peer_uid: uid,
            },
            PushSocketCandidate {
                path: live,
                peer_uid: uid,
            },
        ],
        uid,
    );
    assert_eq!(
        transport.deliver(request()).await,
        Err(TransportError::Unavailable)
    );
    assert!(live_listener.accept().is_err());
}

/// No candidate exists, or none at all: `Unavailable`.
#[tokio::test]
async fn test_pss10_no_candidate_is_unavailable() {
    let uid = own_uid();
    let dir = tempfile::tempdir().unwrap();
    let transport = UnixPushTransport::with_candidates(
        vec![
            PushSocketCandidate {
                path: dir.path().join("a.sock"),
                peer_uid: 0,
            },
            PushSocketCandidate {
                path: dir.path().join("b.sock"),
                peer_uid: uid,
            },
        ],
        uid,
    );
    assert_eq!(
        transport.deliver(request()).await,
        Err(TransportError::Unavailable)
    );
    let empty = UnixPushTransport::with_candidates(Vec::new(), uid);
    assert_eq!(
        empty.deliver(request()).await,
        Err(TransportError::Unavailable)
    );
}

/// The socket inode must be owned by the owner uid for every candidate, whatever its peer.
#[tokio::test]
async fn test_pss10_socket_owner_is_checked_for_every_candidate() {
    let uid = own_uid();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("push.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let transport = UnixPushTransport::with_candidates(
        vec![PushSocketCandidate {
            path,
            peer_uid: uid,
        }],
        uid.wrapping_add(1),
    );
    assert_eq!(
        transport.deliver(request()).await,
        Err(TransportError::Unavailable)
    );
    assert!(listener.accept().is_err(), "never connected");
}
