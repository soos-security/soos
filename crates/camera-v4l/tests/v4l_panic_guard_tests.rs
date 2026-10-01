//! `v4l` 0.14 panic guard (GitHub #287, matrix row CVF4).
//!
//! `v4l` 0.14 unwraps `str::from_utf8` on the `VIDIOC_QUERYCAP` strings (and on the
//! `VIDIOC_ENUM_FMT` description), so a device reporting a non-UTF-8 card name panics inside
//! the crate. Every metadata call of the daemon probe, the diagnostics probe and the capture
//! open path goes through `guard_v4l_call`, which turns such a panic into an `io::Error`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use soos_camera_v4l::v4l_guard::{guard_v4l_call, V4L_PANIC_MESSAGE};
use soos_camera_v4l::{CameraError, CameraErrorKind};
use std::path::PathBuf;

/// A raw `v4l2_capability` as the kernel would return it, with `card` bytes chosen by the test.
fn raw_capability(card: &[u8]) -> v4l::v4l_sys::v4l2_capability {
    let mut raw = v4l::v4l_sys::v4l2_capability {
        driver: [0; 16],
        card: [0; 32],
        bus_info: [0; 32],
        version: 0x0006_0800,
        capabilities: 0x8420_0001,
        device_caps: 0x0420_0001,
        reserved: [0; 3],
    };
    raw.driver[..8].copy_from_slice(b"uvcvideo");
    raw.bus_info[..18].copy_from_slice(b"usb-0000:00:14.0-8");
    raw.card[..card.len()].copy_from_slice(card);
    raw
}

#[test]
fn test_cvf_guard_turns_an_injected_panic_into_an_error() {
    let ok = guard_v4l_call(|| 41_u32 + 1).unwrap();
    assert_eq!(ok, 42, "a call that does not panic is passed through");

    let err = guard_v4l_call(|| -> u32 { panic!("simulated v4l fault") }).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.raw_os_error(), None);
    assert_eq!(err.to_string(), V4L_PANIC_MESSAGE);
    assert!(
        !err.to_string().contains("simulated"),
        "the panic payload is not propagated (it may carry device-chosen bytes)"
    );

    // An io::Result returned by the call is kept as-is (no double wrapping).
    let inner: std::io::Result<std::io::Result<u8>> =
        guard_v4l_call(|| Err(std::io::Error::from_raw_os_error(libc::EBUSY)));
    assert_eq!(
        inner.unwrap().unwrap_err().raw_os_error(),
        Some(libc::EBUSY)
    );
}

#[test]
fn test_cvf_guard_catches_the_real_v4l_non_utf8_capability_panic() {
    // Proof that the upstream panic exists: the conversion used by `query_caps` unwraps.
    let valid = guard_v4l_call(|| v4l::capability::Capabilities::from(raw_capability(b"Cam")))
        .expect("valid UTF-8 converts");
    assert_eq!(valid.card, "Cam");
    assert_eq!(valid.driver, "uvcvideo");

    let err = guard_v4l_call(|| {
        v4l::capability::Capabilities::from(raw_capability(b"Integrated \xff\xfe Camera"))
    })
    .unwrap_err();
    assert_eq!(err.to_string(), V4L_PANIC_MESSAGE);

    // Mapped by the capture open path, the guarded failure is an unsupported device (the
    // supervisor backs off and retries), never a supervisor panic that marks the camera Dead.
    let mapped = CameraError::QueryCapabilities {
        path: PathBuf::from("/dev/video0"),
        reason: err.to_string(),
    };
    assert_eq!(mapped.kind(), CameraErrorKind::UnsupportedDevice);
}
