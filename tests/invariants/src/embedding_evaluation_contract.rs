//! Real-face embedding evaluation harness contract (GitHub #278, walkthrough 160; row
//! EVR6).
//!
//! The LFW evaluation (`crates/vision/tests/embedding_lfw_evaluation_tests.rs`) needs a public
//! face dataset, the real models and tens of CPU minutes, so it is an ignored test gated on
//! environment variables, and its bounded downloader `scripts/fetch_lfw_eval.sh` keeps every
//! byte of the dataset outside the repository. These checks run in normal CI without network.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contract test suite uses assertions"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const HARNESS: &str = "crates/vision/tests/embedding_lfw_evaluation_tests.rs";
const FETCH_SCRIPT: &str = "scripts/fetch_lfw_eval.sh";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read_repo(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

#[test]
fn test_lfw_evaluation_harness_is_ignored_and_env_gated() {
    let harness = read_repo(HARNESS);
    let eval = harness
        .find("fn test_lfw_real_face_evaluation_report")
        .expect("the LFW evaluation test exists");
    let ignore = harness[..eval]
        .rfind("#[ignore")
        .expect("the LFW evaluation test is #[ignore]d");
    assert!(
        !harness[ignore..eval].contains("fn "),
        "the #[ignore] attribute must belong to the evaluation test"
    );
    for var in [
        "SOOS_EVAL_LFW_DIR",
        "SOOS_EVAL_LFW_PAIRS",
        "SOOS_MODELS_DIR",
    ] {
        assert!(harness.contains(var), "the harness is gated on {var}");
    }
    for test in [
        "fn test_lfw_harness_refuses_to_run_without_env_vars",
        "fn test_lfw_harness_refuses_data_inside_the_repository",
    ] {
        let at = harness
            .find(test)
            .unwrap_or_else(|| panic!("{test} exists"));
        assert!(
            !harness[at.saturating_sub(80)..at].contains("#[ignore"),
            "{test} must run in normal CI"
        );
    }
    assert!(
        harness.contains("ModelRegistry") && harness.contains("verify_integrity"),
        "the harness loads every model through the attested registry"
    );
}

#[test]
fn test_lfw_fetch_script_refuses_a_cache_inside_the_repository() {
    let root = workspace_root();
    let script = root.join(FETCH_SCRIPT);
    let text = read_repo(FETCH_SCRIPT);
    assert!(text.contains("set -euo pipefail"));
    assert!(
        text.contains("--max-filesize") && text.contains("--proto '=https'"),
        "downloads are HTTPS-only and size-bounded"
    );
    assert!(
        text.contains("sha256sum --check"),
        "every download is verified against a pinned SHA-256"
    );

    let probe = root.join("target/soos-lfw-cache-probe-must-not-exist");
    let output = Command::new("bash")
        .arg(&script)
        .arg(&probe)
        .output()
        .expect("run fetch_lfw_eval.sh");
    assert!(
        !output.status.success(),
        "a cache inside the repository is refused"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("outside the repository"),
        "the refusal names the reason"
    );
    assert!(
        !probe.exists(),
        "a refused cache directory is never created"
    );
}

#[test]
fn test_no_lfw_data_is_in_the_source_tree() {
    // Walks the source tree (build output, VCS metadata and agent worktrees excluded).
    const SKIPPED: [&str; 4] = ["target", ".git", ".claude", "node_modules"];
    let mut pending = vec![workspace_root()];
    let mut offending = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).expect("read source directory").flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if name == "lfw" || name == "soos-eval" {
                    offending.push(entry.path());
                } else if !SKIPPED.contains(&name.as_str()) {
                    pending.push(entry.path());
                }
            } else if matches!(name.as_str(), "pairs.txt" | "lfw.tgz" | "lfw-funneled.tgz") {
                offending.push(entry.path());
            }
        }
    }
    assert!(
        offending.is_empty(),
        "LFW data must never enter the repository: {offending:?}"
    );
}
