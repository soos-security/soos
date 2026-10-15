//! Lock-screen state channel contract (GitHub #380, PR 1 of #352; ADR 2026-10-13
//! "Read-Only Lock-Screen State Channel"; matrix LKS14-LKS16).
//!
//! - `LockStateResponse` carries exactly `version`, `state` and `sequence`: no score, reason
//!   class, frame, embedding, template, session id or timestamp field;
//! - `LockWidgetState` has exactly the three owner-approved values (no success value) and
//!   `Docs/IPC_PROTOCOL.md` lists their wire indices;
//! - the daemon snapshot module stores nothing biometric, forbids `unsafe`, has no
//!   `unwrap`/`expect` and logs nothing;
//! - `soos-daemon` shares one board between the dispatcher and the presence worker;
//! - `pam_soos.so` is untouched (never sends or decodes `LockState`);
//! - the fuzz harness and its nightly workflow cover the new response.

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

/// Returns the source text before the first `#[cfg(test)]` (production code only).
fn production_part(src: &str) -> &str {
    src.split("#[cfg(test)]").next().unwrap_or(src)
}

/// Returns the body (between the braces) of the first item whose header is `header`.
fn item_body<'a>(src: &'a str, header: &str) -> &'a str {
    let start = src
        .find(header)
        .unwrap_or_else(|| panic!("`{header}` not found"));
    let rest = &src[start + header.len()..];
    let end = rest.find("\n}").expect("item end");
    &rest[..end]
}

/// Field names of `pub struct <name> {` (lines `pub <field>: <type>,`).
fn struct_fields(src: &str, name: &str) -> Vec<String> {
    item_body(src, &format!("pub struct {name} {{"))
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("pub ") && l.contains(':'))
        .map(|l| {
            l.trim_start_matches("pub ")
                .split(':')
                .next()
                .unwrap()
                .trim()
                .to_string()
        })
        .collect()
}

/// `(variant, index)` of `pub enum <name> {` with explicit discriminants.
fn enum_variants(src: &str, name: &str) -> Vec<(String, u32)> {
    item_body(src, &format!("pub enum {name} {{"))
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//") && !l.starts_with("#[") && l.contains('='))
        .map(|l| {
            let (variant, index) = l.split_once('=').expect("explicit discriminant");
            (
                variant.trim().to_string(),
                index
                    .trim()
                    .trim_end_matches(',')
                    .trim()
                    .parse()
                    .expect("index"),
            )
        })
        .collect()
}

const FORBIDDEN_FIELD_WORDS: [&str; 12] = [
    "score",
    "reason",
    "frame",
    "embedding",
    "template",
    "session",
    "pixel",
    "image",
    "similarity",
    "liveness",
    "timestamp",
    "monotonic",
];

#[test]
fn test_lks_lock_state_response_has_only_coarse_fields() {
    let types = read("crates/protocol/src/types.rs");
    let fields = struct_fields(&types, "LockStateResponse");
    assert_eq!(
        fields,
        vec!["version", "state", "sequence"],
        "LockStateResponse must carry exactly version, state and sequence (ADR 2026-10-13)"
    );
    for field in &fields {
        for word in FORBIDDEN_FIELD_WORDS {
            assert!(
                !field.to_ascii_lowercase().contains(word),
                "LockStateResponse field `{field}` names forbidden data `{word}`"
            );
        }
    }
}

#[test]
fn test_lks_widget_state_has_exactly_the_owner_approved_values() {
    let types = read("crates/protocol/src/types.rs");
    assert_eq!(
        enum_variants(&types, "LockWidgetState"),
        vec![
            ("Idle".to_string(), 0),
            ("Scanning".to_string(), 1),
            ("NotRecognized".to_string(), 2),
        ],
        "owner decision 2026-10-13 (#352 Q2): Scanning / Idle / Not recognized only"
    );
    let variants = enum_variants(&types, "RequestKind");
    assert!(
        variants.contains(&("LockState".to_string(), 3)),
        "RequestKind::LockState is wire value 3"
    );
}

#[test]
fn test_lks_ipc_doc_lists_lock_state_wire_rows() {
    let types = read("crates/protocol/src/types.rs");
    let doc = read("Docs/IPC_PROTOCOL.md");
    for (variant, index) in enum_variants(&types, "LockWidgetState") {
        let row = format!("| `LockWidgetState::{variant}` | {index} |");
        assert!(doc.contains(&row), "Docs/IPC_PROTOCOL.md must list `{row}`");
    }
    for needle in [
        "RequestKind::LockState",
        "`LockStateResponse`",
        "LOCK_STATE_RESPONSE_MAX_PAYLOAD",
        "NOT_RECOGNIZED_REVEAL_FLOOR_MS",
        "LOCK_STATE_MAX_REQUESTS_PER_SEC",
    ] {
        assert!(
            doc.contains(needle),
            "Docs/IPC_PROTOCOL.md must document `{needle}`"
        );
    }
    let daemon = read("Docs/DAEMON.md");
    assert!(
        daemon.contains("| `RequestKind::LockState` |"),
        "Docs/DAEMON.md §3 must list the authorization of RequestKind::LockState"
    );
}

#[test]
fn test_lks_daemon_snapshot_stores_nothing_biometric() {
    let src = read("crates/daemon/src/lock_state.rs");
    let prod = production_part(&src);
    assert!(
        prod.contains("#![forbid(unsafe_code)]"),
        "crates/daemon/src/lock_state.rs must forbid unsafe code"
    );
    for forbidden in [
        "ReasonClass",
        "Verdict",
        "embedding",
        "Embedding",
        "Frame",
        "template",
        "Template",
        "score",
        "SessionId",
        "session_id",
        ".unwrap(",
        ".expect(",
        "panic!",
        "info!",
        "warn!",
        "debug!",
        "error!",
        "trace!",
    ] {
        assert!(
            !prod.contains(forbidden),
            "crates/daemon/src/lock_state.rs must not contain `{forbidden}` \
             (no biometric data, no panic path, no logging)"
        );
    }
}

#[test]
fn test_lks_daemon_shares_one_board() {
    let main = read("crates/daemon/src/main.rs");
    assert_eq!(
        main.matches(".with_lock_state_board(").count(),
        2,
        "soos-daemon must hand the same LockStateBoard to the dispatcher and the presence worker"
    );
    assert!(main.contains("LockStateBoard::new()"));
}

#[test]
fn test_lks_pam_module_is_untouched() {
    let dir = workspace_root().join("crates/pam/src");
    for entry in fs::read_dir(&dir).expect("pam src") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = fs::read_to_string(&path).expect("read");
        for forbidden in ["LockState", "LockWidgetState"] {
            assert!(
                !src.contains(forbidden),
                "{} must not reference `{forbidden}`: pam_soos.so is untouched by #380",
                path.display()
            );
        }
    }
}

#[test]
fn test_lks_fuzz_harness_covers_the_lock_state_response() {
    let target = read("crates/protocol/fuzz/fuzz_targets/decode_lock_state.rs");
    assert!(target.contains("LockStateResponse"));
    assert!(read("crates/protocol/fuzz/Cargo.toml").contains("name = \"decode_lock_state\""));
    assert!(read("crates/protocol/fuzz/README.md").contains("decode_lock_state"));
    let workflow = read(".github/workflows/fuzz.yml");
    let matrix = workflow
        .lines()
        .find(|l| l.trim_start().starts_with("target: ["))
        .expect("fuzz target matrix");
    assert!(
        matrix.contains("decode_lock_state"),
        "the nightly fuzz workflow must run decode_lock_state"
    );
}

/// Body of `fn <name>(` in `src`, up to the next line that closes an `impl` method (`\n    }`).
fn method_body<'a>(src: &'a str, name: &str) -> &'a str {
    let header = format!("fn {name}(");
    let start = src
        .find(&header)
        .unwrap_or_else(|| panic!("`{header}` not found"));
    let rest = &src[start..];
    let end = rest.find("\n    }\n").expect("method end");
    &rest[..end]
}

#[test]
fn test_lks_dispatcher_read_path_has_no_side_effect() {
    let src = read("crates/daemon/src/dispatcher.rs");
    let body = method_body(production_part(&src), "handle_lock_state_request");
    for forbidden in [
        "notify_activity",
        "register_interactive",
        "policy",
        "pipeline",
        "biometric_store",
        "camera",
        "evidence",
        "info!",
        "warn!",
        "error!",
        "trace!",
        "event!",
        "begin_scan",
    ] {
        assert!(
            !body.contains(forbidden),
            "handle_lock_state_request must not contain `{forbidden}` (read-only, side-effect free)"
        );
    }
    for line in body.lines().filter(|l| l.contains("ReasonClass::")) {
        assert!(
            line.contains("ReasonClass::RateLimited"),
            "the read path refuses only with RateLimited, found `{}`",
            line.trim()
        );
    }
    let mut rest = body;
    while let Some(at) = rest.find("debug!(") {
        let call = &rest[at..];
        let end = call.find(");").expect("debug! call end");
        let call_text = &call[..end];
        assert!(
            !call_text.contains("state") && !call_text.contains("sequence"),
            "the read path never logs the state: `{call_text}`"
        );
        rest = &call[end..];
    }
}

/// Candid review of 2026-10-13, suggestion: the per-poll logind sessions scan runs off the
/// async runtime, bounded by `LOCK_STATE_SESSION_CHECK_TIMEOUT_MS`, and fails neutral.
#[test]
fn test_lks_session_check_runs_off_the_runtime_with_a_bound() {
    let src = read("crates/daemon/src/dispatcher.rs");
    let body = method_body(production_part(&src), "handle_lock_state_request");
    for needle in [
        "spawn_blocking",
        "LOCK_STATE_SESSION_CHECK_TIMEOUT_MS",
        "has_local_seat_session",
    ] {
        assert!(
            body.contains(needle),
            "handle_lock_state_request must use `{needle}` for the sessions scan"
        );
    }
    let lock_state = read("crates/daemon/src/lock_state.rs");
    assert!(
        lock_state.contains("pub const LOCK_STATE_SESSION_CHECK_TIMEOUT_MS: u64 = 250;"),
        "the sessions scan bound is 250 ms"
    );
}
