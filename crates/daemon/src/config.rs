//! Configuration structures for the soos daemon.

use std::path::PathBuf;
use std::time::Duration;

/// Configuration for the Unix domain socket listener.
#[derive(Debug, Clone)]
pub struct SocketConfig {
    /// Full path to the Unix domain socket.
    pub socket_path: PathBuf,
    /// Parent runtime directory (e.g. `/run/soos`).
    pub socket_dir: PathBuf,
    /// File permission mode for the socket (typically `0660`).
    pub socket_mode: u32,
    /// Whether to enforce that the parent directory is owned by root (`uid == 0`).
    pub enforce_root_owner: bool,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from("/run/soos/daemon.sock"),
            socket_dir: PathBuf::from("/run/soos"),
            socket_mode: 0o660,
            enforce_root_owner: true,
        }
    }
}

/// Configuration for connection dispatching and bounded concurrency.
#[derive(Debug, Clone)]
pub struct DispatcherConfig {
    /// Maximum concurrent client connections accepted simultaneously.
    pub max_concurrent_connections: usize,
    /// Per-connection execution timeout deadline.
    pub connection_timeout: Duration,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            max_concurrent_connections: 8,
            connection_timeout: Duration::from_millis(250),
        }
    }
}

/// Global daemon configuration.
#[derive(Debug, Clone, Default)]
pub struct DaemonConfig {
    /// Socket listener configuration.
    pub socket: SocketConfig,
    /// Connection dispatcher configuration.
    pub dispatcher: DispatcherConfig,
    /// Logging filter directive (e.g. "info", "debug").
    pub log_level: String,
}
