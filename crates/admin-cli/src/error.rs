//! Typed errors for `soos-admin-cli`.

use thiserror::Error;

/// Diagnostic CLI error variants.
#[derive(Debug, Error)]
pub enum AdminCliError {
    /// Socket connection failure.
    #[error("failed to connect to daemon socket at {path}: {source}")]
    SocketConnect {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Socket I/O failure.
    #[error("socket I/O error: {0}")]
    SocketIo(#[from] std::io::Error),

    /// Protocol codec error.
    #[error("protocol codec error: {0}")]
    Codec(#[from] soos_protocol::codec::CodecError),

    /// Cryptographic random number generator error.
    #[error("random generator failure: {0}")]
    Random(getrandom::Error),

    /// Communication deadline expired.
    #[error("daemon communication timed out")]
    Timeout,

    /// Unexpected response payload or format.
    #[error("unexpected daemon response: {0}")]
    UnexpectedResponse(String),

    /// Systemd unit query failure.
    #[error("systemd query error: {0}")]
    Systemd(String),

    /// Journal log retrieval error.
    #[error("failed to retrieve logs: {0}")]
    Logs(String),

    /// Invalid username format.
    #[error("invalid username '{0}': must follow POSIX naming rules and not exceed 32 characters")]
    InvalidUsername(String),

    /// User does not exist on the system.
    #[error("user '{0}' does not exist on this system")]
    UserNotFound(String),

    /// Execution failure for administrative user provisioning command.
    #[error("failed to execute user provisioning command: {0}")]
    CommandFailed(String),

    /// GDM configuration error.
    #[error("GDM configuration error: {0}")]
    GdmConfig(String),
}

impl From<getrandom::Error> for AdminCliError {
    fn from(err: getrandom::Error) -> Self {
        Self::Random(err)
    }
}
