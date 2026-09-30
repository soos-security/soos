//! Shared test-fixture contract (GitHub #241, review finding TCI-10).
//!
//! - `tests/fixtures` is a real workspace member, the dev-only crate `soos-test-fixtures`,
//!   instead of a loose `mod.rs` reachable only through `#[path]` includes;
//! - it carries no stale 128-dimension embedding fixtures (embeddings are 512D);
//! - it is never a normal (runtime) dependency of any crate;
//! - no new `#[path = ".../tests/fixtures/mod.rs"]` include may appear: the three legacy
//!   includes are frozen (they live in pre-existing test files that this change may not edit)
//!   and new tests must use `soos-test-fixtures` as a dev-dependency.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// Test files that still include the fixtures through `#[path]` (frozen; shrink only).
const LEGACY_PATH_INCLUDES: [&str; 3] = [
    "crates/daemon/tests/pad_wiring_tests.rs",
    "crates/enrollment-cli/tests/pad_wiring_tests.rs",
    "crates/vision/tests/pad_tests.rs",
];

fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn collect_files(dir: &Path, name_suffix: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, name_suffix, out);
        } else if path.to_string_lossy().ends_with(name_suffix) {
            out.push(path);
        }
    }
}

/// Dependency names declared in the `[dependencies]` tables (normal dependencies, any target)
/// of a `Cargo.toml`, excluding `[dev-dependencies]` and `[build-dependencies]`.
fn normal_dependencies(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_normal = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            let header = trimmed.trim_matches(|c| c == '[' || c == ']');
            in_normal = header == "dependencies" || header.ends_with(".dependencies");
            continue;
        }
        if in_normal {
            if let Some((name, _)) = trimmed.split_once('=') {
                names.push(name.trim().to_string());
            }
        }
    }
    names
}

#[test]
fn test_fil_fixtures_is_a_workspace_member_crate() {
    let root_manifest = read("Cargo.toml");
    assert!(
        root_manifest.contains("\"tests/fixtures\""),
        "tests/fixtures must be listed in the workspace members"
    );
    let manifest = read("tests/fixtures/Cargo.toml");
    assert!(
        manifest.contains("name = \"soos-test-fixtures\""),
        "{manifest}"
    );
    assert!(manifest.contains("publish.workspace = true"), "{manifest}");
    assert!(manifest.contains("[lints]\nworkspace = true"), "{manifest}");
    assert!(manifest.contains("path = \"mod.rs\""), "{manifest}");
}

#[test]
fn test_fil_fixtures_carry_no_stale_128d_embeddings() {
    let source = read("tests/fixtures/mod.rs");
    assert!(
        !source.contains("pub mod embeddings"),
        "the unused embeddings fixture module must be removed"
    );
    assert!(
        !source.contains("128D") && !source.contains("; 128]"),
        "fixtures must not describe or build 128-dimension embeddings (the contract is 512D)"
    );
}

#[test]
fn test_fil_fixtures_crate_is_never_a_runtime_dependency() {
    let root = workspace_root();
    let mut manifests = Vec::new();
    collect_files(&root.join("crates"), "Cargo.toml", &mut manifests);
    manifests.push(root.join("tests").join("invariants").join("Cargo.toml"));
    assert!(
        manifests.len() > 5,
        "manifest scan is vacuous: {manifests:?}"
    );
    for manifest in manifests {
        let content = fs::read_to_string(&manifest).expect("read manifest");
        let normal = normal_dependencies(&content);
        assert!(
            !normal.iter().any(|name| name == "soos-test-fixtures"),
            "{} lists soos-test-fixtures as a normal dependency; it is dev-only",
            manifest.display()
        );
    }
    // Self-test of the table parser.
    let sample = "[dependencies]\na = 1\n[dev-dependencies]\nsoos-test-fixtures = 1\n\
                  [target.'cfg(unix)'.dependencies]\nb = 1\n";
    assert_eq!(normal_dependencies(sample), ["a", "b"]);
}

#[test]
fn test_fil_fixture_path_includes_are_limited_to_legacy_files() {
    let root = workspace_root();
    let mut sources = Vec::new();
    collect_files(&root.join("crates"), ".rs", &mut sources);
    assert!(!sources.is_empty(), "source scan is vacuous");
    let mut includes: Vec<String> = sources
        .iter()
        .filter(|path| {
            fs::read_to_string(path)
                .map(|text| text.contains("tests/fixtures/mod.rs\"]"))
                .unwrap_or(false)
        })
        .map(|path| {
            path.strip_prefix(&root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    includes.sort();
    for include in &includes {
        assert!(
            LEGACY_PATH_INCLUDES.contains(&include.as_str()),
            "{include} includes tests/fixtures/mod.rs through #[path]; add \
             `soos-test-fixtures` as a dev-dependency instead"
        );
    }
}
