//! Build-dependent panic message policy (GitHub #287, owner decision 2026-10-01).
//!
//! The process panic hook logs the panic message only in debug builds
//! (`cfg!(debug_assertions)`); a release build keeps the behaviour of GitHub #259 and
//! withholds it (location only). The release path is also covered by
//! `panic_hook_tests::test_panic_hook_logs_location_via_tracing_without_payload`.
//!
//! The hook is process-global, so every test that installs it lives in one test function.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::io::Write;
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

use soos_daemon::shutdown::{
    install_panic_hook_with, PanicMessagePolicy, MAX_DEBUG_PANIC_MESSAGE_CHARS,
};

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Runs `body` (which panics) under a fresh capturing subscriber and returns the logs.
fn logs_of_panic(body: impl FnOnce() + std::panic::UnwindSafe) -> String {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_ansi(false)
        .finish();
    let outcome = tracing::subscriber::with_default(subscriber, || std::panic::catch_unwind(body));
    assert!(outcome.is_err(), "the body must panic");
    buffer.contents()
}

#[test]
fn test_panic_message_policy_follows_debug_assertions() {
    assert_eq!(
        PanicMessagePolicy::for_debug_assertions(false),
        PanicMessagePolicy::Withhold,
        "a release build (no debug assertions) must withhold the panic message"
    );
    assert_eq!(
        PanicMessagePolicy::for_debug_assertions(true),
        PanicMessagePolicy::LogMessage,
        "a debug build logs the panic message"
    );
    assert_eq!(
        PanicMessagePolicy::for_build(),
        PanicMessagePolicy::for_debug_assertions(cfg!(debug_assertions))
    );
    assert_eq!(
        MAX_DEBUG_PANIC_MESSAGE_CHARS, 512,
        "documented debug message bound"
    );
}

#[test]
fn test_debug_policy_logs_message_and_release_policy_withholds_it() {
    // Debug policy: formatted (String) and literal (&str) payloads are both logged.
    install_panic_hook_with(PanicMessagePolicy::LogMessage);
    let text = logs_of_panic(|| panic!("DFU1-DEBUG-FORMATTED {}", 7));
    assert!(
        text.contains("ERROR"),
        "panic must log at error level: {text}"
    );
    assert!(text.contains("soos-daemon panicked"), "{text}");
    assert!(
        text.contains("panic_hook_build_policy_tests.rs"),
        "the location stays logged: {text}"
    );
    assert!(
        text.contains("DFU1-DEBUG-FORMATTED 7"),
        "a debug build must log the panic message: {text}"
    );
    let text = logs_of_panic(|| panic!("DFU1-DEBUG-LITERAL"));
    assert!(text.contains("DFU1-DEBUG-LITERAL"), "{text}");

    // Debug policy: an oversized message is truncated to the documented bound.
    let long = "x".repeat(MAX_DEBUG_PANIC_MESSAGE_CHARS * 4);
    let text = logs_of_panic(move || panic!("{long}"));
    assert!(
        !text.contains(&"x".repeat(MAX_DEBUG_PANIC_MESSAGE_CHARS + 1)),
        "the logged panic message must be bounded"
    );
    assert!(text.contains(&"x".repeat(MAX_DEBUG_PANIC_MESSAGE_CHARS)));

    // Release policy: the message is withheld exactly as before (GitHub #259).
    install_panic_hook_with(PanicMessagePolicy::Withhold);
    let text = logs_of_panic(|| panic!("DFU1-RELEASE-SENSITIVE {}", 9));
    let _ = std::panic::take_hook();
    assert!(text.contains("ERROR"), "{text}");
    assert!(text.contains("soos-daemon panicked"), "{text}");
    assert!(text.contains("panic message withheld from logs"), "{text}");
    assert!(
        text.contains("panic_hook_build_policy_tests.rs"),
        "the location must be logged: {text}"
    );
    assert!(
        !text.contains("DFU1-RELEASE-SENSITIVE"),
        "a release build must never log the panic message: {text}"
    );
}

#[test]
fn test_production_main_selects_panic_policy_from_build_profile() {
    let main_rs = include_str!("../src/main.rs");
    assert!(
        main_rs.contains("PanicMessagePolicy::for_build()"),
        "soos-daemon must derive the panic message policy from the build profile"
    );
    assert!(
        main_rs.contains("install_panic_hook();"),
        "the release path must keep the payload-free hook of GitHub #259"
    );
}
