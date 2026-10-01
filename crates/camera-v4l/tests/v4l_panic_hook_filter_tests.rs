//! Panic-hook filter of the `v4l` guard (GitHub #289, matrix row CAG3).
//!
//! A panic caught by `guard_v4l_call` is an expected, recovered condition (malformed driver
//! metadata): it must not reach the process panic hook (the daemon would log a panic location,
//! the admin CLI would print Rust's default panic line on stderr). Every other panic, on any
//! thread, must still reach the hook that was installed before. The process panic hook is
//! global, so this binary holds a single test.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stderr,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use soos_camera_v4l::v4l_guard::{guard_v4l_call, V4L_PANIC_MESSAGE};
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

static REPORTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn reported() -> Vec<String> {
    REPORTED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

#[test]
fn test_cag_guarded_panics_are_silent_and_other_panics_reach_the_previous_hook() {
    // The "previous" hook, installed before any guarded call (as the daemon does at start-up).
    // It also prints every report, so a failed assertion of this test stays visible.
    panic::set_hook(Box::new(|info| {
        let text = payload_text(info.payload());
        eprintln!("previous hook: {text}");
        REPORTED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(text);
    }));

    // 1. A guarded panic is converted and produces no hook output.
    let err = guard_v4l_call(|| -> u8 { panic!("guarded-first") }).unwrap_err();
    assert_eq!(err.to_string(), V4L_PANIC_MESSAGE);
    assert!(
        reported().is_empty(),
        "guarded panic reached the hook: {:?}",
        reported()
    );

    // 2. An unguarded panic on another thread still reaches the previous hook.
    let joined = thread::spawn(|| panic!("unguarded-other-thread")).join();
    assert!(joined.is_err());
    assert_eq!(reported(), vec!["unguarded-other-thread".to_string()]);

    // 3. An unguarded panic on another thread *while* this thread is inside a guarded call is
    //    still reported (the "inside the guard" flag is per thread), and the guarded panic that
    //    follows is not.
    let inside = Arc::new(Barrier::new(2));
    let other_done = Arc::new(Barrier::new(2));
    let guarded = {
        let inside = Arc::clone(&inside);
        let other_done = Arc::clone(&other_done);
        thread::spawn(move || {
            guard_v4l_call(|| -> u8 {
                inside.wait();
                other_done.wait();
                panic!("guarded-concurrent")
            })
        })
    };
    inside.wait();
    let concurrent = thread::spawn(|| panic!("unguarded-concurrent")).join();
    assert!(concurrent.is_err());
    other_done.wait();
    let guarded_result = guarded.join().expect("the guard catches the panic");
    assert_eq!(guarded_result.unwrap_err().to_string(), V4L_PANIC_MESSAGE);
    assert_eq!(
        reported(),
        vec![
            "unguarded-other-thread".to_string(),
            "unguarded-concurrent".to_string()
        ]
    );

    // 4. Nested guards: the inner panic is silent, the outer call still sees the flag set, and
    //    the flag is cleared once the outermost guard returns.
    let nested = guard_v4l_call(|| {
        let inner = guard_v4l_call(|| -> u8 { panic!("guarded-inner") });
        assert!(inner.is_err());
        panic!("guarded-outer")
    });
    assert!(nested.is_err());
    assert_eq!(reported().len(), 2, "nested guarded panics are silent");

    // 5. After the guard returned, an unguarded panic on this same thread is reported again
    //    (the flag never leaks past the guarded call).
    let same_thread = panic::catch_unwind(AssertUnwindSafe(|| panic!("unguarded-same-thread")));
    assert!(same_thread.is_err());
    assert_eq!(
        reported().last().map(String::as_str),
        Some("unguarded-same-thread")
    );

    // 6. A call that does not panic is passed through and reports nothing.
    assert_eq!(guard_v4l_call(|| 7_u8).unwrap(), 7);
    assert_eq!(reported().len(), 3);
}
