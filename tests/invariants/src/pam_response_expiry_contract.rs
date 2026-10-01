//! PAM response expiry and protocol follow-ups (GitHub #287, rows PRE6-PRE10).
//!
//! - PRE6: the Docker mock daemon can stamp responses from CLOCK_MONOTONIC exactly like
//!   `soos-daemon`, and every PAM matrix / distro / physical invocation asks for it.
//! - PRE7: no PAM test fixture sends a literal (therefore past or zero) response stamp.
//! - PRE8: the expiry enforcement and the legacy-frame decision are recorded.
//! - PRE9: matrix rows backed by the QFX5 Docker evidence are no longer pending, and the
//!   duplicated documentation blocks are gone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const MOCK: &str = "tests/docker/mock_daemon.py";
/// `soos-daemon` `RESPONSE_VALIDITY_NS`.
const VALIDITY_NS: u64 = 2_000_000_000;

/// Scripts that start the mock daemon for a PAM exchange.
const MOCK_CALLERS: [&str; 5] = [
    "tests/docker/test_suite.sh",
    "tests/distro/arch_linux_test.sh",
    "tests/distro/debian_ubuntu_test.sh",
    "tests/distro/fedora_rhel_test.sh",
    "tests/physical/pam_integration_test.sh",
];

fn workspace_root() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    Path::new(&manifest_dir)
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn matrix_row(matrix: &str, id: &str) -> String {
    let prefix = format!("| {id} |");
    matrix
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("matrix row {id} is missing"))
        .to_string()
}

fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "soos-pre-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// CLOCK_MONOTONIC as seen by another process (the clock the mock and PAM share).
fn monotonic_ns_via_python() -> u64 {
    let out = Command::new("python3")
        .args([
            "-c",
            "import time; print(time.clock_gettime_ns(time.CLOCK_MONOTONIC))",
        ])
        .output()
        .expect("run python3");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("monotonic nanoseconds")
}

fn current_group_name() -> String {
    let out = Command::new("id").arg("-gn").output().expect("id -gn");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Minimal untagged protocol-v1 `Request` (Auth, id 0xAB.., uid 0, "test", deadline 1).
fn framed_test_request() -> Vec<u8> {
    let mut body = vec![1u8, 0u8];
    body.extend_from_slice(&[0xAB; 32]);
    body.push(0);
    body.push(4);
    body.extend_from_slice(b"test");
    body.push(1);
    let mut frame = u32::try_from(body.len())
        .expect("small body")
        .to_be_bytes()
        .to_vec();
    frame.extend(body);
    frame
}

/// Sends one request to the mock started with `args` and returns its answer.
fn mock_answer(args: &[&str]) -> Vec<u8> {
    let dir = scratch_dir("mock");
    let sock = dir.join("daemon.sock");
    let mut child = Command::new("python3")
        .arg(workspace_root().join(MOCK))
        .args(args)
        .args(["--one-shot", "--socket"])
        .arg(&sock)
        .args(["--socket-group", &current_group_name()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mock_daemon.py");
    let start = Instant::now();
    while !sock.exists() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut answer = Vec::new();
    if sock.exists() {
        let mut stream = UnixStream::connect(&sock).expect("connect mock");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("read timeout");
        stream
            .write_all(&framed_test_request())
            .expect("write request");
        stream.read_to_end(&mut answer).expect("read answer");
    }
    child.kill().ok();
    child.wait().ok();
    fs::remove_dir_all(&dir).ok();
    assert!(!answer.is_empty(), "mock_daemon.py {args:?} never answered");
    answer
}

fn read_varint(buf: &[u8], idx: &mut usize) -> u64 {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = buf[*idx];
        *idx += 1;
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
        shift += 7;
        assert!(shift < 64, "varint too long");
    }
}

/// Decodes `(verdict, issued, expires)` from a framed mock `Response` and checks that the
/// frame is consumed exactly.
fn decode_stamps(frame: &[u8]) -> (u64, u64, u64) {
    let declared = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    assert_eq!(frame.len(), 4 + declared, "one complete frame");
    assert_eq!(frame[4], 1, "protocol version 1");
    let mut idx = 4 + 1 + 32;
    let verdict = read_varint(frame, &mut idx);
    let _reason = read_varint(frame, &mut idx);
    let issued = read_varint(frame, &mut idx);
    let expires = read_varint(frame, &mut idx);
    assert_eq!(idx, frame.len(), "no trailing byte after the stamps");
    (verdict, issued, expires)
}

/// PRE6: `--stamps monotonic` stamps like `soos-daemon` (issued from CLOCK_MONOTONIC at
/// send time, expires 2 s later); `--mode expired` sends an Allow whose window closed.
#[test]
fn test_pre_mock_daemon_stamps_from_the_monotonic_clock() {
    let before = monotonic_ns_via_python();
    let allow = mock_answer(&["--mode", "allow", "--stamps", "monotonic"]);
    let after = monotonic_ns_via_python();
    let (verdict, issued, expires) = decode_stamps(&allow);
    assert_eq!(verdict, 0, "allow verdict");
    assert!(
        (before..=after).contains(&issued),
        "issued {issued} must be read from CLOCK_MONOTONIC between {before} and {after}"
    );
    assert_eq!(expires, issued + VALIDITY_NS, "expires = issued + 2 s");

    let before = monotonic_ns_via_python();
    let expired = mock_answer(&["--mode", "expired", "--stamps", "monotonic"]);
    let (verdict, issued, expires) = decode_stamps(&expired);
    assert_eq!(verdict, 0, "expired mode carries an Allow verdict");
    assert!(issued > 0 && issued <= expires, "consistent window");
    assert!(
        expires < before,
        "expired mode must send a window that closed before the request ({expires} >= {before})"
    );

    let zero = mock_answer(&["--mode", "allow", "--stamps", "zero"]);
    let (_, issued, expires) = decode_stamps(&zero);
    assert_eq!(
        (issued, expires),
        (0, 0),
        "--stamps zero keeps the unstamped form"
    );
}

/// PRE6: every script that runs a PAM exchange against the mock asks for real stamps, and
/// the T15 rejection loop exercises the expired and unstamped Allow.
#[test]
fn test_pre_every_mock_invocation_requests_monotonic_stamps() {
    for rel in MOCK_CALLERS {
        let script = read(rel);
        let invocations: Vec<&str> = script
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter(|l| l.contains("python3 tests/docker/mock_daemon.py"))
            .collect();
        assert!(!invocations.is_empty(), "{rel} starts no mock daemon");
        for line in invocations {
            assert!(
                line.contains("--stamps monotonic"),
                "{rel}: mock invocation without --stamps monotonic: {line}"
            );
        }
    }
    let suite = read("tests/docker/test_suite.sh");
    let t15 = &suite[suite.find("# T15:").expect("T15 case")..];
    assert!(
        t15.contains("expired"),
        "T15 must exercise mock mode 'expired'"
    );
    assert!(
        t15.contains("--stamps zero"),
        "T15 must exercise an unstamped Allow (--stamps zero)"
    );
}

/// PRE7: PAM test fixtures never hard-code a response stamp (a literal is either zero or
/// long past on any booted machine, so the enforcing client would reject it).
#[test]
fn test_pre_pam_fixtures_never_send_literal_response_stamps() {
    let mut violations = Vec::new();
    let mut stack = vec![workspace_root().join("crates/pam/tests")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = fs::read_to_string(&path).expect("read fixture");
            for (n, line) in text.lines().enumerate() {
                for field in ["issued_monotonic_ns: ", "expires_monotonic_ns: "] {
                    if let Some(pos) = line.find(field) {
                        let value = &line[pos + field.len()..];
                        if value.starts_with(|c: char| c.is_ascii_digit()) {
                            violations.push(format!("{}:{}: {}", path.display(), n + 1, line));
                        }
                    }
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "PAM fixtures must stamp responses from CLOCK_MONOTONIC (common/stamps.rs):\n{}",
        violations.join("\n")
    );
}

/// PRE8: the expiry enforcement is recorded in the ADR register and the protocol docs.
#[test]
fn test_pre_expiry_enforcement_is_documented() {
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.contains("PAM Client Enforces Response Expiry"),
        "AI/DECISIONS.md must record the expiry enforcement ADR"
    );
    for rel in ["Docs/IPC_PROTOCOL.md", "Docs/PAM_MODULE.md"] {
        let doc = read(rel);
        for needle in [
            "MAX_RESPONSE_FUTURE_SKEW_NS",
            "StaleResponse",
            "check_freshness",
        ] {
            assert!(doc.contains(needle), "{rel} must document `{needle}`");
        }
    }
}

/// PRE8: untagged legacy client frames stay accepted by owner decision; the decision is
/// recorded and the contract test that pins it is still present.
#[test]
fn test_pre_legacy_untagged_frames_decision_is_recorded() {
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.contains("Untagged Legacy Client Frames Stay Accepted"),
        "AI/DECISIONS.md must record the owner decision on untagged legacy frames"
    );
    let ipc = read("Docs/IPC_PROTOCOL.md");
    assert!(
        ipc.contains("Untagged Legacy Client Frames Stay Accepted"),
        "Docs/IPC_PROTOCOL.md §12 must cite the legacy-frame ADR"
    );
    let daemon_test = read("crates/daemon/tests/wire_discriminator_tests.rs");
    assert!(
        daemon_test.contains("fn test_204_legacy_untagged_status_request_still_served"),
        "the legacy-frame contract test must stay in place"
    );
}

/// PRE9: rows whose Docker evidence QFX5 records cite that green run and are verified.
#[test]
fn test_pre_rows_backed_by_qfx5_cite_the_green_run() {
    let matrix = read("AI/VERIFICATION_MATRIX.md");
    for (id, job) in [
        ("DDS6", "110097611926"),
        ("QFU3", "110097611816"),
        ("PK7", "110097611919"),
        ("DV3", "110097611919"),
    ] {
        let row = matrix_row(&matrix, id);
        assert!(row.contains(job), "{id} must cite QFX5 job {job}: {row}");
        assert!(
            !row.contains("Pending"),
            "{id} must no longer be pending: {row}"
        );
        assert!(
            !row.contains("no green CI run URL yet"),
            "{id} still claims there is no green run: {row}"
        );
        assert!(
            row.trim_end().ends_with("✅ Verified |"),
            "{id} status: {row}"
        );
    }
}

/// PRE9: the duplicated documentation blocks are gone (one `StatusResponse` description,
/// one statement of the PCZ6 supersession).
#[test]
fn test_pre_duplicated_protocol_blocks_are_merged() {
    let ipc = read("Docs/IPC_PROTOCOL.md");
    assert_eq!(
        ipc.matches("Non-biometric health snapshot returned for")
            .count(),
        1,
        "Docs/IPC_PROTOCOL.md must describe StatusResponse once"
    );
    let matrix = read("AI/VERIFICATION_MATRIX.md");
    assert_eq!(
        matrix
            .matches("## Component: `strict-codec-client-tag-integration`")
            .count(),
        1,
        "the strict-codec-client-tag-integration section must appear once"
    );
    assert_eq!(
        matrix
            .matches("It closes the #224 discriminator item")
            .count(),
        0,
        "the PCZ6 supersession is stated once, in the strict-codec section"
    );
}
