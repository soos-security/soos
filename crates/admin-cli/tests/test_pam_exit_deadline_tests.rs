//! Contract tests for GitHub #312 (review findings STO-NEW-3, STO-NEW-4).
//!
//! - STO-NEW-3: `soos-admin test-pam` exits 0 only when the response is accepted (nonce
//!   binding, deadline and freshness) AND its verdict is `Allow`; a rejected `Allow` (bound
//!   to another nonce, or stale) exits 1, like every other verdict.
//! - STO-NEW-4: `test-pam` reproduces the cumulative PAM deadline: one deadline started
//!   before `connect`, re-armed before every read and write, so a daemon that trickles its
//!   response over more than `timeout_ms` (each piece within `timeout_ms`) is a timeout, and
//!   a response completed after the deadline is rejected as `deadline_exceeded`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing,
    reason = "Contractual test suites use assertions, unwrap, and expect"
)]

#[path = "../../pam/tests/common/stamps.rs"]
mod stamps;

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use soos_admin_cli::error::AdminCliError;
use soos_admin_cli::test_pam::{simulate_pam_auth, PamTestRejection};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, Response, Verdict, CURRENT_VERSION};
use stamps::fresh_stamps;
use tempfile::tempdir;

fn read_request(stream: &mut UnixStream) -> Request {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).expect("read length");
    let mut body = vec![0u8; u32::from_be_bytes(len_buf) as usize];
    stream.read_exact(&mut body).expect("read body");
    let mut full = len_buf.to_vec();
    full.extend_from_slice(&body);
    decode(&full).expect("decode request")
}

fn response_for(req: &Request, verdict: Verdict, id_for: fn([u8; 32]) -> [u8; 32]) -> Vec<u8> {
    let (issued, expires) = fresh_stamps();
    encode(&Response {
        version: CURRENT_VERSION,
        request_id: id_for(req.request_id),
        verdict,
        reason_class: ReasonClass::FaceMatch,
        issued_monotonic_ns: issued,
        expires_monotonic_ns: expires,
    })
    .expect("encode response")
}

fn same(id: [u8; 32]) -> [u8; 32] {
    id
}

fn flip_last_bit(mut id: [u8; 32]) -> [u8; 32] {
    id[31] ^= 0x01;
    id
}

/// One-shot daemon answering `verdict` with `request_id = id_for(nonce)`.
fn one_shot_daemon(
    socket_path: &Path,
    verdict: Verdict,
    id_for: fn([u8; 32]) -> [u8; 32],
) -> thread::JoinHandle<()> {
    let listener = UnixListener::bind(socket_path).expect("bind socket");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let req = read_request(&mut stream);
        let _ = stream.write_all(&response_for(&req, verdict, id_for));
    })
}

fn run_test_pam_binary(socket_path: &Path) -> i32 {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_soos-admin"))
        .arg("--socket-path")
        .arg(socket_path)
        .args(["test-pam", "--uid", "1000", "--timeout-ms", "3000"])
        .output()
        .expect("run soos-admin");
    out.status.code().expect("exit code")
}

#[test]
fn test_312_test_pam_exits_1_on_an_allow_bound_to_another_nonce() {
    let dir = tempdir().expect("tempdir");
    let sock = dir.path().join("mismatch.sock");
    let server = one_shot_daemon(&sock, Verdict::Allow, flip_last_bit);
    let code = run_test_pam_binary(&sock);
    server.join().expect("join server");
    assert_eq!(code, 1, "a rejected Allow must not exit 0");
}

#[test]
fn test_312_test_pam_exits_0_only_on_an_accepted_allow() {
    let dir = tempdir().expect("tempdir");
    let sock = dir.path().join("allow.sock");
    let server = one_shot_daemon(&sock, Verdict::Allow, same);
    let code = run_test_pam_binary(&sock);
    server.join().expect("join server");
    assert_eq!(code, 0, "an accepted Allow exits 0");

    let sock = dir.path().join("deny.sock");
    let server = one_shot_daemon(&sock, Verdict::Deny, same);
    let code = run_test_pam_binary(&sock);
    server.join().expect("join server");
    assert_eq!(code, 1, "an accepted Deny exits 1");
}

#[test]
fn test_312_test_pam_deadline_is_cumulative_across_reads() {
    const TIMEOUT_MS: u64 = 400;
    const GAP: Duration = Duration::from_millis(260);
    let dir = tempdir().expect("tempdir");
    let sock = dir.path().join("trickle.sock");
    let listener = UnixListener::bind(&sock).expect("bind socket");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let req = read_request(&mut stream);
        // Each piece arrives within `timeout_ms`, the whole response does not.
        thread::sleep(GAP);
        let mut resp = Vec::new();
        resp.extend_from_slice(&response_for(&req, Verdict::Allow, same));
        let _ = stream.write_all(&resp[..4]);
        thread::sleep(GAP);
        let _ = stream.write_all(&resp[4..]);
    });

    let started = Instant::now();
    let res = simulate_pam_auth(&sock, 1000, "soos-admin", TIMEOUT_MS);
    let elapsed = started.elapsed();
    let _ = server.join();

    match res {
        Err(AdminCliError::Timeout) => {}
        Ok(report) => {
            panic!("a response trickled over more than timeout_ms must not be accepted: {report:?}")
        }
        Err(other) => panic!("expected AdminCliError::Timeout, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_millis(TIMEOUT_MS + 150),
        "the cumulative deadline bounds the whole exchange, took {elapsed:?}"
    );
}

#[test]
fn test_312_test_pam_silent_daemon_times_out_within_the_deadline() {
    const TIMEOUT_MS: u64 = 200;
    let dir = tempdir().expect("tempdir");
    let sock = dir.path().join("silent.sock");
    let listener = UnixListener::bind(&sock).expect("bind socket");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _req = read_request(&mut stream);
        thread::sleep(Duration::from_millis(TIMEOUT_MS * 3));
    });
    let started = Instant::now();
    let res = simulate_pam_auth(&sock, 1000, "soos-admin", TIMEOUT_MS);
    let elapsed = started.elapsed();
    let _ = server.join();
    assert!(
        matches!(res, Err(AdminCliError::Timeout)),
        "expected AdminCliError::Timeout, got {res:?}"
    );
    assert!(elapsed < Duration::from_millis(TIMEOUT_MS + 150));
}

#[test]
fn test_312_deadline_exceeded_rejection_has_a_stable_name() {
    assert_eq!(
        PamTestRejection::DeadlineExceeded.as_str(),
        "deadline_exceeded"
    );
    assert_eq!(
        serde_json::to_string(&PamTestRejection::DeadlineExceeded).unwrap(),
        "\"deadline_exceeded\""
    );
}
