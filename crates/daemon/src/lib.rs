//! Privileged background daemon for soos local facial biometric PAM verification.

#![deny(clippy::undocumented_unsafe_blocks)]

pub mod config;
pub mod dispatcher;
pub mod error;
pub mod health;
pub mod inference;
pub mod logging;
pub mod mlock;
pub mod peercred;
pub mod pipeline;
pub mod preview;
pub mod session;
pub mod session_policy;
pub mod socket;

pub use config::{DaemonConfig, DispatcherConfig, PipelineConfig, SocketConfig};
pub use error::DaemonError;
pub use health::{HealthState, HealthStatus};
pub use pipeline::{initialize_pipeline, PipelineComponents};
pub use preview::{authorize_preview, PreviewConfig, PreviewDenied};
pub use session::SessionValidator;
pub use session_policy::{LocalSessionPolicy, LogindSource, SessionDenial};
pub use socket::{bind_socket, validate_directory, SocketGuard};
