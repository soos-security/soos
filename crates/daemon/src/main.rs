//! Privileged background daemon entry point (`soos-daemon`).

#![forbid(unsafe_code)]

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, info, warn};

use soos_daemon::config::DaemonConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::logging::init_logging;
use soos_daemon::pipeline::{initialize_pipeline, warmed_inference_gate, EMBEDDING_MODEL_ID};
use soos_daemon::socket::bind_socket;

/// Poll interval of the camera health transition logger.
const CAMERA_HEALTH_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let mut config = DaemonConfig::load_or_default(cli.config.as_deref())?;

    if cli.mock_camera {
        config.pipeline.use_mock_camera = true;
    }

    // Initialize structured logging
    let _ = init_logging(&config.log_level);

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

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _addr)) => {
                        let disp = dispatcher.clone();
                        tokio::spawn(async move {
                            if let Err(err) = disp.handle_connection(stream).await {
                                warn!(error = %err, "Connection handler finished with error");
                            }
                        });
                    }
                    Err(err) => {
                        error!(error = %err, "Accept failed");
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Received SIGINT signal; shutting down gracefully");
                break;
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM signal; shutting down gracefully");
                break;
            }
        }
    }

    health.set_socket_ready(false);
    drop(socket_guard);
    info!("soos-daemon terminated cleanly");
    Ok(())
}
