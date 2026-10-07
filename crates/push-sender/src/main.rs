#![forbid(unsafe_code)]
//! `soos-push-sender`: the optional sandboxed user service performing the outbound Web Push
//! requests of the soos remote companion (ADR 2026-10-06, spec §8.1). It listens on
//! `$XDG_RUNTIME_DIR/soos-push/push.sock` and serves one connection at a time until SIGTERM.

use std::path::PathBuf;
use std::process::ExitCode;

use soos_push_protocol::{EXIT_CONFIG, EXIT_RUNTIME, PUSH_SOCKET_DIR_NAME};
use soos_push_sender::{
    bind_socket, check_not_root, install_logging, serve_connection, SendPolicy, UreqDeliverer,
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
