//! Fail-closed startup check of the linked ONNX Runtime (GitHub #375, PR 1 of #361; ADR
//! 2026-10-13 "Distribution Builds: System ONNX Runtime"; matrix SORT1–SORT6).
//!
//! The system build (`--no-default-features --features system-onnxruntime`) links the
//! distribution `libonnxruntime` instead of the SHA-256-pinned prebuilt archive, so the daemon
//! checks at start that the library answers the C API handshake and lists the CPU execution
//! provider. `check_cpu_execution_provider` is the pure decision (injectable probe);
//! `verify_cpu_execution_provider` runs it against the linked library: the downloaded binary in
//! the normal CI jobs, the Arch `onnxruntime-cpu` library in the `system-onnxruntime` job.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_inference_ort::runtime::execution_provider_listed;
use soos_inference_ort::{
    check_cpu_execution_provider, verify_cpu_execution_provider, InferenceError,
    CPU_EXECUTION_PROVIDER, MAX_RUNTIME_ERROR_DETAIL_BYTES,
};

/// Fixed prefix the check adds in front of a caught panic payload (spec §2).
const PANIC_PREFIX: &str = "ONNX Runtime initialization panicked";

fn detail_of(result: Result<(), InferenceError>) -> String {
    match result {
        Err(InferenceError::CpuExecutionProviderUnavailable(detail)) => detail,
        other => panic!("expected CpuExecutionProviderUnavailable, got {other:?}"),
    }
}

/// SORT6: the provider name is the exact ONNX Runtime identifier and the detail bound is 256.
#[test]
fn test_sort_constants() {
    assert_eq!(CPU_EXECUTION_PROVIDER, "CPUExecutionProvider");
    assert_eq!(MAX_RUNTIME_ERROR_DETAIL_BYTES, 256);
}

/// SORT1: a library that lists the CPU provider passes.
#[test]
fn test_sort_check_accepts_listed_cpu_provider() {
    assert!(check_cpu_execution_provider(|| Ok(true)).is_ok());
}

/// SORT2: a library that does not list the CPU provider fails closed (never a default pass).
#[test]
fn test_sort_check_rejects_unlisted_cpu_provider() {
    let detail = detail_of(check_cpu_execution_provider(|| Ok(false)));
    assert!(
        detail.contains(CPU_EXECUTION_PROVIDER),
        "the detail must name the missing provider: {detail}"
    );
    let shown = InferenceError::CpuExecutionProviderUnavailable(detail).to_string();
    assert!(
        shown.starts_with("ONNX Runtime CPU execution provider unavailable: "),
        "{shown}"
    );
}

/// SORT3: an ORT error fails closed and its text is kept, bounded on a character boundary.
#[test]
fn test_sort_check_rejects_probe_error_with_bounded_detail() {
    let short = detail_of(check_cpu_execution_provider(|| {
        Err("GetAvailableProviders failed".to_string())
    }));
    assert_eq!(short, "GetAvailableProviders failed");

    // 'é' is two bytes: an odd bound position must not split it.
    let long: String = std::iter::repeat_n('é', 5_000).collect();
    assert_eq!(long.len(), 10_000);
    let detail = detail_of(check_cpu_execution_provider(move || Err(long)));
    assert!(
        detail.len() <= MAX_RUNTIME_ERROR_DETAIL_BYTES,
        "detail is {} bytes, bound {MAX_RUNTIME_ERROR_DETAIL_BYTES}",
        detail.len()
    );
    assert!(
        detail.len() > MAX_RUNTIME_ERROR_DETAIL_BYTES - 2,
        "the cut keeps the largest prefix that fits: {} bytes",
        detail.len()
    );
    assert!(detail.chars().all(|c| c == 'é'), "valid UTF-8 prefix only");

    let empty = detail_of(check_cpu_execution_provider(|| Err(String::new())));
    assert!(
        empty.is_empty(),
        "an empty ORT error stays empty: {empty:?}"
    );
}

/// SORT4: the `ort` handshake panics on a library older than the compiled API level
/// (`expect("Failed to initialize ORT API")`); the check catches every panic payload type and
/// fails closed instead of propagating it.
#[test]
fn test_sort_check_catches_handshake_panic() {
    let from_str = detail_of(check_cpu_execution_provider(|| {
        panic!("Failed to initialize ORT API")
    }));
    assert!(from_str.starts_with(PANIC_PREFIX), "{from_str}");
    assert!(
        from_str.contains("Failed to initialize ORT API"),
        "{from_str}"
    );

    let from_string = detail_of(check_cpu_execution_provider(|| {
        std::panic::panic_any(format!("handshake {}", 17))
    }));
    assert!(from_string.starts_with(PANIC_PREFIX), "{from_string}");
    assert!(from_string.contains("handshake 17"), "{from_string}");

    let opaque = detail_of(check_cpu_execution_provider(|| {
        std::panic::panic_any(42_u32)
    }));
    assert_eq!(opaque, PANIC_PREFIX);

    let huge: String = "x".repeat(4 * MAX_RUNTIME_ERROR_DETAIL_BYTES);
    let bounded = detail_of(check_cpu_execution_provider(move || {
        std::panic::panic_any(huge)
    }));
    assert!(bounded.starts_with(PANIC_PREFIX), "{bounded}");
    assert!(
        bounded.len() <= MAX_RUNTIME_ERROR_DETAIL_BYTES,
        "panic detail is {} bytes, bound {MAX_RUNTIME_ERROR_DETAIL_BYTES}",
        bounded.len()
    );
}

/// SORT5: the library this test binary is linked against answers the handshake and lists
/// `CPUExecutionProvider` (both the default and the system build run this test).
#[test]
fn test_sort_linked_runtime_lists_cpu_provider() {
    verify_cpu_execution_provider().expect("the linked ONNX Runtime must list the CPU provider");
    // Idempotent: the daemon may call it again after a restart of the pipeline.
    verify_cpu_execution_provider().expect("second call must also pass");
}

/// SORT9: the probe really asks the linked library (`GetAvailableProviders`) by exact name: a
/// constant answer (such as `ort::ep::CPU::is_available`, which never asks) fails this test.
#[test]
fn test_sort_provider_probe_asks_the_linked_library() {
    assert_eq!(execution_provider_listed(CPU_EXECUTION_PROVIDER), Ok(true));
    assert_eq!(
        execution_provider_listed("SoosNoSuchExecutionProvider"),
        Ok(false),
        "an unknown provider must not be reported as available"
    );
    assert_eq!(
        execution_provider_listed("cpuexecutionprovider"),
        Ok(false),
        "the match is exact and case-sensitive"
    );
}
