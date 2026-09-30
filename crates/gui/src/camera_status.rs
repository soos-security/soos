//! User-visible camera status banner (review finding CAM-07).
//!
//! Maps a [`CameraStatus`] to a distinct title and an actionable hint, so the window no longer
//! shows the same "Connecting to camera..." spinner for a busy device, a permission problem, a
//! missing device and a stopped daemon. Texts contain no frame data or biometric material.

#![forbid(unsafe_code)]

use eframe::egui::{self, Color32};
use soos_camera_v4l::{CameraErrorKind, CameraStatus};

/// Visual severity of a [`StatusBanner`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerSeverity {
    /// Normal progress or healthy state.
    Info,
    /// Degraded but expected state (standby, stopped).
    Warning,
    /// A failure the user must act on.
    Error,
}

/// Presentation of a camera status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBanner {
    /// Visual severity.
    pub severity: BannerSeverity,
    /// Short, state-specific headline.
    pub title: String,
    /// Actionable explanation.
    pub detail: String,
}

fn error_text(kind: CameraErrorKind) -> (&'static str, &'static str) {
    match kind {
        CameraErrorKind::DeviceNotFound => (
            "Camera not found",
            "No video device exists at the configured path. Plug the camera in or set \
             `camera_device` in /etc/soos/daemon.toml.",
        ),
        CameraErrorKind::DeviceBusy => (
            "Camera is busy",
            "Another process (often soos-daemon) holds the device. Pause the daemon or \
             enable the daemon preview proxy.",
        ),
        CameraErrorKind::PermissionDenied => (
            "Camera permission denied",
            "This user cannot open the video device. Add it to the `video` group or use the \
             soos-daemon preview proxy.",
        ),
        CameraErrorKind::UnsupportedDevice => (
            "Camera format unsupported",
            "The device offers no capture format soos can decode (RGB24, YUYV, NV12, MJPEG, \
             GREY).",
        ),
        CameraErrorKind::Starved => (
            "Camera stopped sending frames",
            "The device is open but delivers no frames. Check the cable or privacy shutter.",
        ),
        CameraErrorKind::Io => (
            "Camera I/O error",
            "The video device reported an I/O failure. Run with RUST_LOG=debug for details.",
        ),
        CameraErrorKind::SourceUnreachable => (
            "Daemon preview unreachable",
            "Cannot connect to soos-daemon at /run/soos/daemon.sock. The daemon may be paused \
             or stopped.",
        ),
        CameraErrorKind::SourceUnauthorized => (
            "Daemon preview not authorized",
            "soos-daemon refuses preview frames for this user. Enable `[preview]` with \
             `allowed_uids` in /etc/soos/daemon.toml.",
        ),
        CameraErrorKind::SourceRateLimited => (
            "Daemon preview rate-limited",
            "soos-daemon is throttling preview requests; frames will resume shortly.",
        ),
        CameraErrorKind::SourceUnavailable => (
            "Daemon camera unavailable",
            "soos-daemon is running but cannot serve camera frames right now.",
        ),
        CameraErrorKind::SourceProtocol => (
            "Daemon preview protocol error",
            "soos-daemon sent a malformed preview reply. Check that GUI and daemon versions \
             match.",
        ),
    }
}

/// Returns whether the camera source keeps retrying after an error of `kind`.
///
/// The V4L2 supervisor retries every failure with bounded backoff and the IPC preview worker
/// reconnects after transport, protocol, rate-limit and availability failures, but it stops
/// for good when the daemon refuses the preview to this user (`SourceUnauthorized`).
pub fn error_is_retried(kind: CameraErrorKind) -> bool {
    !matches!(kind, CameraErrorKind::SourceUnauthorized)
}

/// Builds the banner describing `status`.
pub fn camera_status_banner(status: &CameraStatus) -> StatusBanner {
    let (severity, title, detail) = match *status {
        CameraStatus::Starting => (
            BannerSeverity::Info,
            "Connecting to camera".to_string(),
            "Opening the video source and initializing models...".to_string(),
        ),
        CameraStatus::Ready => (
            BannerSeverity::Info,
            "Camera ready".to_string(),
            "Waiting for the first analyzed frame...".to_string(),
        ),
        CameraStatus::Suspended => (
            BannerSeverity::Warning,
            "Camera in standby".to_string(),
            "The device was released after inactivity and will resume on demand.".to_string(),
        ),
        CameraStatus::Stopped => (
            BannerSeverity::Warning,
            "Camera stopped".to_string(),
            "The camera source was shut down. Restart the application to reconnect.".to_string(),
        ),
        CameraStatus::Error { kind, failures } => {
            let (title, hint) = error_text(kind);
            let retry = if error_is_retried(kind) {
                "retrying automatically"
            } else {
                "the preview stopped and is not retried until the daemon state changes"
            };
            (
                BannerSeverity::Error,
                title.to_string(),
                format!("{hint} (failed attempts: {failures}, {retry})"),
            )
        }
    };
    StatusBanner {
        severity,
        title,
        detail,
    }
}

/// Draws `banner` as a centered status block (spinner for in-progress states).
pub fn render_status_banner(ui: &mut egui::Ui, banner: &StatusBanner) {
    let color = match banner.severity {
        BannerSeverity::Info => ui.visuals().text_color(),
        BannerSeverity::Warning => Color32::YELLOW,
        BannerSeverity::Error => Color32::from_rgb(0xFF, 0x60, 0x60),
    };
    ui.vertical_centered(|ui| {
        if banner.severity == BannerSeverity::Info {
            ui.spinner();
        }
        ui.heading(egui::RichText::new(banner.title.as_str()).color(color));
        ui.label(banner.detail.as_str());
    });
}
