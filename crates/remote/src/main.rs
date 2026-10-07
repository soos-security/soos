//! `soos-remote` binary entry point (GitHub #339, spec §2.2, §4; Funnel/passkey spec §9.1).
//!
//! Start sequence: tracing to stderr (journald) → refuse root → command line → default
//! configuration path → bounded configuration load → either one of the local subcommands
//! (`enroll-code`, `passkeys list`, `passkeys remove N`) or the service: current-thread Tokio
//! runtime → socket directory and listener → `serve` until SIGTERM / SIGINT.
//! Configuration-class failures exit `EXIT_CONFIG` (78, never restarted by the user unit),
//! runtime failures exit `EXIT_RUNTIME` (1, restarted).
//!
//! With `password_alerts = true` the service resolves the owner's login once (passwd entry
//! of the process uid) and wires the `journalctl` source of the failed-password alerts (ADR
//! 2026-10-06 "Failed-Password Alerts in `soos-remote` From the System Journal"); an
//! unresolvable login leaves the alerts `unavailable` and the service still starts.
//!
//! With `push_notifications = true` the service also wires Web Push (ADR 2026-10-06 "Web
//! Push Notifications for Failed-Password Alerts Through a Separate Sender Unit"): the store
//! `remote-push.json` next to the credential store and the Unix-socket transport to
//! `soos-push-sender`. `push list`, `push remove N` and `push reset` manage the store
//! locally; their lines never contain an endpoint path or a key.
//!
//! With `camera_view = true` the service also wires the live camera view (ADR 2026-10-07
//! "Live Camera View in `soos-remote` Through the Daemon Preview Channel"): one daemon
//! preview client per view on `/run/soos/daemon.sock` (root peer only), never a device.
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

use soos_remote::alerts::AlertSettings;
use soos_remote::auth::system_random;
use soos_remote::camera::CameraSettings;
use soos_remote::camera_ipc::{DaemonPreviewClient, PreviewSource, PreviewSourceFactory};
use soos_remote::config::{
    check_not_root, default_config_path, load_config, resolve_alerts_ack_path,
    resolve_credentials_path, resolve_push_store_path, RemoteConfig,
};
use soos_remote::credentials::{credential_hash, CredentialStore, StoreError};
use soos_remote::enroll::{write_code_file, CodeFile, EnrollCode};
use soos_remote::journal::{JournalctlSource, OwnerLogin};
use soos_remote::logind::ZbusSessionSource;
use soos_remote::push::{
    subscription_line, PushSettings, PushStore, PushStoreError, UnixPushTransport,
};
use soos_remote::server::{serve, ServerState};
use soos_remote::socket::{bind_listener, prepare_socket_dir, SocketError};
use soos_remote::{
    CAMERA_DAEMON_SOCKET_PATH, ENROLL_CODE_TTL_S, EXIT_CONFIG, EXIT_RUNTIME, MAX_PASSKEYS,
    MAX_PUSH_SUBSCRIPTIONS, STORE_LOCK_TIMEOUT_MS,
};

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
    /// List or remove the Web Push subscriptions, or replace the push key.
    Push {
        #[command(subcommand)]
        action: PushAction,
    },
}

/// `soos-remote push …`.
#[derive(Debug, Subcommand)]
enum PushAction {
    /// Index, push service host and creation time of every subscription.
    List,
    /// Remove subscription number N (as shown by `list`, 1-based).
    Remove {
        /// 1-based index.
        index: usize,
    },
    /// Replace the push key and drop every subscription (each phone must enable
    /// notifications again).
    Reset,
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
        Some(Command::Push { action }) => push_command(action, &credentials_path, uid),
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

/// Retries `op` while the push store is locked by the service, for at most
/// `STORE_LOCK_TIMEOUT_MS` (blocking sleeps are fine in the CLI).
fn with_push_lock<T>(
    mut op: impl FnMut() -> Result<T, PushStoreError>,
) -> Result<T, PushStoreError> {
    let started = std::time::Instant::now();
    loop {
        match op() {
            Err(PushStoreError::Locked)
                if started.elapsed() < std::time::Duration::from_millis(STORE_LOCK_TIMEOUT_MS) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            other => return other,
        }
    }
}

/// `soos-remote push list|remove N|reset` on the push store next to the credential store.
fn push_command(action: PushAction, credentials_path: &std::path::Path, uid: u32) -> ExitCode {
    let path = match resolve_push_store_path(credentials_path) {
        Ok(path) => path,
        Err(err) => {
            error!(%err, "configuration refused");
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    let store = PushStore::new(path, uid);
    let random = system_random();
    let lines = match action {
        PushAction::List => match store.load() {
            Ok(Some(file)) if !file.subscriptions.is_empty() => (1usize..)
                .zip(file.subscriptions.iter())
                .map(|(number, sub)| subscription_line(number, sub))
                .collect(),
            Ok(_) => vec!["No push subscription.".to_string()],
            Err(err) => {
                error!(%err, "push store refused");
                return ExitCode::from(EXIT_RUNTIME);
            }
        },
        PushAction::Remove { index } => {
            if index == 0 || index > MAX_PUSH_SUBSCRIPTIONS {
                error!("no such push subscription");
                return ExitCode::from(EXIT_CONFIG);
            }
            match with_push_lock(|| store.remove_index(index, &random)) {
                Ok(true) => vec![format!("Push subscription {index} removed.")],
                Ok(false) => {
                    error!("no such push subscription");
                    return ExitCode::from(EXIT_CONFIG);
                }
                Err(err) => {
                    error!(%err, "push store refused");
                    return ExitCode::from(EXIT_RUNTIME);
                }
            }
        }
        PushAction::Reset => match with_push_lock(|| store.reset(&random)) {
            Ok(_) => vec![
                "Push key replaced; every subscription was removed.".to_string(),
                "Open the soos app on each phone and tap \"Enable notifications\" again."
                    .to_string(),
            ],
            Err(err) => {
                error!(%err, "push store refused");
                return ExitCode::from(EXIT_RUNTIME);
            }
        },
    };
    for line in lines {
        if say(&line).is_err() {
            return ExitCode::from(EXIT_RUNTIME);
        }
    }
    ExitCode::SUCCESS
}

/// The Web Push settings and transport (`None` when push is off or `rp_id`/the socket path
/// is missing, which the configuration already refuses).
fn push_wiring(
    config: &RemoteConfig,
    credentials_path: &std::path::Path,
    uid: u32,
) -> Option<(PushSettings, Arc<UnixPushTransport>)> {
    if !config.push.enabled {
        return None;
    }
    let rp_id = config.auth.rp_id.clone()?;
    let socket_path = config.push.socket_path.clone()?;
    let store_path = match resolve_push_store_path(credentials_path) {
        Ok(path) => path,
        Err(err) => {
            warn!(%err, "push store path refused: push notifications unavailable");
            return None;
        }
    };
    let subject = config
        .push
        .vapid_subject
        .clone()
        .unwrap_or_else(|| format!("https://{rp_id}"));
    Some((
        PushSettings {
            store_path,
            subject,
            previews: config.push.previews,
            rp_id,
        },
        Arc::new(UnixPushTransport::new(socket_path, uid)),
    ))
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

    let alerts = config
        .alerts
        .enabled
        .then(|| alert_settings(&config, &credentials_path, uid));
    let push = push_wiring(&config, &credentials_path, uid);
    let camera = config.camera.enabled.then(|| {
        let factory: PreviewSourceFactory = Arc::new(move || -> Box<dyn PreviewSource> {
            Box::new(DaemonPreviewClient::new(
                PathBuf::from(CAMERA_DAEMON_SOCKET_PATH),
                uid,
            ))
        });
        (CameraSettings::from_config(&config.camera), factory)
    });
    let mut state = ServerState::new(config, uid, ZbusSessionSource::new())
        .with_credentials_path(credentials_path);
    if let Some(settings) = alerts {
        state = state.with_password_alerts(settings, Arc::new(JournalctlSource));
    }
    if let Some((settings, transport)) = push {
        state = state.with_push(settings, transport);
    }
    if let Some((settings, factory)) = camera {
        state = state.with_camera(settings, factory);
    }
    let state = Arc::new(state);
    match serve(listener, state, shutdown).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!(%err, "server stopped");
            ExitCode::from(EXIT_RUNTIME)
        }
    }
}

/// The failed-password alert settings: the owner's login (passwd entry of `uid`; `None`
/// when unresolvable or invalid), the configured lock-screen programs and the
/// acknowledgement file next to the credential store (`None`: in memory only).
fn alert_settings(
    config: &RemoteConfig,
    credentials_path: &std::path::Path,
    uid: u32,
) -> AlertSettings {
    let owner_login = match nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)) {
        Ok(Some(user)) => OwnerLogin::parse(&user.name),
        Ok(None) | Err(_) => None,
    };
    if owner_login.is_none() {
        warn!("owner login unresolved: password alerts unavailable");
    }
    AlertSettings {
        owner_login,
        lock_screen_programs: config.alerts.lock_screen_programs.clone(),
        ack_path: resolve_alerts_ack_path(credentials_path).ok(),
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
