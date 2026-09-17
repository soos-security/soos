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
use soos_daemon::pipeline::initialize_pipeline;
use soos_daemon::socket::bind_socket;

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

    // Enable swap protection by locking process memory into physical RAM
    if soos_daemon::mlock::mlock_process_address_space() {
        info!("Swap protection active: process memory pages locked into RAM via mlockall");
    } else {
        tracing::debug!("Swap protection mlockall not granted (requires CAP_IPC_LOCK); continuing with granular buffer protection");
    }

    let health = Arc::new(HealthState::new());

    // Initialize full pipeline components fail-closed before opening socket
    info!("Initializing pipeline components and verifying attested machine learning models");
    let components = match initialize_pipeline(&config.pipeline) {
        Ok(comps) => {
            health.set_camera_ready(true);
            health.set_models_verified(true);
            comps
        }
        Err(err) => {
            error!(error = %err, "Failed to initialize pipeline components; refusing to start daemon (fail-closed)");
            return Err(err.into());
        }
    };

    let dispatcher = Arc::new(ConnectionDispatcher::with_pipeline(
        config.dispatcher,
        health.clone(),
        components,
    ));

    let (listener, socket_guard) = bind_socket(&config.socket).await?;
    health.set_socket_ready(true);

    let mut sigterm = signal(SignalKind::terminate())?;

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
