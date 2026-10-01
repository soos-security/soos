//! Diagnostics parity and the shared `daemon.toml` reader (GitHub #289, rows DGP1-DGP9).
//!
//! - DGP1 / DGP2: `soos-admin test-pam` and `soos-gui` interpret a daemon `Response` with the
//!   predicates `pam_soos.so` uses (`Response::matches_request`, `Response::check_freshness`
//!   with the shared `MAX_RESPONSE_FUTURE_SKEW_NS`).
//! - DGP4: one `daemon.toml` camera-settings reader (`soos_camera_v4l::daemon_config`); the
//!   admin, enrollment and GUI crates do not parse the file themselves.
//! - DGP7: the reader opens first (`O_NONBLOCK | O_CLOEXEC`, following symbolic links like
//!   `soos-daemon`) and checks the handle, never a path-based `metadata()` before a blocking
//!   `open()`; the notes print the configuration path through the admin sanitizer.
//! - DGP8: the root helper of `pcx_wire_routing_tests` selects `SYS_setresuid32` on 32-bit x86.
//! - DGP9: the decision is recorded in the ADR register and the user documentation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Architectural invariant test runner utilizes direct assertions and panics"
)]

use std::fs;
use std::path::{Path, PathBuf};

const SHARED_READER: &str = "crates/camera-v4l/src/daemon_config.rs";

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

/// Source without its `#[cfg(test)]` tail and without `//` comment lines.
fn production_code(source: &str) -> String {
    let body = source.split("#[cfg(test)]").next().unwrap_or(source);
    body.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// DGP1: `test-pam` binds the response to its nonce with the protocol-owned predicate.
#[test]
fn test_dgp_test_pam_binds_the_response_to_its_nonce() {
    let pam = production_code(&read("crates/pam/src/ipc.rs"));
    let admin = production_code(&read("crates/admin-cli/src/test_pam.rs"));
    for (rel, code) in [
        ("crates/pam/src/ipc.rs", &pam),
        ("crates/admin-cli/src/test_pam.rs", &admin),
    ] {
        assert!(
            code.contains(".matches_request(&"),
            "{rel} must use Response::matches_request"
        );
    }
    let bind = admin.find(".matches_request(&").expect("nonce binding");
    let fresh = admin.find("check_freshness(").expect("freshness check");
    assert!(
        bind < fresh,
        "test-pam checks the nonce before the stamps, like pam_soos.so"
    );
}

/// DGP2: the GUI applies the shared staleness rule to the `Response` frames it consumes.
#[test]
fn test_dgp_gui_applies_the_shared_freshness_rule() {
    let gui = production_code(&read("crates/gui/src/ipc_camera.rs"));
    assert!(
        gui.contains("check_freshness("),
        "soos-gui must call Response::check_freshness"
    );
    assert!(
        gui.contains("MAX_RESPONSE_FUTURE_SKEW_NS"),
        "soos-gui must pass the shared MAX_RESPONSE_FUTURE_SKEW_NS"
    );
    assert!(
        !gui.contains("const MAX_RESPONSE_FUTURE_SKEW_NS"),
        "soos-gui must not redefine the skew bound"
    );
    assert!(
        gui.contains("CLOCK_MONOTONIC"),
        "soos-gui compares against CLOCK_MONOTONIC, the daemon's clock"
    );
}

/// DGP4: a single `daemon.toml` camera-settings reader, used by the three diagnostics clients.
#[test]
fn test_dgp_single_daemon_config_reader() {
    let shared = production_code(&read(SHARED_READER));
    assert!(shared.contains("pub fn read_daemon_camera_config("));
    assert!(shared.contains("pub const MAX_DAEMON_CONFIG_BYTES: u64 = 1024 * 1024;"));
    let lib = read("crates/camera-v4l/src/lib.rs");
    assert!(lib.contains("pub mod daemon_config;"));

    for rel in [
        "crates/admin-cli/src/daemon_config.rs",
        "crates/admin-cli/src/camera.rs",
        "crates/enrollment-cli/src/service.rs",
        "crates/gui/src/camera_source.rs",
    ] {
        let code = production_code(&read(rel));
        for parser in [
            "toml::from_str",
            "parse::<toml::",
            "toml::Value",
            "toml::Table",
        ] {
            assert!(
                !code.contains(parser),
                "{rel} parses daemon.toml itself ({parser}); use soos_camera_v4l::daemon_config"
            );
        }
    }
    let admin = production_code(&read("crates/admin-cli/src/daemon_config.rs"));
    assert!(admin.contains("soos_camera_v4l::daemon_config"));
    let enroll = production_code(&read("crates/enrollment-cli/src/service.rs"));
    assert!(enroll.contains("read_daemon_camera_config("));
    // The GUI goes through the enrollment resolver, which reports the reader's notes.
    let gui = production_code(&read("crates/gui/src/camera_source.rs"));
    assert!(gui.contains("resolve_camera_device_from_config_reported("));
}

/// DGP7: open first, check the handle, read with the bound.
#[test]
fn test_dgp_reader_opens_first_then_checks_the_handle() {
    let shared = production_code(&read(SHARED_READER));
    for needle in [
        "custom_flags(",
        "libc::O_NONBLOCK",
        "libc::O_CLOEXEC",
        ".metadata()",
        "is_file()",
        ".take(",
    ] {
        assert!(shared.contains(needle), "{SHARED_READER} must use {needle}");
    }
    for forbidden in [
        "fs::metadata(",
        "symlink_metadata(",
        "File::open(",
        "read_to_string(",
        "fs::read(",
        "path.is_file()",
        "O_NOFOLLOW",
        "path.exists()",
    ] {
        assert!(
            !shared.contains(forbidden),
            "{SHARED_READER} must not use {forbidden} (path-based check or unbounded read)"
        );
    }
    // The notes of soos-enroll / soos-gui print the path like the admin note does.
    let enroll = production_code(&read("crates/enrollment-cli/src/service.rs"));
    assert!(
        enroll.contains("sanitize_display_text(&path.to_string_lossy())"),
        "the enrollment notes must sanitize the configuration path"
    );
    let admin = production_code(&read("crates/admin-cli/src/camera.rs"));
    assert!(admin.contains("sanitize_display_text(&path.to_string_lossy())"));
    let open = shared.find("custom_flags(").expect("open call");
    let fstat = shared.find(".metadata()").expect("fstat on the handle");
    assert!(open < fstat, "the handle is opened before it is checked");
}

/// DGP8: the per-thread root helper selects the 32-bit-UID syscall on 32-bit x86.
#[test]
fn test_dgp_pcx_root_helper_selects_setresuid32_on_x86() {
    let src = read("crates/daemon/tests/pcx_wire_routing_tests.rs");
    assert!(
        src.contains("target_arch = \"x86\""),
        "x86 selection missing"
    );
    assert!(src.contains("libc::SYS_setresuid32"));
    assert!(src.contains("libc::SYS_setresuid;"));
    assert!(
        !src.contains("libc::syscall(libc::SYS_setresuid,"),
        "the syscall number must come from the per-architecture selection"
    );
}

/// DGP9: the decision is recorded.
#[test]
fn test_dgp_shared_reader_decision_is_documented() {
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.contains("One Shared `daemon.toml` Camera Reader"),
        "AI/DECISIONS.md must record the shared reader ADR"
    );
    let docs = read("Docs/CAMERA_V4L_CRATE.md");
    for needle in [
        "read_daemon_camera_config",
        "O_NONBLOCK",
        "wrong type",
        "symbolic links",
    ] {
        assert!(
            docs.contains(needle),
            "Docs/CAMERA_V4L_CRATE.md must mention {needle}"
        );
    }
}
