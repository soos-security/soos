//! Deadline, poll rounding and UID-resolution budget contract tests (walkthrough 121).
//!
//! Validates:
//! - PDR2 (GitHub #222): the absolute deadline sent to the daemon is captured once, when the
//!   exchange budget starts (before connect), so the daemon never works past the instant the
//!   client gives up.
//! - PDR3 (GitHub #222): a sub-millisecond `poll()` budget is rounded UP to 1 ms instead of
//!   truncating to 0 (which made `poll` return immediately and produced a spurious timeout).
//! - PDR4 (GitHub #223): username -> UID resolution (NSS, possibly LDAP/SSSD) runs inside the
//!   authentication budget: a resolution that consumes the whole `timeout_ms` budget falls back
//!   to `PAM_IGNORE` without contacting the daemon, and the `event=password-failed` path stays
//!   bounded by `EVENT_TIMEOUT_MS` after resolution; `uid=` is honoured only when the resolved
//!   PAM user matches it (GitHub #302).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::ffi::CString;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use pam_soos::config::{parse_cstrs, PamConfig};
use pam_soos::ipc::{authenticate_before, poll_timeout_ms, ExchangeDeadline, EVENT_TIMEOUT_MS};
use pam_soos::{PamResultCode, SoosPam};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{
    Event, EventKind, ReasonClass, Request, Response, Verdict, CURRENT_VERSION,
};
use tempfile::tempdir;

/// Reads the CLOCK_MONOTONIC clock the PAM client and the daemon share.
fn monotonic_now_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec on the stack.
    let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    assert_eq!(ret, 0, "clock_gettime(CLOCK_MONOTONIC) must succeed");
    (ts.tv_sec as u64) * 1_000_000_000 + (ts.tv_nsec as u64)
}

fn config_from(args: &[String]) -> PamConfig {
    let cstrings: Vec<CString> = args
        .iter()
        .map(|s| CString::new(s.as_str()).expect("valid cstring"))
        .collect();
    parse_cstrs(cstrings.iter().map(CString::as_c_str))
}

/// Base arguments pointing at a private socket.
fn base_args(sock: &Path) -> Vec<String> {
    vec![
        format!("socket_path={}", sock.display()),
        "service=soos-test".to_string(),
    ]
}

/// Reads one length-prefixed frame from `stream`.
fn read_frame(stream: &mut UnixStream) -> Vec<u8> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).expect("read length");
    let size = u32::from_be_bytes(len_buf) as usize;
    let mut frame = Vec::with_capacity(size + 4);
    frame.extend_from_slice(&len_buf);
    let mut body = vec![0u8; size];
    stream.read_exact(&mut body).expect("read body");
    frame.extend_from_slice(&body);
    frame
}

fn allow_for(req: &Request) -> Vec<u8> {
    let now = monotonic_now_ns();
    encode(&Response {
        version: CURRENT_VERSION,
        request_id: req.request_id,
        verdict: Verdict::Allow,
        reason_class: ReasonClass::FaceMatch,
        issued_monotonic_ns: now,
        expires_monotonic_ns: now + 2_000_000_000,
    })
    .expect("encoded response")
}

// ---------------------------------------------------------------------------
// GitHub #222 — deadline captured before connect, poll rounding
// ---------------------------------------------------------------------------

/// PDR2: the monotonic deadline is fixed when the budget starts; time spent before the
/// exchange (connect, NSS) shrinks what is left instead of pushing the deadline later.
#[test]
fn test_exchange_deadline_is_fixed_when_the_budget_starts() {
    let before_ns = monotonic_now_ns();
    let deadline = ExchangeDeadline::start(200);
    let after_ns = monotonic_now_ns();

    let deadline_ns = deadline.monotonic_deadline_ns();
    assert!(
        deadline_ns >= before_ns + 200_000_000 && deadline_ns <= after_ns + 200_000_000,
        "deadline must be start + timeout_ms on CLOCK_MONOTONIC"
    );

    thread::sleep(Duration::from_millis(40));

    assert_eq!(
        deadline.monotonic_deadline_ns(),
        deadline_ns,
        "the absolute deadline must never move once the budget started"
    );
    let remaining = deadline.remaining().expect("budget left after 40 ms");
    assert!(
        remaining <= Duration::from_millis(160),
        "40 ms spent before the exchange must be charged to the budget, got {remaining:?}"
    );
    let now_ns = monotonic_now_ns();
    assert!(
        deadline_ns.saturating_sub(now_ns) <= 160_000_000,
        "the daemon-facing deadline must not exceed what the client still waits for"
    );
}

/// PDR2: `authenticate_before` sends exactly the deadline captured when the budget started,
/// even when part of the budget was already consumed before the connection.
#[test]
fn test_request_deadline_is_captured_before_connect() {
    let tmp = tempdir().expect("tempdir");
    let sock = tmp.path().join("deadline.sock");
    let listener = UnixListener::bind(&sock).expect("bind");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connected");
        let frame = read_frame(&mut stream);
        let req: Request = decode(&frame).expect("decoded request");
        stream.write_all(&allow_for(&req)).expect("wrote response");
        req.deadline_monotonic_ns
    });

    let config = config_from(&base_args(&sock));
    let deadline = ExchangeDeadline::start(400);
    let expected_ns = deadline.monotonic_deadline_ns();
    // Simulates budget consumed before the exchange (slow connect, NSS lookup).
    thread::sleep(Duration::from_millis(50));

    let outcome = authenticate_before(&config, 4242, deadline).expect("exchange succeeds");
    assert_eq!(outcome.0, Verdict::Allow);

    let sent_ns = server.join().expect("server thread");
    assert_eq!(
        sent_ns, expected_ns,
        "the request deadline must be the one captured before connect, not recomputed later"
    );
}

/// PDR2: the public `authenticate` entry point sends a deadline no later than
/// `start + timeout_ms`, measured before the call (and therefore before connect).
#[test]
fn test_authenticate_request_deadline_never_exceeds_the_client_budget() {
    let tmp = tempdir().expect("tempdir");
    let sock = tmp.path().join("deadline_public.sock");
    let listener = UnixListener::bind(&sock).expect("bind");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connected");
        let frame = read_frame(&mut stream);
        let req: Request = decode(&frame).expect("decoded request");
        stream.write_all(&allow_for(&req)).expect("wrote response");
        req.deadline_monotonic_ns
    });

    let mut args = base_args(&sock);
    args.push("timeout_ms=300".to_string());
    let config = config_from(&args);

    let before_ns = monotonic_now_ns();
    let outcome = pam_soos::ipc::authenticate(&config, 4242).expect("exchange succeeds");
    let after_ns = monotonic_now_ns();
    assert_eq!(outcome.0, Verdict::Allow);

    let sent_ns = server.join().expect("server thread");
    assert!(
        sent_ns >= before_ns + 300_000_000 && sent_ns <= after_ns + 300_000_000,
        "request deadline must be start + timeout_ms"
    );
}

/// PDR3: a sub-millisecond remaining budget must be rounded up, never truncated to a
/// zero-millisecond `poll()` that returns immediately.
#[test]
fn test_poll_timeout_rounds_sub_millisecond_budget_up() {
    assert_eq!(poll_timeout_ms(Duration::ZERO), 0);
    assert_eq!(poll_timeout_ms(Duration::from_nanos(1)), 1);
    assert_eq!(poll_timeout_ms(Duration::from_micros(900)), 1);
    assert_eq!(poll_timeout_ms(Duration::from_millis(1)), 1);
    assert_eq!(poll_timeout_ms(Duration::from_micros(1_001)), 2);
    assert_eq!(poll_timeout_ms(Duration::from_millis(250)), 250);
    assert_eq!(poll_timeout_ms(Duration::MAX), i32::MAX);
}

// ---------------------------------------------------------------------------
// GitHub #223 — UID resolution inside the budget
// ---------------------------------------------------------------------------

/// Spawns a daemon stub that answers Allow to any Auth request and records whether a
/// client ever connected.
fn spawn_allow_daemon(sock: &Path) -> (Arc<AtomicBool>, thread::JoinHandle<()>) {
    let listener = UnixListener::bind(sock).expect("bind");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let contacted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&contacted);
    let handle = thread::spawn(move || {
        let stop = Instant::now() + Duration::from_millis(600);
        while Instant::now() < stop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    flag.store(true, Ordering::SeqCst);
                    stream.set_nonblocking(false).expect("blocking stream");
                    let frame = read_frame(&mut stream);
                    if let Ok(req) = decode::<Request>(&frame) {
                        let _ = stream.write_all(&allow_for(&req));
                    }
                    return;
                }
                Err(_) => thread::sleep(Duration::from_millis(2)),
            }
        }
    });
    (contacted, handle)
}

/// PDR4: a UID resolution that consumes the whole `timeout_ms` budget falls back to
/// `PAM_IGNORE` and never starts a face exchange (the daemon is not even contacted).
#[test]
fn test_slow_uid_resolution_exhausting_the_budget_returns_ignore() {
    let tmp = tempdir().expect("tempdir");
    let sock = tmp.path().join("slow_nss.sock");
    let (contacted, server) = spawn_allow_daemon(&sock);

    let mut args = base_args(&sock);
    args.push("timeout_ms=100".to_string());
    let config = config_from(&args);

    let start = Instant::now();
    let code = SoosPam::authenticate_with_uid_resolver(None, &config, |_| {
        thread::sleep(Duration::from_millis(150));
        Some(4242)
    });
    let elapsed = start.elapsed();

    assert_eq!(code, PamResultCode::PAM_IGNORE);
    assert!(
        elapsed < Duration::from_millis(240),
        "no exchange may start after the budget is spent (elapsed {elapsed:?})"
    );
    let _ = server.join();
    assert!(
        !contacted.load(Ordering::SeqCst),
        "the daemon must not be contacted once resolution consumed the whole budget"
    );
}

/// PDR4: a fast resolution keeps working: the resolved UID is sent as `uid_hint`.
#[test]
fn test_fast_uid_resolution_sends_resolved_uid() {
    let tmp = tempdir().expect("tempdir");
    let sock = tmp.path().join("fast_nss.sock");
    let listener = UnixListener::bind(&sock).expect("bind");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connected");
        let frame = read_frame(&mut stream);
        let req: Request = decode(&frame).expect("decoded request");
        stream.write_all(&allow_for(&req)).expect("wrote response");
        req.uid_hint
    });

    let mut args = base_args(&sock);
    args.push("timeout_ms=500".to_string());
    let config = config_from(&args);

    let code = SoosPam::authenticate_with_uid_resolver(None, &config, |_| Some(4242));
    assert_eq!(code, PamResultCode::PAM_SUCCESS);
    assert_eq!(server.join().expect("server thread"), 4242);
}

/// PDR4 / GitHub #302 (assertion migrated with owner approval 2026-10-02; formerly
/// `test_uid_argument_bypasses_the_resolver`, which asserted that the resolver is never
/// called): `uid=` is honoured only when PAM_USER resolves to the same UID. The resolver
/// always runs once; a matching resolution delivers the event, a different UID or a
/// failed resolution sends nothing.
#[test]
fn test_uid_argument_requires_matching_resolved_uid() {
    for (resolved, delivered) in [(Some(1000), true), (Some(1001), false), (None, false)] {
        let tmp = tempdir().expect("tempdir");
        let sock = tmp.path().join("uid_arg.sock");
        let listener = UnixListener::bind(&sock).expect("bind");
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");

        let mut args = base_args(&sock);
        args.push("event=password-failed".to_string());
        args.push("uid=1000".to_string());
        let config = config_from(&args);

        let calls = std::sync::atomic::AtomicUsize::new(0);
        let code = SoosPam::authenticate_with_uid_resolver(None, &config, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            resolved
        });
        assert_eq!(code, PamResultCode::PAM_IGNORE, "{resolved:?}");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "uid= must be checked against the resolved PAM user ({resolved:?})"
        );

        match listener.accept() {
            Ok((mut stream, _)) => {
                assert!(delivered, "{resolved:?}: no event may be sent");
                stream.set_nonblocking(false).expect("blocking stream");
                let frame = read_frame(&mut stream);
                let event: Event = decode(&frame).expect("decoded event");
                assert_eq!(event.uid, Some(1000));
            }
            Err(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::WouldBlock, "{e}");
                assert!(!delivered, "{resolved:?}: the event must be delivered");
            }
        }
    }
}

/// PDR4: on the `event=password-failed` path a slow resolution is not charged to the
/// event socket budget: the event is still delivered (security telemetry preserved) and the
/// socket part stays bounded by `EVENT_TIMEOUT_MS`; the outcome is always `PAM_IGNORE`.
#[test]
fn test_slow_uid_resolution_event_path_stays_bounded() {
    let tmp = tempdir().expect("tempdir");
    let sock = tmp.path().join("slow_nss_event.sock");
    let listener = UnixListener::bind(&sock).expect("bind");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connected");
        let frame = read_frame(&mut stream);
        let event: Event = decode(&frame).expect("decoded event");
        (event.kind, event.uid)
    });

    let mut args = base_args(&sock);
    args.push("event=password-failed".to_string());
    let config = config_from(&args);

    let resolver_delay = Duration::from_millis(60);
    let start = Instant::now();
    let code = SoosPam::authenticate_with_uid_resolver(None, &config, |_| {
        thread::sleep(resolver_delay);
        Some(4242)
    });
    let elapsed = start.elapsed();

    assert_eq!(code, PamResultCode::PAM_IGNORE);
    assert!(
        elapsed
            < resolver_delay + Duration::from_millis(EVENT_TIMEOUT_MS) + Duration::from_millis(150),
        "event path must add at most EVENT_TIMEOUT_MS after resolution (elapsed {elapsed:?})"
    );
    let (kind, uid) = server.join().expect("server thread");
    assert_eq!(kind, EventKind::PasswordFailed);
    assert_eq!(uid, Some(4242));
}

// ---------------------------------------------------------------------------
// GitHub #219 — replay protection is the single-use request_id, not the timestamps
// ---------------------------------------------------------------------------

/// PDR1: a genuine `Allow` response captured from one exchange and replayed verbatim on a
/// later exchange (still inside its `expires_monotonic_ns` window) is rejected because the
/// fresh 256-bit `request_id` no longer matches. This is the replay protection the protocol
/// documents; the timestamps are informational.
#[test]
fn test_replayed_allow_response_is_rejected_by_request_id_binding() {
    let tmp = tempdir().expect("tempdir");
    let sock = tmp.path().join("replay.sock");
    let listener = UnixListener::bind(&sock).expect("bind");

    let server = thread::spawn(move || {
        let (mut first, _) = listener.accept().expect("first client");
        let frame = read_frame(&mut first);
        let req: Request = decode(&frame).expect("decoded request");
        let captured = allow_for(&req);
        first.write_all(&captured).expect("wrote first response");
        drop(first);

        let (mut second, _) = listener.accept().expect("second client");
        let _ = read_frame(&mut second);
        second.write_all(&captured).expect("replayed response");
    });

    let mut args = base_args(&sock);
    args.push("timeout_ms=500".to_string());
    let config = config_from(&args);

    let first = pam_soos::ipc::authenticate(&config, 4242).expect("first exchange");
    assert_eq!(first.0, Verdict::Allow);

    let replayed = pam_soos::ipc::authenticate(&config, 4242);
    assert!(
        matches!(replayed, Err(pam_soos::ipc::IpcError::RequestIdMismatch)),
        "a replayed response must be rejected, got {replayed:?}"
    );
    server.join().expect("server thread");
}
