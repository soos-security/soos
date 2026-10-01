//! SFace switch follow-ups (GitHub #298; walkthrough 164).
//!
//! - the biometric-store CRUD example builds a template of the shipped embedding model
//!   (`sface_2021dec`, 128 values), not the retired 512-D `glintr100` one (SGF4).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contract test suite uses assertions"
)]

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// SGF4: the `BiometricTemplate::new` example in `Docs/BIOMETRIC_STORE_CRATE.md` uses the
/// shipped model id and dimension.
#[test]
fn test_biometric_store_doc_example_uses_shipped_sface_template() {
    let rel = "Docs/BIOMETRIC_STORE_CRATE.md";
    let text = fs::read_to_string(workspace_root().join(rel)).unwrap();
    assert!(
        !text.contains("glintr100"),
        "{rel} still names the retired glintr100 model"
    );
    let start = text
        .find("let template = BiometricTemplate::new(")
        .expect("CRUD example builds a template");
    let end = text[start..]
        .find(")?;")
        .map(|offset| start + offset)
        .expect("end of the template example");
    let example = &text[start..end];
    assert!(
        example.contains("\"sface_2021dec\""),
        "{rel} template example must use the shipped model id: {example}"
    );
    assert!(
        example.contains("; 128]"),
        "{rel} template example must hold 128 values: {example}"
    );
    assert!(
        !example.contains("; 512]"),
        "{rel} template example must not hold 512 values: {example}"
    );
}
