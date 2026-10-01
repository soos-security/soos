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
//!
//! The MMAP stream is created and dropped through the guard as well (GitHub #289):
//! `Drop for mmap::Stream` panics when `VIDIOC_STREAMOFF` fails with anything but `ENODEV`, and
//! `Drop for Arena` when unmapping or `VIDIOC_REQBUFS(0)` fails (also reached when
//! `Stream::with_buffers` fails half-way).
//!
//! A caught panic is an expected, recovered condition, so it is kept out of the process panic
//! hook (the daemon's hook would log a panic location, Rust's default hook prints a line on
//! stderr): the first guarded call installs, once, a wrapper hook that stays silent while the
//! current thread is inside a guarded call and forwards every other panic to the hook that was
//! installed before. The hook is never swapped per call (race-free across threads) and never
//! removed; a hook installed later by the host replaces the filter, which only brings the
//! report back.

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

/// Error message of a `v4l` call that panicked. The panic payload is never propagated: it may
/// quote bytes chosen by the device.
pub const V4L_PANIC_MESSAGE: &str = "v4l crate panicked on malformed driver metadata";

thread_local! {
    /// Number of guarded `v4l` calls currently running on this thread (nesting is allowed).
    static GUARD_DEPTH: Cell<u32> = const { Cell::new(0) };
}

static HOOK_FILTER: Once = Once::new();

/// Returns whether the current thread is inside a guarded `v4l` call. Never panics: during
/// thread-local destruction the answer is `false` (the panic is reported).
fn inside_guarded_call() -> bool {
    GUARD_DEPTH
        .try_with(|depth| depth.get() > 0)
        .unwrap_or(false)
}

/// Installs, once per process, the panic-hook filter described in the module documentation.
///
/// Idempotent and thread-safe; called by the first guarded call. Hosts that install their own
/// panic hook should do so before (the daemon does, at start-up), so that the filter chains
/// to it. Does nothing while the current thread is panicking (`set_hook` would panic).
pub fn install_v4l_panic_hook_filter() {
    if std::thread::panicking() {
        return;
    }
    HOOK_FILTER.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !inside_guarded_call() {
                previous(info);
            }
        }));
    });
}

/// Marks the current thread as inside a guarded call for its lifetime.
struct DepthGuard;

impl DepthGuard {
    fn enter() -> Self {
        let _ = GUARD_DEPTH.try_with(|depth| depth.set(depth.get().saturating_add(1)));
        Self
    }
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        let _ = GUARD_DEPTH.try_with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Runs one `v4l` call, converting a panic into an [`std::io::Error`].
///
/// The caught panic does not reach the process panic hook; panics elsewhere (other threads,
/// or this thread outside the guard) still do.
///
/// # Errors
///
/// Returns `ErrorKind::Other` with [`V4L_PANIC_MESSAGE`] when `call` panics.
pub fn guard_v4l_call<T, F>(call: F) -> std::io::Result<T>
where
    F: FnOnce() -> T,
{
    install_v4l_panic_hook_filter();
    let depth = DepthGuard::enter();
    // `AssertUnwindSafe`: the closures only borrow a `v4l::Device` (a file descriptor) and
    // plain values, or own the stream being dropped; a panic leaves nothing half-updated that
    // the caller would observe, because the caller only receives the error.
    let outcome = panic::catch_unwind(AssertUnwindSafe(call));
    drop(depth);
    outcome.map_err(|_| std::io::Error::other(V4L_PANIC_MESSAGE))
}

/// Drops `value` (a `v4l` MMAP stream, or a value owning one) through the guard.
///
/// # Errors
///
/// Returns `ErrorKind::Other` with [`V4L_PANIC_MESSAGE`] when the drop panics (a failing
/// `VIDIOC_STREAMOFF`, `munmap` or `VIDIOC_REQBUFS(0)` in `v4l` 0.14). If a second panic
/// occurs while the first unwinds (stream and arena teardown both failing), Rust aborts the
/// process; no guard can catch that (see the ADR).
pub fn guarded_v4l_drop<T>(value: T) -> std::io::Result<()> {
    guard_v4l_call(move || drop(value))
}

/// `VIDIOC_REQBUFS` + `VIDIOC_QUERYBUF` + `mmap` through the guard (the half-built arena is
/// dropped inside `v4l` on failure, which can panic).
pub(crate) fn mmap_stream_guarded<'a>(
    device: &v4l::Device,
    buffer_count: u32,
) -> std::io::Result<v4l::io::mmap::Stream<'a>> {
    guard_v4l_call(|| {
        v4l::io::mmap::Stream::with_buffers(device, v4l::buffer::Type::VideoCapture, buffer_count)
    })
    .and_then(|result| result)
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
