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
