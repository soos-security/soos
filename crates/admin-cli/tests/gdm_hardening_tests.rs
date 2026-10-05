//! Contract tests for GitHub #333 (spec `AI/architect_spec_gdm_hardening_flaky_tests.md`,
//! matrix rows GHF2–GHF5) through the public `configure_gdm` / `get_gdm_status` API:
//!
//! - GHF2: the edited GDM file is refused when it is a symlink, a FIFO (without blocking),
//!   oversized, not UTF-8 or missing, with the kept texts; a group-writable file is accepted
//!   and rewritten without group/world write.
//! - GHF3: `MAX_PAM_STACK_READS` (32 stack files per analysis) applies to `enable` and
//!   `status`.
//! - GHF4: `enable` and `status` agree when a pre-anchor jump skips the delegation; over every
//!   fixture, `enable` `Ok` implies `installed: true`, and `shared_stack: Some` implies an
//!   `enable` that succeeds without writing.
//! - GHF5: a shared `[success=N]` rule whose jump does not land in its own file is refused by
//!   `enable` and reported `installed: false` by `status`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite uses direct assertions and unwrap"
)]

use soos_admin_cli::args::GdmAction;
use soos_admin_cli::gdm::{
    configure_gdm, get_gdm_status, pam_backup_path, GdmStatus, GDM_PAM_LINE, MAX_PAM_STACK_READS,
};
use soos_admin_cli::AdminCliError;
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;
use tempfile::tempdir;

// ----------------------------------------------------------------------------
// Fixtures and helpers
// ----------------------------------------------------------------------------

/// GDM service delegating to `shared-auth` after one in-file gate.
const SIMPLE_GDM_PASSWORD: &str = "\
#%PAM-1.0
auth       requisite   pam_nologin.so
auth       include     shared-auth
auth       optional    pam_gnome_keyring.so
account    include     shared-auth
";

/// GDM service whose pre-anchor jump lands exactly on the delegation (crosses, no skip).
const JUMP_ON_ANCHOR_GDM_PASSWORD: &str = "\
#%PAM-1.0
auth       [success=1 default=ignore]  pam_succeed_if.so user ingroup nopasswdlogin
auth       requisite                   pam_nologin.so
auth       include                     shared-auth
account    include                     shared-auth
";

/// GDM service whose pre-anchor jump lands beyond the delegation: that branch never runs
/// the shared stack (GitHub #333 item 4).
const JUMP_SKIPS_ANCHOR_GDM_PASSWORD: &str = "\
#%PAM-1.0
auth       [success=2 default=ignore]  pam_succeed_if.so user ingroup nopasswdlogin
auth       requisite                   pam_nologin.so
auth       include                     shared-auth
auth       optional                    pam_gnome_keyring.so
account    include                     shared-auth
";

/// GDM service without delegation (enable inserts the managed block).
const PLAIN_GDM_PASSWORD: &str = "\
#%PAM-1.0
auth       requisite   pam_nologin.so
auth       required    pam_unix.so
account    required    pam_unix.so
";

/// The soos-edited Arch `system-auth` shipped by the package.
const ARCH_PACKAGED_SYSTEM_AUTH: &str = include_str!("../../../packaging/pam/arch/system-auth");

const ARCH_GDM_PASSWORD: &str = "\
#%PAM-1.0

auth       include                     system-local-login
auth       optional                    pam_gnome_keyring.so
account    include                     system-local-login
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
";

/// Debian `common-auth` after pam-auth-update rewrote the soos control into `[success=2]`.
const DEBIAN_COMMON_AUTH_SOOS_JUMP: &str = "\
auth\t[success=2 default=ignore]\tpam_soos.so
auth\t[success=1 default=ignore]\tpam_unix.so nullok
auth\trequisite\t\t\tpam_deny.so
auth\trequired\t\t\tpam_permit.so
auth\toptional\t\t\tpam_cap.so
";

const UBUNTU_GDM_PASSWORD: &str = "\
#%PAM-1.0
auth    requisite       pam_nologin.so
auth\trequired\tpam_succeed_if.so user != root quiet_success
@include common-auth
auth    optional        pam_gnome_keyring.so
";

/// A `shared-auth` stack: one gate, `rule`, the credential and the post-credential rules.
fn shared_auth_with(rule: &str) -> String {
    format!(
        "#%PAM-1.0\n\
         auth       required                    pam_faillock.so preauth\n\
         {rule}\n\
         auth       [success=1 default=bad]     pam_unix.so try_first_pass nullok\n\
         auth       [default=die]               pam_faillock.so authfail\n\
         auth       required                    pam_deny.so\n\
         account    required                    pam_unix.so\n"
    )
}

/// A shared stack whose `[success=4]` jump runs past the end of the file (libpam "bad jump").
const PAST_END_SHARED: &str = "\
#%PAM-1.0
auth       required                    pam_faillock.so preauth
auth       [success=4 default=ignore]  pam_soos.so
auth       [success=1 default=bad]     pam_unix.so try_first_pass nullok
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
    fs::set_permissions(&pam_file, fs::Permissions::from_mode(0o644)).unwrap();
    let disable_file = dir.path().join("soos").join("gdm.disable");
    fs::create_dir_all(disable_file.parent().unwrap()).unwrap();
    fs::write(&disable_file, "disabled\n").unwrap();
    Fixture {
        _dir: dir,
        pam_dir,
        pam_file,
        disable_file,
    }
}

fn dir_entries(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

fn enable(f: &Fixture) -> Result<GdmStatus, AdminCliError> {
    configure_gdm(&GdmAction::Enable, &f.pam_file, &f.disable_file)
}

fn status(f: &Fixture) -> GdmStatus {
    get_gdm_status(&f.pam_file, &f.disable_file)
}

/// Runs `enable` and asserts a refusal containing every `needle`, with nothing written
/// (file byte-identical, no backup, no temporary file) and `gdm.disable` kept.
fn assert_enable_refused(f: &Fixture, needles: &[&str]) -> String {
    let before_bytes = fs::read(&f.pam_file).ok();
    let before_entries = dir_entries(&f.pam_dir);
    let msg = enable(f)
        .expect_err("gdm enable must refuse this stack")
        .to_string();
    for needle in needles {
        assert!(msg.contains(needle), "expected `{needle}` in: {msg}");
    }
    assert_eq!(fs::read(&f.pam_file).ok(), before_bytes, "nothing written");
    assert!(!pam_backup_path(&f.pam_file).exists(), "no backup created");
    assert_eq!(dir_entries(&f.pam_dir), before_entries, "no file left");
    assert!(f.disable_file.exists(), "gdm.disable is kept on refusal");
    msg
}

fn assert_status_not_installed(f: &Fixture) {
    let s = status(f);
    assert!(!s.installed, "status must report installed: false: {s:?}");
    assert_eq!(s.shared_stack, None, "{s:?}");
}

/// `count` includes of `leaf` followed by `tail` lines (the GDM file's delegated part).
fn gdm_with_includes(count: usize, leaf: &str, tail: &str) -> String {
    let mut gdm = String::from("#%PAM-1.0\n");
    for _ in 0..count {
        gdm.push_str(&format!("auth       include     {leaf}\n"));
    }
    gdm.push_str(tail);
    gdm
}

const LEAF: &str = "auth       required    pam_env.so\n";
const SOOS_AUTH: &str = "\
auth       [success=done default=ignore]  pam_soos.so
auth       sufficient                     pam_unix.so nullok
";

// ----------------------------------------------------------------------------
// GHF2 — the edited file is checked and read through one O_NOFOLLOW descriptor
// ----------------------------------------------------------------------------

#[test]
fn test_ghf2_enable_refuses_a_symlinked_gdm_file_with_kept_text() {
    let f = fixture(PLAIN_GDM_PASSWORD, &[]);
    let target = f.pam_dir.join("real-gdm-password");
    fs::rename(&f.pam_file, &target).unwrap();
    symlink(&target, &f.pam_file).unwrap();
    let msg = assert_enable_refused(&f, &[]);
    assert_eq!(
        msg,
        format!(
            "GDM configuration error: Refusing to use '{}': not a regular file (symlink or special file)",
            f.pam_file.display()
        )
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), PLAIN_GDM_PASSWORD);
    assert!(fs::symlink_metadata(&f.pam_file)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn test_ghf2_enable_refuses_a_fifo_without_blocking() {
    let f = fixture(PLAIN_GDM_PASSWORD, &[]);
    fs::remove_file(&f.pam_file).unwrap();
    let made = Command::new("mkfifo").arg(&f.pam_file).status().unwrap();
    assert!(made.success(), "mkfifo must create the fixture");
    let (tx, rx) = mpsc::channel();
    let pam_file = f.pam_file.clone();
    let disable_file = f.disable_file.clone();
    std::thread::spawn(move || {
        let result = configure_gdm(&GdmAction::Enable, &pam_file, &disable_file);
        tx.send(result.map_err(|e| e.to_string())).unwrap();
    });
    let result = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("gdm enable must never block on a FIFO");
    let msg = result.expect_err("a FIFO must be refused");
    assert_eq!(
        msg,
        format!(
            "GDM configuration error: Refusing to use '{}': not a regular file (symlink or special file)",
            f.pam_file.display()
        )
    );
    assert!(f.disable_file.exists());
    assert!(!pam_backup_path(&f.pam_file).exists());
}

#[test]
fn test_ghf2_enable_refuses_oversized_non_utf8_and_missing_files_with_kept_texts() {
    let f = fixture(PLAIN_GDM_PASSWORD, &[]);
    let mut big = String::from(PLAIN_GDM_PASSWORD);
    while big.len() <= 64 * 1024 {
        big.push_str("# padding comment line to exceed the bound\n");
    }
    fs::write(&f.pam_file, &big).unwrap();
    let msg = assert_enable_refused(&f, &[]);
    assert_eq!(
        msg,
        format!(
            "GDM configuration error: PAM file '{}' exceeds 65536 bytes",
            f.pam_file.display()
        )
    );

    let mut bytes = PLAIN_GDM_PASSWORD.as_bytes().to_vec();
    bytes.extend_from_slice(b"# \xff\xfe not utf-8\n");
    fs::write(&f.pam_file, &bytes).unwrap();
    let msg = assert_enable_refused(&f, &[]);
    assert_eq!(
        msg,
        format!(
            "GDM configuration error: PAM file '{}' is not valid UTF-8; refusing to edit it",
            f.pam_file.display()
        )
    );

    fs::remove_file(&f.pam_file).unwrap();
    let msg = assert_enable_refused(&f, &[]);
    assert_eq!(
        msg,
        format!(
            "GDM configuration error: PAM file '{}' does not exist",
            f.pam_file.display()
        )
    );
    assert!(!f.pam_file.exists(), "a missing file is never created");
}

/// Plan evaluator R2-1: the mode is read from the descriptor; a `0o664` file is accepted and
/// the rewritten file and its backup drop group/world write (`0o644`).
#[test]
fn test_ghf2_enable_accepts_group_writable_file_and_writes_0644() {
    let f = fixture(PLAIN_GDM_PASSWORD, &[]);
    fs::set_permissions(&f.pam_file, fs::Permissions::from_mode(0o664)).unwrap();
    let s = enable(&f).expect("an unchanged 0o664 file must be accepted");
    assert!(s.installed);
    assert!(fs::read_to_string(&f.pam_file)
        .unwrap()
        .contains(GDM_PAM_LINE));
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode(&f.pam_file), 0o644);
    let backup = pam_backup_path(&f.pam_file);
    assert_eq!(mode(&backup), 0o644);
    assert_eq!(fs::read_to_string(&backup).unwrap(), PLAIN_GDM_PASSWORD);
    let meta = fs::metadata(&f.pam_file).unwrap();
    let me = fs::metadata(&f.pam_dir).unwrap();
    assert_eq!((meta.uid(), meta.gid()), (me.uid(), me.gid()), "owner kept");
    assert!(!f.disable_file.exists());
}

// ----------------------------------------------------------------------------
// GHF3 — MAX_PAM_STACK_READS for enable and status
// ----------------------------------------------------------------------------

#[test]
fn test_ghf3_constant_is_reexported_with_value_32() {
    assert_eq!(MAX_PAM_STACK_READS, 32);
}

#[test]
fn test_ghf3_enable_refuses_33_stack_reads() {
    let gdm = gdm_with_includes(
        MAX_PAM_STACK_READS + 1,
        "leaf",
        "auth       sufficient  pam_unix.so\n",
    );
    let f = fixture(&gdm, &[("leaf", LEAF)]);
    assert_enable_refused(&f, &["more than 32 stack files"]);
}

#[test]
fn test_ghf3_shared_rule_on_the_33rd_read_is_refused_and_not_installed() {
    let gdm = gdm_with_includes(
        MAX_PAM_STACK_READS,
        "leaf",
        "auth       include     soos-auth\n",
    );
    let f = fixture(&gdm, &[("leaf", LEAF), ("soos-auth", SOOS_AUTH)]);
    assert_status_not_installed(&f);
    assert_enable_refused(&f, &["more than 32 stack files"]);
}

#[test]
fn test_ghf3_shared_rule_on_the_32nd_read_is_installed() {
    let gdm = gdm_with_includes(
        MAX_PAM_STACK_READS - 1,
        "leaf",
        "auth       include     soos-auth\n",
    );
    let f = fixture(&gdm, &[("leaf", LEAF), ("soos-auth", SOOS_AUTH)]);
    let s = status(&f);
    assert!(s.installed, "{s:?}");
    assert_eq!(s.shared_stack.as_deref(), Some("soos-auth"));
    let s = enable(&f).expect("32 reads are within the budget");
    assert!(s.installed);
    assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), gdm);
}

// ----------------------------------------------------------------------------
// GHF4 — enable / status agreement
// ----------------------------------------------------------------------------

/// Issue #333 item 4: the status test of `jump_skips_anchor` (unchanged behaviour).
#[test]
fn test_ghf4_status_reports_not_installed_when_a_jump_skips_the_shared_stack() {
    let shared = shared_auth_with("auth  [success=done default=ignore]  pam_soos.so");
    let f = fixture(JUMP_SKIPS_ANCHOR_GDM_PASSWORD, &[("shared-auth", &shared)]);
    assert_status_not_installed(&f);
}

/// GHF4: `enable` refuses (E4) where `status` reports `installed: false`.
#[test]
fn test_ghf4_enable_refuses_a_jump_that_skips_the_shared_stack() {
    let shared = shared_auth_with("auth  [success=done default=ignore]  pam_soos.so");
    let f = fixture(JUMP_SKIPS_ANCHOR_GDM_PASSWORD, &[("shared-auth", &shared)]);
    let msg = assert_enable_refused(
        &f,
        &[
            "lands beyond the shared auth stack 'shared-auth'",
            "nothing was written",
            "Docs/DISTRIBUTION_DEPLOYMENT.md section 2.1",
        ],
    );
    assert!(msg.contains("[...=N] jump"), "{msg}");
    assert_status_not_installed(&f);
}

/// A named set of PAM trees covering every IGF14–IGF18 and GHF outcome.
fn agreement_fixtures() -> Vec<(&'static str, Fixture)> {
    let done = shared_auth_with("auth  [success=done default=ignore]  pam_soos.so");
    let jump1 = shared_auth_with("auth  [success=1 default=ignore]  pam_soos.so timeout_ms=1500");
    let plain = shared_auth_with("auth  required  pam_env.so");
    let past33 = gdm_with_includes(
        MAX_PAM_STACK_READS,
        "leaf",
        "auth       include     soos-auth\n",
    );
    vec![
        (
            "arch packaged",
            fixture(
                ARCH_GDM_PASSWORD,
                &[
                    ("system-local-login", ARCH_SYSTEM_LOCAL_LOGIN),
                    ("system-login", ARCH_SYSTEM_LOGIN),
                    ("system-auth", ARCH_PACKAGED_SYSTEM_AUTH),
                ],
            ),
        ),
        (
            "debian jump",
            fixture(
                UBUNTU_GDM_PASSWORD,
                &[("common-auth", DEBIAN_COMMON_AUTH_SOOS_JUMP)],
            ),
        ),
        (
            "simple done",
            fixture(SIMPLE_GDM_PASSWORD, &[("shared-auth", &done)]),
        ),
        (
            "simple jump in file",
            fixture(SIMPLE_GDM_PASSWORD, &[("shared-auth", &jump1)]),
        ),
        (
            "simple no soos rule",
            fixture(SIMPLE_GDM_PASSWORD, &[("shared-auth", &plain)]),
        ),
        (
            "jump on anchor",
            fixture(JUMP_ON_ANCHOR_GDM_PASSWORD, &[("shared-auth", &done)]),
        ),
        (
            "jump skips anchor",
            fixture(JUMP_SKIPS_ANCHOR_GDM_PASSWORD, &[("shared-auth", &done)]),
        ),
        (
            "jump past end of shared file",
            fixture(SIMPLE_GDM_PASSWORD, &[("shared-auth", PAST_END_SHARED)]),
        ),
        (
            "33 stack reads",
            fixture(&past33, &[("leaf", LEAF), ("soos-auth", SOOS_AUTH)]),
        ),
        ("plain file", fixture(PLAIN_GDM_PASSWORD, &[])),
    ]
}

/// GHF4 agreement invariant (spec §4.4, regular readable GDM file): `enable` `Ok` implies the
/// returned and a later status report `installed: true`; a status `shared_stack: Some`
/// implies an `enable` that returns `Ok` without writing.
#[test]
fn test_ghf4_enable_ok_implies_installed_and_shared_stack_implies_ok() {
    for (name, f) in agreement_fixtures() {
        let before = status(&f);
        let bytes = fs::read(&f.pam_file).unwrap();
        match enable(&f) {
            Ok(returned) => {
                assert!(
                    returned.installed,
                    "{name}: enable returned Ok but status says not installed: {returned:?}"
                );
                assert!(
                    status(&f).installed,
                    "{name}: a later gdm status must report installed"
                );
            }
            Err(err) => assert!(
                before.shared_stack.is_none(),
                "{name}: status reported shared_stack {:?} but enable refused: {err}",
                before.shared_stack
            ),
        }
        if before.shared_stack.is_some() {
            assert_eq!(
                fs::read(&f.pam_file).unwrap(),
                bytes,
                "{name}: a shared stack means enable writes nothing"
            );
        }
    }
}

// ----------------------------------------------------------------------------
// GHF5 — shared [success=N] rule must land in its own file
// ----------------------------------------------------------------------------

#[test]
fn test_ghf5_enable_refuses_a_shared_jump_past_the_end_of_its_file() {
    let f = fixture(SIMPLE_GDM_PASSWORD, &[("shared-auth", PAST_END_SHARED)]);
    assert_enable_refused(
        &f,
        &[
            "the shared auth stack 'shared-auth'",
            "[success=4]",
            "does not land on a rule of the same file",
        ],
    );
    assert_status_not_installed(&f);
}

#[test]
fn test_ghf5_enable_refuses_a_shared_jump_over_an_include() {
    let shared = "\
auth       [success=2 default=ignore]  pam_soos.so
auth       sufficient                  pam_unix.so nullok
auth       include                     post-auth
auth       required                    pam_permit.so
auth       required                    pam_env.so
";
    let f = fixture(
        SIMPLE_GDM_PASSWORD,
        &[("shared-auth", shared), ("post-auth", LEAF)],
    );
    assert_enable_refused(
        &f,
        &["[success=2]", "does not land on a rule of the same file"],
    );
    assert_status_not_installed(&f);
}
