//! Privileged background daemon entry point (`soos-daemon`).

#![forbid(unsafe_code)]

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, info, warn};

use soos_daemon::config::DaemonConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::logging::init_logging;
use soos_daemon::pipeline::{initialize_pipeline, warmed_inference_gate, EMBEDDING_MODEL_ID};
use soos_daemon::sd_notify::{self, NotifyOutcome};
use soos_daemon::shutdown::{
    accept_until_shutdown, install_panic_hook, install_panic_hook_with, remaining_budget,
    shutdown_runtime, ConnectionTasks, PanicMessagePolicy,
};
use soos_daemon::socket::bind_socket;

/// Poll interval of the camera health transition logger.
const CAMERA_HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Privileged background daemon for soos local biometric PAM verification.
#[derive(Parser, Debug)]
#[command(
    name = "soos-daemon",
    author = "soos contributors",
    version,
    about = "Privileged background daemon for soos local biometric PAM verification"
)]
struct Cli {
    /// Path to TOML configuration file (defaults to /etc/soos/daemon.toml if present).
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Force mock camera capture manager rather than hardware V4L2 device.
    #[arg(long)]
    mock_camera: bool,
}

/// Runs the daemon until SIGINT/SIGTERM and the bounded drain; returns what is left of the
/// one-`connection_timeout` shutdown budget for the runtime shutdown (GitHub #289).
async fn run() -> Result<Duration, Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let mut config = DaemonConfig::load_or_default(cli.config.as_deref())?;

    if cli.mock_camera {
        config.pipeline.use_mock_camera = true;
    }

    // Initialize structured logging
    let _ = init_logging(&config.log_level);
    // Report panics through tracing (GitHub #259). Release builds log the location only,
    // never the payload; debug builds also log the bounded panic message (GitHub #287).
    match PanicMessagePolicy::for_build() {
        PanicMessagePolicy::Withhold => {
            install_panic_hook();
        }
        policy @ PanicMessagePolicy::LogMessage => install_panic_hook_with(policy),
    }

    info!("Starting soos-daemon (Linux Local Biometric PAM Daemon)");

    let health = Arc::new(HealthState::new());

    // Swap protection (GitHub #201): mlockall is the only page-locking layer; a refusal is
    // logged at warn level and recorded in HealthState::memory_locked.
    soos_daemon::mlock::enable_swap_protection(&health);

    // Initialize full pipeline components fail-closed before opening socket
    info!("Initializing pipeline components and verifying attested machine learning models");
    let components = match initialize_pipeline(&config.pipeline) {
        Ok(comps) => {
            // camera_ready is derived live from the capture supervisor state (GitHub #153):
            // spawning the supervisor does not mean a device was opened.
            health.attach_camera(comps.camera.clone());
            health.set_models_verified(true);
            comps
        }
        Err(err) => {
            error!(error = %err, "Failed to initialize pipeline components; refusing to start daemon (fail-closed)");
            return Err(err.into());
        }
    };

    if config.preview.enabled {
        info!(
            allowed_uids = ?config.preview.allowed_uids,
            max_requests_per_sec = config.preview.max_requests_per_sec,
            "Camera preview stream enabled for the configured unprivileged peers"
        );
    }

    // Seed the inference latency estimate before the socket opens, so the first Auth
    // request is admitted against a measured latency (GitHub #276).
    let inference_gate = warmed_inference_gate(
        components.vision.clone(),
        config.pipeline.camera.width,
        config.pipeline.camera.height,
    )
    .await;

    // Shutdown drain budget: one connection_timeout (GitHub #259).
    let drain_budget = config.dispatcher.connection_timeout;
    let dispatcher = Arc::new(
        ConnectionDispatcher::with_pipeline(config.dispatcher, health.clone(), components)
            .with_inference_gate(inference_gate)
            .with_preview_config(config.preview)
            .with_peer_limits(config.peer_limits)
            .with_expected_embedding_model(EMBEDDING_MODEL_ID),
    );

    let (listener, socket_guard) = bind_socket(&config.socket).await?;
    health.set_socket_ready(true);

    let mut sigterm = signal(SignalKind::terminate())?;

    // Log camera lifecycle transitions (missing device, recovery, dead capture thread).
    let monitor_health = health.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(CAMERA_HEALTH_POLL_INTERVAL);
        let mut last = None;
        loop {
            ticker.tick().await;
            let current = monitor_health.camera_health();
            if current != last {
                match current {
                    Some(state) if state.is_operational() => {
                        info!(camera_state = %state, "Camera health changed");
                    }
                    Some(state) => {
                        warn!(camera_state = %state, "Camera health changed; camera not ready");
                    }
                    None => {}
                }
                last = current;
            }
        }
    });

    info!(
        status = %health.snapshot(),
        "soos-daemon initialized and listening for PAM requests"
    );

    // Type=notify (GitHub #203): report readiness only now that the socket is bound, so
    // Before=display-manager.service holds the greeter until PAM requests can be served.
    match sd_notify::notify_ready() {
        Ok(NotifyOutcome::Sent) => info!("Reported readiness to systemd"),
        Ok(NotifyOutcome::NotSupervised) => {}
        Err(err) => warn!(error = %err, "Failed to report readiness to systemd"),
    }

    let shutdown_signal = async {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received SIGINT signal; shutting down gracefully");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM signal; shutting down gracefully");
            }
        }
    };

    let mut tasks = ConnectionTasks::new();
    accept_until_shutdown(
        &listener,
        Arc::clone(&dispatcher),
        &mut tasks,
        shutdown_signal,
    )
    .await;

    // Stop accepting and unlink the socket before draining, so no new PAM client can
    // connect to a daemon that is going away (GitHub #259).
    drop(listener);
    if let Err(err) = sd_notify::notify_stopping() {
        warn!(error = %err, "Failed to report shutdown to systemd");
    }
    health.set_socket_ready(false);
    drop(socket_guard);

    let drain_started = Instant::now();
    let report = tasks.drain(drain_budget).await;
    if !report.is_clean() {
        warn!(
            panicked = report.panicked,
            aborted = report.aborted,
            "Some connections ended without a verdict during shutdown (fail-closed)"
        );
    }
    // Spoof evidence writes spawned by those handlers get what is left of the same
    // one-connection_timeout budget (GitHub #287).
    let evidence_budget = drain_budget.saturating_sub(drain_started.elapsed());
    let evidence_report = dispatcher.evidence_writes().drain(evidence_budget).await;
    if !evidence_report.is_clean() {
        warn!(
            panicked = evidence_report.panicked,
            abandoned = evidence_report.aborted,
            "Some evidence writes did not finish before shutdown"
        );
    }
    info!("soos-daemon terminated cleanly");
    Ok(remaining_budget(
        drain_started,
        drain_budget,
        Instant::now(),
    ))
}

/// Builds the Tokio runtime explicitly instead of the `tokio::main` attribute, whose runtime drop waits
/// without bound for every blocking job (GitHub #289): `shutdown_runtime` gives the blocking
/// pool what is left of the shutdown budget, then exits without awaiting abandoned jobs.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    match runtime.block_on(run()) {
        Ok(remaining) => {
            shutdown_runtime(runtime, remaining);
            Ok(())
        }
        Err(err) => {
            // Startup failed before any connection was served: no tracked work to wait for.
            shutdown_runtime(runtime, Duration::ZERO);
            Err(err)
        }
    }
}
