#![forbid(unsafe_code)]
//! `soos-push-sender`: the sandboxed service performing the outbound Web Push requests of the
//! soos remote companion (ADR 2026-10-06, spec §8.1). Two modes:
//!
//! - `soos-push-sender --system-instance <uid>`: the socket-activated system template unit
//!   `soos-push-sender@<uid>.service` (`DynamicUser=yes`, ADR 2026-10-23 "Push Sender as a
//!   Sandboxed System Service"). systemd hands over `/run/soos-push/<uid>/push.sock` as the
//!   standard input; only peer `<uid>` is served.
//! - no argument: the legacy user unit, listening on `$XDG_RUNTIME_DIR/soos-push/push.sock`.
//!
//! Either mode serves one connection at a time until SIGTERM.

use std::os::fd::AsFd;
use std::path::PathBuf;
use std::process::ExitCode;

use soos_push_protocol::{EXIT_CONFIG, EXIT_RUNTIME, PUSH_SOCKET_DIR_NAME};
use soos_push_sender::{
    bind_socket, check_instance_identity, check_not_root, install_logging, listener_from_fd,
    parse_run_mode, probe_sandbox, probe_system_sandbox, serve_connection, RunMode, SendPolicy,
    UreqDeliverer,
};

fn exit_code(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn main() -> ExitCode {
    install_logging();
    let uid = nix::unistd::getuid().as_raw();
    let euid = nix::unistd::geteuid().as_raw();
    if check_not_root(uid, euid).is_err() {
        tracing::error!("the push sender refuses to run as root");
        return exit_code(EXIT_CONFIG);
    }
    match parse_run_mode(std::env::args_os().skip(1)) {
        Ok(RunMode::System { allowed_uid }) => return run_system_instance(uid, euid, allowed_uid),
        Ok(RunMode::User) => {}
        Err(_) => {
            tracing::error!("push sender arguments refused");
            return exit_code(EXIT_CONFIG);
        }
    }
    // ADR 2026-10-12 (S-7): refuse to run outside the unit's sandbox, before any directory
    // or socket is created.
    if probe_sandbox().is_err() {
        tracing::error!("push sender sandbox not detected; refusing to start");
        return exit_code(EXIT_CONFIG);
    }
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
        tracing::error!("XDG_RUNTIME_DIR is not set");
        return exit_code(EXIT_CONFIG);
    };
    if !runtime.is_absolute() {
        tracing::error!("XDG_RUNTIME_DIR is not absolute");
        return exit_code(EXIT_CONFIG);
    }
    let listener = match bind_socket(&runtime.join(PUSH_SOCKET_DIR_NAME), uid) {
        Ok(listener) => listener,
        Err(_) => {
            tracing::error!("push sender socket setup failed");
            return exit_code(EXIT_RUNTIME);
        }
    };
    let deliverer = UreqDeliverer::new(SendPolicy::default());
    tracing::info!("push sender ready");
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => serve_connection(stream, uid, &deliverer),
            Err(_) => {
                tracing::warn!("push sender accept failed");
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    exit_code(EXIT_RUNTIME)
}

/// The system template unit (`packaging/soos-push-sender@.service`): the identity, then the
/// unit's sandbox, then the socket systemd passed as the standard input, each refused with
/// exit 78 (`RestartPreventExitStatus=78`); only then connections of `allowed_uid` are served.
fn run_system_instance(uid: u32, euid: u32, allowed_uid: u32) -> ExitCode {
    if check_instance_identity(uid, euid, allowed_uid).is_err() {
        tracing::error!("push sender instance identity refused");
        return exit_code(EXIT_CONFIG);
    }
    if probe_system_sandbox().is_err() {
        tracing::error!("push sender sandbox not detected; refusing to start");
        return exit_code(EXIT_CONFIG);
    }
    let inherited = std::io::stdin().as_fd().try_clone_to_owned();
    let Some(listener) = inherited.ok().and_then(|fd| listener_from_fd(fd).ok()) else {
        tracing::error!("push sender socket setup failed");
        return exit_code(EXIT_CONFIG);
    };
    let deliverer = UreqDeliverer::new(SendPolicy::default());
    tracing::info!("push sender ready");
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => serve_connection(stream, allowed_uid, &deliverer),
            Err(_) => {
                tracing::warn!("push sender accept failed");
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    exit_code(EXIT_RUNTIME)
}
