//! PAM feedback seam, `PAM_SILENT` and fuzzing contracts (GitHub #220, #221, #227;
//! review findings PAM-07, PAM-08, PAM-15). Matrix rows PHS1–PHS9.
//!
//! - The authentication flow reaches libpam only through `impl PamFeedback for PamHandle`:
//!   no `get_user(` / `get_item::<` call and no pointer-address heuristic outside it.
//! - Both authentication entry points thread the Linux-PAM `flags` (no discarded `_flags`).
//! - Every cargo-fuzz target is built by a standalone fuzz workspace and run by the
//!   nightly `.github/workflows/fuzz.yml` job with a fixed wall-clock budget.

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

fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Production part of `crates/pam/src/lib.rs` (unit tests stripped).
fn pam_lib_production() -> String {
    let src = read("crates/pam/src/lib.rs");
    match src.find("#[cfg(test)]") {
        Some(idx) => src[..idx].to_owned(),
        None => src,
    }
}

/// Splits the production source into the `impl PamFeedback for PamHandle` block (up to the
/// next top-level item after its closing brace) and everything else.
fn split_handle_adapter(prod: &str) -> (String, String) {
    let start = prod
        .find("impl PamFeedback for PamHandle {")
        .expect("impl PamFeedback for PamHandle must exist (GitHub #220)");
    let end = prod[start..]
        .find("\n}\n")
        .map(|off| start + off + 3)
        .expect("adapter block end");
    let adapter = prod[start..end].to_owned();
    let rest = format!("{}{}", &prod[..start], &prod[end..]);
    (adapter, rest)
}

/// PHS1: libpam is reached only through the `PamHandle` adapter of `PamFeedback`.
#[test]
fn test_pam_flow_reaches_libpam_only_through_feedback_adapter() {
    let prod = pam_lib_production();
    let (adapter, rest) = split_handle_adapter(&prod);
    for needle in [".get_user(", ".get_item::<"] {
        assert!(
            adapter.contains(needle),
            "the PamHandle adapter must implement the lookup via {needle}"
        );
        assert!(
            !rest.contains(needle),
            "{needle} must only be called inside `impl PamFeedback for PamHandle`"
        );
    }
    assert!(
        !prod.contains("fn send_pam_info") && !prod.contains("fn usable_handle"),
        "the pre-#220 conversation helpers must be gone"
    );
}

/// PHS2: the transitional pointer-address guard exists exactly once (the adapter's
/// helper) and the flow never inspects handle addresses.
#[test]
fn test_pam_handle_address_guard_is_confined_to_one_helper() {
    let prod = pam_lib_production();
    assert_eq!(
        prod.matches("0x10000").count(),
        prod.matches("addr >= 0x10000").count(),
        "0x10000 may only appear in the guard expression"
    );
    assert!(
        prod.matches("addr >= 0x10000").count() <= 1,
        "at most one address guard"
    );
    let flow_start = prod
        .find("pub fn authenticate_with_feedback(")
        .expect("authenticate_with_feedback exists");
    let flow_end = prod[flow_start..]
        .find("\nimpl PamHooks for SoosPam")
        .map(|off| flow_start + off)
        .expect("flow end");
    let flow = &prod[flow_start..flow_end];
    assert!(
        !flow.contains("as usize") && !flow.contains("is_libpam_handle"),
        "the authentication flow must never inspect PAM handle addresses"
    );
}

/// PHS3: both entry points thread the Linux-PAM flags and the flow checks `PAM_SILENT`.
#[test]
fn test_pam_entry_points_thread_flags_and_honor_pam_silent() {
    let prod = pam_lib_production();
    assert!(
        prod.contains("fn sm_authenticate(pamh: &mut PamHandle, args: Vec<&CStr>, flags: PamFlag)"),
        "PamHooks::sm_authenticate must keep its flags"
    );
    let c_entry = prod
        .find("pub extern \"C\" fn pam_sm_authenticate(")
        .expect("C entry exists");
    let signature = &prod[c_entry..c_entry + 200];
    assert!(
        signature.contains("    flags: i32,") && !signature.contains("_flags"),
        "pam_sm_authenticate must not discard its flags"
    );
    assert!(prod.contains("constants::PAM_SILENT"));
}

/// Names of the `[[bin]]` targets declared by the fuzz harness.
fn fuzz_targets() -> Vec<String> {
    let manifest = read("crates/protocol/fuzz/Cargo.toml");
    let mut names = Vec::new();
    let mut in_bin = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_bin = line == "[[bin]]";
            continue;
        }
        if in_bin {
            if let Some(rest) = line.strip_prefix("name = \"") {
                names.push(rest.trim_end_matches('"').to_owned());
            }
        }
    }
    names
}

/// PHS9: the fuzz harness is buildable (own workspace, lock file) and declares the
/// preview decoder target.
#[test]
fn test_fuzz_harness_is_standalone_and_covers_preview_decoder() {
    let manifest = read("crates/protocol/fuzz/Cargo.toml");
    assert!(
        manifest.lines().any(|l| l.trim() == "[workspace]"),
        "without its own [workspace] table cargo refuses to build the harness"
    );
    assert!(workspace_root()
        .join("crates/protocol/fuzz/Cargo.lock")
        .is_file());
    let targets = fuzz_targets();
    for expected in [
        "decode_request",
        "decode_response",
        "decode_event",
        "decode_preview",
    ] {
        assert!(
            targets.iter().any(|t| t == expected),
            "missing fuzz target {expected}"
        );
        assert!(workspace_root()
            .join(format!("crates/protocol/fuzz/fuzz_targets/{expected}.rs"))
            .is_file());
    }
}

/// PHS9: a scheduled workflow runs EVERY fuzz target for a bounded wall-clock budget,
/// with SHA-pinned actions and no persisted credentials.
#[test]
fn test_nightly_fuzz_workflow_runs_every_target() {
    let wf = read(".github/workflows/fuzz.yml");
    assert!(wf.contains("schedule:") && wf.contains("cron:"));
    assert!(wf.contains("-max_total_time="));
    assert!(wf.contains("timeout-minutes:"));
    assert!(wf.contains("persist-credentials: false"));
    assert!(wf.contains("permissions:\n  contents: read"));
    let matrix = wf
        .lines()
        .find(|l| l.trim_start().starts_with("target: ["))
        .expect("target matrix");
    for target in fuzz_targets() {
        assert!(
            matrix.contains(&target),
            "fuzz target {target} is not run by fuzz.yml"
        );
    }
    for line in wf.lines().filter(|l| l.contains("uses:")) {
        let reference = line.split('@').nth(1).unwrap_or("");
        let sha: String = reference
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect();
        assert_eq!(sha.len(), 40, "action not pinned to a full SHA: {line}");
    }
    assert!(
        !wf.contains("fault-injection"),
        "fuzzing must never enable the PAM fault-injection feature"
    );
}
