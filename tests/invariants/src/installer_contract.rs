//! Installer and onboarding contract tests.
//!
//! - GitHub #164 (ONB-06): `scripts/install.sh` fails closed on missing artifacts, never falls
//!   back to debug builds, requires root for a live install, never re-modes pre-existing system
//!   directories and rolls back every change when a step fails.
//! - GitHub #165 (ONB-07): build and runtime dependency lists are complete and consistent between
//!   `scripts/check_build_deps.sh`, the packaging metadata and the documentation.
//! - GitHub #167 (ONB-09): `scripts/download_models.sh` needs no Python, validates the manifest
//!   strictly and runs a tool preflight before touching the disk.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Artifacts that `install.sh` must find in its artifact directory.
pub(crate) const REQUIRED_ARTIFACTS: [&str; 5] = [
    "soos-daemon",
    "soos-admin",
    "soos-enroll",
    "soos-gui",
    "libpam_soos.so",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// Fresh, empty scratch directory unique to this test process.
pub(crate) fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soos_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Writes stub release artifacts (mode 0755, like cargo output) into `dir`.
pub(crate) fn stage_fixture_artifacts(dir: &Path) {
    fs::create_dir_all(dir).expect("create artifact dir");
    for name in REQUIRED_ARTIFACTS {
        let path = dir.join(name);
        fs::write(&path, format!("fixture:{name}\n")).expect("write fixture artifact");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod artifact");
    }
}

fn run_script<I, S>(script: &str, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new("bash")
        .arg(workspace_root().join(script))
        .args(args)
        .output()
        .expect("execute script")
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn combined(out: &Output) -> String {
    format!("{}{}", stdout_of(out), stderr_of(out))
}

/// Every non-directory entry below `dir`, relative to `dir`, sorted.
fn list_entries(dir: &Path) -> Vec<PathBuf> {
    fn walk(base: &Path, dir: &Path, files: &mut Vec<PathBuf>, dirs: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(base).expect("relative").to_path_buf();
            if entry.file_type().expect("file type").is_dir() {
                dirs.push(rel);
                walk(base, &path, files, dirs);
            } else {
                files.push(rel);
            }
        }
    }
    let (mut files, mut dirs) = (Vec::new(), Vec::new());
    walk(dir, dir, &mut files, &mut dirs);
    files.sort();
    files
}

/// Every directory below `dir`, relative to `dir`, sorted.
fn list_dirs(dir: &Path) -> Vec<PathBuf> {
    fn walk(base: &Path, dir: &Path, dirs: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().expect("file type").is_dir() {
                dirs.push(path.strip_prefix(base).expect("relative").to_path_buf());
                walk(base, &path, dirs);
            }
        }
    }
    let mut dirs = Vec::new();
    walk(dir, dir, &mut dirs);
    dirs.sort();
    dirs
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).expect("metadata").permissions().mode() & 0o7777
}

fn staging_args(stage: &Path, artifacts: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from("--destdir"),
        stage.to_path_buf(),
        PathBuf::from("--artifact-dir"),
        artifacts.to_path_buf(),
        PathBuf::from("--pam-dir"),
        PathBuf::from("/usr/lib/security"),
        PathBuf::from("--skip-systemd"),
    ]
}

fn current_uid_is_root() -> bool {
    let out = Command::new("id").arg("-u").output().expect("run id -u");
    String::from_utf8_lossy(&out.stdout).trim() == "0"
}

// ---------------------------------------------------------------------------
// GitHub #164 (ONB-06) — install.sh fails closed and is transactional
// ---------------------------------------------------------------------------

/// An empty artifact directory must abort before any file is written.
#[test]
fn test_install_fails_closed_when_artifact_dir_is_empty() {
    let stage = scratch_dir("inst_empty_stage");
    let artifacts = scratch_dir("inst_empty_artifacts");

    let mut args = staging_args(&stage, &artifacts);
    args.push(PathBuf::from("--skip-models"));
    let out = run_script("scripts/install.sh", &args);

    assert!(
        !out.status.success(),
        "install.sh must exit non-zero when no artifact exists; output:\n{}",
        combined(&out)
    );
    let err = stderr_of(&out);
    for name in REQUIRED_ARTIFACTS {
        assert!(
            err.contains(name),
            "stderr must name the missing artifact '{name}'; stderr:\n{err}"
        );
    }
    assert!(
        list_entries(&stage).is_empty() && list_dirs(&stage).is_empty(),
        "preflight failure must leave the destination untouched, found: {:?} {:?}",
        list_entries(&stage),
        list_dirs(&stage)
    );
    assert!(
        !stdout_of(&out).contains("completed successfully"),
        "a failed install must never report success"
    );

    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&artifacts);
}

/// A single missing artifact (the PAM module) is fatal unless `--allow-missing` is explicit.
#[test]
fn test_install_fails_closed_when_one_artifact_is_missing() {
    let stage = scratch_dir("inst_onemissing_stage");
    let artifacts = scratch_dir("inst_onemissing_artifacts");
    stage_fixture_artifacts(&artifacts);
    fs::remove_file(artifacts.join("libpam_soos.so")).expect("remove pam fixture");

    let mut args = staging_args(&stage, &artifacts);
    args.push(PathBuf::from("--skip-models"));
    let out = run_script("scripts/install.sh", &args);
    assert!(
        !out.status.success(),
        "a missing libpam_soos.so must be fatal; output:\n{}",
        combined(&out)
    );
    assert!(
        stderr_of(&out).contains("libpam_soos.so"),
        "stderr must name libpam_soos.so"
    );
    assert!(
        list_entries(&stage).is_empty(),
        "nothing may be installed when an artifact is missing, found: {:?}",
        list_entries(&stage)
    );

    // Explicit opt-in keeps a developer escape hatch, loudly.
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage).expect("recreate stage");
    args.push(PathBuf::from("--allow-missing"));
    let out = run_script("scripts/install.sh", &args);
    assert!(
        out.status.success(),
        "--allow-missing must accept a partial artifact set; output:\n{}",
        combined(&out)
    );
    assert!(stage.join("usr/libexec/soos/soos-daemon").is_file());
    assert!(!stage.join("usr/lib/security/pam_soos.so").exists());
    assert!(
        combined(&out).contains("INCOMPLETE"),
        "a partial install must be reported as INCOMPLETE"
    );

    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&artifacts);
}

/// Artifacts from a cargo `debug` profile directory are refused unless explicitly allowed,
/// and the installer never searches `target/debug` on its own.
#[test]
fn test_install_refuses_debug_profile_artifacts() {
    let stage = scratch_dir("inst_debug_stage");
    let root = scratch_dir("inst_debug_target");
    let debug_dir = root.join("target/debug");
    stage_fixture_artifacts(&debug_dir);

    let mut args = staging_args(&stage, &debug_dir);
    args.push(PathBuf::from("--skip-models"));
    let out = run_script("scripts/install.sh", &args);
    assert!(
        !out.status.success(),
        "debug-profile artifacts must be refused; output:\n{}",
        combined(&out)
    );
    assert!(
        stderr_of(&out).contains("debug"),
        "stderr must explain the debug refusal"
    );
    assert!(list_entries(&stage).is_empty(), "nothing may be installed");

    args.push(PathBuf::from("--allow-debug-artifacts"));
    let out = run_script("scripts/install.sh", &args);
    assert!(
        out.status.success(),
        "--allow-debug-artifacts must be an explicit override; output:\n{}",
        combined(&out)
    );

    let install_sh =
        fs::read_to_string(workspace_root().join("scripts/install.sh")).expect("read install.sh");
    assert!(
        !install_sh.contains("}/target/debug"),
        "install.sh must never search target/debug implicitly"
    );

    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&root);
}

/// A complete artifact set is staged with the documented names and modes.
#[test]
fn test_install_stages_every_artifact_with_expected_modes() {
    let stage = scratch_dir("inst_full_stage");
    let artifacts = scratch_dir("inst_full_artifacts");
    stage_fixture_artifacts(&artifacts);

    let mut args = staging_args(&stage, &artifacts);
    args.push(PathBuf::from("--skip-models"));
    let out = run_script("scripts/install.sh", &args);
    assert!(
        out.status.success(),
        "install.sh must succeed with a complete artifact set; output:\n{}",
        combined(&out)
    );

    for (artifact, installed, mode) in [
        ("soos-daemon", "usr/libexec/soos/soos-daemon", 0o755),
        ("soos-admin", "usr/bin/soos-admin", 0o755),
        ("soos-enroll", "usr/bin/soos-enroll", 0o755),
        ("soos-gui", "usr/bin/soos-gui", 0o755),
        ("libpam_soos.so", "usr/lib/security/pam_soos.so", 0o644),
    ] {
        let path = stage.join(installed);
        assert!(path.is_file(), "{installed} must be installed");
        assert_eq!(
            fs::read_to_string(&path).expect("read installed"),
            format!("fixture:{artifact}\n"),
            "{installed} must come from the artifact directory"
        );
        assert_eq!(mode_of(&path), mode, "{installed} mode");
    }
    assert!(
        stdout_of(&out).contains("completed successfully"),
        "a complete install reports success"
    );
    assert!(
        !list_entries(&stage)
            .iter()
            .any(|p| p.to_string_lossy().contains(".soos-")),
        "no temporary or rollback file may remain after success: {:?}",
        list_entries(&stage)
    );

    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&artifacts);
}

/// Pre-existing system directories keep their mode; soos-owned directories are enforced.
#[test]
fn test_install_never_chmods_preexisting_system_dirs() {
    let stage = scratch_dir("inst_chmod_stage");
    let artifacts = scratch_dir("inst_chmod_artifacts");
    stage_fixture_artifacts(&artifacts);
    for dir in ["usr/bin", "usr/lib/security", "var/lib/soos/biometrics"] {
        fs::create_dir_all(stage.join(dir)).expect("pre-create dir");
    }
    fs::set_permissions(stage.join("usr/bin"), fs::Permissions::from_mode(0o775)).expect("chmod");
    fs::set_permissions(
        stage.join("usr/lib/security"),
        fs::Permissions::from_mode(0o775),
    )
    .expect("chmod");
    fs::set_permissions(
        stage.join("var/lib/soos/biometrics"),
        fs::Permissions::from_mode(0o755),
    )
    .expect("chmod");

    let mut args = staging_args(&stage, &artifacts);
    args.push(PathBuf::from("--skip-models"));
    let out = run_script("scripts/install.sh", &args);
    assert!(out.status.success(), "install output:\n{}", combined(&out));

    assert_eq!(
        mode_of(&stage.join("usr/bin")),
        0o775,
        "pre-existing usr/bin must keep its mode"
    );
    assert_eq!(
        mode_of(&stage.join("usr/lib/security")),
        0o775,
        "pre-existing PAM module directory must keep its mode"
    );
    assert_eq!(
        mode_of(&stage.join("var/lib/soos/biometrics")),
        0o700,
        "soos-owned biometrics directory must be tightened to 0700"
    );

    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&artifacts);
}

/// A failure after files were written (model verification) rolls back every change: created
/// files and directories disappear and overwritten files get their previous content back.
#[test]
fn test_install_rolls_back_every_change_on_failure() {
    let stage = scratch_dir("inst_rollback_stage");
    let artifacts = scratch_dir("inst_rollback_artifacts");
    let fixtures = scratch_dir("inst_rollback_fixtures");
    stage_fixture_artifacts(&artifacts);

    // Pre-existing state that must survive untouched.
    fs::create_dir_all(stage.join("usr/bin")).expect("create usr/bin");
    let previous_admin = stage.join("usr/bin/soos-admin");
    fs::write(&previous_admin, b"previous-admin\n").expect("write previous admin");
    fs::set_permissions(&previous_admin, fs::Permissions::from_mode(0o755)).expect("chmod");
    let before_files = list_entries(&stage);
    let before_dirs = list_dirs(&stage);

    // A manifest whose checksum can never match: the model step fails late.
    let model_src = fixtures.join("model.onnx");
    fs::write(&model_src, b"not-the-attested-model").expect("write model");
    let manifest = fixtures.join("manifest.toml");
    fs::write(
        &manifest,
        format!(
            "[manifest]\nversion = \"2.0.0\"\n\n[models.bad_model]\nid = \"bad_model\"\n\
             filename = \"bad_model.onnx\"\nsha256 = \"{}\"\nlicense = \"MIT\"\n\
             source_url = \"file://{}\"\n",
            "0".repeat(64),
            model_src.display()
        ),
    )
    .expect("write manifest");

    let mut args = staging_args(&stage, &artifacts);
    args.push(PathBuf::from("--manifest"));
    args.push(manifest.clone());
    let out = run_script("scripts/install.sh", &args);

    assert!(
        !out.status.success(),
        "a model verification failure must fail the install; output:\n{}",
        combined(&out)
    );
    assert!(
        combined(&out).to_lowercase().contains("rolled back"),
        "the failure must report the rollback; output:\n{}",
        combined(&out)
    );
    assert_eq!(
        list_entries(&stage),
        before_files,
        "rollback must remove every file created by the failed run"
    );
    assert_eq!(
        list_dirs(&stage),
        before_dirs,
        "rollback must remove every directory created by the failed run"
    );
    assert_eq!(
        fs::read_to_string(&previous_admin).expect("read admin"),
        "previous-admin\n",
        "rollback must restore the previous content of overwritten files"
    );
    assert_eq!(
        mode_of(&previous_admin),
        0o755,
        "restored file keeps its mode"
    );

    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&artifacts);
    let _ = fs::remove_dir_all(&fixtures);
}

/// A live install (no `--destdir`) is refused for non-root users before any mutation.
#[test]
fn test_install_live_mode_requires_root() {
    if current_uid_is_root() {
        // Never run a live install against the real system from the test suite.
        return;
    }
    let artifacts = scratch_dir("inst_root_artifacts");
    stage_fixture_artifacts(&artifacts);

    let out = run_script(
        "scripts/install.sh",
        [
            OsStr::new("--artifact-dir"),
            artifacts.as_os_str(),
            OsStr::new("--skip-models"),
            OsStr::new("--skip-systemd"),
        ],
    );
    assert!(
        !out.status.success(),
        "a non-root live install must fail; output:\n{}",
        combined(&out)
    );
    assert!(
        stderr_of(&out).contains("root"),
        "stderr must explain that root is required; stderr:\n{}",
        stderr_of(&out)
    );
    assert!(
        !stdout_of(&out).contains("Provisioning directories"),
        "no provisioning step may start before the root check"
    );

    let _ = fs::remove_dir_all(&artifacts);
}

/// `--dry-run` runs the read-only preflight: missing artifacts fail it, nothing is written.
#[test]
fn test_install_dry_run_runs_preflight_without_mutation() {
    let stage = scratch_dir("inst_dryrun_stage");
    let empty = scratch_dir("inst_dryrun_empty");
    let artifacts = scratch_dir("inst_dryrun_artifacts");
    stage_fixture_artifacts(&artifacts);

    let mut args = staging_args(&stage, &empty);
    args.push(PathBuf::from("--skip-models"));
    args.push(PathBuf::from("--dry-run"));
    let out = run_script("scripts/install.sh", &args);
    assert!(
        !out.status.success(),
        "a dry run with missing artifacts must fail its preflight"
    );

    let mut args = staging_args(&stage, &artifacts);
    args.push(PathBuf::from("--dry-run"));
    let out = run_script("scripts/install.sh", &args);
    assert!(
        out.status.success(),
        "a dry run with a complete artifact set passes; output:\n{}",
        combined(&out)
    );
    assert!(
        list_entries(&stage).is_empty() && list_dirs(&stage).is_empty(),
        "a dry run never writes"
    );

    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&empty);
    let _ = fs::remove_dir_all(&artifacts);
}

/// Static ordering and build contract: models are verified before the unit is enabled, the
/// optional build is an explicit locked release build, and the unit cannot crash-loop forever.
#[test]
fn test_install_orders_models_before_unit_enable_and_builds_release() {
    let root = workspace_root();
    let install_sh = fs::read_to_string(root.join("scripts/install.sh")).expect("read install.sh");

    let models_pos = install_sh
        .find("download_models.sh\" \\")
        .expect("install.sh must invoke download_models.sh");
    let enable_pos = install_sh
        .find("systemctl enable")
        .expect("install.sh must enable the unit on a live install");
    assert!(
        models_pos < enable_pos,
        "models must be deployed and verified before the unit is enabled"
    );
    assert!(
        install_sh.contains("cargo build --release --locked --workspace"),
        "install.sh --build must run an explicit locked release build"
    );
    assert!(
        install_sh.contains("check_build_deps.sh"),
        "install.sh --build must run the build dependency preflight first"
    );

    let unit =
        fs::read_to_string(root.join("packaging/soos-daemon.service")).expect("read unit file");
    let unit_section = unit.split("[Service]").next().expect("[Unit] section");
    assert!(
        unit_section.contains("StartLimitIntervalSec=")
            && unit_section.contains("StartLimitBurst="),
        "soos-daemon.service must bound restarts in [Unit] (no endless crash loop)"
    );
}

// ---------------------------------------------------------------------------
// GitHub #167 (ONB-09) — download_models.sh without Python, with preflight
// ---------------------------------------------------------------------------

/// Builds a PATH directory exposing only common base tools (never python or curl).
fn restricted_path(tag: &str) -> PathBuf {
    let bin = scratch_dir(tag);
    let tools = [
        "bash",
        "sh",
        "env",
        "dirname",
        "basename",
        "cat",
        "cp",
        "mv",
        "rm",
        "mkdir",
        "chmod",
        "chown",
        "id",
        "sha256sum",
        "mktemp",
        "head",
        "tail",
        "tr",
        "cut",
        "sed",
        "awk",
        "grep",
        "ls",
        "ln",
        "stat",
        "touch",
        "sort",
        "wc",
        "uname",
        "find",
        "rmdir",
        "install",
    ];
    let path_var = std::env::var_os("PATH").expect("PATH");
    for tool in tools {
        if let Some(found) = std::env::split_paths(&path_var)
            .map(|d| d.join(tool))
            .find(|p| p.is_file())
        {
            std::os::unix::fs::symlink(&found, bin.join(tool)).expect("symlink tool");
        }
    }
    for forbidden in ["python", "python3", "curl"] {
        assert!(
            !bin.join(forbidden).exists(),
            "restricted PATH must not expose {forbidden}"
        );
    }
    bin
}

fn run_restricted<I, S>(bin: &Path, script: &str, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(bin.join("bash"))
        .arg(workspace_root().join(script))
        .args(args)
        .env_clear()
        .env("PATH", bin)
        .env("LC_ALL", "C")
        .output()
        .expect("execute script with restricted PATH")
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

/// Deploy, verify and tamper-detect local models with no python3 and no curl on PATH.
#[test]
fn test_download_models_deploys_and_verifies_without_python() {
    let bin = restricted_path("dm_nopy_bin");
    let work = scratch_dir("dm_nopy_work");
    let src = work.join("src.onnx");
    fs::write(&src, b"attested-model-bytes").expect("write model");
    let manifest = work.join("manifest.toml");
    fs::write(
        &manifest,
        format!(
            "# fixture\n[manifest]\nversion = \"2.0.0\"\n\n[models.m1]\nid = \"m1\"\n\
             filename = \"m1.onnx\"\nsha256 = \"{}\"\nlicense = \"MIT\"  # trailing comment\n\
             source_url = \"file://{}\"\ninput_shape = [1, 3, 80, 80]\noutput_shapes = [\n    \
             [1, 3],\n]\n",
            sha256_hex(&src),
            src.display()
        ),
    )
    .expect("write manifest");
    let target = work.join("models");

    let out = run_restricted(
        &bin,
        "scripts/download_models.sh",
        [
            OsStr::new("--manifest"),
            manifest.as_os_str(),
            OsStr::new("--target-dir"),
            target.as_os_str(),
        ],
    );
    assert!(
        out.status.success(),
        "download_models.sh must work without python3; output:\n{}",
        combined(&out)
    );
    let deployed = target.join("m1.onnx");
    assert_eq!(fs::read(&deployed).expect("read"), b"attested-model-bytes");
    assert_eq!(mode_of(&deployed), 0o644, "model files are 0644");
    assert_eq!(mode_of(&target), 0o755, "models dir is 0755");
    assert!(target.join("manifest.toml").is_file(), "manifest deployed");

    let check = |expect_ok: bool| {
        let out = run_restricted(
            &bin,
            "scripts/download_models.sh",
            [
                OsStr::new("--check-only"),
                OsStr::new("--manifest"),
                manifest.as_os_str(),
                OsStr::new("--target-dir"),
                target.as_os_str(),
            ],
        );
        assert_eq!(
            out.status.success(),
            expect_ok,
            "--check-only result; output:\n{}",
            combined(&out)
        );
    };
    check(true);
    fs::write(&deployed, b"tampered").expect("tamper");
    check(false);

    let _ = fs::remove_dir_all(&bin);
    let _ = fs::remove_dir_all(&work);
}

/// The repository manifest parses without python and yields every attested entry.
#[test]
fn test_download_models_parses_repository_manifest_without_python() {
    let bin = restricted_path("dm_repo_bin");
    let root = workspace_root();
    let manifest = root.join("models/manifest.toml");
    let out = run_restricted(
        &bin,
        "scripts/download_models.sh",
        [
            OsStr::new("--dry-run"),
            OsStr::new("--manifest"),
            manifest.as_os_str(),
        ],
    );
    assert!(
        out.status.success(),
        "dry-run on models/manifest.toml must succeed without python; output:\n{}",
        combined(&out)
    );
    let stdout = stdout_of(&out);
    let text = fs::read_to_string(&manifest).expect("read manifest");
    let mut count = 0;
    for line in text.lines() {
        for key in ["sha256 = \"", "filename = \""] {
            if let Some(rest) = line.strip_prefix(key) {
                let value = rest.trim_end_matches('"');
                assert!(
                    stdout.contains(value),
                    "dry-run must report {key}{value}; stdout:\n{stdout}"
                );
                if key.starts_with("sha256") {
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 3, "the repository manifest attests three models");
    assert!(
        stdout.contains("3 models cataloged"),
        "dry-run must count every model; stdout:\n{stdout}"
    );

    let _ = fs::remove_dir_all(&bin);
}

/// Malformed or unsafe manifest entries are rejected before anything is written.
#[test]
fn test_download_models_rejects_malformed_or_unsafe_entries() {
    let work = scratch_dir("dm_reject_work");
    let src = work.join("src.onnx");
    fs::write(&src, b"bytes").expect("write model");
    let good_sha = sha256_hex(&src);
    let url = format!("file://{}", src.display());

    let cases = [
        (
            "path traversal",
            "../escape.onnx",
            good_sha.as_str(),
            url.as_str(),
        ),
        (
            "absolute path",
            "/tmp/escape.onnx",
            good_sha.as_str(),
            url.as_str(),
        ),
        (
            "hidden file",
            ".hidden.onnx",
            good_sha.as_str(),
            url.as_str(),
        ),
        ("short digest", "m.onnx", "abc123", url.as_str()),
        ("non-hex digest", "m.onnx", &"z".repeat(64), url.as_str()),
        ("empty url", "m.onnx", good_sha.as_str(), ""),
        (
            "plain http",
            "m.onnx",
            good_sha.as_str(),
            "http://example.invalid/m.onnx",
        ),
    ];
    for (label, filename, sha, source) in cases {
        let manifest = work.join("manifest.toml");
        fs::write(
            &manifest,
            format!(
                "[models.m]\nid = \"m\"\nfilename = \"{filename}\"\nsha256 = \"{sha}\"\n\
                 license = \"MIT\"\nsource_url = \"{source}\"\n"
            ),
        )
        .expect("write manifest");
        let target = work.join("models");
        let _ = fs::remove_dir_all(&target);
        let out = run_script(
            "scripts/download_models.sh",
            [
                OsStr::new("--manifest"),
                manifest.as_os_str(),
                OsStr::new("--target-dir"),
                target.as_os_str(),
            ],
        );
        assert!(
            !out.status.success(),
            "{label}: malformed entry must be rejected; output:\n{}",
            combined(&out)
        );
        assert!(
            !target.exists(),
            "{label}: rejection must happen before the target directory is created"
        );
        assert!(!work.join("escape.onnx").exists(), "{label}: no escape");
    }

    // Missing sha256 key and an empty manifest are rejected too.
    for (label, body) in [
        (
            "missing sha256",
            format!("[models.m]\nfilename = \"m.onnx\"\nsource_url = \"{url}\"\n"),
        ),
        ("no models", "[manifest]\nversion = \"2.0.0\"\n".to_string()),
    ] {
        let manifest = work.join("manifest.toml");
        fs::write(&manifest, body).expect("write manifest");
        let out = run_script(
            "scripts/download_models.sh",
            [
                OsStr::new("--dry-run"),
                OsStr::new("--manifest"),
                manifest.as_os_str(),
            ],
        );
        assert!(!out.status.success(), "{label}: must be rejected");
    }

    let _ = fs::remove_dir_all(&work);
}

/// Remote sources need curl: the preflight reports it and nothing is written.
#[test]
fn test_download_models_preflight_requires_curl_for_remote_sources() {
    let bin = restricted_path("dm_curl_bin");
    let work = scratch_dir("dm_curl_work");
    let manifest = work.join("manifest.toml");
    fs::write(
        &manifest,
        format!(
            "[models.m]\nid = \"m\"\nfilename = \"m.onnx\"\nsha256 = \"{}\"\n\
             license = \"MIT\"\nsource_url = \"https://example.invalid/m.onnx\"\n",
            "a".repeat(64)
        ),
    )
    .expect("write manifest");
    let target = work.join("models");

    for extra in [None, Some("--preflight")] {
        let mut args: Vec<&OsStr> = vec![
            OsStr::new("--manifest"),
            manifest.as_os_str(),
            OsStr::new("--target-dir"),
            target.as_os_str(),
        ];
        if let Some(flag) = extra {
            args.push(OsStr::new(flag));
        }
        let out = run_restricted(&bin, "scripts/download_models.sh", args);
        assert!(
            !out.status.success(),
            "missing curl must fail the preflight ({extra:?})"
        );
        assert!(
            stderr_of(&out).contains("curl"),
            "stderr must name curl; stderr:\n{}",
            stderr_of(&out)
        );
        assert!(!target.exists(), "preflight failure writes nothing");
    }

    let script = fs::read_to_string(workspace_root().join("scripts/download_models.sh"))
        .expect("read download_models.sh");
    assert!(
        !script.contains("python") && !script.contains("tomllib"),
        "download_models.sh must not depend on python or tomllib"
    );

    let _ = fs::remove_dir_all(&bin);
    let _ = fs::remove_dir_all(&work);
}

// ---------------------------------------------------------------------------
// GitHub #165 (ONB-07) — complete and consistent dependency lists
// ---------------------------------------------------------------------------

fn print_packages(distro: &str, group: &str) -> Vec<String> {
    let out = run_script(
        "scripts/check_build_deps.sh",
        ["--distro", distro, "--print-packages", group],
    );
    assert!(
        out.status.success(),
        "check_build_deps.sh --distro {distro} --print-packages {group} must succeed; output:\n{}",
        combined(&out)
    );
    let pkgs: Vec<String> = stdout_of(&out)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    assert!(!pkgs.is_empty(), "{distro}/{group} package list is empty");
    pkgs
}

/// The helper's package lists contain the packages the locked dependency graph needs.
#[test]
fn test_check_build_deps_lists_required_packages() {
    let root = workspace_root();
    let helper = root.join("scripts/check_build_deps.sh");
    assert!(helper.is_file(), "scripts/check_build_deps.sh must exist");
    assert_ne!(
        mode_of(&helper) & 0o111,
        0,
        "scripts/check_build_deps.sh must be executable"
    );

    for (distro, required) in [
        (
            "debian",
            &[
                "build-essential",
                "pkg-config",
                "libpam0g-dev",
                "libclang-dev",
                "clang",
                "libssl-dev",
            ][..],
        ),
        (
            "fedora",
            &[
                "gcc",
                "gcc-c++",
                "make",
                "pkgconf-pkg-config",
                "pam-devel",
                "clang-devel",
                "openssl-devel",
            ][..],
        ),
        (
            "arch",
            &["base-devel", "clang", "openssl", "pkgconf", "pam"][..],
        ),
    ] {
        let build = print_packages(distro, "build");
        for pkg in required {
            assert!(
                build.iter().any(|p| p == pkg),
                "{distro} build list must contain {pkg}: {build:?}"
            );
        }
        let models = print_packages(distro, "models");
        for pkg in ["curl", "ca-certificates"] {
            assert!(
                models.iter().any(|p| p == pkg),
                "{distro} model-fetch list must contain {pkg}: {models:?}"
            );
        }
        assert!(
            !models.iter().any(|p| p.starts_with("python")),
            "model fetching must not require python ({distro})"
        );
        print_packages(distro, "gui");
    }

    let out = run_script(
        "scripts/check_build_deps.sh",
        ["--distro", "plan9", "--print-packages", "build"],
    );
    assert!(!out.status.success(), "an unknown distro must be rejected");

    // Preflight mode: a PATH without cargo fails and names the missing tool.
    let bin = restricted_path("deps_nocargo_bin");
    let out = run_restricted(&bin, "scripts/check_build_deps.sh", ["--distro", "debian"]);
    assert!(
        !out.status.success(),
        "the build preflight must fail without cargo"
    );
    assert!(
        combined(&out).contains("cargo"),
        "the preflight must name cargo; output:\n{}",
        combined(&out)
    );
    let _ = fs::remove_dir_all(&bin);
}

/// Packaging metadata and the installation documentation match the helper's lists.
#[test]
fn test_packaging_and_docs_declare_complete_dependencies() {
    let root = workspace_root();
    let read = |rel: &str| fs::read_to_string(root.join(rel)).expect(rel);
    let control = read("packaging/debian/control");
    let build_deb = read("scripts/build_deb.sh");
    let spec = read("packaging/rpm/soos.spec");
    let pkgbuild = read("packaging/arch/PKGBUILD");
    let docs = read("Docs/PACKAGING_AND_PROVISIONING.md");
    let readme = read("README.md");

    let build_depends = control
        .lines()
        .find(|l| l.starts_with("Build-Depends:"))
        .expect("Build-Depends line");
    for pkg in [
        "libssl-dev",
        "pkg-config",
        "libclang-dev",
        "clang",
        "libpam0g-dev",
    ] {
        assert!(
            build_depends.contains(pkg),
            "debian/control Build-Depends must contain {pkg}"
        );
    }
    for pkg in [
        "openssl-devel",
        "pkgconf-pkg-config",
        "clang-devel",
        "pam-devel",
    ] {
        assert!(
            spec.lines()
                .any(|l| l.starts_with("BuildRequires:") && l.contains(pkg)),
            "soos.spec must declare BuildRequires: {pkg}"
        );
    }
    let makedepends = pkgbuild
        .lines()
        .find(|l| l.starts_with("makedepends="))
        .expect("makedepends line");
    for pkg in ["'clang'", "'openssl'", "'pkgconf'"] {
        assert!(
            makedepends.contains(pkg),
            "PKGBUILD makedepends must contain {pkg}"
        );
    }

    // The .deb ships soos-gui: its dlopen'ed runtime libraries are recommended.
    for (name, text) in [
        ("packaging/debian/control", control.as_str()),
        ("scripts/build_deb.sh", build_deb.as_str()),
    ] {
        let recommends = text
            .lines()
            .find(|l| l.starts_with("Recommends:"))
            .unwrap_or_else(|| panic!("{name} must declare Recommends:"));
        for pkg in print_packages("debian", "gui") {
            assert!(
                recommends.contains(&pkg),
                "{name} Recommends must contain GUI runtime library {pkg}"
            );
        }
    }

    for distro in ["debian", "fedora", "arch"] {
        for group in ["build", "gui", "models"] {
            for pkg in print_packages(distro, group) {
                assert!(
                    docs.contains(&pkg),
                    "Docs/PACKAGING_AND_PROVISIONING.md must list {distro}/{group} package {pkg}"
                );
            }
        }
    }
    for needle in [
        "scripts/check_build_deps.sh",
        "install.sh --build",
        "ORT_LIB_LOCATION",
    ] {
        assert!(
            docs.contains(needle),
            "Docs/PACKAGING_AND_PROVISIONING.md must document {needle}"
        );
    }
    for needle in ["scripts/check_build_deps.sh", "scripts/install.sh --build"] {
        assert!(
            readme.contains(needle),
            "README.md must document {needle} in its installation section"
        );
    }
}
