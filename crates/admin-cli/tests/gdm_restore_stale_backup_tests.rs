//! Contract tests for GitHub #312 (review finding STO-NEW-7): `soos-admin gdm restore` never
//! puts a stale backup over changes made after `gdm enable`. The backup is restored only
//! when the current PAM file, without the soos managed rules, is byte-for-byte the backup;
//! otherwise the restore is refused and nothing changes, unless `--force` is given.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual test suite uses direct assertions and unwrap"
)]

use clap::Parser;
use soos_admin_cli::args::{Cli, Commands, GdmAction, GdmArgs};
use soos_admin_cli::gdm::{configure_gdm, configure_gdm_with_options, pam_backup_path, GdmOptions};
use std::fs;
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};

/// Minimal `gdm-password` with its own pre-credential gate and credential module.
const PRISTINE: &str = "\
#%PAM-1.0
auth    requisite       pam_nologin.so
auth    required        pam_unix.so
account required        pam_unix.so
";

struct Fixture {
    _dir: TempDir,
    pam_file: PathBuf,
    disable_file: PathBuf,
}

fn enabled_fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let pam_dir = dir.path().join("pam.d");
    fs::create_dir(&pam_dir).unwrap();
    let pam_file = pam_dir.join("gdm-password");
    fs::write(&pam_file, PRISTINE).unwrap();
    let disable_file = dir.path().join("etc-soos").join("gdm.disable");
    configure_gdm(&GdmAction::Enable, &pam_file, &disable_file).expect("enable");
    assert_eq!(
        fs::read_to_string(pam_backup_path(&pam_file)).unwrap(),
        PRISTINE
    );
    Fixture {
        _dir: dir,
        pam_file,
        disable_file,
    }
}

/// An administrator edit made after `gdm enable` (outside the managed block).
fn edit_after_enable(f: &Fixture) -> String {
    let mut content = fs::read_to_string(&f.pam_file).unwrap();
    content.push_str("session optional        pam_keyinit.so force revoke\n");
    fs::write(&f.pam_file, &content).unwrap();
    content
}

#[test]
fn test_312_gdm_restore_refuses_a_stale_backup() {
    let f = enabled_fixture();
    let edited = edit_after_enable(&f);

    let res = configure_gdm(&GdmAction::Restore, &f.pam_file, &f.disable_file);
    assert!(res.is_err(), "a stale backup must not be restored");
    let msg = res.unwrap_err().to_string();
    assert!(msg.contains("--force"), "the refusal names --force: {msg}");
    assert_eq!(
        fs::read_to_string(&f.pam_file).unwrap(),
        edited,
        "the edited file is kept"
    );
    assert_eq!(
        fs::read_to_string(pam_backup_path(&f.pam_file)).unwrap(),
        PRISTINE,
        "the backup is kept"
    );
}

#[test]
fn test_312_gdm_restore_force_restores_a_stale_backup() {
    let f = enabled_fixture();
    edit_after_enable(&f);

    configure_gdm_with_options(
        &GdmAction::Restore,
        &f.pam_file,
        &f.disable_file,
        GdmOptions { force: true },
    )
    .expect("forced restore");
    assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), PRISTINE);
    assert!(!pam_backup_path(&f.pam_file).exists(), "backup consumed");
}

#[test]
fn test_312_gdm_restore_of_an_unchanged_enable_still_succeeds() {
    let f = enabled_fixture();
    configure_gdm(&GdmAction::Restore, &f.pam_file, &f.disable_file).expect("restore");
    assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), PRISTINE);
    assert!(!pam_backup_path(&f.pam_file).exists());
}

#[test]
fn test_312_gdm_restore_force_flag_parses() {
    let cli = Cli::try_parse_from(["soos-admin", "gdm", "restore", "--force"]).unwrap();
    match cli.command {
        Commands::Gdm(GdmArgs {
            action: GdmAction::Restore,
            force: true,
            ..
        }) => {}
        _ => panic!("expected gdm restore --force"),
    }
    let cli = Cli::try_parse_from(["soos-admin", "gdm", "restore"]).unwrap();
    match cli.command {
        Commands::Gdm(GdmArgs { force: false, .. }) => {}
        _ => panic!("--force defaults to false"),
    }
}
