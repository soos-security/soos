//! Camera alias lookup and `v4l` guard follow-ups (GitHub #289, matrix rows CAG1-CAG5).
//!
//! - CAG1: the capture open path completes its hints with `supervisor_alias_hints` (the by-id
//!   alias of a plain `/dev/videoN` path) through the single bounded by-id scanner.
//! - CAG3: the guard silences the panic hook only for the panics it catches, with one wrapper
//!   hook installed once that chains to the previous hook.
//! - CAG4: the MMAP stream is created and dropped through the guard.
//! - CAG5: the decisions are recorded as an ADR.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};

const GUARD_RS: &str = "crates/camera-v4l/src/v4l_guard.rs";
const V4L_IMPL_RS: &str = "crates/camera-v4l/src/v4l_impl.rs";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Production part of a Rust source (before the first `#[cfg(test)]`), without `//` comments.
fn production_code(source: &str) -> String {
    let production = source
        .find("#[cfg(test)]")
        .map_or(source, |pos| &source[..pos]);
    production
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// CAG1: the open path looks up the alias after the single hint builder, without a scanner of
/// its own.
#[test]
fn test_cag_open_path_looks_up_the_by_id_alias() {
    let code = production_code(&read(V4L_IMPL_RS));
    assert!(
        code.contains("pub fn supervisor_alias_hints("),
        "{V4L_IMPL_RS} must define supervisor_alias_hints"
    );
    let builder = code
        .find("let hints = supervisor_sensor_hints(")
        .expect("open_and_stream builds its hints with supervisor_sensor_hints");
    let lookup = code
        .find("let hints = supervisor_alias_hints(")
        .expect("open_and_stream completes its hints with supervisor_alias_hints");
    assert!(
        lookup > builder,
        "the alias lookup completes the built hints"
    );
    assert!(
        code.contains("DEFAULT_BY_ID_DIR"),
        "the open path reads the udev by-id directory"
    );
    assert!(
        code.contains("by_id_aliases"),
        "the alias lookup goes through the single bounded by-id scanner"
    );
    assert!(
        !code.contains("read_dir"),
        "{V4L_IMPL_RS} must not scan a directory itself"
    );
}

/// CAG3: one wrapper hook, installed once, chaining to the previous hook, gated by a
/// thread-local flag.
#[test]
fn test_cag_guard_installs_one_chained_hook_filter() {
    let guard = production_code(&read(GUARD_RS));
    assert_eq!(
        guard.matches("take_hook()").count(),
        1,
        "{GUARD_RS} takes the previous hook exactly once"
    );
    assert_eq!(
        guard.matches("set_hook(").count(),
        1,
        "{GUARD_RS} installs exactly one wrapper hook"
    );
    for needle in [
        "call_once",
        "thread_local!",
        "previous(info)",
        "thread::panicking()",
    ] {
        assert!(guard.contains(needle), "{GUARD_RS} must contain `{needle}`");
    }
    for rel in [V4L_IMPL_RS, "crates/camera-v4l/src/sensor.rs"] {
        let code = production_code(&read(rel));
        assert!(
            !code.contains("set_hook(") && !code.contains("take_hook("),
            "{rel} must not touch the process panic hook"
        );
    }
}

/// CAG4: the MMAP stream is created and torn down only through the guard.
#[test]
fn test_cag_stream_create_and_teardown_through_the_guard() {
    let guard = production_code(&read(GUARD_RS));
    for needle in [
        "pub fn guarded_v4l_drop<",
        "pub(crate) fn mmap_stream_guarded",
    ] {
        assert!(guard.contains(needle), "{GUARD_RS} must define `{needle}`");
    }
    let code = production_code(&read(V4L_IMPL_RS));
    assert!(
        !code.contains("Stream::with_buffers("),
        "{V4L_IMPL_RS} must create the stream with mmap_stream_guarded"
    );
    assert!(
        code.contains("mmap_stream_guarded("),
        "{V4L_IMPL_RS} must use mmap_stream_guarded"
    );
    assert!(
        code.contains("guarded_v4l_drop(source)"),
        "{V4L_IMPL_RS} must drop the capture source through the guard"
    );
    assert!(
        code.contains("CameraError::StreamTeardown"),
        "a teardown panic maps to CameraError::StreamTeardown"
    );
}

/// CAG5: the decisions are recorded.
#[test]
fn test_cag_decision_recorded() {
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions
            .contains("By-Id Alias Lookup for Plain Capture Nodes; Silent, Guarded v4l Teardown"),
        "AI/DECISIONS.md must record the camera alias / v4l guard decision"
    );
}
