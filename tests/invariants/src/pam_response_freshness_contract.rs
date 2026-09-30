//! Response freshness documentation contract (GitHub #219, review finding PAM-06).
//!
//! The PAM client does not validate `Response::issued_monotonic_ns` /
//! `Response::expires_monotonic_ns`: replay protection is the fresh, single-use 256-bit
//! `request_id` that the client generates per connection and checks bit-for-bit, plus the
//! client's own cumulative deadline (ADR 2026-09-30 "Response Timestamps Are Informational").
//! The normative protocol documents must therefore not claim that the expiration timestamp
//! prevents replay, and must name the mechanism that actually does.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Architectural invariant test runner utilizes direct assertions and panics"
)]

use std::fs;
use std::path::{Path, PathBuf};

/// Wording that attributes replay protection to the response timestamps.
const TIMESTAMP_REPLAY_CLAIMS: [&str; 3] = [
    "timestamp preventing replay",
    "expiration timestamp preventing",
    "and short expiration",
];

/// Files that describe the `Response` schema.
const SCHEMA_FILES: [&str; 3] = [
    "Docs/IPC_PROTOCOL.md",
    "crates/protocol/src/types.rs",
    "AI/ARCHITECTURE.md",
];

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

#[test]
fn test_response_timestamps_are_not_documented_as_replay_protection() {
    let mut violations = Vec::new();
    for rel in SCHEMA_FILES {
        for (n, line) in read(rel).lines().enumerate() {
            for claim in TIMESTAMP_REPLAY_CLAIMS {
                if line.contains(claim) {
                    violations.push(format!("{rel}:{}: states `{claim}`", n + 1));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "response timestamps are informational and never validated by the PAM client; \
         replay protection is the single-use request_id (GitHub #219):\n{}",
        violations.join("\n")
    );
}

#[test]
fn test_ipc_protocol_documents_the_real_replay_protection() {
    let doc = read("Docs/IPC_PROTOCOL.md");
    let expires_line = doc
        .lines()
        .find(|l| l.contains("`expires_monotonic_ns: u64`"))
        .expect("Docs/IPC_PROTOCOL.md documents expires_monotonic_ns");
    assert!(
        expires_line.contains("informational") && expires_line.contains("not validated"),
        "the expires_monotonic_ns entry must say it is informational and not validated by the \
         PAM client, got: {expires_line}"
    );
    assert!(
        doc.contains("Response Freshness"),
        "Docs/IPC_PROTOCOL.md must carry a `Response Freshness` section naming the request_id \
         binding and the client deadline as the replay protection"
    );
}
