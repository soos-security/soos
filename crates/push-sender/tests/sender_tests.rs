//! Contract tests of the push sender library `soos-push-sender` (ADR 2026-10-06 "Web Push
//! Notifications for Failed-Password Alerts Through a Separate Sender Unit", architect spec
//! `AI/architect_spec_remote_web_push.md` §8.1, tests 7–12 and 52; matrix RMC62, RMC71).
//!
//! No network: DNS answers come from an injected `AddressLookup`, the HTTP side of
//! `serve_connection` is a fake `Deliverer`, and test 52 only drives `UreqDeliverer` into
//! addresses its resolver must refuse (no connection is ever attempted). Socket tests use a
//! `TempDir`; nothing touches `$XDG_RUNTIME_DIR`.
//!
//! Plan-evaluator round-2 finding encoded here: F-16 (a frame is bounded by one cumulative
//! deadline, not a per-read timeout: a trickling peer is dropped within the frame bound).

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

use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use soos_push_protocol::{
    decode_reply, encode_reply, encode_request, DeliveryReply, DeliveryRequest, Outcome,
    PushEndpoint, Urgency, MAX_PUSH_FRAME_BYTES, MAX_PUSH_RESPONSE_BODY_BYTES,
    MAX_PUSH_RESPONSE_HEADER_BYTES, MAX_RESOLVED_ADDRESSES, PUSH_CONNECT_TIMEOUT_MS,
    PUSH_FRAME_IO_TIMEOUT_MS, PUSH_SEND_TIMEOUT_MS, PUSH_SOCKET_DIR_NAME, PUSH_SOCKET_FILE_NAME,
};
use soos_push_sender::{
    bind_socket, check_not_root, filter_addresses, send_error_outcome, serve_connection,
    AddressLookup, Deliverer, LookupError, ResolveRefused, SendPolicy, UreqDeliverer,
};

// ---------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------

fn sa(ip: &str, port: u16) -> SocketAddr {
    SocketAddr::new(ip.parse::<IpAddr>().unwrap(), port)
}

/// An `AddressLookup` answering `answer`, recording every host it is asked for.
fn scripted_lookup(
    answer: Result<Vec<SocketAddr>, LookupError>,
) -> (AddressLookup, Arc<Mutex<Vec<String>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&asked);
    let lookup: AddressLookup = Arc::new(move |host: &str| {
        seen.lock().unwrap().push(host.to_string());
        answer.clone()
    });
    (lookup, asked)
}

fn valid_request() -> DeliveryRequest {
    DeliveryRequest {
        endpoint: PushEndpoint::parse("https://web.push.apple.com/QGuQyavXutnMH8l2ce1fmbpF")
            .unwrap(),
        authorization: Zeroizing::new(
            "vapid t=eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.e30.c2ln, k=BAAA".to_string(),
        ),
        ttl_s: 43_200,
        urgency: Urgency::High,
        topic: Some("soos-alerts".to_string()),
        body: Zeroizing::new(vec![0x5a; 120]),
    }
}

/// A `Deliverer` that records each request and answers a fixed reply.
struct FakeDeliverer {
    seen: Mutex<Vec<DeliveryRequest>>,
    reply: DeliveryReply,
}

impl FakeDeliverer {
    fn new(reply: DeliveryReply) -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            reply,
        }
    }

    fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

impl Deliverer for FakeDeliverer {
    fn deliver(&self, request: &DeliveryRequest) -> DeliveryReply {
        self.seen.lock().unwrap().push(request.clone());
        self.reply
    }
}

const RETRY_120: DeliveryReply = DeliveryReply {
    outcome: Outcome::Retry,
    status: Some(429),
    retry_after_s: Some(120),
};

const REFUSED: DeliveryReply = DeliveryReply {
    outcome: Outcome::Refused,
    status: None,
    retry_after_s: None,
};

fn own_uid_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().uid()
}

/// Runs `serve_connection` on one end of a fresh pair in a thread; returns the client end,
/// the server thread and the time the thread ended.
fn serve_in_thread(
    deliverer: Arc<FakeDeliverer>,
    uid: u32,
) -> (UnixStream, thread::JoinHandle<Duration>) {
    let (client, server) = UnixStream::pair().unwrap();
    let handle = thread::spawn(move || {
        let start = Instant::now();
        serve_connection(server, uid, &*deliverer);
        start.elapsed()
    });
    (client, handle)
}

/// Reads one reply frame (or `None` at EOF before any byte).
fn read_reply(client: &mut UnixStream) -> Option<Vec<u8>> {
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut prefix = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match client.read(&mut prefix[got..]) {
            Ok(0) => {
                assert_eq!(got, 0, "partial reply prefix");
                return None;
            }
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return None,
            Err(e) => panic!("read: {e}"),
        }
    }
    let len = u32::from_be_bytes(prefix) as usize;
    let mut payload = vec![0u8; len];
    client.read_exact(&mut payload).unwrap();
    Some(payload)
}

fn own_uid() -> u32 {
    let dir = tempfile::tempdir().unwrap();
    own_uid_of(dir.path())
}

// ---------------------------------------------------------------------------------------
// Test 7 — address filter
// ---------------------------------------------------------------------------------------

/// Test 7 (RMC62, W-5 b, §8.1): public answers become port 443, IPv4 first (stable within a
/// family), at most 16; one private address, an empty answer or a non-allowlisted host is
/// `Refused` (the lookup is never called for the latter); a lookup error is `Retry`; the
/// lookup is called exactly once with exactly the host.
#[test]
fn test_rwp_sender_filter_addresses() {
    let answer = vec![
        sa("2a04:4e42:6a::347", 0),
        sa("17.188.143.78", 0),
        sa("2620:149:a44::1", 8080),
        sa("17.188.143.79", 80),
    ];
    let (lookup, asked) = scripted_lookup(Ok(answer));
    let addrs = filter_addresses("web.push.apple.com", &*lookup).unwrap();
    assert_eq!(
        addrs,
        vec![
            sa("17.188.143.78", 443),
            sa("17.188.143.79", 443),
            sa("2a04:4e42:6a::347", 443),
            sa("2620:149:a44::1", 443),
        ]
    );
    assert_eq!(
        *asked.lock().unwrap(),
        vec!["web.push.apple.com".to_string()]
    );

    // Truncated to MAX_RESOLVED_ADDRESSES (IPv4 first).
    let mut many = Vec::new();
    for i in 0..12u8 {
        many.push(SocketAddr::new(
            IpAddr::V6(Ipv6Addr::new(
                0x2a04,
                0x4e42,
                0,
                0,
                0,
                0,
                0,
                u16::from(i) + 1,
            )),
            443,
        ));
        many.push(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(17, 188, 143, i + 1)),
            443,
        ));
    }
    let (lookup, _) = scripted_lookup(Ok(many));
    let addrs = filter_addresses("fcm.googleapis.com", &*lookup).unwrap();
    assert_eq!(addrs.len(), MAX_RESOLVED_ADDRESSES);
    assert!(addrs[..12].iter().all(SocketAddr::is_ipv4));
    assert!(addrs[12..].iter().all(SocketAddr::is_ipv6));
    assert_eq!(addrs[0], sa("17.188.143.1", 443));
    assert_eq!(addrs[12], sa("2a04:4e42::1", 443));

    // One private address refuses the whole answer.
    for private in [
        "10.0.0.1",
        "127.0.0.1",
        "100.100.100.100",
        "192.168.1.1",
        "fd7a:115c:a1e0::53",
        "::1",
        "::ffff:17.188.143.78",
    ] {
        let (lookup, asked) = scripted_lookup(Ok(vec![sa("17.188.143.78", 0), sa(private, 0)]));
        assert_eq!(
            filter_addresses("web.push.apple.com", &*lookup),
            Err(Outcome::Refused),
            "{private}"
        );
        assert_eq!(asked.lock().unwrap().len(), 1);
    }
    // Empty answer.
    let (lookup, _) = scripted_lookup(Ok(Vec::new()));
    assert_eq!(
        filter_addresses("updates.push.services.mozilla.com", &*lookup),
        Err(Outcome::Refused)
    );
    // Lookup errors are transient.
    for error in [LookupError::Failed, LookupError::Timeout] {
        let (lookup, asked) = scripted_lookup(Err(error));
        assert_eq!(
            filter_addresses("web.push.apple.com", &*lookup),
            Err(Outcome::Retry)
        );
        assert_eq!(asked.lock().unwrap().len(), 1);
    }
    // A host outside the allowlist: refused, the lookup never runs.
    for host in [
        "evil.example",
        "web.push.apple.com.",
        "WEB.PUSH.APPLE.COM",
        "web.push.apple.com.evil.example",
        "127.0.0.1",
        "",
    ] {
        let (lookup, asked) = scripted_lookup(Ok(vec![sa("17.188.143.78", 0)]));
        assert_eq!(
            filter_addresses(host, &*lookup),
            Err(Outcome::Refused),
            "{host:?}"
        );
        assert!(
            asked.lock().unwrap().is_empty(),
            "{host:?}: lookup not called"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Tests 8, 9 — one connection
// ---------------------------------------------------------------------------------------

/// Test 8 (RMC71, §8.1): a valid frame reaches the deliverer exactly once, decoded; the
/// reply frame is exactly `encode_reply` of the deliverer's answer; the connection closes.
#[test]
fn test_rwp_sender_serves_one_request_with_fake_deliverer() {
    let fake = Arc::new(FakeDeliverer::new(RETRY_120));
    let (mut client, handle) = serve_in_thread(Arc::clone(&fake), own_uid());
    client
        .write_all(&encode_request(&valid_request()).unwrap())
        .unwrap();
    let payload = read_reply(&mut client).expect("a reply");
    let expected = encode_reply(&RETRY_120).unwrap();
    assert_eq!(payload, expected[4..].to_vec());
    assert_eq!(decode_reply(&payload).unwrap(), RETRY_120);
    let mut rest = Vec::new();
    client.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty(), "one reply, then the connection closes");
    handle.join().unwrap();
    let seen = fake.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0] == valid_request(),
        "the decoded request is handed over"
    );
}

/// Test 9 (RMC61, RMC71, §8.1, F-16): an oversized prefix closes without a reply and without
/// the deliverer; malformed JSON and a non-allowlisted endpoint are answered `refused`
/// without the deliverer; a stalled peer is dropped within the frame bound; a peer that
/// trickles bytes (every read succeeds) is dropped within the same cumulative bound, never
/// held for the length of its trickle.
#[test]
fn test_rwp_sender_refuses_bad_frames() {
    let uid = own_uid();
    let delivered = DeliveryReply {
        outcome: Outcome::Delivered,
        status: Some(201),
        retry_after_s: None,
    };

    // Oversized prefix.
    let fake = Arc::new(FakeDeliverer::new(delivered));
    let (mut client, handle) = serve_in_thread(Arc::clone(&fake), uid);
    client
        .write_all(&((MAX_PUSH_FRAME_BYTES as u32) + 1).to_be_bytes())
        .unwrap();
    let _ = client.write_all(&[b'{'; 64]);
    assert_eq!(read_reply(&mut client), None, "closed without a reply");
    handle.join().unwrap();
    assert_eq!(fake.calls(), 0);

    // Malformed JSON → refused.
    let fake = Arc::new(FakeDeliverer::new(delivered));
    let (mut client, handle) = serve_in_thread(Arc::clone(&fake), uid);
    let junk = b"{\"v\":1,\"endpoint\":";
    client
        .write_all(&(junk.len() as u32).to_be_bytes())
        .unwrap();
    client.write_all(junk).unwrap();
    let payload = read_reply(&mut client).expect("a refused reply");
    assert_eq!(decode_reply(&payload).unwrap(), REFUSED);
    handle.join().unwrap();
    assert_eq!(fake.calls(), 0);

    // Non-allowlisted endpoint (the frame is otherwise valid) → refused.
    let fake = Arc::new(FakeDeliverer::new(delivered));
    let (mut client, handle) = serve_in_thread(Arc::clone(&fake), uid);
    let frame = encode_request(&valid_request()).unwrap();
    let text = String::from_utf8(frame[4..].to_vec()).unwrap();
    let evil = text.replace(
        "https://web.push.apple.com/QGuQyavXutnMH8l2ce1fmbpF",
        "https://evil.example/QGuQyavXutnMH8l2ce1fmbpF",
    );
    assert_ne!(evil, text);
    client
        .write_all(&(evil.len() as u32).to_be_bytes())
        .unwrap();
    client.write_all(evil.as_bytes()).unwrap();
    let payload = read_reply(&mut client).expect("a refused reply");
    assert_eq!(decode_reply(&payload).unwrap(), REFUSED);
    handle.join().unwrap();
    assert_eq!(fake.calls(), 0, "the deliverer never sees it");

    // A peer that sends 2 bytes and stalls.
    let fake = Arc::new(FakeDeliverer::new(delivered));
    let (mut client, handle) = serve_in_thread(Arc::clone(&fake), uid);
    client.write_all(&[0, 0]).unwrap();
    let elapsed = handle.join().unwrap();
    assert!(
        elapsed <= Duration::from_millis(PUSH_FRAME_IO_TIMEOUT_MS + 1000),
        "stalled peer held for {elapsed:?}"
    );
    assert_eq!(read_reply(&mut client), None);
    assert_eq!(fake.calls(), 0);

    // F-16: a peer trickling the prefix (one byte every 1.5 s: each single read would
    // succeed within PUSH_FRAME_IO_TIMEOUT_MS) is dropped at the prefix deadline.
    let fake = Arc::new(FakeDeliverer::new(delivered));
    let (client, handle) = serve_in_thread(Arc::clone(&fake), uid);
    let trickler = thread::spawn(move || {
        let mut client = client;
        for _ in 0..8 {
            if client.write_all(&[0]).is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(1500));
        }
    });
    let elapsed = handle.join().unwrap();
    assert!(
        elapsed <= Duration::from_millis(PUSH_FRAME_IO_TIMEOUT_MS + 1000),
        "a trickled prefix held the sender for {elapsed:?}"
    );
    assert_eq!(fake.calls(), 0);
    trickler.join().unwrap();

    // F-16: a valid prefix, then the payload trickled one byte per second.
    let fake = Arc::new(FakeDeliverer::new(delivered));
    let (client, handle) = serve_in_thread(Arc::clone(&fake), uid);
    let frame = encode_request(&valid_request()).unwrap();
    let trickler = thread::spawn(move || {
        let mut client = client;
        if client.write_all(&frame[..4]).is_err() {
            return;
        }
        for byte in frame[4..].iter().take(30) {
            if client.write_all(&[*byte]).is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(1000));
        }
    });
    let elapsed = handle.join().unwrap();
    assert!(
        elapsed <= Duration::from_millis(2 * PUSH_FRAME_IO_TIMEOUT_MS + 1000),
        "a trickled payload held the sender for {elapsed:?}"
    );
    assert_eq!(fake.calls(), 0);
    trickler.join().unwrap();

    // A peer with another uid is closed without reading or delivering.
    let fake = Arc::new(FakeDeliverer::new(delivered));
    let (mut client, handle) = serve_in_thread(Arc::clone(&fake), uid.wrapping_add(1));
    let _ = client.write_all(&encode_request(&valid_request()).unwrap());
    assert_eq!(read_reply(&mut client), None);
    handle.join().unwrap();
    assert_eq!(fake.calls(), 0, "peer uid mismatch: closed");
}

// ---------------------------------------------------------------------------------------
// Test 10 — socket setup
// ---------------------------------------------------------------------------------------

/// Test 10 (RMC71, §8.1): `bind_socket(<runtime>/soos-push, uid)` creates the directory
/// `0700` when absent and the socket `0600`; a stale socket of the uid is replaced; a
/// regular file or a symlink at the socket path is an error and stays untouched.
#[test]
fn test_rwp_sender_socket_setup() {
    let runtime = tempfile::tempdir().unwrap();
    let uid = own_uid_of(runtime.path());
    let dir = runtime.path().join(PUSH_SOCKET_DIR_NAME);
    let sock = dir.join(PUSH_SOCKET_FILE_NAME);

    // Absent directory: created 0700, socket 0600, connectable.
    let listener = bind_socket(&dir, uid).unwrap();
    let meta = fs::symlink_metadata(&dir).unwrap();
    assert!(meta.is_dir());
    assert_eq!(meta.permissions().mode() & 0o7777, 0o700);
    let meta = fs::symlink_metadata(&sock).unwrap();
    assert!(meta.file_type().is_socket());
    assert_eq!(meta.permissions().mode() & 0o7777, 0o600);
    assert_eq!(meta.uid(), uid);
    let _client = UnixStream::connect(&sock).unwrap();
    drop(listener);

    // Stale socket of the uid (left by the previous run): replaced.
    assert!(fs::symlink_metadata(&sock).unwrap().file_type().is_socket());
    let listener = bind_socket(&dir, uid).unwrap();
    assert_eq!(
        fs::symlink_metadata(&sock).unwrap().permissions().mode() & 0o7777,
        0o600
    );
    let _client = UnixStream::connect(&sock).unwrap();
    drop(listener);

    // An existing directory (RuntimeDirectory=) is used as is.
    let runtime2 = tempfile::tempdir().unwrap();
    let dir2 = runtime2.path().join(PUSH_SOCKET_DIR_NAME);
    fs::create_dir(&dir2).unwrap();
    fs::set_permissions(&dir2, fs::Permissions::from_mode(0o700)).unwrap();
    let listener = bind_socket(&dir2, uid).unwrap();
    drop(listener);

    // A regular file at the socket path.
    fs::remove_file(&sock).unwrap();
    fs::write(&sock, b"not a socket").unwrap();
    assert!(bind_socket(&dir, uid).is_err());
    assert_eq!(fs::read(&sock).unwrap(), b"not a socket");

    // A symlink at the socket path (to a socket elsewhere).
    fs::remove_file(&sock).unwrap();
    let elsewhere = runtime.path().join("elsewhere.sock");
    let _other = UnixListener::bind(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &sock).unwrap();
    assert!(bind_socket(&dir, uid).is_err());
    assert!(fs::symlink_metadata(&sock)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_link(&sock).unwrap(), elsewhere);
    assert!(fs::symlink_metadata(&elsewhere)
        .unwrap()
        .file_type()
        .is_socket());

    // A stale socket owned by another uid is not ours to replace.
    fs::remove_file(&sock).unwrap();
    let _stale = UnixListener::bind(&sock).unwrap();
    assert!(bind_socket(&dir, uid.wrapping_add(1)).is_err());
}

/// Test 11 (RMC71, §8.1): root is refused (real or effective uid 0).
#[test]
fn test_rwp_sender_refuses_root() {
    assert!(check_not_root(0, 1000).is_err());
    assert!(check_not_root(1000, 0).is_err());
    assert!(check_not_root(0, 0).is_err());
    assert!(check_not_root(1000, 1000).is_ok());
}

// ---------------------------------------------------------------------------------------
// Tests 12, 52 — outbound policy and resolver wiring
// ---------------------------------------------------------------------------------------

/// Test 12 (RMC62, §8.1, F-3): the policy defaults are the protocol constants; both
/// constructors build without panicking; a resolver refusal is `Refused`, every other
/// `ureq` error `Retry`, never with a status or a `Retry-After`; `ResolveRefused` has fixed
/// texts.
#[test]
fn test_rwp_sender_policy_defaults() {
    assert_eq!(
        SendPolicy::default(),
        SendPolicy {
            https_only: true,
            max_redirects: 0,
            use_env_proxy: false,
            timeout_global_ms: PUSH_SEND_TIMEOUT_MS,
            timeout_connect_ms: PUSH_CONNECT_TIMEOUT_MS,
            max_response_header_bytes: MAX_PUSH_RESPONSE_HEADER_BYTES,
            max_response_body_bytes: MAX_PUSH_RESPONSE_BODY_BYTES,
        }
    );
    let _production = UreqDeliverer::new(SendPolicy::default());
    let (lookup, asked) = scripted_lookup(Ok(vec![sa("17.188.143.78", 0)]));
    let _seam = UreqDeliverer::with_lookup(SendPolicy::default(), lookup);
    assert!(
        asked.lock().unwrap().is_empty(),
        "construction resolves nothing"
    );

    assert_eq!(
        send_error_outcome(&ureq::Error::Other(Box::new(ResolveRefused))),
        REFUSED
    );
    let retry = DeliveryReply {
        outcome: Outcome::Retry,
        status: None,
        retry_after_s: None,
    };
    assert_eq!(send_error_outcome(&ureq::Error::HostNotFound), retry);
    assert_eq!(
        send_error_outcome(&ureq::Error::Timeout(ureq::Timeout::Global)),
        retry
    );
    assert_eq!(
        send_error_outcome(&ureq::Error::Timeout(ureq::Timeout::Connect)),
        retry
    );
    assert_eq!(send_error_outcome(&ureq::Error::ConnectionFailed), retry);
    assert_eq!(
        send_error_outcome(&ureq::Error::Other(Box::new(std::io::Error::other("x")))),
        retry,
        "only the resolver marker is a refusal"
    );
    assert_eq!(ResolveRefused.to_string(), "push host address refused");
    assert_eq!(format!("{ResolveRefused:?}"), "ResolveRefused");
}

/// Test 52 (RMC62, F-3; network-free): `UreqDeliverer::with_lookup` resolves through the
/// injected lookup and the filtering resolver: a loopback answer, or a public answer with
/// one private address, is exactly `Refused` without any connection attempt; a lookup error
/// is `Retry`; the lookup is called exactly once with the endpoint host. A deliverer that
/// used ureq's own resolver or mapped the refusal to `Retry` fails here.
#[test]
fn test_rwp_sender_deliverer_uses_filtering_resolver() {
    // A local listener on 127.0.0.1 that counts accepted connections: a deliverer that
    // connected to the refused address would show up here (on the ephemeral port the
    // lookup returns, which the filter rewrites to 443 anyway).
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    tcp.set_nonblocking(true).unwrap();
    let port = tcp.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));

    let cases: Vec<(Result<Vec<SocketAddr>, LookupError>, DeliveryReply)> = vec![
        (Ok(vec![sa("127.0.0.1", port)]), REFUSED),
        (
            Ok(vec![sa("17.188.143.78", 443), sa("10.0.0.1", 443)]),
            REFUSED,
        ),
        (Ok(Vec::new()), REFUSED),
        (
            Err(LookupError::Failed),
            DeliveryReply {
                outcome: Outcome::Retry,
                status: None,
                retry_after_s: None,
            },
        ),
    ];
    for (answer, expected) in cases {
        let (lookup, asked) = scripted_lookup(answer);
        let deliverer = UreqDeliverer::with_lookup(SendPolicy::default(), lookup);
        let start = Instant::now();
        let reply = deliverer.deliver(&valid_request());
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "no connection attempt: {:?}",
            start.elapsed()
        );
        assert_eq!(reply, expected);
        assert_eq!(
            *asked.lock().unwrap(),
            vec!["web.push.apple.com".to_string()],
            "exactly one lookup of the endpoint host"
        );
        while tcp.accept().is_ok() {
            accepted.fetch_add(1, Ordering::SeqCst);
        }
    }
    assert_eq!(accepted.load(Ordering::SeqCst), 0, "nothing ever connected");
}
