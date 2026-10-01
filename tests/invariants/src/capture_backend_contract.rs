//! Capture-backend seam below the V4L2 supervisor (GitHub #198, CAM-16, matrix rows CCB11–CCB12).
//!
//! - CCB11: the supervisor reaches the device only through `CaptureBackend` / `CaptureDevice`;
//!   both public spawn functions use the production `V4lBackend`, which is the only place that
//!   opens a `v4l::Device`, and the fake backend exists only under `#[cfg(test)]`.
//! - CCB12: the decision is recorded as an ADR.
//! - CCB14: frame-size enumeration goes through the guarded, bounded wrapper only.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Architectural invariant test runner utilizes direct assertions and panics"
)]

use std::fs;
use std::path::{Path, PathBuf};

const V4L_IMPL_RS: &str = "crates/camera-v4l/src/v4l_impl.rs";
const SUPERVISOR_TESTS_RS: &str = "crates/camera-v4l/src/v4l_impl/supervisor_tests.rs";

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

/// CCB11: one seam, one production backend, the fake confined to the test module.
#[test]
fn test_ccb_supervisor_reaches_the_device_only_through_the_backend_seam() {
    let source = read(V4L_IMPL_RS);
    let code = production_code(&source);
    for needle in [
        "trait CaptureBackend",
        "trait CaptureDevice",
        "struct V4lBackend",
        "impl CaptureBackend for V4lBackend",
        "impl CaptureDevice for v4l::Device",
        "fn supervise<B: CaptureBackend>(",
        "fn open_and_stream<B: CaptureBackend>(",
        "Self::spawn_inner(config, None, V4lBackend)",
        "Self::spawn_inner(config, Some(resolver), V4lBackend)",
    ] {
        assert!(
            code.contains(needle),
            "{V4L_IMPL_RS} must contain `{needle}`"
        );
    }
    assert_eq!(
        code.matches("v4l::Device::with_path(").count(),
        1,
        "only V4lBackend opens a v4l::Device"
    );
    assert!(
        !code.contains("Fake"),
        "no fake backend in the production part of {V4L_IMPL_RS}"
    );
    assert!(
        source.contains("#[cfg(test)]\nmod supervisor_tests;"),
        "the supervisor tests are a #[cfg(test)] module"
    );
    let tests = read(SUPERVISOR_TESTS_RS);
    assert!(
        tests.contains("impl CaptureBackend for FakeBackend"),
        "{SUPERVISOR_TESTS_RS} must drive the supervisor through a fake backend"
    );
}

/// CCB12: the decision is recorded.
#[test]
fn test_ccb_decision_recorded() {
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.contains("Capture Backend Seam Below the V4L2 Supervisor"),
        "AI/DECISIONS.md must record the capture-backend seam decision"
    );
}

/// CCB14: `VIDIOC_ENUM_FRAMESIZES` is issued only through the guarded, bounded wrapper (extends
/// CVF4, whose list of raw calls is unchanged): `v4l`'s own `enum_framesizes` loops until the
/// driver returns an error, so it is never called anywhere in the crate.
#[test]
fn test_ccb_frame_size_enumeration_is_guarded_and_bounded() {
    const GUARD_RS: &str = "crates/camera-v4l/src/v4l_guard.rs";
    let dir = workspace_root().join("crates/camera-v4l/src");
    let mut scanned = 0usize;
    let mut violations = Vec::new();
    for entry in fs::read_dir(&dir).expect("read crates/camera-v4l/src") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let rel = format!(
            "crates/camera-v4l/src/{}",
            path.file_name().unwrap().to_string_lossy()
        );
        scanned += 1;
        if production_code(&read(&rel)).contains("enum_framesizes(") {
            violations.push(rel);
        }
    }
    assert!(
        scanned >= 10,
        "the source scan is vacuous ({scanned} files)"
    );
    assert!(
        violations.is_empty(),
        "unbounded v4l enum_framesizes called in: {violations:?}"
    );

    let guard = production_code(&read(GUARD_RS));
    for needle in [
        "pub(crate) fn enumerate_indexed_bounded",
        "pub(crate) fn enum_framesizes_guarded",
        "VIDIOC_ENUM_FRAMESIZES",
        "guard_v4l_call(",
    ] {
        assert!(guard.contains(needle), "{GUARD_RS} must contain `{needle}`");
    }
    let sensor = production_code(&read("crates/camera-v4l/src/sensor.rs"));
    assert!(
        sensor.contains("enum_framesizes_guarded(") && sensor.contains("MAX_FRAME_SIZE_HINTS"),
        "device_frame_sizes must use the guarded wrapper with the MAX_FRAME_SIZE_HINTS bound"
    );
}
