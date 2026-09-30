//! Contract tests: the camera banner only promises an automatic retry when one happens
//! (candid review finding 5). The IPC preview worker stops for good on an authorization
//! refusal, so `SourceUnauthorized` must not say "retrying automatically".

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite utilizes direct assertions"
)]

use soos_camera_v4l::{CameraErrorKind, CameraStatus};
use soos_gui::camera_status::{camera_status_banner, error_is_retried};

#[test]
fn test_unauthorized_preview_banner_does_not_claim_retrying() {
    let banner = camera_status_banner(&CameraStatus::Error {
        kind: CameraErrorKind::SourceUnauthorized,
        failures: 1,
    });
    assert!(
        !banner.detail.contains("retrying automatically"),
        "detail: {}",
        banner.detail
    );
    assert!(
        banner.detail.contains("stopped"),
        "detail: {}",
        banner.detail
    );
    assert!(banner.detail.contains('1'), "detail: {}", banner.detail);
}

#[test]
fn test_retried_error_kinds_keep_the_retry_notice() {
    for kind in CameraErrorKind::ALL {
        let banner = camera_status_banner(&CameraStatus::Error { kind, failures: 3 });
        assert_eq!(
            banner.detail.contains("retrying automatically"),
            error_is_retried(kind),
            "{kind:?}: {}",
            banner.detail
        );
    }
    assert!(!error_is_retried(CameraErrorKind::SourceUnauthorized));
    for kind in [
        CameraErrorKind::DeviceBusy,
        CameraErrorKind::DeviceNotFound,
        CameraErrorKind::SourceUnreachable,
        CameraErrorKind::SourceRateLimited,
    ] {
        assert!(error_is_retried(kind), "{kind:?}");
    }
}
