//! Privileged background daemon for soos local facial biometric PAM verification.

#![deny(clippy::undocumented_unsafe_blocks)]

pub mod config;
pub mod dispatcher;
pub mod error;
pub mod health;
pub mod logging;
pub mod mlock;
pub mod peercred;
pub mod pipeline;
pub mod socket;

pub use config::{DaemonConfig, DispatcherConfig, PipelineConfig, SocketConfig};
pub use error::DaemonError;
pub use health::{HealthState, HealthStatus};
pub use pipeline::{initialize_pipeline, PipelineComponents};
pub use socket::{bind_socket, validate_directory, SocketGuard};
