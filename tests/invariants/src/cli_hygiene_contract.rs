//! Storage CLI hygiene invariants (GitHub #231, #232, #233, #237; review findings STO-15,
//! STO-16, STO-17, STO-21).
//!
//! - `soos-admin test-pam` takes its deadline from `CLOCK_MONOTONIC`, never the wall clock,
//!   and clamps `timeout_ms` to the same range as the PAM module.
//! - JSON output of `soos-admin` and `soos-enroll` is produced by `serde_json`, never by
//!   hand-formatted `"key": "{}"` templates.
//! - Dead code named by STO-17 stays removed and `AlreadyEnrolled` is actually produced.
//! - `soos-enroll` parses its arguments before checking privileges.

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
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Extracts the `u64` value of `pub const <name>: u64 = <value>;` from `source`.
fn const_u64(source: &str, name: &str) -> u64 {
    let needle = format!("pub const {name}: u64 = ");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("constant {name} not found"))
        + needle.len();
    let rest = &source[start..];
    let end = rest.find(';').expect("constant terminator");
    rest[..end]
        .trim()
        .replace('_', "")
        .parse()
        .unwrap_or_else(|e| panic!("constant {name} is not a u64 literal: {e}"))
}

#[test]
fn test_admin_test_pam_uses_monotonic_clock() {
    let src = read("crates/admin-cli/src/test_pam.rs");
    assert!(
        !src.contains("SystemTime") && !src.contains("UNIX_EPOCH"),
        "test-pam must not derive deadline_monotonic_ns from the wall clock"
    );
    assert!(
        src.contains("CLOCK_MONOTONIC"),
        "test-pam must read CLOCK_MONOTONIC, the daemon's clock"
    );
}

#[test]
fn test_admin_timeout_clamp_matches_pam_module() {
    let pam = read("crates/pam/src/config.rs");
    let admin = read("crates/admin-cli/src/args.rs");
    for name in ["MIN_TIMEOUT_MS", "MAX_TIMEOUT_MS"] {
        assert_eq!(
            const_u64(&admin, name),
            const_u64(&pam, name),
            "soos-admin {name} must equal the PAM module value"
        );
    }
}

#[test]
fn test_cli_json_output_is_not_hand_formatted() {
    for rel in [
        "crates/admin-cli/src/status.rs",
        "crates/admin-cli/src/test_pam.rs",
        "crates/enrollment-cli/src/main.rs",
        "crates/enrollment-cli/src/service.rs",
    ] {
        let src = read(rel);
        assert!(
            !src.contains("\\\": \\\"{}\\\"") && !src.contains("\\\": \\\"{:?}\\\""),
            "{rel} hand-formats JSON string fields; use serde_json"
        );
    }
}

#[test]
fn test_sto17_dead_code_stays_removed() {
    let html = read("crates/enrollment-cli/src/html_report.rs");
    assert!(!html.contains("fn encode_bmp"), "encode_bmp is dead code");
    let service = read("crates/enrollment-cli/src/service.rs");
    assert!(
        !service.contains("fn build_service("),
        "the build_service alias is dead code; use build_full_service"
    );
    assert!(
        !service.contains("Overwriting silently allowed"),
        "the empty overwrite branch must not come back"
    );
    let lib = read("crates/enrollment-cli/src/lib.rs");
    assert!(!lib.contains("build_service,"));
}

#[test]
fn test_already_enrolled_is_constructed_in_production_code() {
    let service = read("crates/enrollment-cli/src/service.rs");
    assert!(
        service.contains("EnrollmentCliError::AlreadyEnrolled(uid)"),
        "AlreadyEnrolled must be produced by the service, not only declared"
    );
}

#[test]
fn test_shred_never_follows_symlinks() {
    let shred = read("crates/enrollment-cli/src/shred.rs");
    assert!(shred.contains("symlink_metadata"));
    assert!(shred.contains("O_NOFOLLOW"));
    assert!(
        !shred.contains("std::fs::metadata("),
        "metadata() follows symbolic links"
    );
}

#[test]
fn test_enroll_parses_arguments_before_privilege_check() {
    let main = read("crates/enrollment-cli/src/main.rs");
    let parse = main.find("Cli::parse()").expect("Cli::parse in main.rs");
    let check = main
        .find("check_privileges(true)")
        .expect("check_privileges in main.rs");
    assert!(
        parse < check,
        "arguments (--help/--version) must be parsed before the root check"
    );
}
