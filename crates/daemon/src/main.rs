//! Privileged background daemon entry point (`soos-daemon`).

#![forbid(unsafe_code)]

use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, info, warn};

use soos_daemon::config::DaemonConfig;
use soos_daemon::dispatcher::ConnectionDispatcher;
use soos_daemon::health::HealthState;
use soos_daemon::logging::init_logging;
use soos_daemon::socket::bind_socket;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = DaemonConfig::default();

    // Initialize structured logging
    let _ = init_logging("info");

    info!("Starting soos-daemon (Linux Local Biometric PAM Daemon)");

    // Enable swap protection by locking process memory into physical RAM
    if soos_daemon::mlock::mlock_process_address_space() {
        info!("Swap protection active: process memory pages locked into RAM via mlockall");
    } else {
        tracing::debug!("Swap protection mlockall not granted (requires CAP_IPC_LOCK); continuing with granular buffer protection");
    }

    let health = Arc::new(HealthState::new());
    let (listener, socket_guard) = bind_socket(&config.socket).await?;
    health.set_socket_ready(true);

    let dispatcher = Arc::new(ConnectionDispatcher::new(config.dispatcher, health.clone()));

    let mut sigterm = signal(SignalKind::terminate())?;

    info!("soos-daemon initialized and listening for PAM requests");

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
