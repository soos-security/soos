//! User-visible camera status banner (review finding CAM-07).
//!
//! Maps a [`CameraStatus`] to a distinct title and an actionable hint, so the window no longer
//! shows the same "Connecting to camera..." spinner for a busy device, a permission problem, a
//! missing device and a stopped daemon. Texts contain no frame data or biometric material.

#![forbid(unsafe_code)]

use eframe::egui;
use soos_camera_v4l::{CameraErrorKind, CameraStatus};

use crate::{motion, theme, widgets};

/// Visual severity of a [`StatusBanner`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// Draws `banner` as a centered brand status card (pending star for in-progress states).
///
/// Truth first (`Docs/UI_MOTION.md` G7): a new state's text is drawn on the frame it arrives;
/// an Info state fades in over `T_SHORT`, its title colour cross-fading from the previous
/// severity's colour, while a Warning or Error is drawn at full opacity in its own colour at
/// once. The previous text is never kept on screen. Reduced motion draws every state at rest.
pub fn render_status_banner(ui: &mut egui::Ui, banner: &StatusBanner) {
    let color = severity_color(banner.severity);
    let ctx = ui.ctx().clone();
    let id = egui::Id::new("soos_camera_status_banner");
    let key = egui::Id::new((banner.severity, banner.title.as_str())).value();
    let previous = ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_insert_with(id.with("state"), || StatusMotion {
            key,
            severity: banner.severity,
            previous: banner.severity,
        });
        if s.key != key {
            s.key = key;
            s.previous = s.severity;
            s.severity = banner.severity;
        }
        s.previous
    });
    let fade = motion::fade_in_on_change(&ctx, id.with("text"), key, theme::T_SHORT);
    // A problem is never faded in: Warning and Error are drawn at full opacity, in their own
    // colour, on the frame they arrive (security truth first, `Docs/UI_MOTION.md` G7).
    let fade = if banner.severity == BannerSeverity::Info {
        fade
    } else {
        1.0
    };
    let title_color = motion::mix_color(severity_color(previous), color, fade);
    widgets::status_card(ui, |ui| {
        motion::with_opacity(ui, fade, |ui| {
            if banner.severity == BannerSeverity::Info {
                let (star, _) =
                    ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                widgets::pending_star(ui, star, theme::PINK);
                ui.add_space(4.0);
            }
            // Same faux-bold title as every other card (the default font has one weight).
            widgets::card_title_colored(ui, banner.title.as_str(), title_color);
            ui.add_space(-10.0);
            ui.label(
                egui::RichText::new(banner.detail.as_str())
                    .size(theme::F_BODY)
                    .color(theme::INK_MUTED),
            );
        });
    });
}

/// Last state shown by [`render_status_banner`] (for the title colour cross-fade).
#[derive(Debug, Clone, Copy)]
struct StatusMotion {
    key: u64,
    severity: BannerSeverity,
    previous: BannerSeverity,
}

/// Title colour of a severity.
fn severity_color(severity: BannerSeverity) -> egui::Color32 {
    match severity {
        BannerSeverity::Info => theme::INK,
        BannerSeverity::Warning => theme::WARN_TEXT,
        BannerSeverity::Error => theme::DANGER,
    }
}
