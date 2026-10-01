//! `scripts/download_models.sh` expected model sizes (GitHub #269, review finding VIS-15;
//! rows VMX1–VMX3).
//!
//! The phantom `tomllib_fallback` import and the unbounded `curl` transfer named by the finding
//! were already removed (GitHub #167 / #208: pure-bash manifest parser, `--max-time 900`). The
//! remaining recommendation is to print the expected file size from the manifest: every entry of
//! `models/manifest.toml` now declares an optional `size_bytes`, which the script prints, uses as
//! the per-model `curl --max-filesize` bound and enforces exactly before hashing (fail closed).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contract test suite uses assertions and indexing"
)]

use std::ffi::OsStr;
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

fn run_download<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new("bash")
        .arg(workspace_root().join("scripts/download_models.sh"))
        .args(args)
        .env_remove("SOOS_MODEL_MAX_BYTES")
        .output()
        .expect("execute download_models.sh")
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Writes a one-model fixture manifest whose `size_bytes` line is `size_line` (verbatim, may be
/// empty) and returns `(work dir, manifest path, target dir)`.
fn fixture(tag: &str, size_line: &str) -> (PathBuf, PathBuf, PathBuf) {
    let work = scratch_dir(tag);
    let src = work.join("src.onnx");
    fs::write(&src, MODEL_BYTES).expect("write model");
    let manifest = work.join("manifest.toml");
    fs::write(
        &manifest,
        format!(
            "[manifest]\nversion = \"2.0.0\"\n\n[models.m1]\nid = \"m1\"\n\
             filename = \"m1.onnx\"\nsha256 = \"{}\"\nlicense = \"MIT\"\n{size_line}\n\
             source_url = \"file://{}\"\ninput_shape = [1, 3, 80, 80]\n",
            sha256_hex(&src),
            src.display()
        ),
    )
    .expect("write manifest");
    let target = work.join("models");
    (work, manifest, target)
}

fn deploy(manifest: &Path, target: &Path) -> Output {
    run_download([
        OsStr::new("--manifest"),
        manifest.as_os_str(),
        OsStr::new("--target-dir"),
        target.as_os_str(),
    ])
}

/// VMX1: the declared size is printed in the dry-run plan and during a deployment, and a
/// matching size deploys normally.
#[test]
fn test_vmx_download_models_prints_expected_size_and_deploys_matching_file() {
    let size = MODEL_BYTES.len();
    let (work, manifest, target) = fixture("vmx_size_ok", &format!("size_bytes = {size}"));

    let dry = run_download([
        OsStr::new("--dry-run"),
        OsStr::new("--manifest"),
        manifest.as_os_str(),
    ]);
    let dry_out = combined(&dry);
    assert!(dry.status.success(), "dry-run output:\n{dry_out}");
    assert!(
        dry_out.contains(&format!("{size} bytes")),
        "the dry-run plan prints the expected size; output:\n{dry_out}"
    );

    let out = deploy(&manifest, &target);
    let text = combined(&out);
    assert!(out.status.success(), "deploy output:\n{text}");
    assert!(
        text.contains(&format!("{size} bytes")),
        "the deployment prints the expected size; output:\n{text}"
    );
    assert_eq!(fs::read(target.join("m1.onnx")).expect("read"), MODEL_BYTES);
    let _ = fs::remove_dir_all(&work);
}

/// VMX2: a file whose size differs from the declared `size_bytes` is discarded fail-closed
/// before it is hashed or installed, and an invalid or over-cap `size_bytes` fails the manifest
/// validation before any write.
#[test]
fn test_vmx_download_models_rejects_size_mismatch_and_invalid_size_fail_closed() {
    let wrong = MODEL_BYTES.len() + 1;
    let (work, manifest, target) = fixture("vmx_size_bad", &format!("size_bytes = {wrong}"));
    let out = deploy(&manifest, &target);
    let text = combined(&out);
    assert!(
        !out.status.success(),
        "size mismatch must fail; output:\n{text}"
    );
    assert!(
        text.to_lowercase().contains("size mismatch"),
        "the failure names the size mismatch; output:\n{text}"
    );
    assert!(
        !target.join("m1.onnx").exists(),
        "a mis-sized file is never installed"
    );
    let leftovers: Vec<_> = fs::read_dir(&target)
        .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "no temporary file survives: {leftovers:?}"
    );
    let _ = fs::remove_dir_all(&work);

    for (tag, line) in [
        ("vmx_size_quoted", "size_bytes = \"20\""),
        ("vmx_size_negative", "size_bytes = -20"),
        ("vmx_size_zero", "size_bytes = 0"),
        ("vmx_size_overcap", "size_bytes = 268435457"),
        ("vmx_size_dup", "size_bytes = 20\nsize_bytes = 20"),
    ] {
        let (work, manifest, target) = fixture(tag, line);
        let out = deploy(&manifest, &target);
        assert!(
            !out.status.success(),
            "{line:?} must fail the manifest validation; output:\n{}",
            combined(&out)
        );
        assert!(
            !target.exists(),
            "{line:?}: nothing is written before validation"
        );
        let _ = fs::remove_dir_all(&work);
    }
}

/// VMX3: every entry of the repository manifest declares `size_bytes`, the dry-run prints it,
/// and `curl` is bounded by the per-model size rather than only the global cap.
#[test]
fn test_vmx_repository_manifest_declares_every_model_size() {
    let manifest = read_repo("models/manifest.toml");
    let tables = manifest
        .lines()
        .filter(|l| l.trim_start().starts_with("[models."))
        .count();
    let sizes: Vec<u64> = manifest
        .lines()
        .filter_map(|l| l.trim().strip_prefix("size_bytes"))
        .map(|rest| {
            rest.trim_start_matches([' ', '='])
                .trim()
                .parse::<u64>()
                .expect("size_bytes is a bare integer")
        })
        .collect();
    assert!(tables >= 3, "the manifest declares the attested models");
    assert_eq!(sizes.len(), tables, "every model declares size_bytes");
    assert!(sizes.iter().all(|&s| s > 0 && s <= 268_435_456));

    let out = run_download([
        OsStr::new("--dry-run"),
        OsStr::new("--manifest"),
        workspace_root().join("models/manifest.toml").as_os_str(),
    ]);
    let text = combined(&out);
    assert!(out.status.success(), "dry-run output:\n{text}");
    for size in &sizes {
        assert!(
            text.contains(&format!("{size} bytes")),
            "dry-run prints {size} bytes; output:\n{text}"
        );
    }

    let script = read_repo("scripts/download_models.sh");
    let curl_start = script.find("curl -fSL").expect("curl invocation");
    let curl_cmd: String = script[curl_start..]
        .lines()
        .take_while(|l| !l.contains("||"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        curl_cmd.contains("--max-filesize \"${curl_max_bytes}\""),
        "curl is bounded by the per-model size; command: {curl_cmd}"
    );
    assert!(
        !script.contains("tomllib_fallback"),
        "no phantom TOML fallback module"
    );
}
