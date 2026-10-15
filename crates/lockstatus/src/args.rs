//! Command line of `soos-lockstatus`.

use std::path::PathBuf;

use clap::Parser;

use crate::format::OutputFormat;

/// Reads the coarse soos face-scan state of the current user for a screen locker.
#[derive(Debug, Clone, Parser)]
#[command(name = "soos-lockstatus", version)]
#[command(group(clap::ArgGroup::new("mode").required(true).args(["once", "watch"])))]
pub struct Args {
    /// Print the current state once and exit 0 (hyprlock `cmd[update:N]`).
    #[arg(long)]
    pub once: bool,
    /// Print one JSON line per state change until stdout closes.
    #[arg(long)]
    pub watch: bool,
    /// Output format of `--once`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, requires = "once")]
    pub format: OutputFormat,
    /// Daemon socket path.
    #[arg(long, default_value = crate::DEFAULT_SOCKET_PATH)]
    pub socket: PathBuf,
    /// Poll interval of `--watch` in milliseconds (clamped to 250..=2000).
    #[arg(long, default_value_t = crate::DEFAULT_POLL_INTERVAL_MS, requires = "watch")]
    pub interval_ms: u64,
    /// Reduce motion (static glyphs).
    #[arg(long)]
    pub reduce_motion: bool,
}
