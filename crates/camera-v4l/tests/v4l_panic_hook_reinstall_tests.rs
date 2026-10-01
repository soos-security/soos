//! Re-installation of the `v4l` panic-hook filter after a host replaces the hook (GitHub #291,
//! matrix row CDF3).
//!
//! A host that installs its own panic hook after the first guarded call replaces the filter, so
//! caught `v4l` panics are reported again. Calling `install_v4l_panic_hook_filter()` right
//! after installing the host hook puts the filter back on top of it: guarded panics are silent
//! again, every other panic is still reported exactly once by the host hook, and calling it
//! again while the filter is already the current hook never wraps it a second time. The process
//! panic hook is global, so this binary holds a single test.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses assertions, unwrap, panics and slicing"
)]

use soos_camera_v4l::v4l_guard::{
    guard_v4l_call, install_v4l_panic_hook_filter, v4l_panic_hook_filter_installations,
    V4L_PANIC_MESSAGE,
};
use std::panic;
use std::sync::Mutex;
use std::thread;

static REPORTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn reported() -> Vec<String> {
    REPORTED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn record(text: String) {
    eprintln!("hook: {text}");
    REPORTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(text);
}

fn payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

fn guarded_panic(message: &'static str) {
    let err = guard_v4l_call(move || -> u8 { panic!("{message}") }).unwrap_err();
    assert_eq!(err.to_string(), V4L_PANIC_MESSAGE);
}

fn unguarded_panic_on_another_thread(message: &'static str) {
    assert!(thread::spawn(move || panic!("{message}")).join().is_err());
}

#[test]
fn test_cdf_filter_is_reinstalled_on_top_of_a_later_host_hook_without_double_wrapping() {
    // 1. Hook A before any guarded call: the first guarded call wraps it once.
    panic::set_hook(Box::new(|info| {
        record(format!("A:{}", payload_text(info.payload())))
    }));
    assert_eq!(v4l_panic_hook_filter_installations(), 0);
    guarded_panic("guarded-1");
    assert_eq!(v4l_panic_hook_filter_installations(), 1);
    assert!(reported().is_empty(), "{:?}", reported());

    // 2. The host replaces the hook (the filter is dropped): caught panics are reported again.
    panic::set_hook(Box::new(|info| {
        record(format!("B:{}", payload_text(info.payload())))
    }));
    guarded_panic("guarded-2");
    assert_eq!(reported(), vec!["B:guarded-2".to_string()]);

    // 3. The explicit call puts the filter back on top of B.
    install_v4l_panic_hook_filter();
    assert_eq!(v4l_panic_hook_filter_installations(), 2);
    guarded_panic("guarded-3");
    unguarded_panic_on_another_thread("unguarded-3");
    assert_eq!(
        reported(),
        vec!["B:guarded-2".to_string(), "B:unguarded-3".to_string()],
        "guarded panics are silent again, other panics reach B exactly once"
    );

    // 4. Calling it again while the filter is the current hook never wraps it twice, and later
    //    guarded calls do not wrap either.
    install_v4l_panic_hook_filter();
    install_v4l_panic_hook_filter();
    guarded_panic("guarded-4");
    assert_eq!(v4l_panic_hook_filter_installations(), 2);
    unguarded_panic_on_another_thread("unguarded-4");
    assert_eq!(reported().last().map(String::as_str), Some("B:unguarded-4"));
    assert_eq!(
        reported().len(),
        3,
        "reported exactly once: {:?}",
        reported()
    );

    // 5. A host hook that chains to the filter (take_hook, then calls it) reports the caught
    //    panics itself; the explicit call wraps that chain once more, and every other panic
    //    still reaches both C and B exactly once.
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        record(format!("C:{}", payload_text(info.payload())));
        previous(info);
    }));
    guarded_panic("guarded-5");
    assert_eq!(
        reported().last().map(String::as_str),
        Some("C:guarded-5"),
        "the outer host hook sees the caught panic before the filter"
    );
    install_v4l_panic_hook_filter();
    assert_eq!(v4l_panic_hook_filter_installations(), 3);
    let before = reported().len();
    guarded_panic("guarded-6");
    assert_eq!(reported().len(), before, "{:?}", reported());
    unguarded_panic_on_another_thread("unguarded-6");
    assert_eq!(
        reported()[before..].to_vec(),
        vec!["C:unguarded-6".to_string(), "B:unguarded-6".to_string()]
    );
    install_v4l_panic_hook_filter();
    assert_eq!(v4l_panic_hook_filter_installations(), 3);
}
