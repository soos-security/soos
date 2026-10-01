//! SFace embedding switch (GitHub #278, owner decisions 2026-10-01; walkthrough 162).
//!
//! - `models/retired_models.toml` keeps the retired ArcFace attestation for the evaluation tests
//!   only: no runtime crate and no deployment script reads it (SFC17);
//! - `scripts/download_models.sh` reports `.onnx` files left in the target directory that the
//!   deployed manifest does not attest (the retired `arcface_w600k_mbf.onnx` after an upgrade),
//!   without deleting them (SFC17);
//! - the model and crate docs and the agent facts state the shipped SFace model and the 0.50
//!   match default, with no stale ArcFace / 0.70 default claim (SFC16).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contract test suite uses assertions and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::installer_contract::scratch_dir;

const MODEL_BYTES: &[u8] = b"attested-model-bytes";

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

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn sha256_hex(path: &Path) -> String {
    let out = Command::new("sha256sum")
        .arg(path)
        .output()
        .expect("sha256sum");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("digest")
        .to_string()
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// SFC17: the retired attestation is read by no runtime source and no deployment script.
#[test]
fn test_retired_models_file_is_read_by_no_runtime_crate_or_script() {
    let root = workspace_root();
    let retired = read_repo("models/retired_models.toml");
    assert!(retired.contains("[models.arcface_w600k_mbf]"));
    assert!(
        !read_repo("models/manifest.toml").contains("arcface_w600k_mbf"),
        "the shipped manifest no longer attests the retired ArcFace model"
    );
    let mut offenders = Vec::new();
    let mut files = Vec::new();
    for dir in [
        "crates/daemon/src",
        "crates/enrollment-cli/src",
        "crates/gui/src",
        "crates/admin-cli/src",
        "crates/inference-ort/src",
        "crates/vision/src",
        "crates/pam/src",
    ] {
        rust_files(&root.join(dir), &mut files);
    }
    for file in files {
        let source = fs::read_to_string(&file).expect("read source");
        if source.contains("retired_models") {
            offenders.push(
                file.strip_prefix(&root)
                    .unwrap_or(&file)
                    .display()
                    .to_string(),
            );
        }
    }
    for script in [
        "scripts/download_models.sh",
        "scripts/install.sh",
        "scripts/uninstall.sh",
    ] {
        if read_repo(script).contains("retired_models") {
            offenders.push(script.to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "models/retired_models.toml must stay evaluation-only (GitHub #278): {offenders:?}"
    );
}

/// SFC17: a deployment lists `.onnx` files of the target directory that the manifest does not
/// attest, names the manifest, and never deletes them.
#[test]
fn test_download_models_reports_unattested_model_files_without_deleting_them() {
    let work = scratch_dir("sfc_unattested");
    let src = work.join("src.onnx");
    fs::write(&src, MODEL_BYTES).expect("write model");
    let manifest = work.join("manifest.toml");
    fs::write(
        &manifest,
        format!(
            "[manifest]\nversion = \"2.0.0\"\n\n[models.m1]\nid = \"m1\"\n\
             filename = \"m1.onnx\"\nsha256 = \"{}\"\nlicense = \"MIT\"\nsize_bytes = {}\n\
             source_url = \"file://{}\"\ninput_shape = [1, 3, 80, 80]\n",
            sha256_hex(&src),
            MODEL_BYTES.len(),
            src.display()
        ),
    )
    .expect("write manifest");
    let target = work.join("models");
    fs::create_dir_all(&target).expect("target dir");
    let orphan = target.join("arcface_w600k_mbf.onnx");
    fs::write(&orphan, b"retired model").expect("write orphan");

    let out = Command::new("bash")
        .arg(workspace_root().join("scripts/download_models.sh"))
        .args([
            std::ffi::OsStr::new("--manifest"),
            manifest.as_os_str(),
            std::ffi::OsStr::new("--target-dir"),
            target.as_os_str(),
        ])
        .env_remove("SOOS_MODEL_MAX_BYTES")
        .output()
        .expect("execute download_models.sh");
    let text = combined(&out);
    assert!(out.status.success(), "deploy output:\n{text}");
    let notice: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("not attested"))
        .collect();
    assert!(
        notice.iter().any(|l| l.contains("arcface_w600k_mbf.onnx")),
        "the unattested file must be reported; output:\n{text}"
    );
    assert!(
        !text
            .lines()
            .any(|l| l.contains("not attested") && l.contains("m1.onnx")),
        "an attested file is never reported; output:\n{text}"
    );
    assert!(orphan.is_file(), "the unattested file must not be deleted");
    let _ = fs::remove_dir_all(&work);
}

/// SFC16: docs and agent facts state the shipped model and the 0.50 default.
#[test]
fn test_docs_state_the_sface_model_and_the_050_default() {
    for rel in [
        "models/README.md",
        "Docs/INFERENCE_ORT_CRATE.md",
        "Docs/VISION_CRATE.md",
        "AI/ARCHITECTURE.md",
        ".agents/skills/dev-workflow/references/project-facts.md",
    ] {
        let text = read_repo(rel);
        assert!(
            text.contains("sface_2021dec"),
            "{rel} must name sface_2021dec"
        );
    }
    for rel in [
        "Docs/VISION_CRATE.md",
        "Docs/POLICY_CRATE.md",
        ".agents/skills/dev-workflow/references/project-facts.md",
    ] {
        let text = read_repo(rel);
        let stale: Vec<&str> = text
            .lines()
            .filter(|l| {
                (l.contains("DEFAULT_MATCH_THRESHOLD") || l.contains("match_threshold("))
                    && l.contains("0.70")
                    && !l.contains("retired")
                    && !l.contains("former")
            })
            .collect();
        assert!(
            stale.is_empty(),
            "{rel} states a stale 0.70 match default (code: 0.50): {stale:?}"
        );
        assert!(
            text.contains("0.50"),
            "{rel} must state the 0.50 match default"
        );
    }
    assert!(
        !read_repo("Docs/POLICY_CRATE.md").contains("MobileFaceNet literature"),
        "Docs/POLICY_CRATE.md still sources the match default from MobileFaceNet literature"
    );
    let decisions = read_repo("AI/DECISIONS.md");
    let proposal = decisions
        .lines()
        .find(|l| {
            l.starts_with(
                "* **[2026-10-01] Real-Face Embedding Evaluation and Recalibration Proposal",
            )
        })
        .expect("evaluation ADR");
    assert!(
        proposal.contains("Status: Accepted"),
        "the evaluation ADR must record the owner decision"
    );
    let adr = decisions
        .lines()
        .find(|l| l.starts_with("* **[2026-10-01] SFace Embedding Model Replaces ArcFace ResNet34"))
        .expect("SFace ADR");
    for needle in ["Apache-2.0", "owner-accepted", "MS1MV2", "0.50"] {
        assert!(adr.contains(needle), "the SFace ADR must state '{needle}'");
    }
}
