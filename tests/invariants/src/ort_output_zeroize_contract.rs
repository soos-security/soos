//! ORT output wiping invariants (GitHub #255, review finding VIS-13).
//!
//! Every production `Session::run` in `soos-inference-ort` must hand its `SessionOutputs`
//! straight to `ZeroizingOutputs::new`, which overwrites every ORT-owned `f32` output tensor
//! in place before the allocation is released. Consuming the outputs by value
//! (`outputs.into_iter()`) would move the tensors out of the guard, and copying an extracted
//! output slice (`slice.to_vec()`) would duplicate raw detector output outside it; both are
//! forbidden in the inference sources.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};

/// Inference sources that run an ORT session.
const RUN_SITES: [&str; 3] = [
    "crates/inference-ort/src/detector.rs",
    "crates/inference-ort/src/embedding.rs",
    "crates/inference-ort/src/pad.rs",
];

/// Guard implementation file.
const GUARD_FILE: &str = "crates/inference-ort/src/outputs.rs";

fn workspace_root() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    Path::new(&manifest_dir)
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Source without `//` comment lines, so that documentation cannot satisfy the scan.
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn test_every_ort_session_run_is_wrapped_in_zeroizing_outputs() {
    for rel in RUN_SITES {
        let code = code_only(&read(rel));
        let runs = code.matches(".run(ort::inputs!").count();
        let guards = code.matches("ZeroizingOutputs::new(").count();
        assert!(runs > 0, "{rel}: expected at least one ORT session run");
        assert_eq!(
            runs, guards,
            "{rel}: every `.run(ort::inputs!` must be wrapped in `ZeroizingOutputs::new(` \
             ({runs} runs, {guards} guards)"
        );
    }
}

#[test]
fn test_ort_outputs_are_never_moved_out_or_copied() {
    for rel in RUN_SITES {
        let code = code_only(&read(rel));
        for forbidden in ["outputs.into_iter()", "slice.to_vec()"] {
            assert!(
                !code.contains(forbidden),
                "{rel}: `{forbidden}` moves or copies ORT-owned output tensors out of the wipe guard"
            );
        }
    }
}

#[test]
fn test_zeroizing_outputs_guard_wipes_in_place_on_drop() {
    let code = code_only(&read(GUARD_FILE));
    assert!(
        code.contains("impl Drop for ZeroizingOutputs"),
        "{GUARD_FILE}: the guard must wipe on drop"
    );
    assert!(
        code.contains("try_extract_tensor_mut::<f32>"),
        "{GUARD_FILE}: the wipe must overwrite the ORT-owned buffer in place"
    );
}
