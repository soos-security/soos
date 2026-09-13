//! Contractual security audit test verifying zero sensitive information in daemon logs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual security audit tests use assertions, unwrap, and expect"
)]

use std::fs;
use std::path::{Path, PathBuf};

fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, files);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
}

#[test]
fn test_daemon_source_code_has_zero_sensitive_data_in_logs() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest.join("src");

    let mut files = Vec::new();
    collect_rs_files(&src_dir, &mut files);

    assert!(
        !files.is_empty(),
        "No .rs files found in daemon src directory: {}",
        src_dir.display()
    );

    let forbidden_sensitive_keywords = [
        "password",
        "secret",
        "credential",
        "embedding",
        "frame",
        "image",
        "raw_payload",
    ];

    let log_macro_patterns = [
        "info!(", "warn!(", "error!(", "debug!(", "trace!(", "event!(",
    ];

    for file in files {
        let content = fs::read_to_string(&file).expect("Failed to read file");
        for (line_no, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                continue;
            }

            for macro_pattern in &log_macro_patterns {
                if trimmed.contains(macro_pattern) {
                    let lower = trimmed.to_lowercase();
                    for keyword in &forbidden_sensitive_keywords {
                        assert!(
                            !lower.contains(keyword),
                            "Acceptance D5 VIOLATION: Sensitive keyword '{}' found in log statement at {}:{}\nLine: {}",
                            keyword,
                            file.display(),
                            line_no + 1,
                            trimmed
                        );
                    }
                }
            }
        }
    }
}
