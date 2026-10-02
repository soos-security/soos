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
//! removed. A hook installed later by the host replaces the filter (which only brings the report
//! back); the host then calls [`install_v4l_panic_hook_filter`] right after installing its hook
//! (GitHub #291), which wraps the current hook again unless it already is the live filter, so
//! the filter is never wrapped around itself and no hook is ever dropped.

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, Once, PoisonError};

/// Error message of a `v4l` call that panicked. The panic payload is never propagated: it may
/// quote bytes chosen by the device.
pub const V4L_PANIC_MESSAGE: &str = "v4l crate panicked on malformed driver metadata";

thread_local! {
    /// Number of guarded `v4l` calls currently running on this thread (nesting is allowed).
    static GUARD_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// The first guarded call places the filter once; later calls never touch the hook.
static HOOK_FILTER: Once = Once::new();

/// Serializes the filter placements of this crate (explicit calls and the first guarded call).
static PLACEMENT: Mutex<()> = Mutex::new(());

/// Number of times a hook was wrapped by the filter (diagnostics; the generation of the latest
/// filter).
static WRAPS: AtomicU64 = AtomicU64::new(0);

/// Generation of the latest filter while it is alive (still installed or chained by a host
/// hook), `0` once it was dropped.
static LIVE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Heap address of the latest filter hook (compared only while it is alive, so a later
/// allocation at the same address cannot be mistaken for it).
static LIVE_ADDRESS: AtomicUsize = AtomicUsize::new(0);

/// The process panic hook type of `std::panic::set_hook`.
type PanicHook = Box<dyn Fn(&panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

/// Owned by a filter hook: clears [`LIVE_GENERATION`] when that filter is dropped (a host
/// `set_hook` replacing it), before its memory can be reused.
struct FilterLiveness(u64);

impl Drop for FilterLiveness {
    fn drop(&mut self) {
        let _ = LIVE_GENERATION.compare_exchange(self.0, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// Address of the closure behind `hook` (never dereferenced).
fn hook_address(hook: &PanicHook) -> usize {
    std::ptr::from_ref(&**hook).cast::<()>().addr()
}

/// Returns whether the current thread is inside a guarded `v4l` call. Never panics: during
/// thread-local destruction the answer is `false` (the panic is reported).
fn inside_guarded_call() -> bool {
    GUARD_DEPTH
        .try_with(|depth| depth.get() > 0)
        .unwrap_or(false)
}

/// Puts the filter on top of the current process hook, unless the current hook already is the
/// live filter (then it is put back unchanged). The current hook is always kept: it becomes the
/// filter's `previous` hook or is reinstalled itself.
fn place_filter_on_top() {
    let _placement = PLACEMENT.lock().unwrap_or_else(PoisonError::into_inner);
    let current = panic::take_hook();
    let is_live_filter = LIVE_GENERATION.load(Ordering::SeqCst) != 0
        && LIVE_ADDRESS.load(Ordering::SeqCst) == hook_address(&current);
    let hook = if is_live_filter {
        current
    } else {
        let generation = WRAPS.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        let liveness = FilterLiveness(generation);
        let previous = current;
        let filter: PanicHook = Box::new(move |info| {
            let _alive = &liveness;
            if !inside_guarded_call() {
                previous(info);
            }
        });
        LIVE_ADDRESS.store(hook_address(&filter), Ordering::SeqCst);
        LIVE_GENERATION.store(generation, Ordering::SeqCst);
        filter
    };
    panic::set_hook(hook);
}

/// Installs the panic-hook filter described in the module documentation on top of the current
/// process hook.
///
/// Called once by the first guarded call. A host that installs its own panic hook after that
/// (or at any time) calls it right after its `set_hook`, so that caught `v4l` panics stay
/// silent: if the current hook is not the live filter it is wrapped again (and keeps reporting
/// every other panic), otherwise nothing changes, so repeated calls never stack filters.
/// Thread-safe. Does nothing while the current thread is panicking (`set_hook` would panic).
pub fn install_v4l_panic_hook_filter() {
    if std::thread::panicking() {
        return;
    }
    place_filter_on_top();
    // The filter is in place: the first guarded call must not place it again.
    HOOK_FILTER.call_once(|| {});
}

/// Places the filter on the first guarded call of the process only.
fn install_filter_once() {
    if std::thread::panicking() {
        return;
    }
    HOOK_FILTER.call_once(place_filter_on_top);
}

/// Number of times the filter wrapped a process hook since start-up (one per placement that
/// found another hook on top; diagnostics and tests).
#[must_use]
pub fn v4l_panic_hook_filter_installations() -> u64 {
    WRAPS.load(Ordering::SeqCst)
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
    install_filter_once();
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

/// Refuses a device path holding a NUL byte (GitHub #314, CAM-NEW-4).
///
/// `v4l` 0.14 unwraps `CString::new` on the path, so such a path (a `camera_device` written
/// `"/dev/vid\u0000eo0"`) would panic inside the crate; every opener checks it first.
///
/// # Errors
///
/// `ErrorKind::InvalidInput` when `path` contains a NUL byte.
pub(crate) fn reject_nul_device_path(path: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt as _;
    if path.as_os_str().as_bytes().contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "camera device path contains a NUL byte",
        ));
    }
    Ok(())
}

/// Opens a V4L2 node (`open(2)`) through the guard (GitHub #314, CAM-NEW-4): NUL bytes are
/// refused ([`reject_nul_device_path`]), any panic of the call is caught, and the returned
/// [`GuardedDevice`] closes the descriptor through the guard as well (`Drop for
/// v4l::device::Handle` unwraps `close(2)`). Used by the enumeration and diagnostics probes;
/// the capture supervisor opens through `V4lBackend` with the same check.
///
/// # Errors
///
/// `InvalidInput` for a NUL byte, the `open(2)` error otherwise.
pub(crate) fn open_device_guarded(path: &Path) -> std::io::Result<GuardedDevice> {
    reject_nul_device_path(path)?;
    guard_v4l_call(|| v4l::Device::with_path(path))
        .and_then(|result| result)
        .map(DropGuarded::new)
}

/// An open `v4l::Device` whose descriptor is closed through the guard on drop.
pub(crate) type GuardedDevice = DropGuarded<v4l::Device>;

/// Owns a `v4l` value (device or anything holding its handle) and drops it through the guard.
pub(crate) struct DropGuarded<T> {
    /// Always `Some` until `Drop` takes it.
    value: Option<T>,
}

impl<T> DropGuarded<T> {
    /// Takes ownership of `value`.
    pub(crate) fn new(value: T) -> Self {
        Self { value: Some(value) }
    }

    /// The owned value; `None` only after `Drop` took it, so every caller treats `None` like a
    /// vanished node instead of panicking.
    pub(crate) fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }
}

impl<T> Drop for DropGuarded<T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            // A failing close(2) is reported by nobody: the descriptor is gone either way.
            let _ = guarded_v4l_drop(value);
        }
    }
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

/// Upper bound on the `VIDIOC_ENUM_FMT` indices queried per node (equal to
/// [`crate::diagnostics::MAX_DIAGNOSTIC_FOURCCS`], so the diagnostics truncation is unchanged).
pub(crate) const MAX_ENUMERATED_FORMATS: u32 = 64;

/// Collects the capture fourccs reported by `query(index)` (the `pixelformat` of one
/// `VIDIOC_ENUM_FMT` entry), at most [`MAX_ENUMERATED_FORMATS`] indices.
///
/// An error at index 0 means no format (empty list, as `v4l` 0.14 and the previous wrapper
/// returned); a later error ends the list. A driver that never reports the end is cut at the
/// bound (the first indices are kept) and the truncation is logged at debug level with the node
/// path only.
fn bounded_format_fourccs<F>(device_path: &Path, query: F) -> Vec<v4l::FourCC>
where
    F: FnMut(u32) -> std::io::Result<u32>,
{
    let mut query = query;
    let Ok(found) = enumerate_indexed_bounded(MAX_ENUMERATED_FORMATS, |index| {
        query(index).map(|code| Some(v4l::FourCC::from(code)))
    }) else {
        return Vec::new();
    };
    if found.truncated {
        tracing::debug!(
            "Format enumeration on '{}' stopped at the bound of {} entries",
            device_path.display(),
            MAX_ENUMERATED_FORMATS
        );
    }
    found.items
}

/// `VIDIOC_ENUM_FMT` capture fourccs, at most [`MAX_ENUMERATED_FORMATS`], through the guard
/// (empty on error or panic, as before).
///
/// Replaces `v4l::video::Capture::enum_formats`, whose loop only ends when the driver returns an
/// error and which unwraps the UTF-8 description: only the `pixelformat` field is read, so the
/// description is never decoded. `device_path` is used for logging only.
pub(crate) fn enum_formats_guarded(device: &v4l::Device, device_path: &Path) -> Vec<v4l::FourCC> {
    let fd = device.handle().fd();
    guard_v4l_call(|| {
        bounded_format_fourccs(device_path, |index| {
            // SAFETY: `v4l2_fmtdesc` is a plain C struct (integers and a byte array) for which
            // the all-zero bit pattern is valid.
            let mut raw: v4l::v4l_sys::v4l2_fmtdesc = unsafe { std::mem::zeroed() };
            raw.index = index;
            raw.type_ = v4l::buffer::Type::VideoCapture as u32;
            // SAFETY: `fd` is the open descriptor owned by `device`, which outlives this call;
            // `raw` is a valid, exclusively borrowed `v4l2_fmtdesc`, the argument type of
            // `VIDIOC_ENUM_FMT`, and the kernel writes only within it.
            unsafe {
                v4l::v4l2::ioctl(
                    fd,
                    v4l::v4l2::vidioc::VIDIOC_ENUM_FMT,
                    std::ptr::from_mut(&mut raw).cast::<std::os::raw::c_void>(),
                )
            }?;
            Ok(raw.pixelformat)
        })
    })
    .unwrap_or_default()
}

/// `VIDIOC_S_FMT` through the guard.
pub(crate) fn set_format_guarded(
    device: &v4l::Device,
    format: &v4l::Format,
) -> std::io::Result<v4l::Format> {
    guard_v4l_call(|| v4l::video::Capture::set_format(device, format)).and_then(|result| result)
}

/// Result of an index-based V4L2 enumeration stopped at a fixed bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoundedEnumeration<T> {
    /// Converted entries, in index order.
    pub(crate) items: Vec<T>,
    /// `true` when the bound stopped the enumeration (the driver had not signalled the end).
    pub(crate) truncated: bool,
}

/// Queries indices `0..max_indices` with `query`, the way V4L2 `VIDIOC_ENUM_*` lists are walked.
///
/// `Ok(Some(item))` is an entry, `Ok(None)` an entry that could not be converted (skipped, but
/// it still counts toward the bound), and an error ends the list (the driver reports `EINVAL`
/// past the last index). As in `v4l` 0.14, an error at index 0 is returned. Unlike `v4l`, at
/// most `max_indices` queries are issued, so a driver that never reports the end cannot keep
/// the caller looping; truncation is deterministic (always the first `max_indices` indices).
///
/// # Errors
///
/// The error of the index-0 query.
pub(crate) fn enumerate_indexed_bounded<T, F>(
    max_indices: u32,
    mut query: F,
) -> std::io::Result<BoundedEnumeration<T>>
where
    F: FnMut(u32) -> std::io::Result<Option<T>>,
{
    let mut items = Vec::new();
    for index in 0..max_indices {
        match query(index) {
            Ok(Some(item)) => items.push(item),
            Ok(None) => {}
            Err(e) if index == 0 => return Err(e),
            Err(_) => {
                return Ok(BoundedEnumeration {
                    items,
                    truncated: false,
                })
            }
        }
    }
    Ok(BoundedEnumeration {
        items,
        truncated: true,
    })
}

/// `VIDIOC_ENUM_FRAMESIZES` for `fourcc`, at most `max_indices` entries, through the guard.
///
/// Replaces `v4l::video::Capture::enum_framesizes`, whose loop only ends when the driver
/// returns an error. Entries of an unknown size type are skipped, as in `v4l` 0.14.
///
/// # Errors
///
/// The index-0 ioctl error, or [`V4L_PANIC_MESSAGE`] if the conversion panics.
pub(crate) fn enum_framesizes_guarded(
    device: &v4l::Device,
    fourcc: v4l::FourCC,
    max_indices: u32,
) -> std::io::Result<BoundedEnumeration<v4l::framesize::FrameSizeEnum>> {
    let fd = device.handle().fd();
    guard_v4l_call(|| {
        enumerate_indexed_bounded(max_indices, |index| {
            // SAFETY: `v4l2_frmsizeenum` is a plain C struct (integers and a union of integer
            // structs) for which the all-zero bit pattern is valid.
            let mut raw: v4l::v4l_sys::v4l2_frmsizeenum = unsafe { std::mem::zeroed() };
            raw.index = index;
            raw.pixel_format = fourcc.into();
            // SAFETY: `fd` is the open descriptor owned by `device`, which outlives this call;
            // `raw` is a valid, exclusively borrowed `v4l2_frmsizeenum`, the argument type of
            // `VIDIOC_ENUM_FRAMESIZES`, and the kernel writes only within it.
            unsafe {
                v4l::v4l2::ioctl(
                    fd,
                    v4l::v4l2::vidioc::VIDIOC_ENUM_FRAMESIZES,
                    std::ptr::from_mut(&mut raw).cast::<std::os::raw::c_void>(),
                )
            }?;
            Ok(v4l::framesize::FrameSizeEnum::try_from(raw).ok())
        })
    })
    .and_then(|result| result)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Unit tests use direct assertions on the bounded enumeration"
)]
mod bounded_enumeration_tests {
    use super::*;
    use std::io;

    /// A driver that never reports the end of its list is cut at the bound, deterministically.
    #[test]
    fn test_ccb_endless_enumeration_is_truncated_at_the_bound() {
        for _ in 0..2 {
            let mut calls = Vec::new();
            let out = enumerate_indexed_bounded(5, |index| {
                calls.push(index);
                Ok(Some(index))
            })
            .unwrap();
            assert_eq!(out.items, vec![0, 1, 2, 3, 4]);
            assert!(out.truncated, "stopped by the bound, not by the driver");
            assert_eq!(calls, vec![0, 1, 2, 3, 4], "never queries past the bound");
        }
    }

    /// The end of the list (`EINVAL` after index 0) ends the enumeration without truncation.
    #[test]
    fn test_ccb_enumeration_ends_at_the_first_error_after_index_zero() {
        let mut calls = 0u32;
        let out = enumerate_indexed_bounded(64, |index| {
            calls += 1;
            if index < 3 {
                Ok(Some(index))
            } else {
                Err(io::Error::from_raw_os_error(libc::EINVAL))
            }
        })
        .unwrap();
        assert_eq!(out.items, vec![0, 1, 2]);
        assert!(!out.truncated);
        assert_eq!(calls, 4);
    }

    /// An error at index 0 is reported (no entry at all), like `v4l` 0.14.
    #[test]
    fn test_ccb_enumeration_error_at_index_zero_is_an_error() {
        let out = enumerate_indexed_bounded::<u32, _>(64, |_| {
            Err(io::Error::from_raw_os_error(libc::ENOTTY))
        });
        assert_eq!(out.unwrap_err().raw_os_error(), Some(libc::ENOTTY));
    }

    /// Unconvertible entries are skipped but still count toward the bound, and a zero bound
    /// queries nothing.
    #[test]
    fn test_ccb_skipped_entries_count_toward_the_bound() {
        let mut calls = 0u32;
        let out = enumerate_indexed_bounded(6, |index| {
            calls += 1;
            Ok(index.is_multiple_of(2).then_some(index))
        })
        .unwrap();
        assert_eq!(out.items, vec![0, 2, 4]);
        assert!(out.truncated);
        assert_eq!(calls, 6);

        let none = enumerate_indexed_bounded::<u32, _>(0, |_| panic!("queried with a zero bound"))
            .unwrap();
        assert!(none.items.is_empty() && none.truncated);
    }

    fn fourcc_code(index: u32) -> u32 {
        u32::from(v4l::FourCC::new(&[
            b'A',
            b'A',
            b'A',
            b'0' + u8::try_from(index % 10).unwrap(),
        ]))
    }

    /// A driver whose `VIDIOC_ENUM_FMT` never reports the end yields exactly
    /// `MAX_ENUMERATED_FORMATS` fourccs, the first indices, in order.
    #[test]
    fn test_ccb_endless_format_enumeration_is_truncated_at_the_bound() {
        let mut calls = 0u32;
        let fourccs = bounded_format_fourccs(Path::new("/dev/video-endless"), |index| {
            calls += 1;
            Ok(fourcc_code(index))
        });
        assert_eq!(calls, MAX_ENUMERATED_FORMATS);
        assert_eq!(
            fourccs.len(),
            usize::try_from(MAX_ENUMERATED_FORMATS).unwrap()
        );
        assert_eq!(fourccs[0].repr, *b"AAA0");
        assert_eq!(fourccs[11].repr, *b"AAA1");
        assert!(
            usize::try_from(MAX_ENUMERATED_FORMATS).unwrap()
                >= crate::diagnostics::MAX_DIAGNOSTIC_FOURCCS,
            "the diagnostics truncation stays reachable"
        );
    }

    /// The end of the list ends the enumeration; an error at index 0 means no format (empty,
    /// as before); a panicking conversion inside the guard is an empty list too.
    #[test]
    fn test_ccb_format_enumeration_end_error_and_panic_semantics() {
        let path = Path::new("/dev/video-fake");
        let two = bounded_format_fourccs(path, |index| {
            if index < 2 {
                Ok(fourcc_code(index))
            } else {
                Err(io::Error::from_raw_os_error(libc::EINVAL))
            }
        });
        assert_eq!(two.len(), 2);
        let none =
            bounded_format_fourccs(path, |_| Err(io::Error::from_raw_os_error(libc::ENOTTY)));
        assert!(none.is_empty());
        let guarded = guard_v4l_call(|| {
            bounded_format_fourccs(path, |index| {
                if index == 1 {
                    panic!("malformed format description");
                }
                Ok(fourcc_code(index))
            })
        });
        assert!(guarded.is_err(), "the panic is caught by the guard");
    }
}
