//! Metadata-only template listing (GitHub #235, STO-19, verification matrix rows EFL).
//!
//! `EnrollmentService::list` and the GUI profile refresh print template metadata only; they
//! must use `BiometricStore::get_metadata`, which never materialises the embedding vector,
//! instead of `BiometricStore::get`, which decrypts and returns the full template.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// Returns the text of the first function whose signature starts with `signature`, up to the
/// matching closing brace (brace counting; the audited functions contain no brace literals).
fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("signature `{signature}` not found"));
    let rest = &source[start..];
    let open = rest.find('{').expect("function body opening brace");
    let mut depth = 0usize;
    for (idx, ch) in rest[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..open + idx + 1];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated body for `{signature}`");
}

fn assert_metadata_only(file: &str, signature: &str) {
    let source = std::fs::read_to_string(workspace_root().join(file)).expect("read source");
    let body = function_body(&source, signature);
    assert!(
        body.contains(".get_metadata("),
        "{file} `{signature}` must list templates through BiometricStore::get_metadata"
    );
    assert!(
        !body.contains(".get(uid)"),
        "{file} `{signature}` must not decrypt full templates (embedding) to list metadata"
    );
}

#[test]
fn test_enroll_list_reads_template_metadata_only() {
    assert_metadata_only("crates/enrollment-cli/src/service.rs", "pub fn list(");
}

#[test]
fn test_gui_profile_refresh_reads_template_metadata_only() {
    assert_metadata_only("crates/gui/src/app.rs", "fn load_local_profiles(");
}
