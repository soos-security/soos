//! Upgrade path of local installs (GitHub #327, matrix rows UPG1–UPG9).
//!
//! - `scripts/install.sh` re-run over a live install restarts an active `soos-daemon.service`
//!   onto the new binaries and its rollback never disables a unit that was enabled before.
//! - The Debian `postinst` never re-enables a PAM profile the administrator disabled with
//!   `pam-auth-update`, and still enables both profiles on a first install.
//! - The Docker harnesses keep the reinstall / upgrade cases, and the README and
//!   `Docs/PACKAGING_AND_PROVISIONING.md` document the procedure per install method.

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
    let dir = std::env::temp_dir().join(format!("soos_upg_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn position(haystack: &str, needle: &str, what: &str) -> usize {
    haystack
        .find(needle)
        .unwrap_or_else(|| panic!("{what} must contain `{needle}`"))
}

/// Body of a shell function `name() {` up to its closing `}` at column 0.
fn shell_function<'a>(text: &'a str, name: &str) -> &'a str {
    let header = format!("{name}() {{");
    let start = position(text, &header, "the script");
    let rest = &text[start..];
    let end = rest.find("\n}\n").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

/// Section of a Markdown document from `heading` up to the next heading of the same level.
fn markdown_section<'a>(doc: &'a str, heading: &str) -> &'a str {
    let start = position(doc, heading, "the document");
    let level = heading.chars().take_while(|c| *c == '#').count();
    let marker = format!("\n{} ", "#".repeat(level));
    let rest = &doc[start..];
    let end = rest[heading.len()..]
        .find(&marker)
        .map_or(rest.len(), |i| i + heading.len());
    &rest[..end]
}

/// Block of the Debian `postinst` that integrates the PAM stack (step 4).
fn postinst_pam_block() -> String {
    let postinst = read("packaging/debian/postinst");
    let start = position(&postinst, "# 4. Integrate PAM", "postinst");
    let end = postinst[start..]
        .find("# 5. ")
        .map(|i| start + i)
        .expect("postinst step 5 marker");
    postinst[start..end].to_string()
}

/// Runs the postinst PAM block under `sh -e` with a recording `pam-auth-update` stub, a
/// scratch `common-auth` and a scratch pam-auth-update state directory holding `state`
/// (`(file name, content)` pairs, e.g. `seen` and `auth`). Returns the output and the
/// recorded `pam-auth-update` argument lines.
fn run_postinst_pam_block(tag: &str, state: &[(&str, &str)]) -> (Output, String) {
    let root = scratch_dir(tag);
    let stubs = root.join("bin");
    fs::create_dir_all(&stubs).expect("stub dir");
    let state_dir = root.join("var-lib-pam");
    fs::create_dir_all(&state_dir).expect("state dir");
    for (name, content) in state {
        fs::write(state_dir.join(name), content).expect("state file");
    }
    let common_auth = root.join("common-auth");
    fs::write(
        &common_auth,
        "auth [success=2 default=ignore] pam_soos.so\nauth [success=1 default=ignore] pam_unix.so nullok\n",
    )
    .expect("common-auth");
    let log = root.join("pam-auth-update.log");
    let stub = stubs.join("pam-auth-update");
    fs::write(
        &stub,
        format!("#!/bin/sh\necho \"$*\" >> '{}'\nexit 0\n", log.display()),
    )
    .expect("stub");
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).expect("chmod stub");

    let block = postinst_pam_block()
        .replace("/etc/pam.d/common-auth", &common_auth.display().to_string())
        .replace("/var/lib/pam", &state_dir.display().to_string());
    let out = Command::new("sh")
        .arg("-e")
        .arg("-c")
        .arg(format!("{block}\necho postinst-continued"))
        .env(
            "PATH",
            format!(
                "{}:{}",
                stubs.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .expect("run postinst PAM block");
    let calls = fs::read_to_string(&log).unwrap_or_default();
    (out, calls)
}

/// UPG5 — an administrator who disabled the soos profiles with `pam-auth-update --disable`
/// keeps them disabled across a `.deb` upgrade: the postinst only refreshes the stack.
#[test]
fn test_upg_postinst_keeps_admin_disabled_profiles() {
    // pam-auth-update knows both profiles (`seen`) but neither is enabled (no `Module:` line).
    let disabled = [
        ("seen", "unix\nsoos\nsoos-notify\n"),
        (
            "auth",
            "Module: unix\n[success=end default=ignore]\tpam_unix.so nullok\n",
        ),
    ];
    let (out, calls) = run_postinst_pam_block("disabled", &disabled);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "postinst aborted: {err}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("postinst-continued"));
    assert!(
        !calls.is_empty(),
        "the postinst must still refresh the PAM stack (pam-auth-update --package)"
    );
    assert!(
        calls.lines().all(|l| !l.contains("--enable")),
        "an upgrade must not re-enable profiles the administrator disabled: {calls}"
    );
    assert!(
        calls.lines().all(|l| !l.contains("--force")),
        "the postinst must never force the PAM stack: {calls}"
    );

    // Only soos-notify disabled: still no --enable (it would re-enable soos-notify).
    let notify_disabled = [
        ("seen", "unix\nsoos\nsoos-notify\n"),
        (
            "auth",
            "Module: soos\n[success=done default=ignore]\tpam_soos.so\nModule: unix\n[success=end default=ignore]\tpam_unix.so nullok\n",
        ),
    ];
    let (out, calls) = run_postinst_pam_block("notify_disabled", &notify_disabled);
    assert!(out.status.success());
    assert!(
        !calls.is_empty() && calls.lines().all(|l| !l.contains("--enable")),
        "a disabled soos-notify profile must stay disabled: {calls}"
    );
}

/// UPG5 — first install (profiles unknown to pam-auth-update), reinstall after `dpkg -r`
/// (`--remove` drops them from `seen`) and an upgrade with both profiles enabled all run
/// `pam-auth-update --package --enable soos soos-notify`.
#[test]
fn test_upg_postinst_enables_profiles_on_first_install() {
    let cases: [(&str, &[(&str, &str)]); 3] = [
        ("first_install", &[]),
        ("after_remove", &[("seen", "unix\n"), ("auth", "Module: unix\n")]),
        (
            "both_enabled",
            &[
                ("seen", "unix\nsoos\nsoos-notify\n"),
                (
                    "auth",
                    "Module: soos\n[success=done default=ignore]\tpam_soos.so\nModule: unix\n[success=end default=ignore]\tpam_unix.so nullok\nModule: soos-notify\n[default=ignore]\tpam_soos.so event=password-failed timeout_ms=20\n",
                ),
            ],
        ),
    ];
    for (tag, state) in cases {
        let (out, calls) = run_postinst_pam_block(tag, state);
        assert!(
            out.status.success(),
            "{tag}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            calls.trim(),
            "--package --enable soos soos-notify",
            "{tag}: the postinst must enable both profiles"
        );
    }
}

/// UPG3 — a re-run of `install.sh` restarts an active unit onto the new binaries (a plain
/// `systemctl start` is a no-op on an active unit), then waits for readiness.
#[test]
fn test_upg_install_restarts_an_active_unit() {
    let install = read("scripts/install.sh");
    let what = "scripts/install.sh";
    let was_active = position(
        &install,
        "systemctl is-active --quiet soos-daemon.service",
        what,
    );
    let was_enabled = position(
        &install,
        "systemctl is-enabled --quiet soos-daemon.service",
        what,
    );
    let enable = position(&install, "systemctl enable soos-daemon.service", what);
    assert!(
        was_active < enable && was_enabled < enable,
        "the unit state must be recorded before step 8 enables it"
    );
    let restart = position(&install, "systemctl restart soos-daemon.service", what);
    let wait = position(&install, "scripts/wait_daemon_ready.sh\" \\", what);
    assert!(
        enable < restart && restart < wait,
        "an active unit is restarted after it is enabled and readiness is awaited afterwards"
    );
    let step9 = &install[enable..wait];
    assert!(
        step9.contains("UNIT_WAS_ACTIVE"),
        "step 9 must run for an active unit even without --start"
    );
}

/// UPG3 — the rollback of a failed re-run never disables a unit that was enabled before.
#[test]
fn test_upg_install_rollback_keeps_a_previously_enabled_unit() {
    let install = read("scripts/install.sh");
    let body = shell_function(&install, "rollback");
    let guard = position(body, "UNIT_WAS_ENABLED", "rollback()");
    let disable = position(body, "systemctl disable soos-daemon.service", "rollback()");
    assert!(
        guard < disable,
        "rollback() must check UNIT_WAS_ENABLED before disabling the unit"
    );
}

/// UPG9 — the Docker harnesses keep the reinstall / upgrade cases.
#[test]
fn test_upg_docker_harnesses_cover_upgrade() {
    let sua = read("tests/docker/systemd_unit_acceptance_test.sh");
    let part6 = shell_function(&sua, "part6_reinstall_preserves_state");
    for needle in [
        "UPGRADE_TEMPLATE",
        "upgrade_state_digest",
        "assert_upgrade_state_unchanged",
        "assert_daemon_upgraded",
        "/etc/soos/daemon.toml",
        "pam-auth-update --package --enable soos soos-notify",
        "scripts/install.sh",
        "--start",
        "MainPID",
        "SKIPPED",
    ] {
        assert!(
            part6.contains(needle),
            "part6_reinstall_preserves_state() must cover `{needle}`"
        );
    }
    let digest = shell_function(&sua, "upgrade_state_digest");
    for needle in [
        "/var/lib/soos/master.key",
        "/etc/soos/daemon.toml",
        "/etc/pam.d",
        "/var/lib/pam",
        "/var/lib/soos/state",
        "is-enabled",
    ] {
        assert!(
            digest.contains(needle),
            "upgrade_state_digest() must cover `{needle}`"
        );
    }
    let upgraded = shell_function(&sua, "assert_daemon_upgraded");
    for needle in [
        "MainPID",
        "/proc/${pid}/exe",
        "(deleted)",
        "scripts/wait_daemon_ready.sh",
        "\"is_healthy\": true",
    ] {
        assert!(
            upgraded.contains(needle),
            "assert_daemon_upgraded() must cover `{needle}`"
        );
    }
    assert!(
        sua.contains("UPGRADE_TEMPLATE=\"/var/lib/soos/biometrics/4242.cbor.enc\""),
        "the systemd harness must use an enrolled-template fixture"
    );
    assert!(
        sua.contains("stage upgrade") && sua.contains("    upgrade)"),
        "the systemd harness must run the upgrade stage"
    );

    let pkgs = read("tests/docker/test_packages.sh");
    let digest = shell_function(&pkgs, "upgrade_state_digest");
    for needle in [
        "/var/lib/soos/master.key",
        "UPGRADE_TEMPLATE",
        "/etc/soos/daemon.toml",
        "/etc/pam.d",
        "/var/lib/pam",
    ] {
        assert!(
            digest.contains(needle),
            "test_packages.sh upgrade_state_digest() must compare `{needle}`"
        );
    }
    assert!(
        pkgs.contains("UPGRADE_TEMPLATE=\"/var/lib/soos/biometrics/4242.cbor.enc\""),
        "the package harness must use an enrolled-template fixture"
    );
    let newer = shell_function(&pkgs, "make_newer_deb");
    assert!(
        newer.contains("dpkg-deb -R")
            && newer.contains("dpkg-deb -b")
            && newer.contains("Version:"),
        "make_newer_deb() must repackage the .deb with a higher version"
    );
    let helper = shell_function(&pkgs, "verify_upgrade_preserves_state");
    assert!(
        helper.contains("upgrade_state_digest") && helper.contains("return 1"),
        "verify_upgrade_preserves_state() must compare digests and fail on a change"
    );
    for (branch, needles) in [
        (
            "Running Debian (.deb) package verification",
            &[
                "verify_upgrade_preserves_state",
                "make_newer_deb",
                "pam-auth-update --package --disable soos soos-notify",
            ][..],
        ),
        (
            "Running Fedora / RHEL (.rpm) package verification",
            &["verify_upgrade_preserves_state", "rpm -U --replacepkgs"][..],
        ),
        (
            "Running Arch Linux package verification",
            &["verify_upgrade_preserves_state", "pacman -U"][..],
        ),
    ] {
        let body = pkgs
            .split(branch)
            .nth(1)
            .and_then(|rest| rest.split(";;").next())
            .unwrap_or_else(|| panic!("test_packages.sh branch {branch}"));
        for needle in needles {
            assert!(
                body.contains(needle),
                "test_packages.sh branch {branch} must contain `{needle}`"
            );
        }
    }
}

/// UPG7 — the README documents the upgrade, command-only, per install method.
#[test]
fn test_upg_readme_documents_upgrade_per_method() {
    let readme = read("README.md");
    let section = markdown_section(&readme, "## Upgrade");
    for needle in [
        "git pull",
        "scripts/build_deb.sh",
        "dpkg -i target/packages/soos_",
        "scripts/build_arch.sh",
        "pacman -U",
        "scripts/build_rpm.sh",
        "dnf upgrade ./",
        "scripts/install.sh --build",
        "systemctl restart soos-daemon",
        "soos-admin status",
        "soos-enroll list",
        "soos-enroll enroll --username",
        "Docs/PACKAGING_AND_PROVISIONING.md",
    ] {
        assert!(
            section.contains(needle),
            "README `## Upgrade` must cover `{needle}`"
        );
    }
}

/// UPG8 — the packaging guide states what an upgrade preserves and how to switch from an
/// `install.sh` install to a native package.
#[test]
fn test_upg_packaging_doc_documents_preservation_and_switch() {
    let doc = read("Docs/PACKAGING_AND_PROVISIONING.md");
    let section = markdown_section(&doc, "## 9. Upgrading an Existing Installation");
    for needle in [
        "/var/lib/soos/master.key",
        "/var/lib/soos/biometrics",
        "/etc/soos/daemon.toml",
        "pam-auth-update --disable soos soos-notify",
        "authselect apply-changes",
        "%systemd_postun_with_restart",
        "systemctl restart soos-daemon",
        "Not Preserved",
        "scripts/uninstall.sh",
        "/etc/systemd/system/soos-daemon.service",
        "soos-enroll list",
        "tests/docker/systemd_unit_acceptance_test.sh",
        "tests/docker/test_packages.sh",
    ] {
        assert!(
            section.contains(needle),
            "Docs/PACKAGING_AND_PROVISIONING.md §9 must cover `{needle}`"
        );
    }
}
