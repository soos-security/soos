//! Pinned Rust toolchain contract (user decision 2026-09-30, matrix rows TCP1–TCP3).
//!
//! `rust-toolchain.toml` pins an exact Rust release instead of the floating `stable` channel,
//! and every sandbox Dockerfile installs that same release as its default toolchain, so local,
//! CI and Docker runs of one commit use one compiler.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contract tests utilize direct assertions and panics"
)]

use std::fs;
use std::path::{Path, PathBuf};

/// The Rust release pinned by the user decision of 2026-09-30.
const PINNED_RUST: &str = "1.98.1";

/// Dockerfiles that install a Rust toolchain for the sandbox and distro images.
const DOCKERFILES: [&str; 4] = [
    "Dockerfile",
    "tests/docker/Dockerfile.ubuntu",
    "tests/docker/Dockerfile.fedora",
    "tests/docker/Dockerfile.arch",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Value of `key = "..."` inside the `[toolchain]` table of `rust-toolchain.toml`.
fn toolchain_value(text: &str, key: &str) -> Option<String> {
    let mut in_table = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_table = line == "[toolchain]";
            continue;
        }
        if !in_table {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() == key {
            return Some(v.trim().to_string());
        }
    }
    None
}

#[test]
fn test_rust_toolchain_pins_exact_release_with_components() {
    let text = read("rust-toolchain.toml");
    assert_eq!(
        toolchain_value(&text, "channel").as_deref(),
        Some(format!("\"{PINNED_RUST}\"").as_str()),
        "rust-toolchain.toml must pin channel = \"{PINNED_RUST}\" (no floating stable channel)"
    );
    let components = toolchain_value(&text, "components").unwrap_or_default();
    for component in ["clippy", "rustfmt"] {
        assert!(
            components.contains(&format!("\"{component}\"")),
            "rust-toolchain.toml must keep the {component} component"
        );
    }
}

#[test]
fn test_sandbox_dockerfiles_install_the_pinned_toolchain() {
    for path in DOCKERFILES {
        let text = read(path);
        // Instructions only: comment lines may describe the flag.
        let defaults: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .flat_map(|line| line.split("--default-toolchain").skip(1))
            .filter_map(|rest| rest.split_whitespace().next())
            .collect();
        assert!(
            !defaults.is_empty(),
            "{path} must install Rust with an explicit --default-toolchain"
        );
        for value in defaults {
            assert_eq!(
                value, PINNED_RUST,
                "{path} installs --default-toolchain {value}; rust-toolchain.toml pins {PINNED_RUST}"
            );
        }
    }
}

#[test]
fn test_toolchain_pin_is_documented() {
    for path in [
        "Docs/CI_CD_AND_SECURITY.md",
        ".agents/skills/dev-workflow/references/project-facts.md",
    ] {
        assert!(
            read(path).contains(PINNED_RUST),
            "{path} must state the pinned Rust release {PINNED_RUST}"
        );
    }
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions
            .lines()
            .any(|l| l.contains("Pinned Rust Toolchain") && l.contains(PINNED_RUST)),
        "AI/DECISIONS.md must record the pinned Rust toolchain decision"
    );
}
