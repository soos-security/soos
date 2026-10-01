//! GitHub #289 (MLS7, MLS8): the shutdown drain budget bounds process exit.
//!
//! An evidence write still running when the drain budget expires is abandoned: it is no
//! longer awaited. Dropping a Tokio runtime waits for every `spawn_blocking` job, so
//! `soos-daemon` shuts its runtime down with `shutdown_runtime(runtime, remaining)`
//! (`Runtime::shutdown_timeout`) instead, with what is left of the one-`connection_timeout`
//! budget.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use soos_daemon::shutdown::{remaining_budget, shutdown_runtime, BlockingTasks};

#[test]
fn test_mls_remaining_budget_is_what_is_left_of_the_budget() {
    let started = Instant::now();
    let budget = Duration::from_millis(1000);
    assert_eq!(remaining_budget(started, budget, started), budget);
    assert_eq!(
        remaining_budget(started, budget, started + Duration::from_millis(400)),
        Duration::from_millis(600)
    );
    assert_eq!(
        remaining_budget(started, budget, started + budget),
        Duration::ZERO
    );
    assert_eq!(
        remaining_budget(started, budget, started + Duration::from_secs(60)),
        Duration::ZERO,
        "an exhausted budget saturates at zero"
    );
}

#[test]
fn test_mls_remaining_budget_never_exceeds_the_budget() {
    let now = Instant::now();
    let started = now + Duration::from_secs(5);
    assert_eq!(
        remaining_budget(started, Duration::from_millis(250), now),
        Duration::from_millis(250),
        "a start in the future never grants more than the budget"
    );
    assert_eq!(remaining_budget(now, Duration::ZERO, now), Duration::ZERO);
}

#[test]
fn test_mls_shutdown_runtime_does_not_wait_for_an_abandoned_blocking_write() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let finished = Arc::new(AtomicBool::new(false));
    let tasks = BlockingTasks::new();

    let report = runtime.block_on(async {
        let finished = Arc::clone(&finished);
        tasks.spawn_blocking(move || {
            std::thread::sleep(Duration::from_secs(4));
            finished.store(true, Ordering::SeqCst);
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        tasks.drain(Duration::from_millis(50)).await
    });
    assert_eq!(report.aborted, 1, "the slow write is abandoned");

    let started = Instant::now();
    shutdown_runtime(runtime, Duration::from_millis(100));
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(2000),
        "process exit must be bounded by the remaining budget, took {elapsed:?}"
    );
    assert!(
        !finished.load(Ordering::SeqCst),
        "the abandoned write was not awaited"
    );
}

#[test]
fn test_mls_shutdown_runtime_lets_a_fast_blocking_job_finish() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let finished = Arc::new(AtomicBool::new(false));
    {
        let finished = Arc::clone(&finished);
        let _detached = runtime.spawn_blocking(move || {
            std::thread::sleep(Duration::from_millis(100));
            finished.store(true, Ordering::SeqCst);
        });
    }

    shutdown_runtime(runtime, Duration::from_secs(5));
    assert!(
        finished.load(Ordering::SeqCst),
        "a blocking job that ends within the remaining budget is awaited"
    );
}

#[test]
fn test_mls_production_main_bounds_runtime_shutdown() {
    let main_rs = include_str!("../src/main.rs");
    assert!(
        !main_rs.contains("#[tokio::main]"),
        "#[tokio::main] drops the runtime, which waits for every blocking job"
    );
    let drain = main_rs
        .find("evidence_writes().drain(")
        .expect("main.rs drains the tracked evidence writes");
    let shutdown = main_rs
        .find("shutdown_runtime(")
        .expect("main.rs must shut the runtime down within the remaining budget");
    assert!(
        drain < shutdown,
        "the runtime is shut down after the evidence drain"
    );
    assert!(
        main_rs.contains("remaining_budget("),
        "the runtime shutdown uses what is left of the drain budget"
    );
}
