//! Command line of `soos-lock-widget`.

use std::path::PathBuf;

use clap::Parser;

/// Animated soos face-scan background for swaylock-plugin (`--command soos-lock-widget`).
#[derive(Debug, Clone, Parser)]
#[command(name = "soos-lock-widget", version)]
pub struct Args {
    /// Daemon socket path.
    #[arg(long, default_value = soos_lockstatus::DEFAULT_SOCKET_PATH)]
    pub socket: PathBuf,
    /// Poll interval in milliseconds (clamped to 250..=2000).
    #[arg(long, default_value_t = soos_lockstatus::DEFAULT_POLL_INTERVAL_MS)]
    pub interval_ms: u64,
    /// Reduce motion (no rotation, no shake, instant colour changes).
    #[arg(long)]
    pub reduce_motion: bool,
}
