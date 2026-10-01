//! Camera diagnostics invariants (GitHub #256 / CAM-17, matrix row CDX10).
//!
//! `soos-admin camera list|probe` is a non-biometric support tool. It may only issue V4L2
//! metadata queries (`VIDIOC_QUERYCAP`, `VIDIOC_ENUM_FMT`, `VIDIOC_ENUM_FRAMESIZES`); it must
//! never negotiate a format, start a stream or read a frame, and it must report the decision of
//! the single shared resolver instead of re-implementing the selection or the directory scans.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};

const DIAGNOSTICS_RS: &str = "crates/camera-v4l/src/diagnostics.rs";
const ADMIN_CAMERA_RS: &str = "crates/admin-cli/src/camera.rs";

/// Identifiers that would stream frames, change the device format or open the capture path.
const STREAMING_TOKENS: [&str; 10] = [
    "MmapStream",
    "UserptrStream",
    "with_buffers",
    "V4lCameraManager",
    "MockCameraManager",
    "open_and_stream",
    "set_format",
    "set_params",
    "latest_frame",
    "Frame {",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// Returns the source text before the first `#[cfg(test)]` (the production part).
fn production(rel: &str) -> String {
    let text = fs::read_to_string(workspace_root().join(rel))
        .unwrap_or_else(|e| panic!("read {rel}: {e}"));
    match text.find("#[cfg(test)]") {
        Some(pos) => text[..pos].to_string(),
        None => text,
    }
}

fn admin_cli_sources() -> Vec<(String, String)> {
    let dir = workspace_root().join("crates/admin-cli/src");
    let mut files: Vec<(String, String)> = fs::read_dir(&dir)
        .expect("read crates/admin-cli/src")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .map(|p| {
            let rel = format!(
                "crates/admin-cli/src/{}",
                p.file_name().unwrap().to_string_lossy()
            );
            let text = production(&rel);
            (rel, text)
        })
        .collect();
    files.sort();
    files
}

#[test]
fn test_cdx_camera_diagnostics_never_stream_frames() {
    let mut scanned = vec![(DIAGNOSTICS_RS.to_string(), production(DIAGNOSTICS_RS))];
    scanned.extend(admin_cli_sources());
    assert!(
        scanned.iter().any(|(rel, _)| rel == ADMIN_CAMERA_RS),
        "{ADMIN_CAMERA_RS} must exist"
    );
    let mut violations = Vec::new();
    for (rel, text) in &scanned {
        for token in STREAMING_TOKENS {
            if text.contains(token) {
                violations.push(format!("{rel} references `{token}`"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "camera diagnostics must stay metadata-only (QUERYCAP / ENUM_FMT / ENUM_FRAMESIZES):\n{}",
        violations.join("\n")
    );
}

#[test]
fn test_cdx_camera_diagnostics_use_the_shared_resolver_and_scanners() {
    let diagnostics = production(DIAGNOSTICS_RS);
    assert!(
        diagnostics.contains("explain_camera_resolution"),
        "diagnostics must report the shared resolver's decision"
    );
    assert!(
        !diagnostics.contains("select_camera_device"),
        "diagnostics must not select a camera on their own"
    );
    assert!(
        !diagnostics.contains("read_dir"),
        "diagnostics must reuse the bounded sysfs and by-id scanners"
    );

    let admin = production(ADMIN_CAMERA_RS);
    for needle in [
        "collect_camera_diagnostics",
        "probe_camera_node",
        "by_id_aliases",
    ] {
        assert!(
            admin.contains(needle),
            "{ADMIN_CAMERA_RS} must use `{needle}`"
        );
    }
    for forbidden in ["read_dir", "select_camera_device", "classify_sensor"] {
        assert!(
            !admin.contains(forbidden),
            "{ADMIN_CAMERA_RS} must not use `{forbidden}` (shared camera-v4l code only)"
        );
    }
    assert!(
        !uses_raw_v4l_crate(&admin),
        "{ADMIN_CAMERA_RS} must not call the `v4l` crate directly (only `soos_camera_v4l`)"
    );
    assert!(
        !fs::read_to_string(workspace_root().join("crates/admin-cli/Cargo.toml"))
            .expect("read crates/admin-cli/Cargo.toml")
            .lines()
            .any(|l| l.trim_start().starts_with("v4l")),
        "soos-admin-cli must not depend on the `v4l` crate directly"
    );
}

/// Returns `true` when `text` names the `v4l` crate itself (`v4l::` not preceded by an
/// identifier character, so `soos_camera_v4l::` does not count).
fn uses_raw_v4l_crate(text: &str) -> bool {
    text.match_indices("v4l::").any(|(pos, _)| {
        text[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
    })
}

#[test]
fn test_cdx_raw_v4l_detector_self_test() {
    assert!(uses_raw_v4l_crate("use v4l::Device;"));
    assert!(uses_raw_v4l_crate("let d = ::v4l::Device::new(0);"));
    assert!(!uses_raw_v4l_crate("use soos_camera_v4l::diagnostics;"));
    assert!(!uses_raw_v4l_crate("no match here"));
}
