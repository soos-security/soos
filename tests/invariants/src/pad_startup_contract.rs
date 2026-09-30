//! PAD startup validation invariant (GitHub #214, review finding PAD-09).
//!
//! The daemon must never serve authentication with a PAD model whose output length or live
//! class index has not been checked against the manifest: `initialize_pipeline` must run the
//! PAD self-test (`validate_pad_detector`) and propagate its error (fail closed).

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions and indexing"
)]

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("Unable to locate workspace root directory")
        .to_path_buf()
}

/// Returns the brace-delimited body of `fn <name>(` in `source`.
fn function_body<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let start = source.find(&format!("fn {name}("))?;
    let open = start + source[start..].find('{')?;
    let mut depth = 0usize;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&source[open..=open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

#[test]
fn test_initialize_pipeline_runs_pad_self_test_fail_closed() {
    let path = workspace_root().join("crates/daemon/src/pipeline.rs");
    let source = std::fs::read_to_string(&path).expect("crates/daemon/src/pipeline.rs must exist");
    let init = function_body(&source, "initialize_pipeline")
        .expect("initialize_pipeline must exist in crates/daemon/src/pipeline.rs");
    let call = init
        .find("validate_pad_detector(")
        .expect("initialize_pipeline must run validate_pad_detector (GitHub #214)");
    let statement_end = init[call..]
        .find(';')
        .map(|end| call + end)
        .expect("validate_pad_detector call must be a statement");
    assert!(
        init[call..statement_end].trim_end().ends_with('?'),
        "initialize_pipeline must propagate the PAD self-test error with `?` (fail closed)"
    );
    let vision = init
        .find("VisionPipeline::new(")
        .expect("initialize_pipeline must assemble the VisionPipeline");
    assert!(
        call < vision,
        "The PAD self-test must run before the VisionPipeline is assembled"
    );

    let validator = function_body(&source, "validate_pad_detector")
        .expect("validate_pad_detector must be defined in crates/daemon/src/pipeline.rs");
    assert!(
        validator.contains("self_test("),
        "validate_pad_detector must run OrtPadDetector::self_test"
    );
    assert!(
        validator.contains("pad_class_count_from_manifest("),
        "validate_pad_detector must derive the class count from the manifest output_shapes"
    );
}
