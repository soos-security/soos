//! User-presentable camera status and error classification (review finding CAM-07).
//!
//! `CameraError` carries full diagnostic detail for logs; `CameraErrorKind` is the stable,
//! detail-free classification that front-ends (the GUI status banner) present to users so that
//! a busy device, a permission problem, a missing device and an unreachable frame source are
//! distinguishable. No frame data or biometric material is ever stored here.

use std::fmt;
use std::sync::Mutex;

use crate::error::CameraError;

/// Stable classification of a camera or frame-source failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraErrorKind {
    /// The device node does not exist or disappeared (`ENOENT`, `ENODEV`).
    DeviceNotFound,
    /// Another process holds the device (`EBUSY`).
    DeviceBusy,
    /// The caller lacks permission to open the device (`EACCES`, `EPERM`).
    PermissionDenied,
    /// The device cannot stream video in any supported format.
    UnsupportedDevice,
    /// The device stopped delivering frames.
    Starved,
    /// Any other device I/O failure.
    Io,
    /// A remote frame source (the `soos-daemon` preview proxy) is not reachable.
    SourceUnreachable,
    /// The remote frame source refused to serve this user.
    SourceUnauthorized,
    /// The remote frame source throttled this user.
    SourceRateLimited,
    /// The remote frame source is reachable but has no camera available.
    SourceUnavailable,
    /// The remote frame source sent a malformed reply.
    SourceProtocol,
}

impl CameraErrorKind {
    /// Every variant, for exhaustive presentation tests.
    pub const ALL: [Self; 11] = [
        Self::DeviceNotFound,
        Self::DeviceBusy,
        Self::PermissionDenied,
        Self::UnsupportedDevice,
        Self::Starved,
        Self::Io,
        Self::SourceUnreachable,
        Self::SourceUnauthorized,
        Self::SourceRateLimited,
        Self::SourceUnavailable,
        Self::SourceProtocol,
    ];

    /// Classifies a raw OS error number.
    pub fn from_os_code(code: i32) -> Self {
        match code {
            libc::ENOENT | libc::ENODEV | libc::ENXIO => Self::DeviceNotFound,
            libc::EBUSY => Self::DeviceBusy,
            libc::EACCES | libc::EPERM => Self::PermissionDenied,
            _ => Self::Io,
        }
    }
}

impl fmt::Display for CameraErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::DeviceNotFound => "camera device not found",
            Self::DeviceBusy => "camera device busy",
            Self::PermissionDenied => "camera permission denied",
            Self::UnsupportedDevice => "camera device unsupported",
            Self::Starved => "camera stopped delivering frames",
            Self::Io => "camera I/O error",
            Self::SourceUnreachable => "frame source unreachable",
            Self::SourceUnauthorized => "frame source refused access",
            Self::SourceRateLimited => "frame source rate-limited",
            Self::SourceUnavailable => "frame source has no camera available",
            Self::SourceProtocol => "frame source protocol error",
        };
        f.write_str(text)
    }
}

impl CameraError {
    /// Returns the stable, user-presentable classification of this error.
    pub fn kind(&self) -> CameraErrorKind {
        match self {
            Self::DeviceNotFound { .. } => CameraErrorKind::DeviceNotFound,
            Self::DeviceBusy { .. } => CameraErrorKind::DeviceBusy,
            Self::Io { source, .. } => source
                .raw_os_error()
                .map_or(CameraErrorKind::Io, CameraErrorKind::from_os_code),
            Self::QueryCapabilities { .. }
            | Self::UnsupportedCapability { .. }
            | Self::VirtualDevice { .. }
            | Self::SetFormat { .. }
            | Self::NoSupportedFormats
            | Self::NoCompatibleFormat { .. } => CameraErrorKind::UnsupportedDevice,
            Self::StreamCreate { .. }
            | Self::StreamTeardown { .. }
            | Self::BufferDequeue { .. }
            | Self::Stopped => CameraErrorKind::Io,
            Self::Starved => CameraErrorKind::Starved,
            Self::Simulated { code, .. } => CameraErrorKind::from_os_code(*code),
        }
    }
}

/// Observable lifecycle state of a camera manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraStatus {
    /// Opening the device or waiting for the first stabilized frame.
    Starting,
    /// Streaming stabilized frames.
    Ready,
    /// Idle auto-standby: the device is released until activity is signaled.
    Suspended,
    /// The manager was stopped and will not deliver frames again.
    Stopped,
    /// The last attempt failed; `failures` counts consecutive failures of this kind.
    Error {
        /// Classification of the failure.
        kind: CameraErrorKind,
        /// Consecutive failed attempts with this kind (saturating).
        failures: u32,
    },
}

/// Thread-safe holder of a [`CameraStatus`], shared between a capture worker and its manager.
#[derive(Debug)]
pub struct CameraStatusCell {
    inner: Mutex<CameraStatus>,
}

impl Default for CameraStatusCell {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraStatusCell {
    /// Creates a cell in the [`CameraStatus::Starting`] state.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(CameraStatus::Starting),
        }
    }

    /// Returns the current status.
    pub fn get(&self) -> CameraStatus {
        *self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Replaces the current status.
    pub fn set(&self, status: CameraStatus) {
        *self.inner.lock().unwrap_or_else(|e| e.into_inner()) = status;
    }

    /// Records a failed attempt, incrementing the consecutive-failure count when the previous
    /// status was an error of the same kind and restarting it at 1 otherwise.
    pub fn record_error(&self, kind: CameraErrorKind) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let failures = match *guard {
            CameraStatus::Error {
                kind: previous,
                failures,
            } if previous == kind => failures.saturating_add(1),
            _ => 1,
        };
        *guard = CameraStatus::Error { kind, failures };
    }
}
