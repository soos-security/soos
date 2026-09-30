//! Contractual tests for the systemd readiness notification of `soos-daemon`
//! (GitHub #203, DMN-14 residual: `Before=display-manager.service` only orders start-up
//! when systemd knows the daemon is actually listening).
//!
//! The notifier is exercised against a local `AF_UNIX` datagram receiver; the process
//! environment is never modified (the `NOTIFY_SOCKET` value is passed explicitly).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::os::unix::net::UnixDatagram;
use std::time::Duration;

use soos_daemon::sd_notify::{
    notify_to, NotifyOutcome, MAX_NOTIFY_SOCKET_PATH_LEN, READY_MESSAGE, STOPPING_MESSAGE,
};

fn receive(receiver: &UnixDatagram) -> String {
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut buf = [0u8; 256];
    let n = receiver.recv(&mut buf).expect("datagram received");
    String::from_utf8(buf[..n].to_vec()).unwrap()
}

#[test]
fn test_notify_messages_are_the_systemd_keywords() {
    assert_eq!(READY_MESSAGE, "READY=1");
    assert_eq!(STOPPING_MESSAGE, "STOPPING=1");
}

#[test]
fn test_notify_without_socket_is_a_no_op() {
    assert_eq!(
        notify_to(None, READY_MESSAGE).unwrap(),
        NotifyOutcome::NotSupervised,
        "Without NOTIFY_SOCKET (manual start, tests) the daemon must not fail"
    );
    assert_eq!(
        notify_to(Some(OsStr::new("")), READY_MESSAGE).unwrap(),
        NotifyOutcome::NotSupervised,
        "An empty NOTIFY_SOCKET means no supervisor"
    );
}

#[test]
fn test_notify_ready_reaches_filesystem_socket() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("notify");
    let receiver = UnixDatagram::bind(&path).unwrap();
    let outcome = notify_to(Some(path.as_os_str()), READY_MESSAGE).unwrap();
    assert_eq!(outcome, NotifyOutcome::Sent);
    assert_eq!(receive(&receiver), "READY=1");
}

#[test]
fn test_notify_reaches_abstract_socket() {
    use std::os::linux::net::SocketAddrExt;
    let name = format!("soos-sd-notify-test-{}", std::process::id());
    let addr = std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
    let receiver = UnixDatagram::bind_addr(&addr).unwrap();
    let env_value = format!("@{name}");
    let outcome = notify_to(Some(OsStr::new(&env_value)), STOPPING_MESSAGE).unwrap();
    assert_eq!(outcome, NotifyOutcome::Sent);
    assert_eq!(receive(&receiver), "STOPPING=1");
}

#[test]
fn test_notify_rejects_relative_or_unsupported_address() {
    for value in ["relative/notify", "vsock:2:1234", "@"] {
        let err = notify_to(Some(OsStr::new(value)), READY_MESSAGE)
            .expect_err("unsupported NOTIFY_SOCKET must be refused");
        assert_eq!(
            err.kind(),
            ErrorKind::InvalidInput,
            "{value:?} must be refused as invalid input"
        );
    }
}

#[test]
fn test_notify_rejects_overlong_path_without_panicking() {
    let long = format!("/{}", "a".repeat(MAX_NOTIFY_SOCKET_PATH_LEN));
    let err = notify_to(Some(OsStr::new(&long)), READY_MESSAGE)
        .expect_err("a path longer than sun_path must be refused");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

#[test]
fn test_notify_to_missing_socket_is_an_error_not_a_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("absent");
    assert!(notify_to(Some(path.as_os_str()), READY_MESSAGE).is_err());
}

#[test]
fn test_notify_rejects_multiline_message() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("notify");
    let _receiver = UnixDatagram::bind(&path).unwrap();
    let err = notify_to(Some(path.as_os_str()), "READY=1\nMAINPID=1")
        .expect_err("only single fixed assignments are sent");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}
