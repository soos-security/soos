//! Error types for camera streaming and hardware management.

use std::path::PathBuf;
use thiserror::Error;

/// Domain error variants for camera initialization, MMAP streaming, and device handling.
#[derive(Debug, Error)]
pub enum CameraError {
    /// The specified camera device node was not found.
    #[error("Device not found at '{path}': {source}")]
    DeviceNotFound {
        /// Target device path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The camera device is currently opened/busy by another process.
    #[error("Device '{path}' is busy: {source}")]
    DeviceBusy {
        /// Target device path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// An I/O error occurred during device interaction or ioctl.
    #[error("I/O error communicating with camera '{path}': {source}")]
    Io {
        /// Target device path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// Failed to query device capabilities.
    #[error("Failed to query capabilities for '{path}': {reason}")]
    QueryCapabilities {
        /// Target device path.
        path: PathBuf,
        /// Description of failure.
        reason: String,
    },

    /// The device does not support video capture streaming.
    #[error("Device at '{path}' does not support video capture streaming")]
    UnsupportedCapability {
        /// Target device path.
        path: PathBuf,
    },

    /// Failed to configure video format on camera.
    #[error("Failed to set format ({width}x{height}, {format:?}) on '{path}': {reason}")]
    SetFormat {
        /// Target device path.
        path: PathBuf,
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
        /// Requested format.
        format: crate::frame::PixelFormat,
        /// Description of failure.
        reason: String,
    },

    /// Failed to allocate or initialize MMAP stream buffers.
    #[error("Failed to initialize MMAP stream on '{path}': {reason}")]
    StreamCreate {
        /// Target device path.
        path: PathBuf,
        /// Description of failure.
        reason: String,
    },

    /// Failed to dequeue a capture buffer from kernel MMAP queue.
    #[error("Failed to dequeue buffer on '{path}': {reason}")]
    BufferDequeue {
        /// Target device path.
        path: PathBuf,
        /// Description of failure.
        reason: String,
    },

    /// Frame capture timed out or starved.
    #[error("Frame capture timed out or starved")]
    Starved,

    /// Camera manager has been stopped.
    #[error("Camera manager stopped")]
    Stopped,

    /// Simulated error for testing mock camera backends.
    #[error("Simulated hardware error ({code}): {message}")]
    Simulated {
        /// Simulated error code (e.g. 19 for ENODEV, 16 for EBUSY, 5 for EIO).
        code: i32,
        /// Error message.
        message: String,
    },

    /// Device reports no supported capture formats.
    #[error("Camera device reports zero supported video capture formats")]
    NoSupportedFormats,

    /// No supported capture format matched supported formats.
    #[error("No compatible video capture format supported (available: {supported:?})")]
    NoCompatibleFormat {
        /// Formats reported by camera hardware.
        supported: Vec<crate::frame::PixelFormat>,
    },
}

impl CameraError {
    /// Maps a raw OS error code (e.g., from `std::io::Error`) to domain error variants.
    pub fn from_io_error(path: PathBuf, err: std::io::Error) -> Self {
        match err.raw_os_error() {
            Some(libc::ENOENT) | Some(libc::ENODEV) => Self::DeviceNotFound { path, source: err },
            Some(libc::EBUSY) => Self::DeviceBusy { path, source: err },
            _ => Self::Io { path, source: err },
        }
    }
}
