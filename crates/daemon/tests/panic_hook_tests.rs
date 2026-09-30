//! Process panic hook contract (GitHub #259, review finding DMN-19).
//!
//! `soos_daemon::shutdown::install_panic_hook` replaces the default stderr hook: a panic
//! anywhere in the daemon is reported through `tracing` at error level with its source
//! location only, never with the panic payload (which could carry request data).
//!
//! This file holds a single test because the panic hook is process-global.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::io::Write;
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

use soos_daemon::shutdown::install_panic_hook;

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

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

#[test]
fn test_panic_hook_logs_location_via_tracing_without_payload() {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_ansi(false)
        .finish();

    install_panic_hook();
    let outcome = tracing::subscriber::with_default(subscriber, || {
        std::panic::catch_unwind(|| {
            panic!("DMN19-HOOK-SENSITIVE-PAYLOAD {}", 42);
        })
    });
    let _ = std::panic::take_hook();
    assert!(outcome.is_err());

    let text = String::from_utf8_lossy(&buffer.0.lock().unwrap()).into_owned();
    assert!(
        text.contains("ERROR"),
        "panic must log at error level: {text}"
    );
    assert!(
        text.contains("soos-daemon panicked"),
        "panic must be reported through tracing: {text}"
    );
    assert!(
        text.contains("panic_hook_tests.rs"),
        "the panic location must be logged: {text}"
    );
    assert!(
        !text.contains("DMN19-HOOK-SENSITIVE-PAYLOAD"),
        "the panic payload must never be logged: {text}"
    );
}
