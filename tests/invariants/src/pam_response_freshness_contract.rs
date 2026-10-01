//! Response freshness documentation contract (GitHub #219 review finding PAM-06, GitHub #287).
//!
//! Replay protection is the fresh, single-use 256-bit `request_id` that the client generates
//! per connection and checks bit-for-bit, plus the client's own cumulative deadline. Since
//! GitHub #287 the PAM client also enforces `Response::issued_monotonic_ns` /
//! `Response::expires_monotonic_ns` as a staleness guard: an expired or unstamped response
//! yields `PAM_IGNORE` (ADR 2026-10-01 "PAM Client Enforces Response Expiry", superseding in
//! part ADR 2026-09-30 "Response Timestamps Are Informational"). The normative protocol
//! documents must therefore not claim that the expiration timestamp prevents replay, must
//! name the mechanism that actually does, and must state the enforced expiry rule.

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
    // GitHub #287 (owner-approved migration 2026-10-01): the entry must state the enforced
    // rule instead of "informational" / "not validated".
    assert!(
        expires_line.contains("Enforced by the PAM client")
            && expires_line.contains("IpcError::StaleResponse")
            && expires_line.contains("PAM_IGNORE"),
        "the expires_monotonic_ns entry must say the PAM client enforces it and that a stale \
         response yields PAM_IGNORE, got: {expires_line}"
    );
    let freshness = &doc[doc
        .find("Response Freshness")
        .expect("Response Freshness section")..];
    for (rule, error) in [
        ("`now >= expires`", "`Expired`"),
        (
            "`issued_monotonic_ns == 0` or `expires_monotonic_ns == 0`",
            "`Unstamped`",
        ),
    ] {
        let row = freshness
            .lines()
            .find(|l| l.contains(rule))
            .unwrap_or_else(|| panic!("Response Freshness must list the rule {rule}"));
        assert!(
            row.contains(error),
            "rule {rule} must map to {error}: {row}"
        );
    }
    assert!(
        freshness.contains("`IpcError::StaleResponse` and therefore `PAM_IGNORE`"),
        "Response Freshness must state that a stale response yields PAM_IGNORE"
    );
    assert!(
        doc.contains("Response Freshness"),
        "Docs/IPC_PROTOCOL.md must carry a `Response Freshness` section naming the request_id \
         binding and the client deadline as the replay protection"
    );
}
