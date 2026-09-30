//! Contractual tests for Criterion C9: Graceful capture thread shutdown and memory ordering.
//!
//! Sub-issue #23.1: Interrupt blocking stream.next() on shutdown (Drop completes < 500ms).
//! Sub-issue #23.2: is_ready.load(Ordering::Acquire) ensures strict frame visibility.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions and unwraps"
)]

use soos_camera_v4l::{
    CameraConfigBuilder, CameraManager, Frame, MockCameraManager, PixelFormat, V4lCameraManager,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

mod common;

use common::{wait_until, SETTLE_TIMEOUT};

/// Criterion C9 & Sub-issue #23.1:
/// Verify that MockCameraManager Drop completes within 500ms even when the camera is idle
/// with a very low effective frame rate (e.g. 1 FPS = 1000ms frame interval).
#[test]
fn test_camera_drop_completes_within_timeout() {
    // Wall-clock benchmark: runs only with SOOS_LATENCY_BENCH=1 (dedicated single-threaded CI
    // step), never gated on a loaded developer machine (GitHub #280, user-approved 2026-09-30).
    if !latency_bench_enabled() {
        eprintln!("skipped: wall-clock benchmark, set SOOS_LATENCY_BENCH=1 to run it");
        return;
    }
    let config = CameraConfigBuilder::new()
        .fps(30)
        .idle_fps(1) // 1 FPS = 1000ms frame interval in idle mode
        .idle_timeout(Duration::from_millis(10)) // Immediately transitions to idle
        .warmup_frames(1)
        .build();

    let camera = MockCameraManager::new(config);

    // Allow camera to warm up and enter idle mode
    thread::sleep(Duration::from_millis(80));

    // Measure drop latency
    let start = Instant::now();
    drop(camera);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(500),
        "MockCameraManager drop took {:?}, must complete within 500ms (Criterion C9)",
        elapsed
    );
}

/// Sub-issue #23.1:
/// Verify that calling stop() signals graceful shutdown and flips is_ready to false immediately.
#[test]
fn test_camera_stop_signals_graceful_shutdown() {
    // Wall-clock benchmark: runs only with SOOS_LATENCY_BENCH=1 (dedicated single-threaded CI
    // step), never gated on a loaded developer machine (GitHub #280, user-approved 2026-09-30).
    if !latency_bench_enabled() {
        eprintln!("skipped: wall-clock benchmark, set SOOS_LATENCY_BENCH=1 to run it");
        return;
    }
    let config = CameraConfigBuilder::new().fps(30).warmup_frames(1).build();

    let camera = MockCameraManager::new(config);
    wait_until(SETTLE_TIMEOUT, || camera.is_ready());
    assert!(camera.is_ready(), "Camera should be ready after warmup");

    camera.stop();

    // is_ready must report false immediately upon stop
    assert!(
        !camera.is_ready(),
        "is_ready must return false immediately after stop()"
    );

    // Drop must be instantaneous once stopped
    let start = Instant::now();
    drop(camera);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(100),
        "Stopped camera drop took {:?}, must complete almost immediately",
        elapsed
    );
}

/// Sub-issue #23.1:
/// Verify that V4lCameraManager Drop completes within 500ms even when device is disconnected
/// or backoff loop is executing.
#[test]
fn test_v4l_camera_drop_completes_within_timeout() {
    let config = CameraConfigBuilder::new()
        .device_path("/dev/video_non_existent_shutdown_test")
        .backoff_limits(Duration::from_millis(500), Duration::from_millis(2000))
        .build();

    let camera = V4lCameraManager::spawn(config)
        .expect("V4lCameraManager::spawn must succeed even before device connection");

    // Allow worker thread to spin up and enter backoff
    thread::sleep(Duration::from_millis(50));

    let start = Instant::now();
    drop(camera);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(500),
        "V4lCameraManager drop took {:?}, must complete within 500ms (Criterion C9)",
        elapsed
    );
}

/// Sub-issue #23.2:
/// Verify Acquire/Release memory ordering: whenever is_ready returns true, latest_frame()
/// is guaranteed to observe the published frame without stale reads or race conditions.
#[test]
fn test_is_ready_memory_visibility_acquire_release() {
    let config = CameraConfigBuilder::new().warmup_frames(0).fps(60).build();

    let camera = Arc::new(MockCameraManager::new(config));
    let stop_signal = Arc::new(AtomicBool::new(false));

    // Spawn 4 concurrent reader threads continuously querying is_ready and latest_frame
    let mut reader_handles = Vec::new();
    for _ in 0..4 {
        let cam = Arc::clone(&camera);
        let stop = Arc::clone(&stop_signal);

        reader_handles.push(thread::spawn(move || {
            let mut observed_frames = 0usize;
            let mut last_seq = 0u64;

            while !stop.load(Ordering::Acquire) {
                if cam.is_ready() {
                    // With Acquire/Release ordering, latest_frame must NEVER be None if is_ready is true
                    let frame = cam.latest_frame();
                    assert!(
                        frame.is_some(),
                        "latest_frame() must never return None when is_ready() is true"
                    );

                    let frame = frame.unwrap();
                    assert!(
                        frame.sequence >= last_seq,
                        "Frame sequence must never go backwards (monotonic visibility): got {} vs last {}",
                        frame.sequence,
                        last_seq
                    );
                    last_seq = frame.sequence;
                    observed_frames = observed_frames.saturating_add(1);
                }
                thread::yield_now();
            }

            observed_frames
        }));
    }

    // Spawn a writer thread rapidly injecting synthetic frames
    let cam_writer = Arc::clone(&camera);
    let stop_writer = Arc::clone(&stop_signal);
    let writer_handle = thread::spawn(move || {
        for seq in 1..=500 {
            let frame = Frame::new(
                vec![0xAA; 640 * 480 * 2],
                640,
                480,
                1_000_000_000 + seq,
                PixelFormat::Yuyv,
                seq,
            );
            cam_writer.push_frame(frame);
            thread::yield_now();
        }
        stop_writer.store(true, Ordering::Release);
    });

    writer_handle.join().expect("Writer thread failed");
    for handle in reader_handles {
        let count = handle.join().expect("Reader thread failed");
        assert!(count > 0, "Reader thread should have observed frames");
    }

    camera.stop();
}

/// Whether wall-clock latency benchmarks are enabled (`SOOS_LATENCY_BENCH=1`, GitHub #280).
fn latency_bench_enabled() -> bool {
    std::env::var("SOOS_LATENCY_BENCH").is_ok_and(|v| v == "1")
}
