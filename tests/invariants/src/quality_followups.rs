//! Quality, packaging and documentation follow-ups (GitHub #281, matrix rows QFU1–QFU6).
//!
//! - RPM `%posttrans` master-key upgrade guard: `cmp` comes from `diffutils`, which the spec
//!   must require, and a `cmp` failure (exit status 2 or 127) is reported as "could not
//!   compare" instead of "the keys differ"; the guard is exercised by an `rpm -U` Docker case.
//! - Debian `postinst`: `pam-auth-update --package` without `--force` leaves a locally
//!   modified stack untouched; the skip is surfaced as a warning, never silent.
//! - GDM `timeout_ms=2500` versus the daemon `[dispatcher] connection_timeout_ms` cap.
//! - The PAM module never opens a camera device (global invariant of the matrix).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Fresh, empty scratch directory unique to this test process and tag.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soos_qfu_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Writes an executable `sh` stub named `name` into `dir`.
fn write_stub(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write stub");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
}

fn path_with(stub_dir: &Path) -> String {
    format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Body of one RPM scriptlet section (`%posttrans`, ...), up to the next section header.
fn rpm_section(spec: &str, section: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in spec.lines() {
        if inside {
            let header = line
                .strip_prefix('%')
                .is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_lowercase()));
            if header {
                break;
            }
            out.push_str(line);
            out.push('\n');
        } else if line.trim_end() == section {
            inside = true;
        }
    }
    assert!(inside, "packaging/rpm/soos.spec has no {section} section");
    out
}

/// Runs the spec's `%posttrans` with `%{_sharedstatedir}` redirected to `state` and a stub
/// `cmp` that exits with `cmp_status`.
fn run_posttrans(tag: &str, cmp_status: i32, same_key: bool) -> (Output, PathBuf) {
    let root = scratch_dir(tag);
    let state = root.join("state");
    let stubs = root.join("bin");
    fs::create_dir_all(state.join("soos")).expect("state dir");
    fs::create_dir_all(&stubs).expect("stub dir");
    write_stub(&stubs, "cmp", &format!("exit {cmp_status}"));
    let key = [7_u8; 32];
    fs::write(state.join("soos/master.key"), key).expect("key");
    let copy = if same_key { key } else { [9_u8; 32] };
    fs::write(state.join("soos/.master.key.upgrade"), copy).expect("copy");

    let body = rpm_section(&read("packaging/rpm/soos.spec"), "%posttrans")
        .replace("%{_sharedstatedir}", &state.display().to_string());
    assert!(
        !body.contains("%{"),
        "unexpanded macro in %posttrans:\n{body}"
    );
    let out = Command::new("sh")
        .arg("-c")
        .arg(&body)
        .env("PATH", path_with(&stubs))
        .output()
        .expect("run %posttrans");
    (out, state.join("soos"))
}

/// QFU1 (GitHub #281): `%posttrans` runs `cmp`, so the spec declares the `diffutils`
/// scriptlet dependency.
#[test]
fn test_rpm_spec_requires_diffutils_for_posttrans_cmp() {
    let spec = read("packaging/rpm/soos.spec");
    assert!(
        rpm_section(&spec, "%posttrans").contains("cmp "),
        "%posttrans is expected to compare the keys with cmp"
    );
    assert!(
        spec.lines()
            .any(|l| l.trim_start().starts_with("Requires(posttrans):") && l.contains("diffutils")),
        "packaging/rpm/soos.spec must declare `Requires(posttrans): diffutils`"
    );
}

/// QFU2 (GitHub #281): a `cmp` that cannot compare (exit status 2, or 127 when missing) is
/// reported as "could not compare"; it never claims that the keys differ and keeps both
/// files. Exit 0 discards the equal copy; exit 1 keeps it with the "differs" warning.
#[test]
fn test_rpm_posttrans_reports_uncomparable_keys_without_claiming_they_differ() {
    for status in [2, 127] {
        let (out, dir) = run_posttrans(&format!("cmp{status}"), status, true);
        let err = stderr_of(&out);
        assert!(out.status.success(), "%posttrans failed: {err}");
        assert!(
            err.contains("could not compare"),
            "cmp exit {status} must be reported as 'could not compare': {err}"
        );
        assert!(
            !err.contains("differs"),
            "cmp exit {status} must not claim the keys differ: {err}"
        );
        assert!(dir.join("master.key").is_file(), "current key kept");
        assert!(
            dir.join(".master.key.upgrade").is_file(),
            "pre-upgrade copy kept when the comparison failed"
        );
    }

    let (out, dir) = run_posttrans("cmp1", 1, false);
    let err = stderr_of(&out);
    assert!(out.status.success(), "%posttrans failed: {err}");
    assert!(
        err.contains("differs"),
        "cmp exit 1 means the keys differ: {err}"
    );
    assert!(!err.contains("could not compare"), "{err}");
    assert!(
        dir.join(".master.key.upgrade").is_file(),
        "differing copy kept"
    );

    let (out, dir) = run_posttrans("cmp0", 0, true);
    let err = stderr_of(&out);
    assert!(out.status.success(), "%posttrans failed: {err}");
    assert!(err.is_empty(), "equal keys produce no warning: {err}");
    assert!(
        !dir.join(".master.key.upgrade").exists(),
        "an equal pre-upgrade copy is discarded"
    );
    assert!(dir.join("master.key").is_file(), "current key kept");
}

/// QFU3 (GitHub #281): the Docker package test upgrades with `rpm -U` from a legacy build
/// that owned `master.key` as `%ghost`, and proves the `%pre`/`%posttrans` guard kept the
/// exact key and removed the temporary copy.
#[test]
fn test_docker_package_test_covers_rpm_upgrade_from_ghost_owned_key() {
    let script = read("tests/docker/test_packages.sh");
    for needle in [
        "verify_rpm_upgrade_keeps_ghost_owned_key",
        "%ghost %attr(0600, root, root) /var/lib/soos/master.key",
        "rpm -U",
        ".master.key.upgrade",
        "master.key changed across rpm -U",
    ] {
        assert!(
            script.contains(needle),
            "tests/docker/test_packages.sh must cover '{needle}'"
        );
    }
    let rpm_branch = script
        .split("fedora|rhel|centos)")
        .nth(1)
        .and_then(|rest| rest.split(";;").next())
        .expect("RPM branch of test_packages.sh");
    assert!(
        rpm_branch.contains("verify_rpm_upgrade_keeps_ghost_owned_key"),
        "the RPM branch must run the rpm -U upgrade case"
    );
}

/// Block of the Debian `postinst` that integrates the PAM stack (step 4).
fn postinst_pam_block() -> String {
    let postinst = read("packaging/debian/postinst");
    let start = postinst
        .find("# 4. Integrate PAM")
        .expect("postinst step 4 marker");
    let end = postinst[start..]
        .find("# 5. ")
        .map(|i| start + i)
        .expect("postinst step 5 marker");
    postinst[start..end].to_string()
}

/// Runs the postinst PAM block under `sh -e` with a stub `pam-auth-update` and a
/// scratch `common-auth`.
fn run_postinst_pam_block(tag: &str, stub_body: &str, initial_common_auth: &str) -> Output {
    let root = scratch_dir(tag);
    let stubs = root.join("bin");
    fs::create_dir_all(&stubs).expect("stub dir");
    let common_auth = root.join("common-auth");
    fs::write(&common_auth, initial_common_auth).expect("common-auth");
    write_stub(
        &stubs,
        "pam-auth-update",
        &stub_body.replace("@COMMON_AUTH@", &common_auth.display().to_string()),
    );
    let block =
        postinst_pam_block().replace("/etc/pam.d/common-auth", &common_auth.display().to_string());
    Command::new("sh")
        .arg("-e")
        .arg("-c")
        .arg(format!("{block}\necho postinst-continued"))
        .env("PATH", path_with(&stubs))
        .output()
        .expect("run postinst PAM block")
}

/// QFU4 (GitHub #281): `pam-auth-update --package` (no `--force`) that skips a locally
/// modified stack, or fails, is reported on stderr; the postinst never aborts because of it
/// and never overwrites the administrator's stack.
#[test]
fn test_debian_postinst_surfaces_skipped_pam_integration() {
    let block = postinst_pam_block();
    assert!(
        block.contains("pam-auth-update --package --enable soos soos-notify"),
        "postinst must enable the soos profiles"
    );
    assert!(
        !block
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .any(|l| l.contains("--force")),
        "postinst must never overwrite a locally modified PAM stack"
    );
    assert!(
        !block
            .lines()
            .any(|l| l.contains("pam-auth-update --package") && l.contains("|| true")),
        "a skipped or failed pam-auth-update must not be silenced with `|| true`"
    );

    // Local modifications: pam-auth-update prints its notice, exits 0 and writes nothing.
    let out = run_postinst_pam_block(
        "pau_local",
        "echo 'Local modifications to /etc/pam.d/common-*, not updating.'\nexit 0",
        "auth [success=1 default=ignore] pam_unix.so nullok\n",
    );
    let err = stderr_of(&out);
    assert!(out.status.success(), "postinst aborted: {err}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("postinst-continued"));
    assert!(
        err.contains("WARNING") && err.contains("pam_soos.so"),
        "a skipped PAM integration must be reported: {err}"
    );
    assert!(
        err.contains("pam-auth-update --enable soos soos-notify"),
        "the warning must name the command that enables the profiles: {err}"
    );

    // Failure: reported, postinst continues.
    let out = run_postinst_pam_block("pau_fail", "exit 1", "");
    let err = stderr_of(&out);
    assert!(out.status.success(), "postinst aborted: {err}");
    assert!(
        err.contains("pam-auth-update failed"),
        "a failing pam-auth-update must be reported: {err}"
    );

    // Success: the stack calls pam_soos.so, no warning.
    let out = run_postinst_pam_block(
        "pau_ok",
        "echo 'auth [success=2 default=ignore] pam_soos.so' >> '@COMMON_AUTH@'\nexit 0",
        "auth [success=1 default=ignore] pam_unix.so nullok\n",
    );
    let err = stderr_of(&out);
    assert!(out.status.success(), "postinst aborted: {err}");
    assert!(
        err.is_empty(),
        "an integrated stack produces no warning: {err}"
    );
}

/// QFU5 (GitHub #281): the GDM documentation states that `timeout_ms=2500` gains no daemon
/// time beyond `[dispatcher] connection_timeout_ms` (default 1000 ms), and the ADR records it.
#[test]
fn test_gdm_timeout_documents_daemon_connection_timeout_cap() {
    let config = read("crates/daemon/src/config.rs");
    assert!(
        config.contains("connection_timeout: Duration::from_millis(1000)"),
        "daemon default connection_timeout changed: update the GDM documentation"
    );
    let deploy = read("Docs/DISTRIBUTION_DEPLOYMENT.md");
    let gdm = deploy
        .split("### 2.1 GDM Login Integration")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("Docs/DISTRIBUTION_DEPLOYMENT.md §2.1");
    for needle in ["timeout_ms=2500", "connection_timeout_ms", "1000 ms"] {
        assert!(
            gdm.contains(needle),
            "Docs/DISTRIBUTION_DEPLOYMENT.md §2.1 must mention '{needle}'"
        );
    }
    assert!(
        read("AI/DECISIONS.md")
            .contains("GDM `timeout_ms=2500` Versus Daemon `connection_timeout_ms`"),
        "AI/DECISIONS.md must record the GDM timeout decision"
    );
}

/// QFU6 (GitHub #281, global invariant "No camera device is opened by the PAM module"): the
/// PAM crate depends on no camera crate and its sources never name a video device.
#[test]
fn test_pam_crate_never_opens_camera_devices() {
    let manifest = read("crates/pam/Cargo.toml");
    for forbidden in ["soos-camera-v4l", "v4l", "nokhwa", "opencv"] {
        assert!(
            !manifest.contains(forbidden),
            "crates/pam/Cargo.toml must not depend on '{forbidden}'"
        );
    }
    let src = workspace_root().join("crates/pam/src");
    let mut scanned = 0_usize;
    for entry in fs::read_dir(&src).expect("read crates/pam/src").flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            scanned += 1;
            let text = fs::read_to_string(&path).expect("read PAM source");
            for forbidden in ["/dev/video", "soos_camera_v4l", "v4l::"] {
                assert!(
                    !text.contains(forbidden),
                    "{} must not reference '{forbidden}'",
                    path.display()
                );
            }
        }
    }
    assert!(scanned > 0, "no PAM source scanned");
}
