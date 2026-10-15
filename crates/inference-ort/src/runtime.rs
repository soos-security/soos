//! Fail-closed start-up check of the linked ONNX Runtime (GitHub #375, ADR 2026-10-13
//! "Distribution Builds: System ONNX Runtime").
//!
//! The distribution build (`--no-default-features --features system-onnxruntime`) links the
//! distribution `libonnxruntime` at C API level 17 instead of the SHA-256-pinned archive that
//! `ort-sys` downloads by default. Before anything else, the daemon checks that the linked
//! library answers the C API handshake and lists the CPU execution provider, the only provider
//! soos uses. Any other outcome is an error and the daemon refuses to start.

use std::any::Any;
use std::panic::{catch_unwind, UnwindSafe};

use ort::ep::ExecutionProvider;
use ort::session::builder::SessionBuilder;

use crate::error::InferenceError;

/// ONNX Runtime name of the CPU execution provider (the only provider soos uses, ADR 2026-09-12).
pub const CPU_EXECUTION_PROVIDER: &str = "CPUExecutionProvider";

/// Upper bound, in bytes, of the detail kept in
/// [`InferenceError::CpuExecutionProviderUnavailable`] (an ORT error string or a panic
/// payload); longer text is truncated on a UTF-8 character boundary.
pub const MAX_RUNTIME_ERROR_DETAIL_BYTES: usize = 256;

/// Prefix of the detail of a caught panic of the ORT handshake.
const PANIC_PREFIX: &str = "ONNX Runtime initialization panicked";

/// Never registered: exists only so the default [`ExecutionProvider::is_available`] asks the
/// linked library (`GetAvailableProviders`, exact case-sensitive name) whether the provider
/// named `.0` is available. `ort`'s own CPU provider type answers `true` without asking the
/// library and cannot be used. `is_available` is deliberately not overridden.
struct ListedProviderProbe(&'static str);

impl ExecutionProvider for ListedProviderProbe {
    fn name(&self) -> &'static str {
        self.0
    }

    fn register(&self, _session_builder: &mut SessionBuilder) -> ort::Result<()> {
        Ok(())
    }
}

/// Fail-closed start-up check of the linked ONNX Runtime: the library must answer the C API
/// handshake at the compiled API level and list `CPUExecutionProvider` among its available
/// providers. Never registers a provider and never creates a session.
pub fn verify_cpu_execution_provider() -> Result<(), InferenceError> {
    check_cpu_execution_provider(|| execution_provider_listed(CPU_EXECUTION_PROVIDER))
}

/// Whether the linked ONNX Runtime lists the execution provider `name` (exact, case-sensitive)
/// among its available providers; an ORT error is returned as its text. May panic inside `ort`
/// when the library fails the C API handshake: production code calls it only through
/// [`verify_cpu_execution_provider`], which catches that panic.
#[doc(hidden)]
pub fn execution_provider_listed(name: &'static str) -> Result<bool, String> {
    ListedProviderProbe(name)
        .is_available()
        .map_err(|err| err.to_string())
}

/// Pure decision behind [`verify_cpu_execution_provider`], injectable for tests.
///
/// `probe` returns whether the CPU provider is listed (`Ok(true)`), not listed (`Ok(false)`)
/// or an ORT error (`Err(text)`). A panic inside `probe` (the `ort` handshake `expect` on a
/// library older than the compiled API level) is caught and is a failure. Only `Ok(true)`
/// passes.
pub fn check_cpu_execution_provider<F>(probe: F) -> Result<(), InferenceError>
where
    F: FnOnce() -> Result<bool, String> + UnwindSafe,
{
    match catch_unwind(probe) {
        Ok(Ok(true)) => Ok(()),
        Ok(Ok(false)) => Err(InferenceError::CpuExecutionProviderUnavailable(format!(
            "{CPU_EXECUTION_PROVIDER} is not listed by the linked ONNX Runtime"
        ))),
        Ok(Err(text)) => Err(InferenceError::CpuExecutionProviderUnavailable(bounded(
            &text,
        ))),
        Err(payload) => Err(InferenceError::CpuExecutionProviderUnavailable(
            panic_detail(payload.as_ref()),
        )),
    }
}

/// Separator between [`PANIC_PREFIX`] and a panic payload.
const PANIC_SEPARATOR: &str = ": ";

/// Detail of a caught panic: the fixed prefix plus a `&str` / `String` payload, the whole
/// detail bounded by [`MAX_RUNTIME_ERROR_DETAIL_BYTES`]. The payload is truncated before it is
/// joined, so no allocation exceeds the bound.
fn panic_detail(payload: &(dyn Any + Send)) -> String {
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
    match message {
        Some(message) => {
            let budget = MAX_RUNTIME_ERROR_DETAIL_BYTES
                .saturating_sub(PANIC_PREFIX.len())
                .saturating_sub(PANIC_SEPARATOR.len());
            let mut detail = String::with_capacity(MAX_RUNTIME_ERROR_DETAIL_BYTES);
            detail.push_str(PANIC_PREFIX);
            detail.push_str(PANIC_SEPARATOR);
            detail.push_str(prefix_within(message, budget));
            detail
        }
        None => PANIC_PREFIX.to_string(),
    }
}

/// At most [`MAX_RUNTIME_ERROR_DETAIL_BYTES`] bytes of `text`, cut at the largest character
/// boundary that fits.
fn bounded(text: &str) -> String {
    prefix_within(text, MAX_RUNTIME_ERROR_DETAIL_BYTES).to_string()
}

/// The longest prefix of `text` of at most `max_bytes` bytes that ends on a character boundary.
fn prefix_within(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.get(..end).unwrap_or_default()
}
