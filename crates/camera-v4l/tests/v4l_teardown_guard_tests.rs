//! Guarded stream teardown (GitHub #289, matrix row CAG4).
//!
//! `v4l` 0.14 panics in `Drop for mmap::Stream` when `VIDIOC_STREAMOFF` fails with anything
//! but `ENODEV` (and in `Drop for Arena` when unmapping or `VIDIOC_REQBUFS(0)` fails). The
//! capture open path drops the stream through `guarded_v4l_drop`, so such a panic becomes a
//! `CameraError::StreamTeardown` (an `Io` error the supervisor backs off from) instead of a
//! supervisor panic that marks the camera `Dead`. A real `v4l` stream needs a device, so the
//! drop path is exercised with a value whose `Drop` panics like the upstream one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use soos_camera_v4l::v4l_guard::{guarded_v4l_drop, V4L_PANIC_MESSAGE};
use soos_camera_v4l::{CameraError, CameraErrorKind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Mimics `Drop for v4l::io::mmap::Stream`: a failing STREAMOFF panics with the error text.
struct FakeStream {
    drops: Arc<AtomicUsize>,
    streamoff_fails: bool,
}

impl Drop for FakeStream {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        if self.streamoff_fails {
            panic!("Os {{ code: 5, kind: Uncategorized, message: \"Input/output error\" }}");
        }
    }
}

#[test]
fn test_cag_guarded_drop_turns_a_teardown_panic_into_an_error() {
    let drops = Arc::new(AtomicUsize::new(0));

    let clean = guarded_v4l_drop(FakeStream {
        drops: Arc::clone(&drops),
        streamoff_fails: false,
    });
    assert!(clean.is_ok());
    assert_eq!(drops.load(Ordering::SeqCst), 1, "the value is dropped");

    let err = guarded_v4l_drop(FakeStream {
        drops: Arc::clone(&drops),
        streamoff_fails: true,
    })
    .unwrap_err();
    assert_eq!(drops.load(Ordering::SeqCst), 2, "dropped exactly once");
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.to_string(), V4L_PANIC_MESSAGE);
    assert!(
        !err.to_string().contains("Input/output"),
        "the panic payload is not propagated"
    );
}

#[test]
fn test_cag_stream_teardown_error_is_recoverable_and_payload_free() {
    let err = CameraError::StreamTeardown {
        path: PathBuf::from("/dev/video0"),
        reason: V4L_PANIC_MESSAGE.to_string(),
    };
    // `Io` is a backoff-and-retry kind: the supervisor marks the camera Recovering, never Dead.
    assert_eq!(err.kind(), CameraErrorKind::Io);
    assert!(!err.is_device_busy());
    let text = err.to_string();
    assert!(text.contains("/dev/video0"), "{text}");
    assert!(text.contains(V4L_PANIC_MESSAGE), "{text}");
}
