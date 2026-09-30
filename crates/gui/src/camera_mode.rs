//! Camera ownership decision for `soos-gui` (GitHub #150, review finding CAM-02).
//!
//! The root daemon is the exclusive owner of `/dev/video*`. The GUI therefore:
//! - streams through the daemon IPC preview when the daemon accepts it;
//! - opens the camera directly (V4L2) **only** when the daemon is provably not running
//!   (socket missing or refusing connections, and `soos-daemon.service` not active);
//! - otherwise stays in a [`CameraMode::Blocked`] state with an actionable message, never
//!   grabbing the device (a direct open while the daemon runs made both processes fight for the
//!   node with `EBUSY`, and face unlock silently fell back to the password).

use crate::ipc_camera::IpcPreviewError;
use nix::errno::Errno;
use soos_camera_v4l::{CameraManager, Frame};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;

/// Outcome of a raw connection attempt to the daemon socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonSocketProbe {
    /// The socket accepted the connection: the daemon is running.
    Reachable,
    /// `EACCES`/`EPERM`: the daemon runtime directory exists but this user is not in `soos`.
    PermissionDenied,
    /// `ENOENT`/`ECONNREFUSED`: no daemon is listening.
    NotRunning,
    /// Any other failure (unexpected errno, timeout).
    Other,
}

/// Classifies a `connect(2)` failure on the daemon socket.
pub fn classify_socket_error(err: &std::io::Error) -> DaemonSocketProbe {
    let Some(code) = err.raw_os_error() else {
        return DaemonSocketProbe::Other;
    };
    match Errno::from_raw(code) {
        Errno::EACCES | Errno::EPERM => DaemonSocketProbe::PermissionDenied,
        Errno::ENOENT | Errno::ECONNREFUSED | Errno::ENOTDIR => DaemonSocketProbe::NotRunning,
        _ => DaemonSocketProbe::Other,
    }
}

/// Connects once to the daemon socket and classifies the result.
///
/// Unlike `Path::exists()`, a connection attempt distinguishes a missing socket from a
/// `/run/soos` directory this user may not traverse (mode `0750 root:soos`).
pub fn probe_daemon_socket<P: AsRef<Path>>(socket_path: P) -> DaemonSocketProbe {
    match UnixStream::connect(socket_path) {
        Ok(_) => DaemonSocketProbe::Reachable,
        Err(err) => classify_socket_error(&err),
    }
}

/// Why the GUI refuses to open the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraBlockReason {
    /// The daemon socket exists but this user cannot reach it (not in the `soos` group).
    PermissionDenied,
    /// The daemon is reachable (it owns the camera) but its preview is not usable for this user.
    PreviewUnavailable(IpcPreviewError),
    /// `soos-daemon.service` is active but its socket does not accept connections.
    DaemonUnreachable,
    /// The direct camera was released for a daemon Resume that is still in progress.
    HandingOver,
    /// The daemon is not running but the direct V4L2 capture could not be started.
    DirectOpenFailed,
}

impl CameraBlockReason {
    /// Returns whether this state is expected to clear by itself, so the camera-source
    /// supervisor re-probes it (a bounded number of times) instead of waiting for a daemon
    /// state change.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::DaemonUnreachable | Self::HandingOver | Self::DirectOpenFailed => true,
            Self::PreviewUnavailable(err) => matches!(
                err,
                IpcPreviewError::RateLimited | IpcPreviewError::Unavailable | IpcPreviewError::Io
            ),
            Self::PermissionDenied => false,
        }
    }

    /// Returns an actionable, English, user-facing explanation (no secrets, no frames).
    pub fn user_message(&self) -> String {
        let tail = "soos-gui will not open the camera directly while soos-daemon owns it.";
        match self {
            Self::PermissionDenied => format!(
                "soos-daemon owns the camera, but this user cannot reach /run/soos/daemon.sock. \
                 Add the user to the 'soos' group (sudo soos-admin add-user <username>), then \
                 log out and log back in. {tail}"
            ),
            Self::PreviewUnavailable(IpcPreviewError::Unauthorized) => format!(
                "soos-daemon owns the camera and refused the live preview for this user. Set \
                 `[preview] enabled = true` and add this UID to `allowed_uids` in \
                 /etc/soos/daemon.toml, then restart soos-daemon and soos-gui. {tail}"
            ),
            Self::PreviewUnavailable(IpcPreviewError::Protocol) => format!(
                "soos-daemon owns the camera but its live preview replies are malformed. Check \
                 that soos-gui and soos-daemon versions match. {tail}"
            ),
            Self::PreviewUnavailable(err) => format!(
                "soos-daemon owns the camera but its live preview is unavailable ({err}). \
                 soos-gui retries automatically for a limited time; pause and resume the daemon \
                 if this persists. {tail}"
            ),
            Self::HandingOver => format!(
                "The camera was released for soos-daemon, which is starting. The live preview \
                 switches to the daemon as soon as it accepts connections; soos-gui retries \
                 automatically. {tail}"
            ),
            Self::DirectOpenFailed => "soos-daemon is not running, but soos-gui could not \
                 start the direct camera capture. soos-gui retries automatically for a limited \
                 time; restart soos-gui if this persists."
                .to_string(),
            Self::DaemonUnreachable => format!(
                "soos-daemon is active but /run/soos/daemon.sock is not accepting connections. \
                 soos-gui retries automatically for a limited time; check `systemctl status \
                 soos-daemon`, or pause the daemon to let soos-gui use the camera directly. \
                 {tail}"
            ),
        }
    }
}

/// Camera access strategy (re-decided at runtime by `camera_source::CameraSourcePlanner`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    /// Stream preview frames through the daemon IPC socket.
    DaemonIpc,
    /// Open the V4L2 device directly (daemon provably not running).
    DirectV4l,
    /// Do not touch the camera; show the reason to the user.
    Blocked(CameraBlockReason),
}

/// Decides the camera mode.
///
/// * `socket` — result of [`probe_daemon_socket`].
/// * `preview` — result of the preview authorization round-trip, performed only when the socket
///   is reachable (`None` when it was not attempted).
/// * `daemon_active` — whether systemd reports `soos-daemon.service` as active.
pub fn decide_camera_mode(
    socket: DaemonSocketProbe,
    preview: Option<Result<(), IpcPreviewError>>,
    daemon_active: bool,
) -> CameraMode {
    match socket {
        DaemonSocketProbe::Reachable => match preview {
            Some(Ok(())) => CameraMode::DaemonIpc,
            Some(Err(err)) => CameraMode::Blocked(CameraBlockReason::PreviewUnavailable(err)),
            None => CameraMode::Blocked(CameraBlockReason::PreviewUnavailable(IpcPreviewError::Io)),
        },
        DaemonSocketProbe::PermissionDenied => {
            CameraMode::Blocked(CameraBlockReason::PermissionDenied)
        }
        DaemonSocketProbe::NotRunning | DaemonSocketProbe::Other => {
            if daemon_active {
                CameraMode::Blocked(CameraBlockReason::DaemonUnreachable)
            } else {
                CameraMode::DirectV4l
            }
        }
    }
}

/// Camera manager used in [`CameraMode::Blocked`]: never opens a device, never serves frames.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableCameraManager;

impl CameraManager for UnavailableCameraManager {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        None
    }

    fn is_ready(&self) -> bool {
        false
    }

    fn notify_activity(&self) {}

    fn stop(&self) {}
}
