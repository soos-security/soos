//! Contractual tests for the camera status surface (review finding CAM-07, GitHub #155).
//!
//! Contract:
//! - `CameraError::kind()` classifies every failure into a distinct, user-presentable
//!   `CameraErrorKind` (missing device, busy device, permission denied, unsupported device,
//!   starvation, generic I/O), including `EACCES` hidden inside the generic `Io` variant.
//! - `CameraStatusCell::record_error` counts consecutive failures of the same kind and restarts
//!   the count when the kind changes or the source recovered.
//! - `CameraManager::status()` has a default implementation derived from `is_ready()`, so
//!   existing implementors keep compiling, and `MockCameraManager` reports injected faults.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use soos_camera_v4l::{
    CameraError, CameraErrorKind, CameraManager, CameraStatus, CameraStatusCell, Frame,
    MockCameraManager, PixelFormat,
};

fn dev() -> PathBuf {
    PathBuf::from("/dev/video0")
}

#[test]
fn test_camera_error_kind_classifies_os_errors() {
    let busy = CameraError::from_io_error(dev(), io::Error::from_raw_os_error(libc::EBUSY));
    assert_eq!(busy.kind(), CameraErrorKind::DeviceBusy);

    let missing = CameraError::from_io_error(dev(), io::Error::from_raw_os_error(libc::ENOENT));
    assert_eq!(missing.kind(), CameraErrorKind::DeviceNotFound);

    let gone = CameraError::from_io_error(dev(), io::Error::from_raw_os_error(libc::ENODEV));
    assert_eq!(gone.kind(), CameraErrorKind::DeviceNotFound);

    let denied = CameraError::from_io_error(dev(), io::Error::from_raw_os_error(libc::EACCES));
    assert_eq!(denied.kind(), CameraErrorKind::PermissionDenied);

    let eperm = CameraError::from_io_error(dev(), io::Error::from_raw_os_error(libc::EPERM));
    assert_eq!(eperm.kind(), CameraErrorKind::PermissionDenied);

    let eio = CameraError::from_io_error(dev(), io::Error::from_raw_os_error(libc::EIO));
    assert_eq!(eio.kind(), CameraErrorKind::Io);
}

#[test]
fn test_camera_error_kind_classifies_domain_variants() {
    assert_eq!(CameraError::Starved.kind(), CameraErrorKind::Starved);
    assert_eq!(
        CameraError::UnsupportedCapability { path: dev() }.kind(),
        CameraErrorKind::UnsupportedDevice
    );
    assert_eq!(
        CameraError::NoSupportedFormats.kind(),
        CameraErrorKind::UnsupportedDevice
    );
    assert_eq!(
        CameraError::NoCompatibleFormat {
            supported: vec![PixelFormat::Grey]
        }
        .kind(),
        CameraErrorKind::UnsupportedDevice
    );
    assert_eq!(
        CameraError::BufferDequeue {
            path: dev(),
            reason: "x".into()
        }
        .kind(),
        CameraErrorKind::Io
    );
}

#[test]
fn test_camera_error_kind_classifies_simulated_codes() {
    let sim = |code: i32| CameraError::Simulated {
        code,
        message: String::new(),
    };
    assert_eq!(sim(libc::EBUSY).kind(), CameraErrorKind::DeviceBusy);
    assert_eq!(sim(libc::ENODEV).kind(), CameraErrorKind::DeviceNotFound);
    assert_eq!(sim(libc::ENOENT).kind(), CameraErrorKind::DeviceNotFound);
    assert_eq!(sim(libc::EACCES).kind(), CameraErrorKind::PermissionDenied);
    assert_eq!(sim(libc::EIO).kind(), CameraErrorKind::Io);
}

#[test]
fn test_status_cell_counts_consecutive_failures_per_kind() {
    let cell = CameraStatusCell::new();
    assert_eq!(cell.get(), CameraStatus::Starting);

    cell.record_error(CameraErrorKind::DeviceBusy);
    cell.record_error(CameraErrorKind::DeviceBusy);
    assert_eq!(
        cell.get(),
        CameraStatus::Error {
            kind: CameraErrorKind::DeviceBusy,
            failures: 2
        }
    );

    cell.record_error(CameraErrorKind::PermissionDenied);
    assert_eq!(
        cell.get(),
        CameraStatus::Error {
            kind: CameraErrorKind::PermissionDenied,
            failures: 1
        }
    );

    cell.set(CameraStatus::Ready);
    cell.record_error(CameraErrorKind::PermissionDenied);
    assert_eq!(
        cell.get(),
        CameraStatus::Error {
            kind: CameraErrorKind::PermissionDenied,
            failures: 1
        }
    );
}

#[test]
fn test_status_cell_failure_count_saturates() {
    let cell = CameraStatusCell::new();
    cell.set(CameraStatus::Error {
        kind: CameraErrorKind::Io,
        failures: u32::MAX,
    });
    cell.record_error(CameraErrorKind::Io);
    assert_eq!(
        cell.get(),
        CameraStatus::Error {
            kind: CameraErrorKind::Io,
            failures: u32::MAX
        }
    );
}

struct MinimalCamera {
    ready: bool,
}

impl CameraManager for MinimalCamera {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        None
    }
    fn is_ready(&self) -> bool {
        self.ready
    }
    fn notify_activity(&self) {}
    fn stop(&self) {}
}

#[test]
fn test_default_status_is_derived_from_readiness() {
    assert_eq!(MinimalCamera { ready: true }.status(), CameraStatus::Ready);
    assert_eq!(
        MinimalCamera { ready: false }.status(),
        CameraStatus::Starting
    );
}

#[test]
fn test_mock_camera_status_reports_injected_error_and_recovery() {
    let cam = MockCameraManager::new_default();
    cam.set_error(Some(CameraError::Simulated {
        code: libc::EBUSY,
        message: "busy".into(),
    }));
    match cam.status() {
        CameraStatus::Error { kind, .. } => assert_eq!(kind, CameraErrorKind::DeviceBusy),
        other => panic!("expected DeviceBusy error status, got {other:?}"),
    }

    cam.set_error(None);
    cam.push_frame(Frame::new(vec![0u8; 12], 2, 2, 1, PixelFormat::Rgb24, 1));
    let deadline = Instant::now() + Duration::from_secs(2);
    while cam.status() != CameraStatus::Ready && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(cam.status(), CameraStatus::Ready);
    cam.stop();
}

#[test]
fn test_mock_camera_status_reports_starvation() {
    let cam = MockCameraManager::new_default();
    cam.set_starved(true);
    match cam.status() {
        CameraStatus::Error { kind, .. } => assert_eq!(kind, CameraErrorKind::Starved),
        other => panic!("expected Starved error status, got {other:?}"),
    }
    cam.stop();
}

#[test]
fn test_error_kind_descriptions_are_distinct_and_english() {
    let kinds = CameraErrorKind::ALL;
    let mut seen = std::collections::HashSet::new();
    for kind in kinds {
        let text = kind.to_string();
        assert!(!text.is_empty());
        assert!(text.is_ascii(), "description must be plain English: {text}");
        assert!(seen.insert(text), "duplicate description for {kind:?}");
    }
}
