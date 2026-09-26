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
