//! Second PAD model attestation (GitHub #212, review finding PAD-07; rows PVA1–PVA4).
//!
//! The 4.0x MiniFASNetV1SE export is attested in `models/optional_models.toml` (SHA-256,
//! size, licence, commit-pinned source, upstream weights and conversion script) but stays
//! disabled: `models/manifest.toml` does not declare it, no runtime crate reads the optional
//! file, and `scripts/download_models.sh` fetches it only with `--with-optional`, without
//! ever writing it into the deployed manifest.

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

const V1SE_ID: &str = "minifasnet_v1se_pad";
const OPTIONAL_FILE: &str = "models/optional_models.toml";
const CONVERSION_SCRIPT: &str = "scripts/convert_pad_models.py";

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

/// Lines of the `[models.<id>]` table of `toml` (up to the next table header).
fn table_lines<'a>(toml: &'a str, id: &str) -> Vec<&'a str> {
    let header = format!("[models.{id}]");
    let mut lines = toml.lines().skip_while(|l| l.trim() != header);
    assert!(lines.next().is_some(), "missing table {header}");
    lines
        .take_while(|l| !l.trim_start().starts_with('['))
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

/// Raw right-hand side of `key = value` in `lines`, with string quotes removed.
fn value<'a>(lines: &[&'a str], key: &str) -> &'a str {
    let matches: Vec<&str> = lines
        .iter()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == key).then(|| v.trim().trim_matches('"'))
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "exactly one '{key}' expected: {matches:?}"
    );
    matches[0]
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The URL embeds a 40-hex commit and no branch name.
fn is_commit_pinned(url: &str) -> bool {
    url.starts_with("https://")
        && url.split('/').any(|seg| is_hex(seg, 40))
        && !url.contains("/main/")
        && !url.contains("/master/")
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
        .env_remove("SOOS_OPTIONAL_MANIFEST_PATH")
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

fn entry(id: &str, file: &str, sha: &str, src: &Path) -> String {
    format!(
        "\n[models.{id}]\nid = \"{id}\"\nfilename = \"{file}\"\nsha256 = \"{sha}\"\n\
         license = \"MIT\"\nsize_bytes = {}\nsource_url = \"file://{}\"\n\
         input_shape = [1, 3, 80, 80]\n",
        fs::metadata(src).expect("source size").len(),
        src.display()
    )
}

/// `(work, main manifest, optional file, target)` with one main model `m1` and one optional
/// model `o1` whose attested digest is `optional_sha` (the real digest when `None`).
fn fixture(tag: &str, optional_sha: Option<&str>, optional_id: &str) -> [PathBuf; 4] {
    let work = scratch_dir(tag);
    let main_src = work.join("main.onnx");
    let opt_src = work.join("optional.onnx");
    fs::write(&main_src, b"main-model-bytes").expect("write main model");
    fs::write(&opt_src, b"optional-model-bytes").expect("write optional model");
    let main = work.join("manifest.toml");
    fs::write(
        &main,
        format!(
            "[manifest]\nversion = \"2.0.0\"\n{}",
            entry("m1", "m1.onnx", &sha256_hex(&main_src), &main_src)
        ),
    )
    .expect("write manifest");
    let optional = work.join("optional_models.toml");
    let sha = optional_sha.map_or_else(|| sha256_hex(&opt_src), str::to_string);
    fs::write(
        &optional,
        format!(
            "[manifest]\nversion = \"2.0.0\"\n{}",
            entry(optional_id, "o1.onnx", &sha, &opt_src)
        ),
    )
    .expect("write optional models");
    let target = work.join("models");
    [work, main, optional, target]
}

fn deploy(main: &Path, optional: &Path, target: &Path, with_optional: bool) -> Output {
    let mut args = vec![
        OsStr::new("--manifest"),
        main.as_os_str(),
        OsStr::new("--optional-manifest"),
        optional.as_os_str(),
        OsStr::new("--target-dir"),
        target.as_os_str(),
    ];
    if with_optional {
        args.push(OsStr::new("--with-optional"));
    }
    run_download(args)
}

/// PVA1: the optional V1SE entry is fully attested: SHA-256, exact size, Apache-2.0 licence,
/// commit-pinned ONNX source, commit-pinned upstream weights with their digest, and a
/// conversion script in the repository that pins the same commit and weights digest.
#[test]
fn test_pva_optional_v1se_entry_is_attested_and_commit_pinned() {
    let optional = read_repo(OPTIONAL_FILE);
    let lines = table_lines(&optional, V1SE_ID);
    assert_eq!(value(&lines, "id"), V1SE_ID);
    assert_eq!(value(&lines, "filename"), "minifasnet_v1se_80x80.onnx");
    assert!(is_hex(value(&lines, "sha256"), 64), "SHA-256 is 64 hex");
    let size: u64 = value(&lines, "size_bytes").parse().expect("bare integer");
    assert!(
        size > 0 && size <= 268_435_456,
        "size within the download cap"
    );
    assert_eq!(value(&lines, "license"), "Apache-2.0");
    assert_eq!(value(&lines, "upstream_license"), "Apache-2.0");
    assert_eq!(value(&lines, "input_shape"), "[1, 3, 80, 80]");
    assert_eq!(value(&lines, "input_layout"), "NCHW");
    assert_eq!(value(&lines, "output_shapes"), "[[1, 3]]");
    for key in ["source_url", "upstream_weights_url"] {
        let url = value(&lines, key);
        assert!(is_commit_pinned(url), "{key} must be commit-pinned: {url}");
    }
    let commit = value(&lines, "upstream_commit");
    assert!(is_hex(commit, 40));
    assert!(value(&lines, "upstream_weights_url").contains(commit));
    let weights_sha = value(&lines, "upstream_weights_sha256");
    assert!(is_hex(weights_sha, 64));
    assert!(is_hex(value(&lines, "converted_sha256"), 64));

    assert_eq!(value(&lines, "conversion_script"), CONVERSION_SCRIPT);
    let script = read_repo(CONVERSION_SCRIPT);
    assert!(script.contains(commit), "conversion script pins {commit}");
    assert!(
        script.contains(weights_sha),
        "conversion script pins the weights digest"
    );
    for pin in ["torch==", "onnx==", "onnxruntime==", "numpy=="] {
        assert!(script.contains(pin), "conversion script pins {pin}");
    }

    let gitignore = read_repo(".gitignore");
    assert!(
        gitignore.lines().any(|l| l.trim() == "models/*.pth"),
        "upstream checkpoints are never committed"
    );
}

/// PVA2: the second model is disabled by default: the main manifest does not declare it, no
/// runtime crate reads the optional file, and the default download plan does not list it.
#[test]
fn test_pva_v1se_disabled_by_default() {
    assert!(
        !read_repo("models/manifest.toml").contains(V1SE_ID),
        "models/manifest.toml must not declare {V1SE_ID} (disabled by default)"
    );
    let crates = workspace_root().join("crates");
    let mut offenders = Vec::new();
    for krate in fs::read_dir(&crates).expect("crates dir").flatten() {
        let src = krate.path().join("src");
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && fs::read_to_string(&path)
                        .unwrap_or_default()
                        .contains("optional_models")
                {
                    offenders.push(path.display().to_string());
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "no runtime crate may read optional_models.toml: {offenders:?}"
    );

    let out = run_download([
        OsStr::new("--dry-run"),
        OsStr::new("--manifest"),
        workspace_root().join("models/manifest.toml").as_os_str(),
    ]);
    let text = combined(&out);
    assert!(out.status.success(), "dry-run output:\n{text}");
    assert!(
        !text.contains(V1SE_ID),
        "default plan lists {V1SE_ID}:\n{text}"
    );

    let out = run_download([
        OsStr::new("--dry-run"),
        OsStr::new("--with-optional"),
        OsStr::new("--manifest"),
        workspace_root().join("models/manifest.toml").as_os_str(),
    ]);
    let text = combined(&out);
    assert!(out.status.success(), "dry-run output:\n{text}");
    assert!(
        text.contains(&format!("Optional model '{V1SE_ID}' (disabled)")),
        "--with-optional lists the disabled member:\n{text}"
    );
}

/// PVA3: `--with-optional` fetches and verifies the optional model but the deployed
/// manifest stays the main manifest (the model is not enabled); without the flag the
/// optional model is never deployed.
#[test]
fn test_pva_download_with_optional_fetches_but_never_enables() {
    let [_work, main, optional, target] = fixture("pva3-default", None, "o1");
    let out = deploy(&main, &optional, &target, false);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(target.join("m1.onnx").is_file());
    assert!(
        !target.join("o1.onnx").exists(),
        "optional model deployed without the flag"
    );

    let [_work, main, optional, target] = fixture("pva3-optional", None, "o1");
    let out = deploy(&main, &optional, &target, true);
    let text = combined(&out);
    assert!(out.status.success(), "{text}");
    assert_eq!(
        fs::read(target.join("o1.onnx")).expect("optional model deployed"),
        b"optional-model-bytes"
    );
    assert_eq!(
        fs::read(target.join("manifest.toml")).expect("deployed manifest"),
        fs::read(&main).expect("main manifest"),
        "the deployed manifest must stay the main manifest (optional model not enabled)"
    );
    assert!(text.contains("stay DISABLED"), "{text}");
}

/// PVA4: an optional entry goes through the same fail-closed checks: a digest mismatch is
/// discarded, and an id that collides with a main-manifest model is rejected before any write.
#[test]
fn test_pva_optional_models_fail_closed() {
    let bad = "0".repeat(64);
    let [_work, main, optional, target] = fixture("pva4-sha", Some(&bad), "o1");
    let out = deploy(&main, &optional, &target, true);
    assert!(!out.status.success(), "{}", combined(&out));
    assert!(
        !target.join("o1.onnx").exists(),
        "mismatched optional model kept"
    );

    let [_work, main, optional, target] = fixture("pva4-dup", None, "m1");
    let out = deploy(&main, &optional, &target, true);
    let text = combined(&out);
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("duplicate model table"), "{text}");
    assert!(!target.exists(), "no write before validation");

    let [_work, main, _optional, target] = fixture("pva4-missing", None, "o1");
    let missing = target.join("absent.toml");
    let out = deploy(&main, &missing, &target, true);
    assert!(!out.status.success(), "{}", combined(&out));
}
