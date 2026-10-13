//! Byte-level proof that a `Frame` wipes its whole buffer when it is dropped (GitHub #367,
//! spec `AI/architect_spec_macos_port_foundation.md` §11, matrix MAC2), and compile-time proof
//! that the camera types are the `soos-frame` types re-exported.
//!
//! A test-only global allocator inspects the watched allocation in `dealloc`, before the memory
//! is returned to the system allocator. The probe is keyed on the recorded buffer address and
//! uses atomics only (no allocation, lock or I/O inside the allocator hooks). This file lives in
//! the adapter crate `soos-camera-v4l` because business crates never contain `unsafe`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Address of the watched allocation (0 = nothing watched).
static WATCH_PTR: AtomicUsize = AtomicUsize::new(0);
/// Set when the watched allocation was deallocated.
static SEEN: AtomicBool = AtomicBool::new(false);
/// Whether every byte of the watched allocation was zero when it was deallocated.
static ALL_ZERO: AtomicBool = AtomicBool::new(false);
/// Size of the watched allocation as reported by its deallocation layout.
static SEEN_SIZE: AtomicUsize = AtomicUsize::new(0);
/// Serialises the tests of this file (taken outside the allocator hooks).
static SERIAL: Mutex<()> = Mutex::new(());

struct WipeProbe;

// SAFETY: every method forwards to `System` with the caller's arguments unchanged, so the
// `GlobalAlloc` contract is the one of `System`. `dealloc` additionally reads the block it is
// about to free; the caller guarantees `ptr` was allocated by this allocator with `layout`,
// so the `layout.size()` bytes at `ptr` are valid for reads until `System.dealloc` runs.
unsafe impl GlobalAlloc for WipeProbe {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged; the caller upholds `GlobalAlloc::alloc`'s contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let watched = WATCH_PTR.load(Ordering::SeqCst);
        if watched != 0 && ptr as usize == watched {
            // SAFETY: `ptr` is a live block of `layout.size()` bytes owned by the caller and not
            // yet released (see the impl-level comment); it is only read.
            let bytes = unsafe { std::slice::from_raw_parts(ptr, layout.size()) };
            ALL_ZERO.store(bytes.iter().all(|&b| b == 0), Ordering::SeqCst);
            SEEN_SIZE.store(layout.size(), Ordering::SeqCst);
            SEEN.store(true, Ordering::SeqCst);
            WATCH_PTR.store(0, Ordering::SeqCst);
        }
        // SAFETY: forwarded unchanged; the caller upholds `GlobalAlloc::dealloc`'s contract.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: WipeProbe = WipeProbe;

const CAPACITY: usize = 4096;
const LEN: usize = 3000;

/// Arms the probe on `buffer`'s allocation.
fn watch(buffer: &[u8]) {
    SEEN.store(false, Ordering::SeqCst);
    ALL_ZERO.store(false, Ordering::SeqCst);
    SEEN_SIZE.store(0, Ordering::SeqCst);
    WATCH_PTR.store(buffer.as_ptr() as usize, Ordering::SeqCst);
}

fn filled_buffer() -> Vec<u8> {
    let mut data = Vec::with_capacity(CAPACITY);
    data.resize(LEN, 0xA5);
    assert!(data.capacity() >= CAPACITY && data.capacity() > data.len());
    data
}

#[test]
fn test_frame_buffer_wiped_on_drop() {
    let _guard = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let frame = soos_camera_v4l::Frame::new(
        filled_buffer(),
        40,
        25,
        1,
        soos_camera_v4l::PixelFormat::Rgb24,
        1,
    );
    watch(&frame.data);
    let capacity = frame.data.capacity();
    drop(frame);
    assert!(
        SEEN.load(Ordering::SeqCst),
        "the frame buffer was never deallocated (leak or forget)"
    );
    assert_eq!(SEEN_SIZE.load(Ordering::SeqCst), capacity);
    assert!(
        ALL_ZERO.load(Ordering::SeqCst),
        "dropping a Frame must wipe its whole buffer capacity"
    );
}

#[test]
fn test_drop_probe_detects_an_unwiped_buffer() {
    let _guard = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Every byte of the allocation is initialised (len == capacity): the probe must never read
    // uninitialised memory of a buffer that is not wiped before deallocation.
    let plain = vec![0xA5_u8; CAPACITY];
    assert_eq!(plain.len(), plain.capacity());
    watch(&plain);
    drop(plain);
    assert!(
        SEEN.load(Ordering::SeqCst),
        "probe must see the plain buffer"
    );
    assert!(
        !ALL_ZERO.load(Ordering::SeqCst),
        "non-vacuity: an unwiped buffer must be reported as non-zero"
    );
}

#[test]
fn test_camera_frame_types_are_the_soos_frame_types() {
    fn takes_frame(frame: soos_frame::Frame) -> soos_frame::Frame {
        frame
    }
    fn takes_format(format: soos_frame::PixelFormat) -> soos_frame::PixelFormat {
        format
    }
    fn takes_sensor(sensor: soos_frame::SensorType) -> soos_frame::SensorType {
        sensor
    }
    let frame: soos_camera_v4l::Frame =
        soos_camera_v4l::frame::Frame::new(vec![1], 1, 1, 0, soos_camera_v4l::PixelFormat::Grey, 0);
    let frame = takes_frame(frame);
    let format: soos_camera_v4l::frame::PixelFormat = takes_format(frame.format);
    let sensor: soos_camera_v4l::sensor::SensorType =
        takes_sensor(soos_camera_v4l::SensorType::Infrared);
    assert_eq!(format, soos_frame::PixelFormat::Grey);
    assert_eq!(sensor, soos_frame::SensorType::Infrared);
}
