//! Contract (GitHub #318, matrix AFC7): the `soos-enroll` program run by the GUI's privileged
//! actions follows the build-time `SOOS_BINDIR` (exported by `scripts/install.sh --build` as
//! `<prefix>/bin`), defaults to `/usr/bin`, and an unsafe value never reaches the binary: the
//! build script rejects anything but a normalized absolute directory. `pkexec` and
//! `systemctl` are system tools and stay under `/usr/bin`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions"
)]

#[path = "../build_support/bindir.rs"]
mod bindir;

use soos_gui::privileged::{
    import_helper_args, PKEXEC_PROGRAM, SOOS_ENROLL_PROGRAM, SYSTEMCTL_PROGRAM,
};

#[test]
fn test_soos_enroll_program_follows_build_time_bindir() {
    let expected = match option_env!("SOOS_BINDIR") {
        None => "/usr/bin/soos-enroll".to_string(),
        Some(dir) => format!("{dir}/soos-enroll"),
    };
    assert_eq!(SOOS_ENROLL_PROGRAM, expected);
    assert_eq!(
        bindir::enroll_program_for(option_env!("SOOS_BINDIR")).as_deref(),
        Ok(expected.as_str())
    );
    let args = import_helper_args(1000);
    assert_eq!(args.first().map(String::as_str), Some(expected.as_str()));
}

#[test]
fn test_system_tools_stay_under_usr_bin() {
    assert_eq!(PKEXEC_PROGRAM, "/usr/bin/pkexec");
    assert_eq!(SYSTEMCTL_PROGRAM, "/usr/bin/systemctl");
}

#[test]
fn test_bindir_validation_accepts_normalized_absolute_directories() {
    for dir in [
        "/usr/bin",
        "/usr/local/bin",
        "/opt/soos/bin",
        "/opt/soos-1.2_x+y/bin",
    ] {
        assert_eq!(bindir::validate_bindir(dir), Ok(()), "{dir}");
    }
    assert_eq!(
        bindir::enroll_program_for(None).as_deref(),
        Ok("/usr/bin/soos-enroll")
    );
    assert_eq!(
        bindir::enroll_program_for(Some("/opt/soos/bin")).as_deref(),
        Ok("/opt/soos/bin/soos-enroll")
    );
}

#[test]
fn test_bindir_validation_rejects_relative_and_unsafe_paths() {
    let too_long = format!("/{}", "a".repeat(bindir::MAX_BINDIR_LEN));
    for dir in [
        "",
        "bin",
        "./bin",
        "/",
        "/usr/bin/",
        "/usr//bin",
        "/usr/../bin",
        "/usr/./bin",
        "/opt/so os/bin",
        "/opt/soos\nbin",
        "/opt/$HOME/bin",
        "/opt/\"x/bin",
        "/opt/x;y/bin",
        "/opt/caf\u{e9}/bin",
        too_long.as_str(),
    ] {
        assert!(
            bindir::validate_bindir(dir).is_err(),
            "{dir:?} must be rejected"
        );
        assert!(
            bindir::enroll_program_for(Some(dir)).is_err(),
            "{dir:?} must not produce a program path"
        );
    }
}
