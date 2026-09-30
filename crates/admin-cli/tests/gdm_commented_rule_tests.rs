//! Contract tests: `soos-admin gdm status|enable` ignore commented-out `pam_soos.so`
//! lines (GitHub #236 / STO-20).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite uses direct assertions and unwrap"
)]

use soos_admin_cli::args::GdmAction;
use soos_admin_cli::gdm::{configure_gdm, get_gdm_status, GDM_PAM_LINE};
use std::fs;
use tempfile::tempdir;

const COMMENTED_GDM: &str = "#%PAM-1.0\n\
     auth requisite pam_nologin.so\n\
     # auth sufficient pam_soos.so timeout_ms=2500\n\
     \t#auth [success=done default=ignore] pam_soos.so\n\
     @include common-auth\n";

fn write_common_auth(dir: &std::path::Path) {
    fs::write(dir.join("common-auth"), "auth required pam_unix.so\n").unwrap();
}

#[test]
fn test_gdm_status_ignores_commented_pam_soos_line() {
    let temp = tempdir().unwrap();
    let pam_file = temp.path().join("gdm-password");
    let disable_file = temp.path().join("gdm.disable");
    fs::write(&pam_file, COMMENTED_GDM).unwrap();

    let status = get_gdm_status(&pam_file, &disable_file);
    assert!(
        !status.installed,
        "a commented-out pam_soos.so line is not an installed rule"
    );
    assert!(!status.enabled);
}

#[test]
fn test_gdm_status_only_comment_line_reports_not_installed() {
    let temp = tempdir().unwrap();
    let pam_file = temp.path().join("gdm-password");
    let disable_file = temp.path().join("gdm.disable");
    fs::write(&pam_file, "# auth sufficient pam_soos.so\n").unwrap();

    assert!(!get_gdm_status(&pam_file, &disable_file).installed);
}

#[test]
fn test_gdm_status_ignores_pam_soos_in_module_arguments() {
    let temp = tempdir().unwrap();
    let pam_file = temp.path().join("gdm-password");
    let disable_file = temp.path().join("gdm.disable");
    fs::write(
        &pam_file,
        "auth optional pam_echo.so pam_soos.so\n@include common-auth\n",
    )
    .unwrap();

    assert!(
        !get_gdm_status(&pam_file, &disable_file).installed,
        "only the module field of an active rule counts"
    );
}

#[test]
fn test_gdm_status_detects_active_rule_with_module_path() {
    let temp = tempdir().unwrap();
    let pam_file = temp.path().join("gdm-password");
    let disable_file = temp.path().join("gdm.disable");
    fs::write(
        &pam_file,
        "auth sufficient /usr/lib/security/pam_soos.so\n@include common-auth\n",
    )
    .unwrap();

    assert!(get_gdm_status(&pam_file, &disable_file).installed);
}

#[test]
fn test_gdm_enable_inserts_real_line_despite_commented_line() {
    let temp = tempdir().unwrap();
    let pam_file = temp.path().join("gdm-password");
    let disable_file = temp.path().join("gdm.disable");
    fs::write(&pam_file, COMMENTED_GDM).unwrap();
    write_common_auth(temp.path());

    let status =
        configure_gdm(&GdmAction::Enable, &pam_file, &disable_file).expect("enable must succeed");
    assert!(status.installed);
    assert!(status.enabled);

    let content = fs::read_to_string(&pam_file).unwrap();
    assert!(
        content.lines().any(|l| l.trim() == GDM_PAM_LINE),
        "an active pam_soos.so rule must be inserted: {content}"
    );
    assert!(
        content.contains("# auth sufficient pam_soos.so timeout_ms=2500"),
        "the administrator's comment is preserved"
    );
}
