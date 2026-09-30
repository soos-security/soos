//! PAM handle-guard removal and fault-injection ordering contracts (GitHub #220, candid
//! review findings 2 and 4). Matrix rows PHS4 and PHS11.
//!
//! - PHS4: the pointer-address heuristic (`is_libpam_handle`, `addr >= 0x10000`) is gone from
//!   every source file of `crates/pam/src`; libpam handles are trusted as given by libpam.
//! - PHS11: in the authentication flow, `fault_injection::trigger` runs before the first
//!   libpam access (`with_pam_service`, which reads `PAM_SERVICE`). The pre-existing
//!   `fault_injection_tests::test_fault_inject_via_pam_hooks_returns_pam_ignore` passes a
//!   synthetic `0x1000` handle and is only safe because the armed panic fires first.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    Path::new(&manifest_dir)
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read crates/pam/src") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// PHS4: no pointer-address heuristic remains anywhere in `crates/pam/src`.
#[test]
fn test_pam_handle_address_heuristic_is_absent_from_pam_sources() {
    let mut files = Vec::new();
    rust_sources(&workspace_root().join("crates/pam/src"), &mut files);
    assert!(
        !files.is_empty(),
        "crates/pam/src must contain Rust sources"
    );
    for file in files {
        let src = fs::read_to_string(&file).expect("read source");
        for needle in ["is_libpam_handle", "0x10000", "0x1_0000"] {
            assert!(
                !src.contains(needle),
                "{} still contains `{needle}`: the handle-address heuristic was removed (GitHub #220)",
                file.display()
            );
        }
    }
}

/// PHS11: `fault_injection::trigger` precedes every libpam access in `authenticate_flow`.
#[test]
fn test_fault_injection_trigger_runs_before_any_libpam_call() {
    let src = fs::read_to_string(workspace_root().join("crates/pam/src/lib.rs")).expect("lib.rs");
    let flow_start = src
        .find("fn authenticate_flow")
        .expect("authenticate_flow must exist");
    // Comment lines are dropped for the ordering check (they may name libpam calls).
    let flow: String = src[flow_start..]
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let trigger = flow
        .find("fault_injection::trigger(")
        .expect("authenticate_flow must call fault_injection::trigger");
    for libpam_access in [
        "with_pam_service(",
        "feedback.service(",
        "feedback.user(",
        "feedback.info(",
        "resolve_uid(",
        "get_item",
        "get_user",
    ] {
        if let Some(pos) = flow.find(libpam_access) {
            assert!(
                trigger < pos,
                "fault_injection::trigger must run before `{libpam_access}` in authenticate_flow"
            );
        }
    }
    let documented = &src[flow_start..];
    let raw_trigger = documented
        .find("fault_injection::trigger(")
        .expect("trigger call");
    assert!(
        documented[..raw_trigger].contains("must run before any libpam call"),
        "the ordering requirement must be documented next to the trigger"
    );
}
