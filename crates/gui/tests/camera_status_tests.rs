//! Contractual tests for user-visible camera error states and GUI logging
//! (review finding CAM-07, GitHub #155).
//!
//! Contract:
//! - Every `CameraStatus` maps to a `StatusBanner` whose title is distinct per error kind, so a
//!   busy device, a permission problem, a missing device and an unreachable daemon are
//!   distinguishable on screen instead of one generic spinner.
//! - `IpcCameraManager::status()` surfaces transport and authorization failures.
//! - The GUI binary installs a stderr tracing subscriber honoring `RUST_LOG` with a safe
//!   fallback for invalid directives.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::collections::HashSet;
use std::time::{Duration, Instant};

use eframe::egui;
use soos_camera_v4l::{
    CameraError, CameraErrorKind, CameraManager, CameraStatus, MockCameraManager,
};
use soos_gui::camera_status::{camera_status_banner, render_status_banner, BannerSeverity};
use soos_gui::IpcCameraManager;

#[test]
fn test_status_banner_shows_camera_error() {
    let cam = MockCameraManager::new_default();
    cam.set_error(Some(CameraError::Simulated {
        code: libc_ebusy(),
        message: "busy".into(),
    }));
    let banner = camera_status_banner(&cam.status());
    assert_eq!(banner.severity, BannerSeverity::Error);
    assert!(
        banner.title.to_lowercase().contains("busy"),
        "EBUSY must be presented as a busy camera, got '{}'",
        banner.title
    );
    cam.stop();
}

/// `EBUSY` on Linux; kept local so the GUI test crate needs no libc dependency.
fn libc_ebusy() -> i32 {
    16
}

#[test]
fn test_status_banner_titles_are_distinct_per_error_kind() {
    let mut titles = HashSet::new();
    for kind in CameraErrorKind::ALL {
        let banner = camera_status_banner(&CameraStatus::Error { kind, failures: 1 });
        assert_eq!(banner.severity, BannerSeverity::Error);
        assert!(banner.title.is_ascii() && banner.detail.is_ascii());
        assert!(
            titles.insert(banner.title.clone()),
            "duplicate banner title for {kind:?}: {}",
            banner.title
        );
    }
}

#[test]
fn test_status_banner_distinguishes_non_error_states() {
    let starting = camera_status_banner(&CameraStatus::Starting);
    let suspended = camera_status_banner(&CameraStatus::Suspended);
    let stopped = camera_status_banner(&CameraStatus::Stopped);
    let ready = camera_status_banner(&CameraStatus::Ready);
    assert_eq!(starting.severity, BannerSeverity::Info);
    assert_eq!(ready.severity, BannerSeverity::Info);
    let titles: HashSet<_> = [starting.title, suspended.title, stopped.title, ready.title]
        .into_iter()
        .collect();
    assert_eq!(titles.len(), 4);
}

#[test]
fn test_status_banner_reports_retry_count() {
    let banner = camera_status_banner(&CameraStatus::Error {
        kind: CameraErrorKind::PermissionDenied,
        failures: 7,
    });
    assert!(banner.detail.contains('7'), "detail: {}", banner.detail);
    assert!(banner.title.to_lowercase().contains("permission"));
}

#[test]
fn test_status_banner_unreachable_daemon_mentions_daemon() {
    let banner = camera_status_banner(&CameraStatus::Error {
        kind: CameraErrorKind::SourceUnreachable,
        failures: 1,
    });
    assert!(banner.title.contains("soos-daemon") || banner.detail.contains("soos-daemon"));
}

#[test]
fn test_render_status_banner_draws_in_egui_context() {
    let ctx = egui::Context::default();
    let banner = camera_status_banner(&CameraStatus::Error {
        kind: CameraErrorKind::DeviceNotFound,
        failures: 2,
    });
    let _ = ctx.run_ui(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            render_status_banner(ui, &banner);
        });
    });
}

#[test]
fn test_ipc_camera_status_reports_unreachable_daemon() {
    let path = std::env::temp_dir().join(format!(
        "soos-gui-status-absent-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let cam = IpcCameraManager::spawn(&path);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let CameraStatus::Error { kind, .. } = cam.status() {
            assert_eq!(kind, CameraErrorKind::SourceUnreachable);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "IPC manager never reported the unreachable daemon"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    cam.stop();
}

#[test]
fn test_log_filter_defaults_and_rejects_invalid_directive() {
    use soos_gui::logging::{build_env_filter, DEFAULT_LOG_DIRECTIVE};
    assert_eq!(DEFAULT_LOG_DIRECTIVE, "info");
    assert_eq!(build_env_filter(None).to_string(), "info");
    assert_eq!(build_env_filter(Some("")).to_string(), "info");
    assert_eq!(
        build_env_filter(Some("soos_gui=debug")).to_string(),
        "soos_gui=debug"
    );
    // An invalid directive must fall back instead of panicking or silencing logs.
    assert_eq!(build_env_filter(Some("=[[[")).to_string(), "info");
}

#[test]
fn test_logging_init_is_idempotent() {
    assert!(
        soos_gui::logging::init(),
        "first init installs the subscriber"
    );
    assert!(!soos_gui::logging::init(), "second init must be a no-op");
}

#[test]
fn test_gui_main_installs_tracing_subscriber_first() {
    let src = include_str!("../src/main.rs");
    let init = src
        .find("soos_gui::logging::init()")
        .expect("main.rs must install the tracing subscriber");
    let parse = src.find("GuiArgs::parse()").expect("main parses args");
    let first_log = src.find("tracing::").expect("main logs");
    assert!(
        init < first_log,
        "subscriber must be installed before the first log"
    );
    assert!(
        init < parse + 200,
        "subscriber must be installed at startup"
    );
}
