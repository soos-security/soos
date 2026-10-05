//! Minimal systemd readiness notification (`sd_notify(3)` protocol) for `Type=notify`
//! (GitHub #203, DMN-14 residual).
//!
//! `Before=display-manager.service` only orders start-up once systemd knows the daemon is
//! ready. With `Type=notify` the daemon sends `READY=1` after its socket is bound, so the
//! greeter never shows its first prompt before `/run/soos/daemon.sock` accepts connections.
//!
//! Safe code only: the message is one datagram on an unbound `AF_UNIX` socket (allowed by
//! `RestrictAddressFamilies=AF_UNIX`). Under `PrivateNetwork=yes` only a filesystem
//! `NOTIFY_SOCKET` (systemd's default, `/run/systemd/notify`) stays reachable; an abstract
//! `@` address is scoped to its network namespace, so a manager that hands out an abstract
//! address cannot be reached from the private namespace and the notification fails (logged
//! at `warn`; with `Type=notify` the start then times out). Only fixed single-line
//! assignments are sent; nothing derived from requests, frames or keys.

use std::ffi::OsStr;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::time::Duration;

/// Environment variable set by systemd for `Type=notify` services.
pub const NOTIFY_SOCKET_ENV: &str = "NOTIFY_SOCKET";

/// Readiness message sent once the IPC socket is bound.
pub const READY_MESSAGE: &str = "READY=1";

/// Message sent when the daemon begins a graceful shutdown.
pub const STOPPING_MESSAGE: &str = "STOPPING=1";

/// Longest accepted `NOTIFY_SOCKET` value in bytes (`sun_path` is 108 bytes including the
/// terminating NUL of a filesystem path).
pub const MAX_NOTIFY_SOCKET_PATH_LEN: usize = 107;

/// Upper bound on the time one notification may block (a full receive queue).
pub const NOTIFY_WRITE_TIMEOUT: Duration = Duration::from_millis(1000);

/// Result of a notification attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyOutcome {
    /// No supervisor asked for notifications (`NOTIFY_SOCKET` unset or empty).
    NotSupervised,
    /// The whole message was delivered in one datagram.
    Sent,
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason)
}

/// Resolves a `NOTIFY_SOCKET` value to a socket address.
fn notify_address(socket: &OsStr) -> io::Result<SocketAddr> {
    let bytes = socket.as_bytes();
    if bytes.len() > MAX_NOTIFY_SOCKET_PATH_LEN {
        return Err(invalid("NOTIFY_SOCKET is longer than sun_path"));
    }
    match bytes.split_first() {
        Some((b'/', _)) => SocketAddr::from_pathname(socket),
        Some((b'@', name)) if !name.is_empty() => {
            use std::os::linux::net::SocketAddrExt;
            SocketAddr::from_abstract_name(name)
        }
        _ => Err(invalid(
            "NOTIFY_SOCKET must be an absolute path or an abstract '@' name",
        )),
    }
}

/// Sends `message` to the notification socket `socket` (the `NOTIFY_SOCKET` value).
///
/// `None` or an empty value is not an error: the daemon was started without a supervisor.
/// `message` must be a single `KEY=VALUE` line.
pub fn notify_to(socket: Option<&OsStr>, message: &str) -> io::Result<NotifyOutcome> {
    send_notification(socket, message, None).map(|(outcome, _)| outcome)
}

/// GitHub #333: [`notify_to`] that also returns the `CLOCK_MONOTONIC` time, in microseconds
/// (`ns / 1000`, floored), read immediately before the datagram is sent. The stamp is `Some`
/// only when the outcome is `Sent` and the clock read succeeded; a clock error never prevents
/// the send. Validation, errors and `NotSupervised` are exactly those of [`notify_to`] (no
/// clock read when not supervised).
pub fn notify_to_stamped(
    socket: Option<&OsStr>,
    message: &str,
) -> io::Result<(NotifyOutcome, Option<u64>)> {
    send_notification(
        socket,
        message,
        Some(crate::pipeline::current_monotonic_nanos),
    )
}

/// Monotonic clock read right before the send (nanoseconds).
type SendClock = fn() -> Result<u64, crate::error::DaemonError>;

/// Shared implementation of [`notify_to`] and [`notify_to_stamped`]: validation, address
/// resolution and socket set-up first, then (when `clock` is given) one clock read, then the
/// send.
fn send_notification(
    socket: Option<&OsStr>,
    message: &str,
    clock: Option<SendClock>,
) -> io::Result<(NotifyOutcome, Option<u64>)> {
    let Some(socket) = socket.filter(|s| !s.is_empty()) else {
        return Ok((NotifyOutcome::NotSupervised, None));
    };
    if message.is_empty() || message.contains('\n') || !message.contains('=') {
        return Err(invalid("notification must be one KEY=VALUE line"));
    }
    let addr = notify_address(socket)?;
    let sender = UnixDatagram::unbound()?;
    sender.set_write_timeout(Some(NOTIFY_WRITE_TIMEOUT))?;
    let stamp_us = clock.and_then(|read| read().ok()).map(|ns| ns / 1_000);
    let sent = sender.send_to_addr(message.as_bytes(), &addr)?;
    if sent != message.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "notification datagram truncated",
        ));
    }
    Ok((NotifyOutcome::Sent, stamp_us))
}

/// Alias of [`notify_ready`] (GitHub #333): `notify_to_stamped(NOTIFY_SOCKET, READY_MESSAGE)`.
pub fn notify_ready_stamped() -> io::Result<(NotifyOutcome, Option<u64>)> {
    notify_ready()
}

fn notify_env(message: &str) -> io::Result<NotifyOutcome> {
    let socket = std::env::var_os(NOTIFY_SOCKET_ENV);
    notify_to(socket.as_deref(), message)
}

/// Reports readiness (`READY=1`) to systemd when supervised, with the `CLOCK_MONOTONIC` send
/// time in microseconds ([`notify_to_stamped`]; GitHub #333, owner approval OA-4).
pub fn notify_ready() -> io::Result<(NotifyOutcome, Option<u64>)> {
    let socket = std::env::var_os(NOTIFY_SOCKET_ENV);
    notify_to_stamped(socket.as_deref(), READY_MESSAGE)
}

/// Reports the start of a graceful shutdown (`STOPPING=1`) to systemd when supervised.
pub fn notify_stopping() -> io::Result<NotifyOutcome> {
    notify_env(STOPPING_MESSAGE)
}
