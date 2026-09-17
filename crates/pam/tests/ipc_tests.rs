//! Contractual integration tests for PAM synchronous IPC client.
//!
//! Validates:
//! - PA1: Returns PAM_IGNORE when daemon is unavailable.
//! - PA2: Returns PAM_IGNORE on timeout (> timeout_ms).
//! - Fail-closed degradation for all errors and anomalies.
//! - Request ID verification (anti-replay binding).
//! - Password-failed event emission (20ms bounded budget).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use pam_soos::{pam_sm_authenticate, pam_sm_setcred};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{
    Event, EventKind, ReasonClass, Request, Response, Verdict, CURRENT_VERSION, MAX_MESSAGE_SIZE,
};
use std::ffi::CString;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::ptr;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

const PAM_SUCCESS: i32 = 0;
const PAM_IGNORE: i32 = 25;

/// Helper to format PAM argv with custom socket path and options.
fn make_pam_args(args: &[&str]) -> (Vec<CString>, Vec<*const u8>) {
    let cstrings: Vec<CString> = args
        .iter()
        .map(|s| CString::new(*s).expect("valid cstring"))
        .collect();
    let ptrs: Vec<*const u8> = cstrings.iter().map(|cs| cs.as_ptr().cast::<u8>()).collect();
    (cstrings, ptrs)
}

/// PA1: PAM returns PAM_IGNORE when daemon socket is unreachable.
#[test]
fn test_ipc_offline_daemon_returns_ignore() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("nonexistent_daemon.sock");
    let sock_arg = format!("socket_path={}", sock_path.display());

    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=100"]);

    let start = Instant::now();
    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    let elapsed = start.elapsed();

    assert_eq!(code, PAM_IGNORE);
    // Offline connect failure must resolve promptly
    assert!(elapsed < Duration::from_millis(200));
}

/// PA2: PAM returns PAM_IGNORE when daemon hangs beyond timeout deadline.
#[test]
fn test_ipc_slow_daemon_timeout() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("slow_daemon.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            // Read 4-byte length prefix + request body
            let mut len_buf = [0u8; 4];
            let _ = stream.read_exact(&mut len_buf);
            let size = u32::from_be_bytes(len_buf) as usize;
            let mut body = vec![0u8; size];
            let _ = stream.read_exact(&mut body);

            // Intentionally sleep beyond PAM client timeout (100ms)
            thread::sleep(Duration::from_millis(300));
        }
    });

    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=100"]);

    let start = Instant::now();
    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    let elapsed = start.elapsed();

    assert_eq!(code, PAM_IGNORE);
    // Latency must respect the client's configured timeout (~100ms), well below 300ms
    assert!(elapsed < Duration::from_millis(250));

    let _ = server_handle.join();
}

/// Nominal authentication success: daemon renders Allow -> PAM returns PAM_SUCCESS.
#[test]
fn test_ipc_nominal_allow_returns_success() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("auth_allow.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connected");

        // Read request length
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let size = u32::from_be_bytes(len_buf) as usize;

        let mut full_req = Vec::with_capacity(size + 4);
        full_req.extend_from_slice(&len_buf);
        let mut body = vec![0u8; size];
        stream.read_exact(&mut body).expect("read body");
        full_req.extend_from_slice(&body);

        let req: Request = decode(&full_req).expect("decoded request");

        // Reply with Verdict::Allow bound to req.request_id
        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict: Verdict::Allow,
            reason_class: ReasonClass::FaceMatch,
            issued_monotonic_ns: 1000,
            expires_monotonic_ns: 2000,
        };
        let encoded = encode(&resp).expect("encoded response");
        stream.write_all(&encoded).expect("wrote response");
    });

    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=250"]);

    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    assert_eq!(code, PAM_SUCCESS);

    let _ = server_handle.join();
}

/// Authentication denial: daemon renders Deny -> PAM returns PAM_IGNORE.
#[test]
fn test_ipc_deny_returns_ignore() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("auth_deny.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connected");

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let size = u32::from_be_bytes(len_buf) as usize;

        let mut full_req = Vec::with_capacity(size + 4);
        full_req.extend_from_slice(&len_buf);
        let mut body = vec![0u8; size];
        stream.read_exact(&mut body).expect("read body");
        full_req.extend_from_slice(&body);

        let req: Request = decode(&full_req).expect("decoded request");

        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict: Verdict::Deny,
            reason_class: ReasonClass::ScoreBelowThreshold,
            issued_monotonic_ns: 1000,
            expires_monotonic_ns: 2000,
        };
        let encoded = encode(&resp).expect("encoded response");
        stream.write_all(&encoded).expect("wrote response");
    });

    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=250"]);

    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    assert_eq!(code, PAM_IGNORE);

    let _ = server_handle.join();
}

/// Daemon unavailable: daemon renders Unavailable -> PAM returns PAM_IGNORE.
#[test]
fn test_ipc_unavailable_returns_ignore() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("auth_unavail.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connected");

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let size = u32::from_be_bytes(len_buf) as usize;

        let mut full_req = Vec::with_capacity(size + 4);
        full_req.extend_from_slice(&len_buf);
        let mut body = vec![0u8; size];
        stream.read_exact(&mut body).expect("read body");
        full_req.extend_from_slice(&body);

        let req: Request = decode(&full_req).expect("decoded request");

        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict: Verdict::Unavailable,
            reason_class: ReasonClass::CameraUnavailable,
            issued_monotonic_ns: 1000,
            expires_monotonic_ns: 2000,
        };
        let encoded = encode(&resp).expect("encoded response");
        stream.write_all(&encoded).expect("wrote response");
    });

    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=250"]);

    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    assert_eq!(code, PAM_IGNORE);

    let _ = server_handle.join();
}

/// Request ID mismatch: daemon returns response with incorrect nonce -> rejected with PAM_IGNORE.
#[test]
fn test_ipc_request_id_mismatch_returns_ignore() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("auth_mismatch.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connected");

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let size = u32::from_be_bytes(len_buf) as usize;
        let mut body = vec![0u8; size];
        stream.read_exact(&mut body).expect("read body");

        // Use arbitrary mismatched nonce
        let mismatched_id = [0xAAu8; 32];
        let resp = Response {
            version: CURRENT_VERSION,
            request_id: mismatched_id,
            verdict: Verdict::Allow, // Even if Allow, nonce mismatch must reject!
            reason_class: ReasonClass::FaceMatch,
            issued_monotonic_ns: 1000,
            expires_monotonic_ns: 2000,
        };
        let encoded = encode(&resp).expect("encoded response");
        stream.write_all(&encoded).expect("wrote response");
    });

    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=250"]);

    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    assert_eq!(code, PAM_IGNORE);

    let _ = server_handle.join();
}

/// Oversized payload rejection: daemon writes length prefix > 4096 bytes -> rejected with PAM_IGNORE.
#[test]
fn test_ipc_oversized_response_rejected() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("auth_oversized.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connected");

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let size = u32::from_be_bytes(len_buf) as usize;
        let mut body = vec![0u8; size];
        stream.read_exact(&mut body).expect("read body");

        // Send declared length prefix of 8192 bytes (> MAX_MESSAGE_SIZE 4096)
        let oversized_len = (MAX_MESSAGE_SIZE as u32 * 2).to_be_bytes();
        stream.write_all(&oversized_len).expect("wrote prefix");
    });

    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=250"]);

    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    assert_eq!(code, PAM_IGNORE);

    let _ = server_handle.join();
}

/// Telemetry event: `event=password-failed` mode transmits Event to daemon within 20ms and returns PAM_IGNORE.
#[test]
fn test_ipc_password_failed_event_notification() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("pw_failed.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connected");

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let size = u32::from_be_bytes(len_buf) as usize;

        let mut full_buf = Vec::with_capacity(size + 4);
        full_buf.extend_from_slice(&len_buf);
        let mut body = vec![0u8; size];
        stream.read_exact(&mut body).expect("read body");
        full_buf.extend_from_slice(&body);

        let event: Event = decode(&full_buf).expect("decoded event");
        assert_eq!(event.version, CURRENT_VERSION);
        assert_eq!(event.kind, EventKind::PasswordFailed);
        assert_eq!(event.service, "pam_soos");
    });

    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "event=password-failed", "timeout_ms=20"]);

    let start = Instant::now();
    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    let elapsed = start.elapsed();

    assert_eq!(code, PAM_IGNORE);
    // Event sending must be fast and bounded to 20ms budget
    assert!(elapsed < Duration::from_millis(50));

    let _ = server_handle.join();
}

/// Direct call to notify_event validates successful delivery.
#[test]
fn test_ipc_notify_event_direct() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("notify_direct.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connected");
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let size = u32::from_be_bytes(len_buf) as usize;
        let mut full = vec![0u8; size + 4];
        full[..4].copy_from_slice(&len_buf);
        stream.read_exact(&mut full[4..]).expect("read body");
        let event: Event = decode(&full).expect("decoded event");
        assert_eq!(event.kind, EventKind::PasswordFailed);
    });

    let config = pam_soos::config::PamConfig {
        socket_path: sock_path,
        ..Default::default()
    };

    let result = pam_soos::ipc::notify_event(&config, 1000, EventKind::PasswordFailed);
    assert!(result.is_ok());

    let _ = server_handle.join();
}

/// Direct call to notify_event with offline daemon returns IpcError::Connect.
#[test]
fn test_ipc_notify_event_offline() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("notify_offline.sock");
    let config = pam_soos::config::PamConfig {
        socket_path: sock_path,
        ..Default::default()
    };
    let result = pam_soos::ipc::notify_event(&config, 1000, EventKind::PasswordFailed);
    assert!(matches!(result, Err(pam_soos::ipc::IpcError::Connect(_))));
}

/// Telemetry event: offline daemon when `event=password-failed` never blocks or panics.
#[test]
fn test_ipc_password_failed_daemon_offline_returns_ignore() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("pw_failed_offline.sock");
    let sock_arg = format!("socket_path={}", sock_path.display());

    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "event=password-failed", "timeout_ms=20"]);

    let start = Instant::now();
    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    let elapsed = start.elapsed();

    assert_eq!(code, PAM_IGNORE);
    assert!(elapsed < Duration::from_millis(50));
}

/// Credential management: `pam_sm_setcred` always returns PAM_IGNORE.
#[test]
fn test_setcred_always_returns_ignore() {
    let code = pam_sm_setcred(ptr::null_mut(), 0, 0, ptr::null());
    assert_eq!(code, PAM_IGNORE);
}

/// Adversarial: daemon sends length prefix of 0 -> rejected cleanly with PAM_IGNORE.
#[test]
fn test_ipc_empty_response_returns_ignore() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("auth_empty.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connected");

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let size = u32::from_be_bytes(len_buf) as usize;
        let mut body = vec![0u8; size];
        stream.read_exact(&mut body).expect("read body");

        // Send declared length of 0 bytes
        let zero_len = 0u32.to_be_bytes();
        stream.write_all(&zero_len).expect("wrote prefix");
    });

    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=250"]);

    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    assert_eq!(code, PAM_IGNORE);

    let _ = server_handle.join();
}

/// Adversarial PA2: daemon sends 4-byte length prefix promptly, but hangs mid-stream before sending body.
/// Client must enforce cumulative deadline and NOT allow secondary read to extend execution.
#[test]
fn test_ipc_slow_daemon_body_timeout() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("slow_body.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut len_buf = [0u8; 4];
            let _ = stream.read_exact(&mut len_buf);
            let size = u32::from_be_bytes(len_buf) as usize;
            let mut body = vec![0u8; size];
            let _ = stream.read_exact(&mut body);

            // Send length prefix for 50-byte response
            let body_len = 50u32.to_be_bytes();
            let _ = stream.write_all(&body_len);

            // Then hang for 350ms before sending the body!
            thread::sleep(Duration::from_millis(350));
        }
    });

    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=100"]);

    let start = Instant::now();
    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    let elapsed = start.elapsed();

    assert_eq!(code, PAM_IGNORE);
    // Cumulative budget must be strictly bounded to ~100ms, well below 300ms
    assert!(elapsed < Duration::from_millis(250));

    let _ = server_handle.join();
}

/// Direct IPC client error mapping: socket timeout maps to IpcError::Timeout.
#[test]
fn test_ipc_direct_authenticate_timeout_mapping() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("timeout_direct.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 128];
            let _ = stream.read(&mut buf);
            thread::sleep(Duration::from_millis(250));
        }
    });

    let config = pam_soos::config::PamConfig {
        timeout_ms: 50,
        socket_path: sock_path,
        ..Default::default()
    };

    let result = pam_soos::ipc::authenticate(&config, 1000);
    assert!(
        matches!(result, Err(pam_soos::ipc::IpcError::Timeout)),
        "Expected IpcError::Timeout but got: {:?}",
        result
    );

    let _ = server_handle.join();
}

/// Daemon crash mid-request: connection closed immediately after accepting connection.
/// Must degrade gracefully to PAM_IGNORE (Sub-issue #13.3).
#[test]
fn test_ipc_daemon_crash_immediate_disconnect_returns_ignore() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("crash_immediate.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            // Read partial or full request then drop stream immediately (simulating daemon SIGKILL / crash)
            let mut buf = [0u8; 16];
            let _ = stream.read(&mut buf);
            drop(stream);
        }
    });

    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=250"]);

    let start = Instant::now();
    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    let elapsed = start.elapsed();

    assert_eq!(code, PAM_IGNORE);
    assert!(elapsed < Duration::from_millis(250));

    let _ = server_handle.join();
}

/// Daemon crash mid-request: connection severed after sending partial 2-byte header.
/// Must degrade gracefully to PAM_IGNORE (Sub-issue #13.3).
#[test]
fn test_ipc_daemon_crash_partial_header_returns_ignore() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("crash_partial_hdr.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut len_buf = [0u8; 4];
            let _ = stream.read_exact(&mut len_buf);
            let size = u32::from_be_bytes(len_buf) as usize;
            let mut body = vec![0u8; size];
            let _ = stream.read_exact(&mut body);

            // Send partial 2 bytes of the 4-byte length prefix and crash
            let partial = [0u8, 0u8];
            let _ = stream.write_all(&partial);
            drop(stream);
        }
    });

    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=250"]);

    let start = Instant::now();
    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    let elapsed = start.elapsed();

    assert_eq!(code, PAM_IGNORE);
    assert!(elapsed < Duration::from_millis(250));

    let _ = server_handle.join();
}

/// Daemon crash mid-request: connection severed after sending full length prefix but truncated payload body.
/// Must degrade gracefully to PAM_IGNORE (Sub-issue #13.3).
#[test]
fn test_ipc_daemon_crash_truncated_body_returns_ignore() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("crash_trunc_body.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut len_buf = [0u8; 4];
            let _ = stream.read_exact(&mut len_buf);
            let size = u32::from_be_bytes(len_buf) as usize;
            let mut body = vec![0u8; size];
            let _ = stream.read_exact(&mut body);

            // Announce 64 bytes of body but write only 10 bytes then crash
            let len_prefix = 64u32.to_be_bytes();
            let _ = stream.write_all(&len_prefix);
            let partial_body = [0xAAu8; 10];
            let _ = stream.write_all(&partial_body);
            drop(stream);
        }
    });

    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=250"]);

    let start = Instant::now();
    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    let elapsed = start.elapsed();

    assert_eq!(code, PAM_IGNORE);
    assert!(elapsed < Duration::from_millis(250));

    let _ = server_handle.join();
}

/// Sub-issue #21.2 TDD Contract: Truncated responses are detected and treated as errors.
///
/// Asserts:
/// 1. Direct call to authenticate returns IpcError::TruncatedResponse when body is severed prematurely.
/// 2. Direct call to authenticate returns IpcError::TruncatedResponse when length header is truncated.
/// 3. PAM FFI entry point pam_sm_authenticate degrades fail-closed to PAM_IGNORE.
#[test]
fn test_pam_ipc_detects_truncated_response() {
    // Case 1: Truncated payload body (announced 64 bytes, sent only 16 bytes)
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("trunc_body_detect.sock");
    let listener = UnixListener::bind(&sock_path).expect("bound test socket");

    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut len_buf = [0u8; 4];
            let _ = stream.read_exact(&mut len_buf);
            let size = u32::from_be_bytes(len_buf) as usize;
            let mut body = vec![0u8; size];
            let _ = stream.read_exact(&mut body);

            // Announce 64 bytes of body but write only 16 bytes then close
            let len_prefix = 64u32.to_be_bytes();
            let _ = stream.write_all(&len_prefix);
            let partial_body = [0x77u8; 16];
            let _ = stream.write_all(&partial_body);
            drop(stream);
        }
    });

    let config = pam_soos::config::PamConfig {
        socket_path: sock_path.clone(),
        timeout_ms: 250,
        ..Default::default()
    };

    let result = pam_soos::ipc::authenticate(&config, 1000);
    match result {
        Err(pam_soos::ipc::IpcError::TruncatedResponse { expected, received }) => {
            assert_eq!(expected, 68, "Expected full frame size of 4 + 64 bytes");
            assert_eq!(received, 20, "Received only 4 header + 16 body bytes");
        }
        other => panic!("Expected IpcError::TruncatedResponse, got: {:?}", other),
    }

    let _ = server_handle.join();

    // Verify PAM FFI fails closed into PAM_IGNORE
    let sock_arg = format!("socket_path={}", sock_path.display());
    let (_storage, ptrs) = make_pam_args(&[&sock_arg, "timeout_ms=250"]);
    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    assert_eq!(code, PAM_IGNORE);

    // Case 2: Truncated length header (expected 4 bytes, sent only 2 bytes)
    let sock_path_hdr = tmp.path().join("trunc_hdr_detect.sock");
    let listener_hdr = UnixListener::bind(&sock_path_hdr).expect("bound test socket");

    let server_handle_hdr = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener_hdr.accept() {
            let mut buf = [0u8; 128];
            let _ = stream.read(&mut buf);
            // Send only 2 bytes of the 4-byte length prefix
            let partial_hdr = [0u8, 1u8];
            let _ = stream.write_all(&partial_hdr);
            drop(stream);
        }
    });

    let config_hdr = pam_soos::config::PamConfig {
        socket_path: sock_path_hdr.clone(),
        timeout_ms: 250,
        ..Default::default()
    };

    let result_hdr = pam_soos::ipc::authenticate(&config_hdr, 1000);
    match result_hdr {
        Err(pam_soos::ipc::IpcError::TruncatedResponse { expected, received }) => {
            assert_eq!(expected, 4, "Expected 4-byte length header");
            assert_eq!(received, 2, "Received only 2 bytes");
        }
        other => panic!("Expected IpcError::TruncatedResponse, got: {:?}", other),
    }

    let _ = server_handle_hdr.join();
}
