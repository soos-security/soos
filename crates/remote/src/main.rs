//! `soos-remote` binary entry point (GitHub #339, spec §2.2, §4; Funnel/passkey spec §9.1).
//!
//! Start sequence: tracing to stderr (journald) → refuse root → command line → default
//! configuration path → bounded configuration load → either one of the local subcommands
//! (`enroll-code`, `passkeys list`, `passkeys remove N`) or the service: current-thread Tokio
//! runtime → socket directory and listener → `serve` until SIGTERM / SIGINT.
//! Configuration-class failures exit `EXIT_CONFIG` (78, never restarted by the user unit),
//! runtime failures exit `EXIT_RUNTIME` (1, restarted).
//!
//! Subcommand output goes through `writeln!` on a locked stdout; nothing printed ever
//! contains a credential id, a public key, a user handle or a Tailscale login. The enrollment
//! code is printed exactly once, by `enroll-code`, and never logged.

#![forbid(unsafe_code)]

use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use soos_remote::auth::system_random;
use soos_remote::config::{
    check_not_root, default_config_path, load_config, resolve_credentials_path, RemoteConfig,
};
use soos_remote::credentials::{credential_hash, CredentialStore, StoreError};
use soos_remote::enroll::{write_code_file, CodeFile, EnrollCode};
use soos_remote::logind::ZbusSessionSource;
use soos_remote::server::{serve, ServerState};
use soos_remote::socket::{bind_listener, prepare_socket_dir, SocketError};
use soos_remote::{ENROLL_CODE_TTL_S, EXIT_CONFIG, EXIT_RUNTIME, MAX_PASSKEYS};

/// Command line of `soos-remote`.
#[derive(Debug, Parser)]
#[command(
    name = "soos-remote",
    about = "soos remote companion: lock status, remote lock and passkey-protected remote unlock over a Unix socket behind Tailscale Serve or Funnel",
    disable_help_subcommand = true
)]
struct Args {
    /// Configuration file (default: $XDG_CONFIG_HOME/soos/remote.toml, else
    /// $HOME/.config/soos/remote.toml).
    #[arg(long, value_name = "PATH", global = true)]
    config: Option<PathBuf>,
    /// Without a subcommand the service runs.
    #[command(subcommand)]
    command: Option<Command>,
}

/// Local administration subcommands (never reachable over the socket).
#[derive(Debug, Subcommand)]
enum Command {
    /// Print a one-time enrollment code (valid 5 minutes, single use) to register a passkey
    /// from the phone over the tailnet.
    EnrollCode,
    /// List or remove the registered passkeys.
    Passkeys {
        #[command(subcommand)]
        action: PasskeysAction,
    },
}

/// `soos-remote passkeys …`.
#[derive(Debug, Subcommand)]
enum PasskeysAction {
    /// Index, registration time, backup state and a short fingerprint of every passkey.
    List,
    /// Remove passkey number N (as shown by `list`, 1-based).
    Remove {
        /// 1-based index.
        index: usize,
    },
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
    let credentials_path = match resolve_credentials_path(&config.auth, &config_path) {
        Ok(path) => path,
        Err(err) => {
            error!(%err, "configuration refused");
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    match args.command {
        Some(Command::EnrollCode) => enroll_code(&config, uid),
        Some(Command::Passkeys { action }) => passkeys(action, credentials_path, uid),
        None => run_service(config, credentials_path, uid),
    }
}

/// Unix time in seconds (0 when the clock is before the epoch).
fn now_unix_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Writes `text` and a newline on a locked stdout.
fn say(text: &str) -> Result<(), std::io::Error> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "{text}")?;
    out.flush()
}

/// `soos-remote enroll-code`: a fresh 5-minute single-use code, written (hashed) to the
/// socket directory and printed once.
fn enroll_code(config: &RemoteConfig, uid: u32) -> ExitCode {
    let Some(rp_id) = config.auth.rp_id.as_deref() else {
        error!("rp_id is not configured");
        return ExitCode::from(EXIT_CONFIG);
    };
    let Some(socket_dir) = config.socket_path.parent() else {
        error!("socket path has no parent directory");
        return ExitCode::from(EXIT_CONFIG);
    };
    if let Err(err) = prepare_socket_dir(socket_dir, uid) {
        return socket_failure(&err, "socket directory refused");
    }
    let random = system_random();
    let code = match EnrollCode::generate(&random) {
        Ok(code) => code,
        Err(err) => {
            error!(%err, "enrollment code not generated");
            return ExitCode::from(EXIT_RUNTIME);
        }
    };
    let file = CodeFile {
        code_hash: code.hash(),
        expires_unix_s: now_unix_s().saturating_add(ENROLL_CODE_TTL_S),
    };
    if let Err(err) = write_code_file(socket_dir, &file, &random) {
        error!(%err, "enrollment code not written");
        return ExitCode::from(EXIT_RUNTIME);
    }
    let shown = code.display();
    let printed = say(&format!(
        "Enrollment code: {shown} (valid 5 min, single use)"
    ))
    .and_then(|()| {
        say(&format!(
            "Open https://{rp_id} on the phone over the tailnet (VPN on), then tap \"Add this device's passkey\" and type the code."
        ))
    });
    match printed {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(EXIT_RUNTIME),
    }
}

/// `soos-remote passkeys list|remove N` on the resolved credential store.
fn passkeys(action: PasskeysAction, credentials_path: PathBuf, uid: u32) -> ExitCode {
    let mut store = CredentialStore::new(credentials_path, uid);
    match action {
        PasskeysAction::List => list_passkeys(&mut store),
        PasskeysAction::Remove { index } => remove_passkey(&mut store, index),
    }
}

/// Prints one line per stored passkey: index, registration time, backup state and the first
/// 8 hex digits of SHA-256(credential id) (never the id itself).
fn list_passkeys(store: &mut CredentialStore) -> ExitCode {
    let file = match store.load() {
        Ok(file) => file.cloned(),
        Err(err) => {
            error!(%err, "credential store refused");
            return ExitCode::from(EXIT_RUNTIME);
        }
    };
    let records = file.map(|f| f.passkeys).unwrap_or_default();
    let mut lines = Vec::with_capacity(records.len().saturating_add(1));
    if records.is_empty() {
        lines.push("No passkey registered.".to_string());
    }
    for (number, record) in (1usize..).zip(records.iter()) {
        let fingerprint: String = credential_hash(&record.credential_id)
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect();
        let backup = match (record.backup_eligible, record.backup_state) {
            (true, true) => "synced",
            (true, false) => "sync-capable, not backed up",
            (false, _) => "device-bound",
        };
        lines.push(format!(
            "{number}  created_unix_s={}  {backup}  fingerprint={fingerprint}",
            record.created_unix_s
        ));
    }
    for line in lines {
        if say(&line).is_err() {
            return ExitCode::from(EXIT_RUNTIME);
        }
    }
    ExitCode::SUCCESS
}

/// Removes record `index` (1-based). The user handle is kept even when the last record goes.
fn remove_passkey(store: &mut CredentialStore, index: usize) -> ExitCode {
    if index == 0 || index > MAX_PASSKEYS {
        error!("no such passkey");
        return ExitCode::from(EXIT_CONFIG);
    }
    let removed = store.update(&system_random(), |current| {
        let mut file = current.ok_or(StoreError::NoSuchPasskey)?;
        let position = index.saturating_sub(1);
        if position >= file.passkeys.len() {
            return Err(StoreError::NoSuchPasskey);
        }
        file.passkeys.remove(position);
        Ok((file, ()))
    });
    match removed {
        Ok(()) => match say(&format!("Passkey {index} removed.")) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::from(EXIT_RUNTIME),
        },
        Err(StoreError::NoSuchPasskey) => {
            error!("no such passkey");
            ExitCode::from(EXIT_CONFIG)
        }
        Err(err) => {
            error!(%err, "credential store refused");
            ExitCode::from(EXIT_RUNTIME)
        }
    }
}

/// The service: current-thread runtime, then [`run`].
fn run_service(config: RemoteConfig, credentials_path: PathBuf, uid: u32) -> ExitCode {
    if config.allow_unlock && config.auth.rp_id.is_none() {
        warn!("allow_unlock is set but rp_id is not: every remote unlock is refused");
    }
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
    runtime.block_on(run(config, credentials_path, uid))
}

/// Socket preparation, signal handling and the server loop (inside the runtime).
async fn run(config: RemoteConfig, credentials_path: PathBuf, uid: u32) -> ExitCode {
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

    let state = Arc::new(
        ServerState::new(config, uid, ZbusSessionSource::new())
            .with_credentials_path(credentials_path),
    );
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
