#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::path::PathBuf;

#[test]
fn test_evidence_store_has_zero_network_dependencies() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cargo_toml = manifest_dir.join("Cargo.toml");
    let content = fs::read_to_string(&cargo_toml).unwrap();

    let banned_dependencies = [
        "reqwest",
        "hyper",
        "curl",
        "ureq",
        "tungstenite",
        "tokio-tungstenite",
        "surf",
        "isahc",
        "attohttpc",
    ];

    for dep in banned_dependencies {
        assert!(
            !content.contains(dep),
            "ACCEPTANCE CRITERION E5 VIOLATION: Forbidden network dependency '{dep}' in evidence-store Cargo.toml!"
        );
    }
}

#[test]
fn test_evidence_store_source_has_no_network_namespaces() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");

    let mut rs_files = Vec::new();
    collect_files(&src_dir, &mut rs_files);

    for file in rs_files {
        let content = fs::read_to_string(&file).unwrap();
        assert!(
            !content.contains("std::net"),
            "ACCEPTANCE CRITERION E5 VIOLATION: Found 'std::net' in {}",
            file.display()
        );
        assert!(
            !content.contains("tokio::net"),
            "ACCEPTANCE CRITERION E5 VIOLATION: Found 'tokio::net' in {}",
            file.display()
        );
    }
}

fn collect_files(dir: &PathBuf, list: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files(&path, list);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                list.push(path);
            }
        }
    }
}
