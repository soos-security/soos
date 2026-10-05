//! `soos-remote` binary entry point (GitHub #339, spec §2.2, §4).
//!
//! Start sequence: tracing to stderr (journald) → refuse root → command line → default
//! configuration path → bounded configuration load → current-thread Tokio runtime → socket
//! directory and listener → `serve` until SIGTERM / SIGINT. Configuration-class failures
//! exit `EXIT_CONFIG` (78, never restarted by the user unit), runtime failures exit
//! `EXIT_RUNTIME` (1, restarted).

#![forbid(unsafe_code)]

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use soos_remote::config::{check_not_root, default_config_path, load_config, RemoteConfig};
use soos_remote::logind::ZbusSessionSource;
use soos_remote::server::{serve, ServerState};
use soos_remote::socket::{bind_listener, prepare_socket_dir, SocketError};
use soos_remote::{EXIT_CONFIG, EXIT_RUNTIME};

/// Command line of `soos-remote`.
#[derive(Debug, Parser)]
#[command(
    name = "soos-remote",
    about = "soos remote companion: lock status and remote lock over a Unix socket behind Tailscale Serve",
    disable_help_subcommand = true
)]
struct Args {
    /// Configuration file (default: $XDG_CONFIG_HOME/soos/remote.toml, else
    /// $HOME/.config/soos/remote.toml).
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
}

fn main() -> ExitCode {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .without_time()
        .try_init();

    let uid = nix::unistd::getuid().as_raw();
    let euid = nix::unistd::geteuid().as_raw();
    if let Err(err) = check_not_root(uid, euid) {
        error!(%err, "refusing to start");
        return ExitCode::from(EXIT_CONFIG);
    }

    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(err) => {
            if matches!(
                err.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                let _ = err.print();
                return ExitCode::SUCCESS;
            }
            error!(kind = ?err.kind(), "invalid command line");
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    let config_path = match args.config {
        Some(path) => path,
        None => match default_config_path(
            env::var_os("XDG_CONFIG_HOME").as_deref(),
            env::var_os("HOME").as_deref(),
        ) {
            Ok(path) => path,
            Err(err) => {
                error!(%err, "no configuration path");
                return ExitCode::from(EXIT_CONFIG);
            }
        },
    };
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let config = match load_config(&config_path, runtime_dir.as_deref()) {
        Ok(config) => config,
        Err(err) => {
            error!(%err, "configuration refused");
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            error!(%err, "runtime start failed");
            return ExitCode::from(EXIT_RUNTIME);
        }
    };
    runtime.block_on(run(config, uid))
}

/// Socket preparation, signal handling and the server loop (inside the runtime).
async fn run(config: RemoteConfig, uid: u32) -> ExitCode {
    let Some(parent) = config.socket_path.parent() else {
        error!("socket path has no parent directory");
        return ExitCode::from(EXIT_CONFIG);
    };
    if let Err(err) = prepare_socket_dir(parent, uid) {
        return socket_failure(&err, "socket directory refused");
    }
    let listener = match bind_listener(&config.socket_path, uid) {
        Ok(listener) => listener,
        Err(err) => return socket_failure(&err, "socket bind refused"),
    };
    info!(socket_path = %config.socket_path.display(), "listening");

    let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        Ok(signal) => signal,
        Err(err) => {
            error!(%err, "signal handler setup failed");
            return ExitCode::from(EXIT_RUNTIME);
        }
    };
    let shutdown = async move {
        tokio::select! {
            _ = sigterm.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        info!("shutdown signal received");
    };

    let state = Arc::new(ServerState::new(config, uid, ZbusSessionSource::new()));
    match serve(listener, state, shutdown).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!(%err, "server stopped");
            ExitCode::from(EXIT_RUNTIME)
        }
    }
}

/// Persistent socket failures are configuration-class (`EXIT_CONFIG`); `Io` is retried by
/// systemd (`EXIT_RUNTIME`).
fn socket_failure(err: &SocketError, context: &'static str) -> ExitCode {
    error!(%err, "{context}");
    if err.is_persistent() {
        ExitCode::from(EXIT_CONFIG)
    } else {
        ExitCode::from(EXIT_RUNTIME)
    }
}
