//! Benchmark tests for Criterion C2: fresh frame available in < 5ms via ArcSwap.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    reason = "Benchmark tests use assertions, panics, and indexing"
)]

use soos_camera_v4l::{CameraConfigBuilder, CameraManager, MockCameraManager};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn test_arcswap_frame_retrieval_latency_under_5ms() {
    let config = CameraConfigBuilder::new().warmup_frames(1).build();
    let camera = Arc::new(MockCameraManager::new(config));
    thread::sleep(Duration::from_millis(100));
    assert!(camera.is_ready());

    const ITERATIONS: usize = 10_000;
    let mut latencies_ns = Vec::with_capacity(ITERATIONS);

    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let frame = camera.latest_frame();
        let elapsed = start.elapsed();

        assert!(frame.is_some());
        latencies_ns.push(elapsed.as_nanos());
    }

    latencies_ns.sort_unstable();

    // 95th percentile index
    let p95_idx = (ITERATIONS * 95) / 100;
    let p95_latency_ns = latencies_ns[p95_idx];
    let p95_latency_ms = p95_latency_ns as f64 / 1_000_000.0;

    // Must be strictly < 5.0 ms (criterion C2), in practice lock-free ArcSwap is < 0.05 ms (50 µs)
    assert!(
        p95_latency_ms < 5.0,
        "p95 latency was {:.4} ms, must be < 5.0 ms",
        p95_latency_ms
    );

    camera.stop();
}

#[test]
fn test_lock_free_concurrent_access() {
    let config = CameraConfigBuilder::new().warmup_frames(1).build();
    let camera = Arc::new(MockCameraManager::new(config));
    thread::sleep(Duration::from_millis(100));
    assert!(camera.is_ready());

    let mut handles = Vec::new();
    const NUM_THREADS: usize = 8;
    const READS_PER_THREAD: usize = 5_000;

    for _ in 0..NUM_THREADS {
        let cam = Arc::clone(&camera);
        handles.push(thread::spawn(move || {
            for _ in 0..READS_PER_THREAD {
                let frame = cam.latest_frame();
                assert!(frame.is_some());
            }
        }));
    }

    for handle in handles {
        handle.join().expect("Reader thread panicked");
    }

    camera.stop();
}
