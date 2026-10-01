//! Installer distribution templates, model download hardening and first-start guards.
//!
//! - GitHub #208 (ONB-12): `scripts/download_models.sh` uses the manifest as the single source
//!   of download URLs (no legacy fallback table), hardens curl and bounds every model file size
//!   before hashing, and never uses a predictable temporary file name.
//! - GitHub #209 (ONB-13): `scripts/install.sh` installs only the PAM template of the selected
//!   distribution (`--distro`, or `/etc/os-release` `ID` / `ID_LIKE`); the Arch snippet lives
//!   in `/usr/share/soos/pam/` and never in `/etc/pam.d` (where it would be a PAM service).
//! - GitHub #211 (ONB-15): `soos-daemon.service` does not start before the models manifest is
//!   deployed, restarts are bounded, a missing `soos` group is fatal on a live install, and
//!   `scripts/wait_daemon_ready.sh` gives a bounded readiness signal.

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
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use crate::installer_contract::{scratch_dir, stage_fixture_artifacts};

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

fn run_script_env<I, S>(script: &str, args: I, envs: &[(&str, &OsStr)]) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new("bash");
    cmd.arg(workspace_root().join(script)).args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().expect("execute script")
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Every file below `dir`, relative to `dir`, sorted.
fn files_below(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().expect("file type").is_dir() {
                walk(base, &path, out);
            } else {
                out.push(
                    path.strip_prefix(base)
                        .expect("relative")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// GitHub #209 (ONB-13) — one distribution template per install
// ---------------------------------------------------------------------------

const DEBIAN_PROFILE: &str = "usr/share/pam-configs/soos";
const DEBIAN_NOTIFY: &str = "usr/share/pam-configs/soos-notify";
const FEDORA_PROFILE: &str = "etc/authselect/custom/soos/system-auth";
const ARCH_SNIPPET: &str = "usr/share/soos/pam/system-auth.snippet";
const LEGACY_ARCH_SNIPPET: &str = "etc/pam.d/soos.snippet";

/// Which template family a staged tree contains.
#[derive(Debug, PartialEq, Eq)]
struct Templates {
    debian: bool,
    fedora: bool,
    arch: bool,
    pam_d_entries: Vec<String>,
}

fn staged_templates(stage: &Path) -> Templates {
    let files = files_below(stage);
    Templates {
        debian: stage.join(DEBIAN_PROFILE).is_file() && stage.join(DEBIAN_NOTIFY).is_file(),
        fedora: stage.join(FEDORA_PROFILE).is_file(),
        arch: stage.join(ARCH_SNIPPET).is_file(),
        pam_d_entries: files
            .into_iter()
            .filter(|p| p.starts_with("etc/pam.d/"))
            .collect(),
    }
}

fn stage_install(tag: &str, extra: &[&str], os_release: Option<&str>) -> (PathBuf, Output) {
    let stage = scratch_dir(&format!("idt_{tag}_stage"));
    let artifacts = scratch_dir(&format!("idt_{tag}_artifacts"));
    stage_fixture_artifacts(&artifacts);
    if let Some(body) = os_release {
        fs::create_dir_all(stage.join("etc")).expect("create etc");
        fs::write(stage.join("etc/os-release"), body).expect("write os-release");
    }
    let mut args: Vec<&OsStr> = vec![
        OsStr::new("--destdir"),
        stage.as_os_str(),
        OsStr::new("--artifact-dir"),
        artifacts.as_os_str(),
        OsStr::new("--pam-dir"),
        OsStr::new("/usr/lib/security"),
        OsStr::new("--skip-systemd"),
        OsStr::new("--skip-models"),
    ];
    args.extend(extra.iter().map(OsStr::new));
    let out = run_script_env("scripts/install.sh", args, &[]);
    let _ = fs::remove_dir_all(&artifacts);
    (stage, out)
}

/// `--distro <family>` stages exactly that family's template and never a file in `etc/pam.d`.
#[test]
fn test_install_stages_only_the_selected_distro_template() {
    for (distro, debian, fedora, arch) in [
        ("debian", true, false, false),
        ("fedora", false, true, false),
        ("arch", false, false, true),
        ("none", false, false, false),
    ] {
        let (stage, out) = stage_install(&format!("sel_{distro}"), &["--distro", distro], None);
        assert!(
            out.status.success(),
            "--distro {distro} must succeed; output:\n{}",
            combined(&out)
        );
        let found = staged_templates(&stage);
        assert_eq!(
            found,
            Templates {
                debian,
                fedora,
                arch,
                pam_d_entries: Vec::new()
            },
            "--distro {distro}: wrong template set (stage: {:?})",
            files_below(&stage)
        );
        assert!(
            !stage.join(LEGACY_ARCH_SNIPPET).exists(),
            "--distro {distro}: /etc/pam.d/soos.snippet would be a PAM service named 'soos.snippet'"
        );
        if !fedora {
            assert!(
                !stage.join("etc/authselect").exists(),
                "--distro {distro}: no authselect directory may be created"
            );
        }
        if !debian {
            assert!(
                !stage.join("usr/share/pam-configs").exists(),
                "--distro {distro}: no pam-configs directory may be created"
            );
        }
        if arch {
            assert_eq!(
                fs::read(stage.join(ARCH_SNIPPET)).expect("read snippet"),
                fs::read(workspace_root().join("packaging/pam/arch/system-auth.snippet"))
                    .expect("read source snippet"),
                "the staged Arch snippet is the packaged one"
            );
            assert_eq!(
                fs::metadata(stage.join(ARCH_SNIPPET))
                    .expect("meta")
                    .permissions()
                    .mode()
                    & 0o7777,
                0o644
            );
        }
        let _ = fs::remove_dir_all(&stage);
    }
}

/// Without `--distro`, the family comes from the staged `etc/os-release` (`ID`, then `ID_LIKE`),
/// never from the build host; an unknown or absent `os-release` installs no template, loudly.
#[test]
fn test_install_detects_distro_from_os_release_id_and_id_like() {
    for (label, body, debian, fedora, arch) in [
        (
            "ubuntu",
            "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n",
            true,
            false,
            false,
        ),
        (
            "mint",
            "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n",
            true,
            false,
            false,
        ),
        ("fedora", "ID=fedora\n", false, true, false),
        (
            "rocky",
            "ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n",
            false,
            true,
            false,
        ),
        ("arch", "ID=arch\n", false, false, true),
        ("manjaro", "ID=manjaro\nID_LIKE=arch\n", false, false, true),
        (
            "opensuse",
            "ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n",
            false,
            false,
            false,
        ),
    ] {
        let (stage, out) = stage_install(&format!("det_{label}"), &[], Some(body));
        assert!(
            out.status.success(),
            "{label}: install must succeed; output:\n{}",
            combined(&out)
        );
        assert_eq!(
            staged_templates(&stage),
            Templates {
                debian,
                fedora,
                arch,
                pam_d_entries: Vec::new()
            },
            "{label}: wrong template set (stage: {:?})",
            files_below(&stage)
        );
        if !(debian || fedora || arch) {
            assert!(
                combined(&out).contains("--distro"),
                "{label}: an unsupported distribution must be reported with a --distro hint"
            );
        }
        let _ = fs::remove_dir_all(&stage);
    }

    // No os-release in the stage: nothing is guessed from the build host.
    let (stage, out) = stage_install("det_absent", &[], None);
    assert!(out.status.success(), "output:\n{}", combined(&out));
    assert_eq!(
        staged_templates(&stage),
        Templates {
            debian: false,
            fedora: false,
            arch: false,
            pam_d_entries: Vec::new()
        },
        "staging without os-release or --distro installs no template"
    );
    assert!(
        combined(&out).contains("--distro"),
        "the skipped templates must be reported with a --distro hint"
    );
    let _ = fs::remove_dir_all(&stage);

    // An explicit --distro wins over the staged os-release.
    let (stage, out) = stage_install("det_override", &["--distro", "arch"], Some("ID=ubuntu\n"));
    assert!(out.status.success(), "output:\n{}", combined(&out));
    let found = staged_templates(&stage);
    assert!(found.arch && !found.debian && !found.fedora, "{found:?}");
    let _ = fs::remove_dir_all(&stage);
}

/// An unknown `--distro` value fails the preflight before anything is written.
#[test]
fn test_install_rejects_unknown_distro_value() {
    for value in ["gentoo", "", "debian;rm"] {
        let (stage, out) = stage_install("rej", &["--distro", value], None);
        assert!(
            !out.status.success(),
            "--distro '{value}' must be rejected; output:\n{}",
            combined(&out)
        );
        assert!(
            files_below(&stage).is_empty(),
            "--distro '{value}': nothing may be written, found {:?}",
            files_below(&stage)
        );
        let _ = fs::remove_dir_all(&stage);
    }
}

/// Every packaging path selects its own template, and no package ships a PAM "service" named
/// `soos.snippet`; the package harness asserts the per-distribution content.
#[test]
fn test_packaging_selects_one_distro_template_and_never_ships_snippet_in_pam_d() {
    for (file, needle) in [
        ("scripts/build_deb.sh", "--distro debian"),
        ("packaging/debian/rules", "--distro debian"),
        ("scripts/build_arch.sh", "--distro arch"),
    ] {
        let text = read_repo(file);
        assert!(
            text.contains(needle),
            "{file} must pass '{needle}' to scripts/install.sh"
        );
    }
    for file in [
        "packaging/arch/PKGBUILD",
        "packaging/arch/soos.install",
        "scripts/install.sh",
        "packaging/rpm/soos.spec",
    ] {
        let text = read_repo(file);
        assert!(
            !text.contains("/etc/pam.d/soos.snippet") && !text.contains("pam.d/soos.snippet"),
            "{file} must never install or reference /etc/pam.d/soos.snippet"
        );
    }
    let pkgbuild = read_repo("packaging/arch/PKGBUILD");
    assert!(
        pkgbuild.contains("/usr/share/soos/pam/system-auth.snippet"),
        "PKGBUILD installs the snippet under /usr/share/soos/pam/"
    );
    assert!(
        read_repo("packaging/arch/soos.install")
            .contains("/usr/share/soos/pam/system-auth.snippet"),
        "the Arch post-install message points at the new snippet location"
    );
    let spec = read_repo("packaging/rpm/soos.spec");
    assert!(
        !spec.contains("pam-configs") && !spec.contains("system-auth.snippet"),
        "the RPM ships only the authselect profile"
    );

    let harness = read_repo("tests/docker/test_packages.sh");
    assert!(
        harness.contains("verify_distro_templates()"),
        "test_packages.sh must define verify_distro_templates"
    );
    for family in ["debian", "fedora", "arch"] {
        assert!(
            harness.contains(&format!("verify_distro_templates {family} ")),
            "test_packages.sh must assert the {family} package content"
        );
    }
}

/// `uninstall.sh` removes the relocated snippet (and still removes the legacy one).
#[test]
fn test_uninstall_removes_relocated_and_legacy_arch_snippet() {
    let stage = scratch_dir("idt_uninst_stage");
    let snippet = stage.join(ARCH_SNIPPET);
    fs::create_dir_all(snippet.parent().expect("parent")).expect("mkdir");
    fs::write(&snippet, "auth optional pam_soos.so\n").expect("write snippet");
    fs::create_dir_all(stage.join("etc/pam.d")).expect("mkdir pam.d");
    fs::write(
        stage.join(LEGACY_ARCH_SNIPPET),
        "auth optional pam_soos.so\n",
    )
    .expect("legacy");

    let out = run_script_env(
        "scripts/uninstall.sh",
        [
            OsStr::new("--destdir"),
            stage.as_os_str(),
            OsStr::new("--keep-data"),
            OsStr::new("--skip-systemd"),
        ],
        &[],
    );
    assert!(
        out.status.success(),
        "uninstall output:\n{}",
        combined(&out)
    );
    assert!(!snippet.exists(), "relocated snippet removed");
    assert!(
        !stage.join("usr/share/soos").exists(),
        "empty /usr/share/soos tree removed"
    );
    assert!(
        !stage.join(LEGACY_ARCH_SNIPPET).exists(),
        "legacy snippet removed on upgrade paths"
    );
    let _ = fs::remove_dir_all(&stage);
}

// ---------------------------------------------------------------------------
// GitHub #208 (ONB-12) — manifest-only URLs, hardened download, bounded size
// ---------------------------------------------------------------------------

/// The dry run reports exactly the manifest `source_url`, even for a model id that the removed
/// legacy table used to rewrite, and the script carries no legacy model id or URL.
#[test]
fn test_download_models_dry_run_reports_only_manifest_urls() {
    let work = scratch_dir("idt_dm_urls");
    let manifest = work.join("manifest.toml");
    let entries = [
        ("scrfd_500m_kps", "https://example.invalid/mirror/scrfd"),
        (
            "arcface_w600k_mbf",
            "https://example.invalid/mirror/arcface",
        ),
        ("minifasnet_v2_pad", "https://example.invalid/mirror/pad"),
        ("ultraface_slim_320", "https://example.invalid/mirror/ultra"),
    ];
    let mut body = String::from("[manifest]\nversion = \"2.0.0\"\n");
    for (id, url) in entries {
        body.push_str(&format!(
            "\n[models.{id}]\nid = \"{id}\"\nfilename = \"{id}.onnx\"\nsha256 = \"{}\"\n\
             license = \"MIT\"\nsource_url = \"{url}\"\n",
            "a".repeat(64)
        ));
    }
    fs::write(&manifest, body).expect("write manifest");
    let out = run_script_env(
        "scripts/download_models.sh",
        [
            OsStr::new("--dry-run"),
            OsStr::new("--manifest"),
            manifest.as_os_str(),
        ],
        &[],
    );
    assert!(out.status.success(), "dry-run output:\n{}", combined(&out));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let sources: Vec<String> = stdout
        .lines()
        .filter_map(|l| l.split("Source:").nth(1))
        .map(|s| s.trim().to_string())
        .collect();
    let expected: Vec<String> = entries.iter().map(|(_, u)| (*u).to_string()).collect();
    assert_eq!(
        sources, expected,
        "dry-run must report only the manifest URLs; stdout:\n{stdout}"
    );

    let script = read_repo("scripts/download_models.sh");
    for legacy in [
        "ultraface_slim_320",
        "landmark_5point",
        "mobilefacenet_arcface",
        "minifasnet_pad)",
        "huggingface.co",
        "raw.githubusercontent.com",
        "github.com/QingHeYang",
    ] {
        assert!(
            !script.contains(legacy),
            "download_models.sh must not carry the legacy URL table ('{legacy}')"
        );
    }
    let _ = fs::remove_dir_all(&work);
}

fn write_single_model_manifest(work: &Path, payload: &[u8]) -> PathBuf {
    let src = work.join("src.onnx");
    fs::write(&src, payload).expect("write model");
    let digest = Command::new("sha256sum")
        .arg(&src)
        .output()
        .expect("sha256");
    let digest = String::from_utf8_lossy(&digest.stdout)
        .split_whitespace()
        .next()
        .expect("digest")
        .to_string();
    let manifest = work.join("manifest.toml");
    fs::write(
        &manifest,
        format!(
            "[manifest]\nversion = \"2.0.0\"\n\n[models.m1]\nid = \"m1\"\nfilename = \"m1.onnx\"\n\
             sha256 = \"{digest}\"\nlicense = \"MIT\"\nsource_url = \"file://{}\"\n",
            src.display()
        ),
    )
    .expect("write manifest");
    manifest
}

/// A model larger than the size cap is discarded before hashing and nothing is deployed; the cap
/// can only be tightened (`SOOS_MODEL_MAX_BYTES`), and a bad value fails before any write.
#[test]
fn test_download_models_enforces_size_cap_before_hashing() {
    let work = scratch_dir("idt_dm_size");
    let manifest = write_single_model_manifest(&work, &[7_u8; 64]);
    let target = work.join("models");
    let args = [
        OsStr::new("--manifest"),
        manifest.as_os_str(),
        OsStr::new("--target-dir"),
        target.as_os_str(),
    ];

    let out = run_script_env(
        "scripts/download_models.sh",
        args,
        &[("SOOS_MODEL_MAX_BYTES", OsStr::new("16"))],
    );
    assert!(
        !out.status.success(),
        "an oversized model must be rejected; output:\n{}",
        combined(&out)
    );
    assert!(
        combined(&out).contains("exceeds"),
        "the size rejection must be explicit; output:\n{}",
        combined(&out)
    );
    assert!(
        files_below(&target).is_empty(),
        "nothing (model, temp file or manifest) may remain: {:?}",
        files_below(&target)
    );

    for bad in ["0", "abc", "-5", "999999999999999999999"] {
        let _ = fs::remove_dir_all(&target);
        let out = run_script_env(
            "scripts/download_models.sh",
            args,
            &[("SOOS_MODEL_MAX_BYTES", OsStr::new(bad))],
        );
        assert!(
            !out.status.success(),
            "SOOS_MODEL_MAX_BYTES={bad} must be rejected; output:\n{}",
            combined(&out)
        );
        assert!(!target.exists(), "SOOS_MODEL_MAX_BYTES={bad}: no write");
    }

    // Within the cap the model deploys, with no leftover temporary file.
    let _ = fs::remove_dir_all(&target);
    let out = run_script_env(
        "scripts/download_models.sh",
        args,
        &[("SOOS_MODEL_MAX_BYTES", OsStr::new("64"))],
    );
    assert!(out.status.success(), "output:\n{}", combined(&out));
    assert_eq!(
        files_below(&target),
        vec!["m1.onnx".to_string(), "manifest.toml".to_string()],
        "only the model and the manifest remain"
    );
    let _ = fs::remove_dir_all(&work);
}

/// The curl invocation is HTTPS-only, TLS >= 1.2, time- and size-bounded, and the temporary
/// file is created with `mktemp` under a restrictive umask (no predictable `.tmp.$$` name).
#[test]
fn test_download_models_hardens_curl_and_temporary_files() {
    let script = read_repo("scripts/download_models.sh");
    let curl_start = script.find("curl -fSL").expect("curl invocation");
    let curl_cmd: String = script[curl_start..]
        .lines()
        .take_while(|l| !l.contains("||"))
        .collect::<Vec<_>>()
        .join(" ");
    for flag in [
        "--proto '=https'",
        "--proto-redir '=https'",
        "--tlsv1.2",
        "--max-time",
        "--connect-timeout",
        "--max-filesize",
    ] {
        assert!(
            curl_cmd.contains(flag),
            "curl must use {flag}; command: {curl_cmd}"
        );
    }
    assert!(
        !script.contains(".tmp.$$"),
        "no predictable temporary file name"
    );
    assert!(
        script.contains("mktemp") && script.contains("umask 077"),
        "temporary model files are created by mktemp under umask 077"
    );
    assert!(
        script.contains("MAX_MODEL_BYTES="),
        "the size cap is a named constant"
    );
}

// ---------------------------------------------------------------------------
// GitHub #211 (ONB-15) — first-start guards and readiness signal
// ---------------------------------------------------------------------------

/// The unit only starts once the models manifest is deployed (at the path `download_models.sh`
/// writes by default) and restarts are bounded.
#[test]
fn test_daemon_unit_requires_deployed_models_and_bounds_restarts() {
    let unit = read_repo("packaging/soos-daemon.service");
    let unit_section = unit.split("[Service]").next().expect("[Unit] section");
    assert!(
        unit_section
            .lines()
            .any(|l| l.trim() == "ConditionPathExists=/var/lib/soos/models/manifest.toml"),
        "[Unit] must carry ConditionPathExists=/var/lib/soos/models/manifest.toml"
    );
    assert!(
        unit_section
            .lines()
            .any(|l| l.trim() == "StartLimitIntervalSec=320")
            && unit_section
                .lines()
                .any(|l| l.trim() == "StartLimitBurst=5"),
        "[Unit] must bound restarts to 5 in 60 s"
    );
    let dm = read_repo("scripts/download_models.sh");
    assert!(
        dm.contains("DEFAULT_TARGET_DIR=\"${SOOS_MODELS_DIR:-/var/lib/soos/models}\"")
            && dm.contains("target_manifest=\"${TARGET_DIR}/manifest.toml\""),
        "the unit condition must match the manifest path deployed by download_models.sh"
    );
}

/// Returns the single active `key=` value of `section` in a systemd unit, as seconds.
///
/// Comments are skipped; a `s` suffix is accepted. Panics when the key is missing, set more
/// than once, or not a plain number of seconds.
fn unit_seconds(unit: &str, section: &str, key: &str) -> u64 {
    let mut current = "";
    let mut values = Vec::new();
    for raw in unit.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current = line;
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if current == section && k.trim() == key {
                values.push(v.trim().to_string());
            }
        }
    }
    assert_eq!(values.len(), 1, "{section} {key}= must be set exactly once");
    values[0]
        .trim_end_matches('s')
        .parse()
        .unwrap_or_else(|_| panic!("{key}= must be a plain number of seconds"))
}

/// GitHub #287: a start that always fails takes `TimeoutStartSec + RestartSec` per attempt, so
/// the start limit is only reachable when `StartLimitBurst` such attempts fit inside
/// `StartLimitIntervalSec`; otherwise the unit retries forever.
#[test]
fn test_daemon_unit_start_limit_interval_covers_burst_of_timed_out_starts() {
    let unit = read_repo("packaging/soos-daemon.service");
    let interval = unit_seconds(&unit, "[Unit]", "StartLimitIntervalSec");
    let burst = unit_seconds(&unit, "[Unit]", "StartLimitBurst");
    let timeout = unit_seconds(&unit, "[Service]", "TimeoutStartSec");
    let restart = unit_seconds(&unit, "[Service]", "RestartSec");
    assert!(burst > 0, "StartLimitBurst must be positive");
    let needed = burst * (timeout + restart);
    assert!(
        interval >= needed,
        "StartLimitIntervalSec={interval} must be >= StartLimitBurst * (TimeoutStartSec + \
         RestartSec) = {burst} * ({timeout} + {restart}) = {needed}, or the start limit is \
         never reached"
    );
}

/// A live install fails closed when the `soos` group cannot be created (`Group=soos` in the
/// unit), and `--start` starts the unit and waits for readiness through the bounded helper.
#[test]
fn test_install_requires_group_and_offers_bounded_readiness() {
    let install = read_repo("scripts/install.sh");
    assert!(
        !install.contains("Please ensure 'soos' group is created"),
        "a missing group must no longer be downgraded to a warning"
    );
    assert!(
        install.contains("Cannot create the 'soos' system group"),
        "the preflight must fail when neither groupadd nor addgroup exists"
    );
    assert!(
        install.contains("--start)") && install.contains("systemctl start soos-daemon.service"),
        "install.sh --start must start the unit"
    );
    let start_pos = install
        .find("systemctl start soos-daemon.service")
        .expect("start");
    let wait_pos = install
        .find("scripts/wait_daemon_ready.sh\" \\")
        .expect("install.sh must invoke scripts/wait_daemon_ready.sh");
    assert!(start_pos < wait_pos, "readiness is awaited after the start");
    let models_pos = install.find("download_models.sh\" \\").expect("models");
    assert!(
        models_pos < start_pos,
        "models are deployed before the start"
    );
}

struct ReadyFixture {
    work: PathBuf,
    manifest: PathBuf,
    socket: PathBuf,
    admin: PathBuf,
    admin_log: PathBuf,
}

fn ready_fixture(tag: &str, admin_rc: i32) -> ReadyFixture {
    let work = scratch_dir(&format!("idt_ready_{tag}"));
    let manifest = work.join("manifest.toml");
    fs::write(&manifest, "[manifest]\nversion = \"2.0.0\"\n").expect("manifest");
    let admin_log = work.join("admin.log");
    let admin = work.join("soos-admin");
    fs::write(
        &admin,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}'\nprintf '{{\"daemon\":\"ok\"}}\\n'\nexit {admin_rc}\n",
            admin_log.display()
        ),
    )
    .expect("fake admin");
    fs::set_permissions(&admin, fs::Permissions::from_mode(0o755)).expect("chmod");
    ReadyFixture {
        socket: work.join("daemon.sock"),
        work,
        manifest,
        admin,
        admin_log,
    }
}

fn run_ready(fx: &ReadyFixture, timeout: &str) -> (Output, Duration) {
    let start = Instant::now();
    let out = run_script_env(
        "scripts/wait_daemon_ready.sh",
        [
            OsStr::new("--socket"),
            fx.socket.as_os_str(),
            OsStr::new("--manifest"),
            fx.manifest.as_os_str(),
            OsStr::new("--admin"),
            fx.admin.as_os_str(),
            OsStr::new("--timeout"),
            OsStr::new(timeout),
        ],
        &[],
    );
    (out, start.elapsed())
}

/// The readiness helper succeeds once the socket exists and `soos-admin status` answers.
#[test]
fn test_wait_daemon_ready_succeeds_when_socket_and_status_answer() {
    let fx = ready_fixture("ok", 0);
    let _listener = UnixListener::bind(&fx.socket).expect("bind socket");
    let (out, _) = run_ready(&fx, "5");
    assert!(out.status.success(), "output:\n{}", combined(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("{\"daemon\":\"ok\"}"),
        "the JSON status is printed; output:\n{}",
        combined(&out)
    );
    let args = fs::read_to_string(&fx.admin_log).expect("admin invoked");
    assert!(
        args.contains("--format json")
            && args.contains("status")
            && args.contains(&format!("--socket-path {}", fx.socket.display())),
        "soos-admin must be queried as '--format json --socket-path <sock> status'; got: {args}"
    );
    let _ = fs::remove_dir_all(&fx.work);
}

/// Missing models fail fast (the unit condition would skip the start), a missing or non-socket
/// path fails after the bounded timeout, and a failing status query is a failure.
#[test]
fn test_wait_daemon_ready_fails_closed_and_bounded() {
    // Models not deployed: immediate, explicit failure.
    let fx = ready_fixture("nomodels", 0);
    fs::remove_file(&fx.manifest).expect("rm manifest");
    let (out, elapsed) = run_ready(&fx, "30");
    assert!(!out.status.success(), "missing manifest must fail");
    assert!(
        combined(&out).contains("download_models.sh"),
        "the failure names the fix; output:\n{}",
        combined(&out)
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "a missing manifest must not wait for the timeout ({elapsed:?})"
    );
    assert!(
        !fx.admin_log.exists(),
        "no status query without deployed models"
    );
    let _ = fs::remove_dir_all(&fx.work);

    // No socket: bounded wait, then failure with diagnostics.
    let fx = ready_fixture("nosock", 0);
    let (out, elapsed) = run_ready(&fx, "1");
    assert!(!out.status.success(), "absent socket must fail");
    assert!(
        combined(&out).contains("journalctl"),
        "the timeout failure points at the journal; output:\n{}",
        combined(&out)
    );
    assert!(
        elapsed < Duration::from_secs(20),
        "the wait is bounded by --timeout ({elapsed:?})"
    );
    // A regular file is not a listening socket.
    fs::write(&fx.socket, b"").expect("regular file");
    let (out, _) = run_ready(&fx, "1");
    assert!(!out.status.success(), "a regular file is not readiness");
    assert!(!fx.admin_log.exists(), "no status query before readiness");
    let _ = fs::remove_dir_all(&fx.work);

    // Socket present but status fails.
    let fx = ready_fixture("badstatus", 1);
    let _listener = UnixListener::bind(&fx.socket).expect("bind socket");
    let (out, _) = run_ready(&fx, "2");
    assert!(
        !out.status.success(),
        "a failing soos-admin status is not readiness; output:\n{}",
        combined(&out)
    );
    let _ = fs::remove_dir_all(&fx.work);

    // Out-of-range or malformed timeouts are usage errors.
    let fx = ready_fixture("badtimeout", 0);
    for bad in ["0", "301", "abc", ""] {
        let (out, _) = run_ready(&fx, bad);
        assert_eq!(
            out.status.code(),
            Some(2),
            "--timeout '{bad}' is a usage error; output:\n{}",
            combined(&out)
        );
    }
    let _ = fs::remove_dir_all(&fx.work);
}
