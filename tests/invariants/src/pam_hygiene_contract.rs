//! PAM module hygiene contracts (review PAM-09 / PAM-16 / PAM-17, GitHub #263, #264, #265).
//! Matrix rows PHY1–PHY9.
//!
//! - PHY6: the IPC client has no tautological truncation re-check and delegates the
//!   request-id acceptance predicate to `Response::matches_request`; the PAM verdict
//!   mapping uses `Verdict::should_ignore`, so the protocol crate owns both predicates.
//! - PHY7: the exported `pam_sm_authenticate` symbol delegates to
//!   `PamHooks::sm_authenticate` for a real handle (the trait impl is production code).
//! - PHY8: `parse_argv` has no unreachable null check on `argv.add(i)`.
//! - PHY9: `crates/pam/build.rs` re-runs when the libpam link candidates change.
//! - PHY1/PHY2: `syslog.rs` chains to the prior panic hook instead of replacing it.
//! - PHY5: rejected PAM arguments are logged at `LOG_WARNING`.

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

/// Source text before the first `#[cfg(test)]` item (unit tests excluded).
fn production(rel: &str) -> String {
    let src = read(rel);
    match src.find("\n#[cfg(test)]") {
        Some(end) => src[..end].to_owned(),
        None => src,
    }
}

/// Body of the item starting at `signature`, up to the next top-level item.
fn item_body(src: &str, signature: &str) -> String {
    let start = src
        .find(signature)
        .unwrap_or_else(|| panic!("`{signature}` not found"));
    let rest = &src[start..];
    let end = rest.find("\n}\n").map_or(rest.len(), |e| e + 3);
    rest[..end].to_owned()
}

/// PHY6: the dead truncation branch is gone and the client uses the protocol predicates.
#[test]
fn test_ipc_client_uses_protocol_acceptance_predicates() {
    let ipc = production("crates/pam/src/ipc.rs");
    assert!(
        !ipc.contains("total_received"),
        "the tautological `total_received != total_capacity` re-check must be removed \
         (truncation is reported by the counted read)"
    );
    assert!(
        ipc.contains("resp.matches_request(&req.request_id)"),
        "the client must accept a response through `Response::matches_request`"
    );
    assert!(
        !ipc.contains("resp.request_id != req.request_id"),
        "no inline copy of the request-id predicate"
    );

    let lib = production("crates/pam/src/lib.rs");
    let flow = item_body(&lib, "    fn authenticate_flow<R>(");
    assert!(
        flow.contains(".should_ignore()"),
        "the PAM verdict mapping must use `Verdict::should_ignore`"
    );
}

/// PHY7: the exported symbol runs the `PamHooks` implementation for a real handle.
#[test]
fn test_exported_authenticate_symbol_delegates_to_pam_hooks() {
    let lib = production("crates/pam/src/lib.rs");
    let entry = item_body(&lib, "pub extern \"C\" fn pam_sm_authenticate(");
    assert!(
        entry.contains("SoosPam::sm_authenticate("),
        "pam_sm_authenticate must delegate to `PamHooks::sm_authenticate`"
    );
    assert!(
        entry.contains("catch_c_entry("),
        "the exported symbol keeps its own catch_unwind boundary"
    );
}

/// PHY8: no unreachable null check on pointer arithmetic from a non-null base.
#[test]
fn test_parse_argv_has_no_unreachable_null_check() {
    let config = production("crates/pam/src/config.rs");
    assert!(
        !config.contains("arg_ptr_ptr.is_null()"),
        "`argv.add(i)` from a non-null `argv` is never null"
    );
}

/// PHY9: build.rs tracks the libpam candidates it inspects.
#[test]
fn test_pam_build_script_reruns_on_libpam_changes() {
    let build = read("crates/pam/build.rs");
    assert!(
        build.contains("cargo:rerun-if-changed={}"),
        "build.rs must emit rerun-if-changed for the libpam paths it inspects"
    );
    assert!(build.contains("cargo:rerun-if-changed=build.rs"));
}

/// PHY1/PHY2: the panic hook chains to the prior hook and never calls `set_hook` per entry.
#[test]
fn test_panic_hook_chains_to_prior_hook() {
    let syslog = production("crates/pam/src/syslog.rs");
    assert!(
        syslog.contains("std::panic::take_hook()"),
        "the module must capture the prior panic hook"
    );
    assert_eq!(
        syslog.matches("set_hook(").count(),
        1,
        "the hook is installed exactly once (Once), never swapped per call"
    );
    assert!(
        syslog.contains("INIT_HOOK.call_once"),
        "installation stays guarded by a Once"
    );
}

/// PHY5: configuration warnings reach syslog at `LOG_WARNING`.
#[test]
fn test_config_warnings_are_logged_at_warning_level() {
    let syslog = production("crates/pam/src/syslog.rs");
    assert!(syslog.contains("libc::LOG_AUTHPRIV | libc::LOG_WARNING"));
    let config = production("crates/pam/src/config.rs");
    assert!(config.contains("syslog::log_warning("));
}
