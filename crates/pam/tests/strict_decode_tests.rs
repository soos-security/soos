//! PAM client strict response decoding (GitHub #224, review PAM-12).
//!
//! A daemon (or an impostor on the socket path) that appends bytes after a valid `Allow`
//! response INSIDE the declared frame length must not authenticate the user: the client
//! rejects the frame (`CodecError::TrailingBytes`) and PAM falls back with `PAM_IGNORE`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

#[path = "common/stamps.rs"]
mod stamps;

use pam_soos::pam_sm_authenticate;
use soos_protocol::codec::{decode, encode, CodecError};
use soos_protocol::types::{ReasonClass, Request, Response, Verdict, CURRENT_VERSION};
use std::ffi::CString;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::ptr;
use std::thread;
use tempfile::tempdir;

const PAM_SUCCESS: i32 = 0;
const PAM_IGNORE: i32 = 25;

/// Serves one connection: answers `Allow` bound to the request nonce, followed by
/// `trailing` bytes counted inside the declared payload length.
fn spawn_allow_daemon_with_trailing_bytes(
    sock_path: &Path,
    trailing: &'static [u8],
) -> thread::JoinHandle<()> {
    let listener = UnixListener::bind(sock_path).expect("bound test socket");
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).expect("read length");
        let size = u32::from_be_bytes(len_buf) as usize;
        let mut full_req = len_buf.to_vec();
        let mut body = vec![0u8; size];
        stream.read_exact(&mut body).expect("read body");
        full_req.extend_from_slice(&body);
        let req: Request = decode(&full_req).expect("decoded request");

        let (issued, expires) = stamps::fresh_stamps();
        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict: Verdict::Allow,
            reason_class: ReasonClass::FaceMatch,
            issued_monotonic_ns: issued,
            expires_monotonic_ns: expires,
        };
        let frame = encode(&resp).expect("encoded response");
        let payload = &frame[4..];
        let mut tampered = ((payload.len() + trailing.len()) as u32)
            .to_be_bytes()
            .to_vec();
        tampered.extend_from_slice(payload);
        tampered.extend_from_slice(trailing);
        let _ = stream.write_all(&tampered);
    })
}

#[test]
fn test_allow_with_trailing_bytes_inside_frame_returns_ignore() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("trailing_allow.sock");
    let server = spawn_allow_daemon_with_trailing_bytes(&sock_path, &[0xDE, 0xAD, 0x00]);

    let args: Vec<CString> = vec![
        CString::new(format!("socket_path={}", sock_path.display())).expect("cstring"),
        CString::new("timeout_ms=500").expect("cstring"),
    ];
    let ptrs: Vec<*const u8> = args.iter().map(|c| c.as_ptr().cast::<u8>()).collect();

    let code = pam_sm_authenticate(ptr::null_mut(), 0, ptrs.len() as i32, ptrs.as_ptr());
    assert_ne!(
        code, PAM_SUCCESS,
        "a tampered Allow frame must never authenticate"
    );
    assert_eq!(code, PAM_IGNORE);

    let _ = server.join();
}

#[test]
fn test_direct_authenticate_reports_trailing_bytes_codec_error() {
    let tmp = tempdir().expect("tempdir created");
    let sock_path = tmp.path().join("trailing_direct.sock");
    let server = spawn_allow_daemon_with_trailing_bytes(&sock_path, &[1]);

    let config = pam_soos::config::PamConfig {
        timeout_ms: 500,
        socket_path: sock_path,
        ..Default::default()
    };
    let result = pam_soos::ipc::authenticate(&config, 1000);
    assert!(
        matches!(
            result,
            Err(pam_soos::ipc::IpcError::Codec(CodecError::TrailingBytes {
                unconsumed: 1
            }))
        ),
        "expected Codec(TrailingBytes {{ unconsumed: 1 }}), got {result:?}"
    );

    let _ = server.join();
}
