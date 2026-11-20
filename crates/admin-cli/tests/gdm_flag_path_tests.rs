//! Contract tests for security review 2026-11-10 batch 10, finding `PSTKC-01` (GitHub #581):
//! `soos-admin gdm disable` and `gdm status` must use exactly the disable flags `pam_soos.so`
//! reads for the service behind `--pam-file`, never a flag the module ignores.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite uses direct assertions and unwrap"
)]

use pam_soos::config::PamConfig;
use soos_admin_cli::args::{Cli, Commands, GdmAction};
use soos_admin_cli::gdm::{
    configure_gdm, default_disable_file, get_gdm_status, module_disabled, module_flag_names,
    pam_service_name, DEFAULT_FLAG_DIR,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

use clap::Parser;

const COMMON_AUTH: &str = "auth [success=1 default=ignore] pam_unix.so\nauth requisite pam_deny.so\nauth required pam_permit.so\n";

struct Fixture {
    _dir: TempDir,
    flag_dir: PathBuf,
    pam_dir: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let flag_dir = dir.path().join("soos");
    let pam_dir = dir.path().join("pam.d");
    fs::create_dir(&flag_dir).unwrap();
    fs::create_dir(&pam_dir).unwrap();
    fs::write(pam_dir.join("common-auth"), COMMON_AUTH).unwrap();
    Fixture {
        _dir: dir,
        flag_dir,
        pam_dir,
    }
}

/// A display-manager stack enabled through `gdm enable` (managed block present).
fn enabled_stack(f: &Fixture, service: &str) -> PathBuf {
    let pam_file = f.pam_dir.join(service);
    fs::write(
        &pam_file,
        "auth requisite pam_nologin.so\n@include common-auth\n",
    )
    .unwrap();
    configure_gdm(
        &GdmAction::Enable,
        &pam_file,
        &f.flag_dir.join(format!("{service}.disable")),
    )
    .expect("enable");
    pam_file
}

/// What the PAM module decides for `service` with the flags of `flag_dir`.
fn module_says_disabled(flag_dir: &Path, service: &str) -> bool {
    let config = PamConfig {
        service: service.to_string(),
        flag_dir: flag_dir.to_path_buf(),
        ..PamConfig::default()
    };
    config.is_disabled()
}

/// PFL1 — the flag directory and flag names are those of the PAM module.
#[test]
fn test_pstkc01_flag_rule_matches_pam_module() {
    assert_eq!(DEFAULT_FLAG_DIR, pam_soos::config::DEFAULT_FLAG_DIR);
    assert_eq!(
        module_flag_names("gdm-password"),
        vec!["disabled", "gdm.disable", "gdm-password.disable"]
    );
    assert_eq!(module_flag_names("sddm"), vec!["disabled", "sddm.disable"]);
    assert_eq!(
        default_disable_file(Path::new("/etc/soos"), "gdm-password"),
        PathBuf::from("/etc/soos/gdm.disable")
    );
    assert_eq!(
        default_disable_file(Path::new("/etc/soos"), "lightdm"),
        PathBuf::from("/etc/soos/lightdm.disable")
    );
    assert_eq!(pam_service_name(Path::new("/etc/pam.d/sddm")), Some("sddm"));
    assert_eq!(pam_service_name(Path::new("/etc/pam.d/.hidden")), None);
    assert_eq!(pam_service_name(Path::new("/etc/pam.d/a b")), None);

    // Every flag the module reads, for a gdm and a non-gdm service, agrees with
    // `PamConfig::is_disabled`, and a flag it does not read never disables.
    for service in ["gdm-password", "gdm-autologin", "sddm", "lightdm"] {
        for flag in [
            "disabled",
            "gdm.disable",
            "sddm.disable",
            "gdm-password.disable",
            "other.disable",
        ] {
            let f = fixture();
            fs::write(f.flag_dir.join(flag), "disabled\n").unwrap();
            assert_eq!(
                module_disabled(&f.flag_dir, Some(service)),
                module_says_disabled(&f.flag_dir, service),
                "service {service}, flag {flag}"
            );
        }
    }
}

/// PFL2 — the issue's regression (1): `gdm disable` on an SDDM stack with the GDM flag would
/// write a flag the module never reads for `sddm`; it is refused and nothing is written.
#[test]
fn test_pstkc01_disable_refuses_flag_not_read_for_service() {
    let f = fixture();
    let pam_file = enabled_stack(&f, "sddm");
    let gdm_flag = f.flag_dir.join("gdm.disable");
    let err = configure_gdm(&GdmAction::Disable, &pam_file, &gdm_flag)
        .expect_err("gdm.disable is not read for the sddm service");
    let message = err.to_string();
    assert!(message.contains("sddm.disable"), "{message}");
    assert!(!gdm_flag.exists(), "nothing written on refusal");
    let status = get_gdm_status(&pam_file, &gdm_flag);
    assert!(status.installed);
    assert!(
        status.enabled,
        "a flag the module ignores never reports Disabled"
    );
    assert!(!module_says_disabled(&f.flag_dir, "sddm"));
}

/// PFL2 — `gdm disable` with the flag the module reads for `sddm`: status and module agree.
#[test]
fn test_pstkc01_disable_with_service_flag_agrees_with_module() {
    let f = fixture();
    let pam_file = enabled_stack(&f, "sddm");
    let flag = f.flag_dir.join("sddm.disable");
    let status = configure_gdm(&GdmAction::Disable, &pam_file, &flag).expect("disable");
    assert!(flag.is_file());
    assert!(!status.enabled);
    assert!(module_says_disabled(&f.flag_dir, "sddm"));
    assert_eq!(status.enabled, !module_says_disabled(&f.flag_dir, "sddm"));
}

/// PFL3 — the issue's regression (2): an arbitrary flag name is refused.
#[test]
fn test_pstkc01_disable_refuses_arbitrary_flag_name() {
    let f = fixture();
    let pam_file = enabled_stack(&f, "gdm-password");
    let odd = f.flag_dir.join("gdm-off");
    assert!(configure_gdm(&GdmAction::Disable, &pam_file, &odd).is_err());
    assert!(!odd.exists());
    fs::write(&odd, "disabled\n").unwrap();
    assert!(
        get_gdm_status(&pam_file, &odd).enabled,
        "a file the module never reads must not report Disabled"
    );
}

/// PFL3 — status reports every flag the module reads for the service: the per-service flag
/// and the global flag of the same directory.
#[test]
fn test_pstkc01_status_counts_every_module_flag() {
    let f = fixture();
    let pam_file = enabled_stack(&f, "gdm-password");
    let gdm_flag = f.flag_dir.join("gdm.disable");
    assert!(get_gdm_status(&pam_file, &gdm_flag).enabled);
    for flag in ["gdm-password.disable", "disabled"] {
        fs::write(f.flag_dir.join(flag), "disabled\n").unwrap();
        assert!(
            !get_gdm_status(&pam_file, &gdm_flag).enabled,
            "{flag} disables the gdm-password service in the module"
        );
        assert!(module_says_disabled(&f.flag_dir, "gdm-password"));
        fs::remove_file(f.flag_dir.join(flag)).unwrap();
    }
}

/// PFL4 — the CLI derives the flag from `--pam-file` when `--disable-file` is not given.
#[test]
fn test_pstkc01_cli_disable_file_is_optional_and_derived() {
    let cli = Cli::try_parse_from([
        "soos-admin",
        "gdm",
        "status",
        "--pam-file",
        "/etc/pam.d/sddm",
    ])
    .unwrap();
    let Commands::Gdm(args) = cli.command else {
        panic!("expected gdm");
    };
    assert_eq!(args.disable_file, None);

    let f = fixture();
    let pam_file = enabled_stack(&f, "sddm");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_soos-admin"))
        .args(["--format", "json", "gdm", "status", "--pam-file"])
        .arg(&pam_file)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["disable_file"], "/etc/soos/sddm.disable");
}

/// PFL4 — the CLI refuses `gdm disable` with a flag outside the directory the module reads.
#[test]
fn test_pstkc01_cli_disable_refuses_flag_outside_module_dir() {
    let f = fixture();
    let pam_file = enabled_stack(&f, "gdm-password");
    let flag = f.flag_dir.join("gdm.disable");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_soos-admin"))
        .args(["gdm", "disable", "--pam-file"])
        .arg(&pam_file)
        .arg("--disable-file")
        .arg(&flag)
        .output()
        .unwrap();
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("/etc/soos"),
        "{out:?}"
    );
    assert!(!flag.exists(), "nothing written");
}
