//! Panic guard for the `v4l` crate (GitHub #287).
//!
//! `v4l` 0.14 converts kernel structures with `unwrap`/`expect`: `Capabilities::from` unwraps
//! `str::from_utf8` on the driver, card and bus strings (`VIDIOC_QUERYCAP`),
//! `FormatDescription::from` on the format description (`VIDIOC_ENUM_FMT`), and `Format::from`
//! expects known field-order and colour-space values (`VIDIOC_S_FMT`). A USB device chooses its
//! own product string, so a non-UTF-8 card name would panic inside the crate. Every such call
//! in this crate goes through the wrappers below, which turn the panic into an
//! [`std::io::Error`] (`ErrorKind::Other`, message [`V4L_PANIC_MESSAGE`]); the callers then map it
//! like any other ioctl failure (node skipped, probe failure, or a capture error the supervisor
//! backs off from). The invariant `camera_vision_followups_contract` forbids direct calls
//! elsewhere in the crate.

use std::panic::{self, AssertUnwindSafe};

/// Error message of a `v4l` call that panicked. The panic payload is never propagated: it may
/// quote bytes chosen by the device.
pub const V4L_PANIC_MESSAGE: &str = "v4l crate panicked on malformed driver metadata";

/// Runs one `v4l` call, converting a panic into an [`std::io::Error`].
///
/// # Errors
///
/// Returns `ErrorKind::Other` with [`V4L_PANIC_MESSAGE`] when `call` panics.
pub fn guard_v4l_call<T, F>(call: F) -> std::io::Result<T>
where
    F: FnOnce() -> T,
{
    // `AssertUnwindSafe`: the closures only borrow a `v4l::Device` (a file descriptor) and
    // plain values; a panic leaves nothing half-updated that the caller would observe, because
    // the caller only receives the error.
    panic::catch_unwind(AssertUnwindSafe(call))
        .map_err(|_| std::io::Error::other(V4L_PANIC_MESSAGE))
}

/// `VIDIOC_QUERYCAP` through the guard.
pub(crate) fn query_caps_guarded(
    device: &v4l::Device,
) -> std::io::Result<v4l::capability::Capabilities> {
    guard_v4l_call(|| device.query_caps()).and_then(|result| result)
}

/// `VIDIOC_ENUM_FMT` fourccs through the guard (empty on error or panic, as before).
pub(crate) fn enum_formats_guarded(device: &v4l::Device) -> Vec<v4l::FourCC> {
    guard_v4l_call(|| v4l::video::Capture::enum_formats(device))
        .and_then(|result| result)
        .unwrap_or_default()
        .into_iter()
        .map(|desc| desc.fourcc)
        .collect()
}

/// `VIDIOC_S_FMT` through the guard.
pub(crate) fn set_format_guarded(
    device: &v4l::Device,
    format: &v4l::Format,
) -> std::io::Result<v4l::Format> {
    guard_v4l_call(|| v4l::video::Capture::set_format(device, format)).and_then(|result| result)
}
