//! Contractual tests for the `READY=1` send timestamp of `soos-daemon` (GitHub #333 item 7,
//! matrix GHF7, owner approval OA-2).
//!
//! `notify_to_stamped` returns the `CLOCK_MONOTONIC` time (µs) read right before the
//! datagram is sent; the daemon logs it in the readiness message text, and the systemd
//! acceptance harness requires it to be no later than `ActiveEnterTimestampMonotonic`.
//! The process environment is never modified (the socket path is passed explicitly).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::ffi::OsStr;
use std::io::{ErrorKind, Write};
use std::os::unix::net::UnixDatagram;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use soos_daemon::pipeline::current_monotonic_nanos;
use soos_daemon::sd_notify::{
    notify_ready_stamped, notify_to_stamped, NotifyOutcome, NOTIFY_SOCKET_ENV, READY_MESSAGE,
};

fn monotonic_us() -> u64 {
    current_monotonic_nanos().expect("CLOCK_MONOTONIC") / 1000
}

/// GHF7: `Sent` carries a stamp within `[before, after]` µs on `CLOCK_MONOTONIC`, and the
/// datagram is the unchanged `READY=1`.
#[test]
fn test_ghf7_notify_to_stamped_returns_the_monotonic_send_time() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("notify");
    let receiver = UnixDatagram::bind(&path).unwrap();
    let before = monotonic_us();
    let (outcome, stamp) = notify_to_stamped(Some(path.as_os_str()), READY_MESSAGE).unwrap();
    let after = monotonic_us();
    assert_eq!(outcome, NotifyOutcome::Sent);
    let us = stamp.expect("a sent READY=1 must carry its CLOCK_MONOTONIC send time");
    assert!(
        (before..=after).contains(&us),
        "send stamp {us} us must lie in [{before}, {after}] us"
    );
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut buf = [0u8; 64];
    let n = receiver.recv(&mut buf).expect("datagram received");
    assert_eq!(&buf[..n], b"READY=1");
}

/// GHF7: not supervised means no send and no stamp.
#[test]
fn test_ghf7_notify_to_stamped_without_supervisor_has_no_stamp() {
    assert_eq!(
        notify_to_stamped(None, READY_MESSAGE).unwrap(),
        (NotifyOutcome::NotSupervised, None)
    );
    assert_eq!(
        notify_to_stamped(Some(OsStr::new("")), READY_MESSAGE).unwrap(),
        (NotifyOutcome::NotSupervised, None)
    );
}

/// GHF7: validation and errors are exactly those of `notify_to`.
#[test]
fn test_ghf7_notify_to_stamped_keeps_the_notify_to_validation() {
    let err = notify_to_stamped(Some(OsStr::new("relative/notify")), READY_MESSAGE)
        .expect_err("a relative NOTIFY_SOCKET must be refused");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("notify");
    let _receiver = UnixDatagram::bind(&path).unwrap();
    let err = notify_to_stamped(Some(path.as_os_str()), "READY=1\nMAINPID=1")
        .expect_err("a multi-line message must be refused");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    let missing = tmp.path().join("absent");
    assert!(notify_to_stamped(Some(missing.as_os_str()), READY_MESSAGE).is_err());
}

/// GHF7: `notify_ready_stamped` without a supervisor is a no-op (skipped when the test runner
/// itself runs under a `Type=notify` supervisor).
#[test]
fn test_ghf7_notify_ready_stamped_without_supervisor_is_a_no_op() {
    if std::env::var_os(NOTIFY_SOCKET_ENV).is_some_and(|v| !v.is_empty()) {
        return;
    }
    assert_eq!(
        notify_ready_stamped().unwrap(),
        (NotifyOutcome::NotSupervised, None)
    );
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Renders both readiness message shapes through the daemon's compact formatter with ANSI
/// colours on (as under journald).
fn render(us: Option<u64>) -> String {
    let captured = Captured::default();
    let sink = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_ansi(true)
        .compact()
        .with_writer(move || sink.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || match us {
        Some(us) => tracing::info!("Reported readiness to systemd (ready_sent_monotonic_us={us})"),
        None => tracing::info!("Reported readiness to systemd (ready_sent_monotonic_us=unknown)"),
    });
    let bytes = captured.0.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}

/// The exact extraction pipeline of `part2_notify_readiness` in
/// `tests/docker/systemd_unit_acceptance_test.sh`.
fn harness_extract(log: &str) -> String {
    let mut child = Command::new("bash")
        .arg("-c")
        .arg(
            "grep -m1 'Reported readiness to systemd' \
             | sed -e 's/\\x1b\\[[0-9;]*m//g' \
             | sed -n 's/.*(ready_sent_monotonic_us=\\([0-9][0-9]*\\)).*/\\1/p'",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run bash");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(log.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// GHF7 (optional rendering test of spec §4.7.4): the harness extracts the digits from an
/// ANSI-coloured compact line, and nothing from the `unknown` shape.
#[test]
fn test_ghf7_harness_parse_extracts_send_time_from_ansi_compact_line() {
    let line = render(Some(123_456_789));
    assert!(line.contains("Reported readiness to systemd"), "{line:?}");
    assert_eq!(harness_extract(&line), "123456789", "{line:?}");
    let unknown = render(None);
    assert!(
        unknown.contains("ready_sent_monotonic_us=unknown"),
        "{unknown:?}"
    );
    assert_eq!(harness_extract(&unknown), "", "{unknown:?}");
}
