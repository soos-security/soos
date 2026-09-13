//! Privileged background daemon for soos local facial biometric PAM verification.

#![forbid(unsafe_code)]

pub mod config;
pub mod dispatcher;
pub mod error;
pub mod health;
pub mod logging;
pub mod peercred;
pub mod socket;

pub use config::{DaemonConfig, DispatcherConfig, SocketConfig};
pub use error::DaemonError;
pub use health::{HealthState, HealthStatus};
pub use socket::{bind_socket, validate_directory, SocketGuard};
