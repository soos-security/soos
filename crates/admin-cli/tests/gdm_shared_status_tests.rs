//! Contract tests for `soos-admin gdm status` on a GDM service that reaches a shared
//! primary `pam_soos.so` rule through its delegated stack (GitHub #331 §3, matrix IGF16).
//!
//! `installed` is true and `shared_stack` names the stack file holding the shared rule,
//! using exactly the analysis of `gdm enable`; any refusal condition fails closed to
//! `installed: false, shared_stack: None`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses direct assertions and unwrap"
)]

use soos_admin_cli::args::GdmAction;
use soos_admin_cli::gdm::{configure_gdm, get_gdm_status, GdmStatus, GDM_BLOCK_BEGIN};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const ARCH_GDM_PASSWORD: &str = "\
#%PAM-1.0

auth       include                     system-local-login
auth       optional                    pam_gnome_keyring.so
account    include                     system-local-login
session    include                     system-local-login
";

const ARCH_SYSTEM_LOCAL_LOGIN: &str = "\
#%PAM-1.0

auth      include   system-login
account   include   system-login
";

const ARCH_SYSTEM_LOGIN: &str = "\
#%PAM-1.0

auth       required   pam_shells.so
auth       requisite  pam_nologin.so
auth       include    system-auth
account    include    system-auth
";

const ARCH_STOCK_SYSTEM_AUTH: &str = "\
#%PAM-1.0

auth       required                    pam_faillock.so      preauth
-auth      [success=2 default=ignore]  pam_systemd_home.so
auth       [success=1 default=bad]     pam_unix.so          try_first_pass nullok
auth       [default=die]               pam_faillock.so      authfail
auth       optional                    pam_permit.so
";

const ARCH_PACKAGED_SYSTEM_AUTH: &str = include_str!("../../../packaging/pam/arch/system-auth");

const UBUNTU_GDM_PASSWORD: &str = "\
#%PAM-1.0
auth    requisite       pam_nologin.so
auth\trequired\tpam_succeed_if.so user != root quiet_success
@include common-auth
auth    optional        pam_gnome_keyring.so
@include common-account
";

const DEBIAN_COMMON_AUTH_SOOS: &str = "\
auth\t[success=done default=ignore]\tpam_soos.so
auth\t[success=1 default=ignore]\tpam_unix.so nullok
auth\trequisite\t\t\tpam_deny.so
auth\trequired\t\t\tpam_permit.so
";

const FEDORA_GDM_PASSWORD: &str = "\
auth     [success=done ignore=ignore default=bad] pam_selinux_permit.so
auth        substack      password-auth
auth        optional      pam_gnome_keyring.so
auth        include       postlogin
account     include       password-auth
";

const FEDORA_SOOS_PASSWORD_AUTH: &str = "\
auth        required                                     pam_env.so
auth        required                                     pam_faildelay.so delay=2000000
auth        required                                     pam_faillock.so preauth silent
auth        [success=done default=ignore]                pam_soos.so
auth        sufficient                                   pam_unix.so nullok
auth        required                                     pam_faillock.so authfail
auth        optional                                     pam_soos.so event=password-failed timeout_ms=20
auth        required                                     pam_deny.so
";

struct Fixture {
    _dir: tempfile::TempDir,
    pam_dir: PathBuf,
    pam_file: PathBuf,
    disable_file: PathBuf,
}

fn fixture(gdm_password: &str, shared: &[(&str, &str)]) -> Fixture {
    let dir = tempdir().unwrap();
    let pam_dir = dir.path().join("pam.d");
    fs::create_dir(&pam_dir).unwrap();
    for (name, content) in shared {
        fs::write(pam_dir.join(name), content).unwrap();
    }
    let pam_file = pam_dir.join("gdm-password");
    fs::write(&pam_file, gdm_password).unwrap();
    let disable_file = dir.path().join("soos").join("gdm.disable");
    Fixture {
        _dir: dir,
        pam_dir,
        pam_file,
        disable_file,
    }
}

fn arch_fixture(system_auth: &str) -> Fixture {
    fixture(
        ARCH_GDM_PASSWORD,
        &[
            ("system-local-login", ARCH_SYSTEM_LOCAL_LOGIN),
            ("system-login", ARCH_SYSTEM_LOGIN),
            ("system-auth", system_auth),
        ],
    )
}

/// The host-wide kill switch also disables GDM; tests cannot control it.
fn globally_disabled() -> bool {
    Path::new("/etc/soos/disabled").exists()
}

fn status(f: &Fixture) -> GdmStatus {
    get_gdm_status(&f.pam_file, &f.disable_file)
}

#[test]
fn test_igf16_arch_shared_rule_reports_installed_with_stack_name() {
    let f = arch_fixture(ARCH_PACKAGED_SYSTEM_AUTH);
    let s = status(&f);
    assert!(
        s.installed,
        "the shared system-auth rule makes GDM installed"
    );
    assert_eq!(s.shared_stack.as_deref(), Some("system-auth"));
    assert_eq!(s.enabled, !globally_disabled());

    fs::create_dir_all(f.disable_file.parent().unwrap()).unwrap();
    fs::write(&f.disable_file, "disabled\n").unwrap();
    let s = status(&f);
    assert!(s.installed);
    assert_eq!(s.shared_stack.as_deref(), Some("system-auth"));
    assert!(!s.enabled, "enabled follows gdm.disable");
}

#[test]
fn test_igf16_debian_and_fedora_shared_rules_report_their_stack() {
    let debian = fixture(
        UBUNTU_GDM_PASSWORD,
        &[("common-auth", DEBIAN_COMMON_AUTH_SOOS)],
    );
    let s = status(&debian);
    assert!(s.installed);
    assert_eq!(s.shared_stack.as_deref(), Some("common-auth"));

    let fedora = fixture(
        FEDORA_GDM_PASSWORD,
        &[
            ("password-auth", FEDORA_SOOS_PASSWORD_AUTH),
            ("postlogin", "session optional pam_umask.so\n"),
        ],
    );
    let s = status(&fedora);
    assert!(s.installed);
    assert_eq!(s.shared_stack.as_deref(), Some("password-auth"));
}

#[test]
fn test_igf16_direct_rule_keeps_shared_stack_none() {
    // Managed block on a stock stack.
    let f = arch_fixture(ARCH_STOCK_SYSTEM_AUTH);
    configure_gdm(&GdmAction::Enable, &f.pam_file, &f.disable_file).unwrap();
    assert!(fs::read_to_string(&f.pam_file)
        .unwrap()
        .contains(GDM_BLOCK_BEGIN));
    let s = status(&f);
    assert!(s.installed);
    assert_eq!(s.shared_stack, None);

    // Administrator rule in the GDM file, even with a shared rule downstream.
    let gdm = "#%PAM-1.0\nauth sufficient pam_soos.so\nauth include system-local-login\n";
    let f = fixture(
        gdm,
        &[
            ("system-local-login", ARCH_SYSTEM_LOCAL_LOGIN),
            ("system-login", ARCH_SYSTEM_LOGIN),
            ("system-auth", ARCH_PACKAGED_SYSTEM_AUTH),
        ],
    );
    let s = status(&f);
    assert!(s.installed);
    assert_eq!(s.shared_stack, None, "a direct rule is not a shared rule");
}

#[test]
fn test_igf16_stock_stack_without_soos_reports_not_installed() {
    let f = arch_fixture(ARCH_STOCK_SYSTEM_AUTH);
    let s = status(&f);
    assert!(!s.installed);
    assert!(!s.enabled);
    assert_eq!(s.shared_stack, None);
}

#[test]
fn test_igf16_unreadable_or_refused_stacks_fail_closed() {
    // Missing include target.
    let f = fixture(
        ARCH_GDM_PASSWORD,
        &[
            ("system-local-login", ARCH_SYSTEM_LOCAL_LOGIN),
            ("system-login", ARCH_SYSTEM_LOGIN),
        ],
    );
    let s = status(&f);
    assert!(!s.installed, "missing include must report not installed");
    assert_eq!(s.shared_stack, None);

    // Include target is a directory (unreadable as a PAM file).
    fs::create_dir(f.pam_dir.join("system-auth")).unwrap();
    let s = status(&f);
    assert!(!s.installed);
    assert_eq!(s.shared_stack, None);

    // Unclassified rule before the shared rule: enable refuses, status says not installed.
    let shared = "auth required pam_mystery.so\nauth [success=done default=ignore] pam_soos.so\nauth sufficient pam_unix.so\n";
    let f = fixture(
        "#%PAM-1.0\nauth include shared-auth\n",
        &[("shared-auth", shared)],
    );
    let s = status(&f);
    assert!(!s.installed);
    assert_eq!(s.shared_stack, None);

    // Non-primary soos rule only.
    let shared = "auth optional pam_soos.so event=password-failed\nauth sufficient pam_unix.so\n";
    let f = fixture(
        "#%PAM-1.0\nauth include shared-auth\n",
        &[("shared-auth", shared)],
    );
    let s = status(&f);
    assert!(!s.installed);
    assert_eq!(s.shared_stack, None);

    // Continuation lines in the GDM file.
    let f = fixture(
        "#%PAM-1.0\nauth required \\\n  pam_nologin.so\nauth include shared-auth\n",
        &[(
            "shared-auth",
            "auth [success=done default=ignore] pam_soos.so\nauth sufficient pam_unix.so\n",
        )],
    );
    let s = status(&f);
    assert!(!s.installed);
    assert_eq!(s.shared_stack, None);

    // Missing GDM file.
    let missing = f.pam_dir.join("absent");
    let s = get_gdm_status(&missing, &f.disable_file);
    assert!(!s.installed);
    assert_eq!(s.shared_stack, None);
}

#[test]
fn test_igf16_enable_returns_status_with_shared_stack() {
    let f = arch_fixture(ARCH_PACKAGED_SYSTEM_AUTH);
    let s = configure_gdm(&GdmAction::Enable, &f.pam_file, &f.disable_file)
        .expect("enable accepts the shared stack");
    assert!(s.installed);
    assert_eq!(s.shared_stack.as_deref(), Some("system-auth"));
}

#[test]
fn test_igf16_status_json_and_table_show_the_shared_stack() {
    let f = arch_fixture(ARCH_PACKAGED_SYSTEM_AUTH);
    let json = serde_json::to_value(status(&f)).unwrap();
    assert_eq!(json["shared_stack"], "system-auth");
    assert_eq!(json["installed"], true);

    let run = |extra: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_soos-admin"))
            .args(extra)
            .args(["gdm", "status", "--pam-file"])
            .arg(&f.pam_file)
            .arg("--disable-file")
            .arg(&f.disable_file)
            .output()
            .unwrap()
    };
    let table = run(&[]);
    assert!(table.status.success());
    let stdout = String::from_utf8_lossy(&table.stdout);
    assert!(
        stdout.contains("  Shared soos Rule:  system-auth"),
        "table output: {stdout}"
    );
    assert!(
        stdout.contains("  Installed in PAM:  Yes"),
        "table output: {stdout}"
    );

    let json_out = run(&["--format", "json"]);
    assert!(json_out.status.success());
    let parsed: serde_json::Value = serde_json::from_slice(&json_out.stdout).unwrap();
    assert_eq!(parsed["shared_stack"], "system-auth");

    // No shared rule: the line is absent and JSON carries null.
    let stock = arch_fixture(ARCH_STOCK_SYSTEM_AUTH);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_soos-admin"))
        .args(["--format", "json", "gdm", "status", "--pam-file"])
        .arg(&stock.pam_file)
        .arg("--disable-file")
        .arg(&stock.disable_file)
        .output()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(parsed["shared_stack"].is_null());
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_soos-admin"))
        .args(["gdm", "status", "--pam-file"])
        .arg(&stock.pam_file)
        .arg("--disable-file")
        .arg(&stock.disable_file)
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains("Shared soos Rule"));
}
