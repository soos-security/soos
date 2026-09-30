//! Contractual unit tests for `soos-admin gdm` management command.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite uses direct assertions and unwrap"
)]

use clap::Parser;
use soos_admin_cli::args::{Cli, Commands, GdmAction, GdmArgs};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_gdm_subcommand_parsing() {
    let args_status = Cli::try_parse_from(["soos-admin", "gdm", "status"]).unwrap();
    match args_status.command {
        Commands::Gdm(GdmArgs {
            action: GdmAction::Status,
            ..
        }) => {}
        _ => panic!("Expected Commands::Gdm(Status)"),
    }

    let args_enable = Cli::try_parse_from(["soos-admin", "gdm", "enable"]).unwrap();
    match args_enable.command {
        Commands::Gdm(GdmArgs {
            action: GdmAction::Enable,
            ..
        }) => {}
        _ => panic!("Expected Commands::Gdm(Enable)"),
    }

    let args_disable = Cli::try_parse_from(["soos-admin", "gdm", "disable"]).unwrap();
    match args_disable.command {
        Commands::Gdm(GdmArgs {
            action: GdmAction::Disable,
            ..
        }) => {}
        _ => panic!("Expected Commands::Gdm(Disable)"),
    }
}

#[test]
fn test_gdm_disable_and_enable_lifecycle() {
    let temp = tempdir().unwrap();
    let disable_file = temp.path().join("gdm.disable");
    let pam_file = temp.path().join("gdm-password");

    fs::write(
        &pam_file,
        "#%PAM-1.0\nauth requisite pam_nologin.so\n@include common-auth\n",
    )
    .unwrap();

    // 1. Enable GDM
    soos_admin_cli::gdm::configure_gdm(&GdmAction::Enable, &pam_file, &disable_file)
        .expect("Enabling GDM should succeed");

    let content = fs::read_to_string(&pam_file).unwrap();
    assert!(
        content.contains("pam_soos.so"),
        "pam-password must include pam_soos.so"
    );
    assert!(
        content.contains("timeout_ms=2500"),
        "GDM configuration must configure timeout_ms=2500 for reliable multi-frame capture"
    );
    assert!(
        !disable_file.exists(),
        "disable_file must not exist after enable"
    );

    // Check status
    let status = soos_admin_cli::gdm::get_gdm_status(&pam_file, &disable_file);
    assert!(status.installed, "Should report installed");
    assert!(status.enabled, "Should report enabled");

    // 2. Disable GDM
    soos_admin_cli::gdm::configure_gdm(&GdmAction::Disable, &pam_file, &disable_file)
        .expect("Disabling GDM should succeed");

    assert!(
        disable_file.exists(),
        "disable_file must exist after disable"
    );
    let status_after_disable = soos_admin_cli::gdm::get_gdm_status(&pam_file, &disable_file);
    assert!(
        status_after_disable.installed,
        "Should remain installed in PAM"
    );
    assert!(!status_after_disable.enabled, "Should report disabled");
}

const PRISTINE_GDM: &str = "#%PAM-1.0\nauth requisite pam_nologin.so\n@include common-auth\n";

fn backup_of(pam_file: &std::path::Path) -> std::path::PathBuf {
    let mut name = pam_file.file_name().unwrap().to_os_string();
    name.push(".soos-backup");
    pam_file.with_file_name(name)
}

/// ONB-08 (GitHub #166): enabling GDM integration keeps a byte-exact copy of the
/// original PAM file (same permissions) that `scripts/uninstall.sh` restores.
#[test]
fn test_gdm_enable_creates_byte_exact_backup_with_original_mode() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempdir().unwrap();
    let pam_file = temp.path().join("gdm-password");
    let disable_file = temp.path().join("gdm.disable");
    fs::write(&pam_file, PRISTINE_GDM).unwrap();
    fs::set_permissions(&pam_file, fs::Permissions::from_mode(0o640)).unwrap();

    soos_admin_cli::gdm::configure_gdm(&GdmAction::Enable, &pam_file, &disable_file)
        .expect("enable must succeed");

    let backup = backup_of(&pam_file);
    assert_eq!(
        fs::read_to_string(&backup).expect("backup must exist"),
        PRISTINE_GDM,
        "backup must hold the original bytes"
    );
    let backup_mode = fs::metadata(&backup).unwrap().permissions().mode() & 0o7777;
    let file_mode = fs::metadata(&pam_file).unwrap().permissions().mode() & 0o7777;
    assert_eq!(backup_mode, 0o640, "backup keeps the original mode");
    assert_eq!(
        file_mode, 0o640,
        "rewritten PAM file keeps the original mode"
    );
    let content = fs::read_to_string(&pam_file).unwrap();
    assert!(content.contains("pam_soos.so timeout_ms=2500"));
    assert!(
        content.contains("@include common-auth"),
        "original lines preserved"
    );
}

/// A pre-existing backup is the pristine state: it must never be overwritten.
#[test]
fn test_gdm_enable_never_overwrites_an_existing_backup() {
    let temp = tempdir().unwrap();
    let pam_file = temp.path().join("gdm-password");
    let disable_file = temp.path().join("gdm.disable");
    let backup = backup_of(&pam_file);
    fs::write(&pam_file, PRISTINE_GDM).unwrap();
    fs::write(&backup, "# pristine from an earlier enable\n").unwrap();

    soos_admin_cli::gdm::configure_gdm(&GdmAction::Enable, &pam_file, &disable_file)
        .expect("enable must succeed");

    assert_eq!(
        fs::read_to_string(&backup).unwrap(),
        "# pristine from an earlier enable\n"
    );
}

/// The replacement is atomic (temp file + fsync + rename): no temporary file is
/// left next to the PAM file, and an already configured file is not rewritten.
#[test]
fn test_gdm_enable_is_atomic_and_idempotent() {
    let temp = tempdir().unwrap();
    let pam_file = temp.path().join("gdm-password");
    let disable_file = temp.path().join("gdm.disable");
    fs::write(&pam_file, PRISTINE_GDM).unwrap();

    soos_admin_cli::gdm::configure_gdm(&GdmAction::Enable, &pam_file, &disable_file).unwrap();
    let first = fs::read_to_string(&pam_file).unwrap();
    soos_admin_cli::gdm::configure_gdm(&GdmAction::Enable, &pam_file, &disable_file).unwrap();
    assert_eq!(
        fs::read_to_string(&pam_file).unwrap(),
        first,
        "second enable is a no-op"
    );
    assert_eq!(
        first.matches("pam_soos.so").count(),
        1,
        "the soos line is inserted exactly once"
    );

    let mut names: Vec<String> = fs::read_dir(temp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "gdm-password".to_string(),
            "gdm-password.soos-backup".to_string()
        ],
        "only the PAM file and its backup may exist (no temporary file left)"
    );
}

/// A symlinked PAM file is refused (fail closed): the rename would silently turn
/// it into a regular file and detach it from the distribution-managed target.
#[test]
fn test_gdm_enable_refuses_a_symlinked_pam_file() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("real-gdm-password");
    let pam_file = temp.path().join("gdm-password");
    let disable_file = temp.path().join("gdm.disable");
    fs::write(&target, PRISTINE_GDM).unwrap();
    std::os::unix::fs::symlink(&target, &pam_file).unwrap();

    let result = soos_admin_cli::gdm::configure_gdm(&GdmAction::Enable, &pam_file, &disable_file);
    assert!(result.is_err(), "a symlinked PAM file must be refused");
    assert_eq!(fs::read_to_string(&target).unwrap(), PRISTINE_GDM);
    assert!(!backup_of(&pam_file).exists(), "no backup on refusal");
}
