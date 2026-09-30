//! Verified, pinned Rust bootstrap (GitHub #260, ONB-16; matrix rows IRP1–IRP5).
//!
//! `scripts/install_rustup.sh` downloads `rustup-init` of a pinned rustup release for the host
//! triple from `static.rust-lang.org`, checks it against SHA-256 digests committed in the
//! repository before it is ever made executable, and installs the exact toolchain release of
//! `rust-toolchain.toml` (never a floating channel). The sandbox Dockerfiles use this script
//! instead of `curl https://sh.rustup.rs | sh`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::installer_contract::scratch_dir;

const SCRIPT: &str = "scripts/install_rustup.sh";

/// Dockerfiles that install a Rust toolchain for the sandbox and distro images.
const DOCKERFILES: [&str; 4] = [
    "Dockerfile",
    "tests/docker/Dockerfile.ubuntu",
    "tests/docker/Dockerfile.fedora",
    "tests/docker/Dockerfile.arch",
];

/// Host triples the installer supports, each with a pinned digest.
const TRIPLES: [&str; 2] = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];

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

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Non-comment lines of a shell script or Dockerfile.
fn instructions(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect()
}

/// Value of a `NAME="value"` shell assignment at the start of a line.
fn shell_assignment(text: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=\"");
    text.lines().find_map(|l| {
        l.trim()
            .strip_prefix(&prefix)
            .and_then(|rest| rest.split('"').next())
            .map(str::to_string)
    })
}

fn pinned_channel() -> String {
    let text = read_repo("rust-toolchain.toml");
    text.lines()
        .find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == "channel").then(|| v.trim().trim_matches('"').to_string())
        })
        .expect("rust-toolchain.toml channel")
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Hermetic fixture: a `bin/` directory put first in PATH with stub `curl` and optionally
/// `uname`, a private TMPDIR and log files.
struct Fixture {
    work: PathBuf,
    bin: PathBuf,
    tmp: PathBuf,
    curl_log: PathBuf,
    marker: PathBuf,
}

fn write_exec(path: &Path, body: &str) {
    fs::write(path, body).expect("write stub");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
}

fn fixture(tag: &str) -> Fixture {
    let work = scratch_dir(&format!("rustup_{tag}"));
    let bin = work.join("bin");
    let tmp = work.join("tmp");
    fs::create_dir_all(&bin).expect("bin");
    fs::create_dir_all(&tmp).expect("tmp");
    let curl_log = work.join("curl.log");
    let marker = work.join("executed.marker");
    // The stub "downloads" a tampered rustup-init: executing it would create the marker.
    let curl = format!(
        "#!/bin/sh\n\
         echo \"$@\" >> '{log}'\n\
         out=''\n\
         while [ $# -gt 0 ]; do\n\
           if [ \"$1\" = '-o' ]; then out=\"$2\"; shift; fi\n\
           shift\n\
         done\n\
         [ -n \"$out\" ] || exit 22\n\
         printf '#!/bin/sh\\ntouch %s\\n' '{marker}' > \"$out\"\n",
        log = curl_log.display(),
        marker = marker.display()
    );
    write_exec(&bin.join("curl"), &curl);
    Fixture {
        work,
        bin,
        tmp,
        curl_log,
        marker,
    }
}

fn run_installer(fx: &Fixture, args: &[&str]) -> Output {
    let path = format!(
        "{}:{}",
        fx.bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new("bash")
        .arg(workspace_root().join(SCRIPT))
        .args(args)
        .env("PATH", path)
        .env("TMPDIR", &fx.tmp)
        .env("HOME", &fx.work)
        .env("CARGO_HOME", fx.work.join("cargo"))
        .env("RUSTUP_HOME", fx.work.join("rustup"))
        .output()
        .expect("execute install_rustup.sh")
}

/// IRP1: the installer pins a rustup release, one SHA-256 per supported triple, the official
/// archive URL over HTTPS only, and defaults to the toolchain of rust-toolchain.toml.
#[test]
fn test_rustup_installer_pins_release_checksums_and_toolchain() {
    let text = read_repo(SCRIPT);
    let version = shell_assignment(&text, "RUSTUP_VERSION").expect("RUSTUP_VERSION=\"x.y.z\"");
    let parts: Vec<&str> = version.split('.').collect();
    assert!(
        parts.len() == 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())),
        "RUSTUP_VERSION must be an exact release, got {version}"
    );
    for (triple, var) in TRIPLES
        .iter()
        .zip(["RUSTUP_INIT_SHA256_X86_64", "RUSTUP_INIT_SHA256_AARCH64"])
    {
        let digest = shell_assignment(&text, var).unwrap_or_else(|| panic!("{var} missing"));
        assert!(
            is_sha256_hex(&digest),
            "{var} ({triple}) must be a lowercase SHA-256 digest, got {digest}"
        );
        assert!(text.contains(triple), "the installer must name {triple}");
    }
    assert!(
        text.contains("https://static.rust-lang.org/rustup/archive/"),
        "rustup-init comes from the versioned official archive"
    );
    assert_eq!(
        shell_assignment(&text, "PINNED_TOOLCHAIN").as_deref(),
        Some(pinned_channel().as_str()),
        "PINNED_TOOLCHAIN must equal the channel of rust-toolchain.toml"
    );
    let code = instructions(&text).join("\n");
    assert!(
        code.contains("--proto '=https'") && code.contains("--tlsv1.2"),
        "downloads are restricted to HTTPS / TLS 1.2+"
    );
    assert!(
        code.contains("sha256sum"),
        "the download is verified with sha256sum"
    );
    assert!(
        !code.contains("sh.rustup.rs"),
        "the installer never runs the sh.rustup.rs shell script"
    );
}

/// IRP2: no Dockerfile pipes a download into a shell; each one copies and runs the verified
/// installer, and `.dockerignore` sends only that script as build context.
#[test]
fn test_sandbox_dockerfiles_use_verified_rustup_installer() {
    for path in DOCKERFILES {
        let text = read_repo(path);
        let code = instructions(&text).join("\n");
        assert!(
            !code.contains("sh.rustup.rs"),
            "{path} must not download the rustup shell script"
        );
        for pipe in ["| sh", "|sh", "| bash", "|bash"] {
            assert!(
                !code.contains(pipe),
                "{path} must not pipe a download into a shell ({pipe})"
            );
        }
        assert!(
            code.lines()
                .any(|l| l.trim_start().starts_with("COPY")
                    && l.contains("scripts/install_rustup.sh")),
            "{path} must COPY scripts/install_rustup.sh"
        );
        assert!(
            code.contains("install_rustup.sh --default-toolchain"),
            "{path} must run install_rustup.sh with an explicit --default-toolchain"
        );
    }
    let ignore = read_repo(".dockerignore");
    let rules: Vec<&str> = instructions(&ignore)
        .into_iter()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        rules,
        vec!["*", "!scripts/install_rustup.sh"],
        ".dockerignore must keep the build context empty except the rustup installer"
    );
}

/// IRP3: a download whose digest does not match is refused, never made executable or run,
/// and the temporary directory is removed.
#[test]
fn test_rustup_installer_refuses_checksum_mismatch_without_executing() {
    let fx = fixture("mismatch");
    let out = run_installer(&fx, &["--default-toolchain", &pinned_channel()]);
    assert!(
        !out.status.success(),
        "a tampered rustup-init must fail; output:\n{}",
        combined(&out)
    );
    assert!(
        combined(&out).to_lowercase().contains("checksum"),
        "the failure must name the checksum mismatch; output:\n{}",
        combined(&out)
    );
    assert!(
        !fx.marker.exists(),
        "the unverified rustup-init must never be executed"
    );
    let log = fs::read_to_string(&fx.curl_log).expect("curl invoked");
    assert!(
        log.contains("https://static.rust-lang.org/rustup/archive/")
            && log.contains("/rustup-init"),
        "the pinned archive URL is fetched; curl args: {log}"
    );
    let leftovers: Vec<_> = fs::read_dir(&fx.tmp).expect("tmp").flatten().collect();
    assert!(
        leftovers.is_empty(),
        "the temporary download directory must be removed: {leftovers:?}"
    );
    let _ = fs::remove_dir_all(&fx.work);
}

/// IRP4: floating channels and unsupported architectures are usage errors raised before any
/// download.
#[test]
fn test_rustup_installer_rejects_floating_channel_and_unknown_arch() {
    for channel in ["stable", "nightly", "beta", "1.98", "1.98.1; rm -rf /"] {
        let fx = fixture("floating");
        let out = run_installer(&fx, &["--default-toolchain", channel]);
        assert_eq!(
            out.status.code(),
            Some(2),
            "--default-toolchain {channel:?} must be a usage error; output:\n{}",
            combined(&out)
        );
        assert!(
            !fx.curl_log.exists(),
            "nothing is downloaded for {channel:?}"
        );
        let _ = fs::remove_dir_all(&fx.work);
    }

    let fx = fixture("arch");
    write_exec(&fx.bin.join("uname"), "#!/bin/sh\necho riscv64\n");
    let out = run_installer(&fx, &[]);
    assert!(
        !out.status.success(),
        "an architecture without a pinned digest must fail; output:\n{}",
        combined(&out)
    );
    assert!(
        combined(&out).contains("riscv64"),
        "the failure names the architecture; output:\n{}",
        combined(&out)
    );
    assert!(!fx.curl_log.exists(), "nothing is downloaded for riscv64");
    let _ = fs::remove_dir_all(&fx.work);
}

/// IRP5: the onboarding documentation points to the verified installer, not to an unverified
/// `curl | sh`.
#[test]
fn test_readme_and_docs_point_to_verified_rustup_installer() {
    for path in ["README.md", "Docs/CI_CD_AND_SECURITY.md"] {
        let text = read_repo(path);
        assert!(
            text.contains("scripts/install_rustup.sh"),
            "{path} must document scripts/install_rustup.sh"
        );
        assert!(
            !text.contains("sh.rustup.rs | sh"),
            "{path} must not recommend piping sh.rustup.rs into a shell"
        );
    }
}
