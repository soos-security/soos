//! PAM deadline contract invariants (GitHub #185, review finding TCI-02).
//!
//! The PAM module has no fixed 200–250 ms deadline: every blocking operation is bounded by one
//! explicit deadline derived from the clamped `timeout_ms` argument
//! (`crates/pam/src/config.rs`: default `DEFAULT_TIMEOUT_MS`, range `MIN_TIMEOUT_MS` to
//! `MAX_TIMEOUT_MS`). The normative documents must state that invariant and must not restate
//! the obsolete literal figure. A historical line may keep it only when it carries the
//! `*(Superseded` marker on the same line.

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

/// Wording of the enforced invariant (ADR 2026-09-30 "PAM Deadline Derived From Clamped
/// `timeout_ms`").
const INVARIANT_PHRASE: &str = "explicit deadline derived from the clamped `timeout_ms`";

/// Marker that exempts a historical line from the scan.
const SUPERSEDED_MARKER: &str = "*(Superseded";

/// Normative documents that state the PAM deadline rule.
const NORMATIVE_FILES: [&str; 8] = [
    "AGENTS.md",
    "AI/ARCHITECTURE.md",
    "AI/DECISIONS.md",
    "Docs/SECURITY_AND_QUALITY_GUIDELINES.md",
    "Docs/IPC_PROTOCOL.md",
    "Docs/PAM_MODULE.md",
    "tests/physical/screensaver_test.md",
    ".agents/skills/dev-workflow/references/project-facts.md",
];

/// Documents that must state the invariant explicitly.
const INVARIANT_FILES: [&str; 3] = ["AGENTS.md", "AI/ARCHITECTURE.md", "AI/DECISIONS.md"];

/// Obsolete statements of a fixed PAM deadline.
const OBSOLETE_DEADLINE_WORDING: [&str; 7] = [
    "200–250",
    "200-250",
    "250ms deadline",
    "250 ms deadline",
    "hard timeout: 250ms",
    "timeouts totaling",
    "After 250ms",
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
fn test_normative_docs_do_not_restate_a_fixed_pam_deadline() {
    let mut violations = Vec::new();
    for rel in NORMATIVE_FILES {
        for (n, line) in read(rel).lines().enumerate() {
            if line.contains(SUPERSEDED_MARKER) {
                continue;
            }
            for wording in OBSOLETE_DEADLINE_WORDING {
                if line.contains(wording) {
                    violations.push(format!("{rel}:{}: states `{wording}`", n + 1));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "the PAM deadline is derived from the clamped timeout_ms, not a fixed 200–250 ms:\n{}",
        violations.join("\n")
    );
}

#[test]
fn test_normative_docs_state_the_enforced_pam_deadline_invariant() {
    for rel in INVARIANT_FILES {
        assert!(
            read(rel).contains(INVARIANT_PHRASE),
            "{rel} must state the enforced invariant \"every blocking PAM operation has an \
             {INVARIANT_PHRASE}\""
        );
    }
}
