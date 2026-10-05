//! Contract tests for GitHub #331 §3: `soos-admin gdm enable` must not add a second face
//! verification when the auth stack delegated from the GDM service file already reaches an
//! active primary `pam_soos.so` rule before any credential module (matrix IGF14, IGF15,
//! IGF17, IGF18; ADR 2026-10-05 "GDM Reuses a Shared Primary soos Rule").
//!
//! - IGF14: packaged Arch, Debian and Fedora shared rules → `Ok`, file byte-identical, no
//!   backup, no temporary file, `gdm.disable` removed.
//! - IGF15: a managed block (or bare / legacy line) left by an earlier enable is removed
//!   atomically; an existing backup is never altered; `gdm restore` keeps its semantics.
//! - IGF17: only primary rules qualify; every other `pam_soos.so` rule keeps the refusal.
//! - IGF18: error priority (jump crossing, unclassified rule, administrator rule).

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
    configure_gdm, configure_gdm_with_options, pam_backup_path, GdmOptions, GDM_BLOCK_BEGIN,
    GDM_PAM_LINE, LEGACY_GDM_PAM_LINE,
};
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use tempfile::tempdir;

// ----------------------------------------------------------------------------
// Distribution fixtures
// ----------------------------------------------------------------------------

/// `/etc/pam.d/gdm-password` from Arch Linux (`gdm` 46).
const ARCH_GDM_PASSWORD: &str = "\
#%PAM-1.0

auth       include                     system-local-login
auth       optional                    pam_gnome_keyring.so
account    include                     system-local-login
password   include                     system-local-login
password   optional                    pam_gnome_keyring.so use_authtok
session    include                     system-local-login
session    optional                    pam_gnome_keyring.so auto_start
";

/// Arch Linux `pambase` `/etc/pam.d/system-local-login`.
const ARCH_SYSTEM_LOCAL_LOGIN: &str = "\
#%PAM-1.0

auth      include   system-login
account   include   system-login
password  include   system-login
session   include   system-login
";

/// Arch Linux `pambase` `/etc/pam.d/system-login`.
const ARCH_SYSTEM_LOGIN: &str = "\
#%PAM-1.0

auth       required   pam_shells.so
auth       requisite  pam_nologin.so
auth       include    system-auth

account    required   pam_access.so
account    required   pam_nologin.so
account    include    system-auth

password   include    system-auth

session    optional   pam_loginuid.so
session    optional   pam_keyinit.so       force revoke
session    include    system-auth
session    optional   pam_motd.so
-session   optional   pam_systemd.so
session    required   pam_env.so
";

/// Stock Arch Linux `pambase` `/etc/pam.d/system-auth` (before the soos edit).
const ARCH_STOCK_SYSTEM_AUTH: &str = "\
#%PAM-1.0

auth       required                    pam_faillock.so      preauth
-auth      [success=2 default=ignore]  pam_systemd_home.so
auth       [success=1 default=bad]     pam_unix.so          try_first_pass nullok
auth       [default=die]               pam_faillock.so      authfail
auth       optional                    pam_permit.so
auth       required                    pam_env.so
auth       required                    pam_faillock.so      authsucc

account    required                    pam_unix.so
";

/// The soos-edited Arch `system-auth` shipped by the package (primary rule `[success=4 ...]`).
const ARCH_PACKAGED_SYSTEM_AUTH: &str = include_str!("../../../packaging/pam/arch/system-auth");

/// `/etc/pam.d/gdm-password` from Ubuntu 24.04 (`gdm3` 46).
const UBUNTU_GDM_PASSWORD: &str = "\
#%PAM-1.0
auth    requisite       pam_nologin.so
auth\trequired\tpam_succeed_if.so user != root quiet_success
@include common-auth
auth    optional        pam_gnome_keyring.so
@include common-account
session required        pam_loginuid.so
@include common-session
@include common-password
";

/// Debian/Ubuntu `common-auth` with the soos pam-auth-update profile (`success=done`).
const DEBIAN_COMMON_AUTH_SOOS_DONE: &str = "\
# /etc/pam.d/common-auth - authentication settings common to all services
auth\t[success=done default=ignore]\tpam_soos.so
auth\t[success=1 default=ignore]\tpam_unix.so nullok
auth\trequisite\t\t\tpam_deny.so
auth\trequired\t\t\tpam_permit.so
auth\toptional\t\t\tpam_cap.so
";

/// Same profile after pam-auth-update rewrote the control into a numeric jump.
const DEBIAN_COMMON_AUTH_SOOS_JUMP: &str = "\
# /etc/pam.d/common-auth - authentication settings common to all services
auth\t[success=2 default=ignore]\tpam_soos.so
auth\t[success=1 default=ignore]\tpam_unix.so nullok
auth\trequisite\t\t\tpam_deny.so
auth\trequired\t\t\tpam_permit.so
auth\toptional\t\t\tpam_cap.so
";

/// `/etc/pam.d/gdm-password` from Fedora 40 (`gdm` 46).
const FEDORA_GDM_PASSWORD: &str = "\
auth     [success=done ignore=ignore default=bad] pam_selinux_permit.so
auth        substack      password-auth
auth        optional      pam_gnome_keyring.so
auth        include       postlogin

account     required      pam_nologin.so
account     include       password-auth

password    substack       password-auth
session     include       password-auth
session     include       postlogin
";

/// Fedora 40 `/etc/pam.d/postlogin`.
const FEDORA_POSTLOGIN: &str = "\
# Generated by authselect
session     optional                   pam_umask.so silent
";

/// The soos authselect `password-auth` template shipped by the package.
const FEDORA_SOOS_PASSWORD_AUTH_TEMPLATE: &str =
    include_str!("../../../packaging/pam/fedora/soos/password-auth");

/// Renders the authselect template as `authselect select custom/soos with-faillock` would:
/// lines guarded by `{include if "with-faillock"}` are kept, other guarded lines dropped,
/// and `{if not "without-nullok":nullok}` becomes `nullok`.
fn render_fedora_password_auth() -> String {
    let mut out = String::new();
    for line in FEDORA_SOOS_PASSWORD_AUTH_TEMPLATE.lines() {
        let line = if line.contains("{include if \"with-faillock\"}") {
            line.replace("{include if \"with-faillock\"}", "")
        } else if line.contains("{include if") {
            continue;
        } else {
            line.to_string()
        };
        let line = line.replace("{if not \"without-nullok\":nullok}", "nullok");
        out.push_str(line.trim_end());
        out.push('\n');
    }
    assert!(
        !out.contains('{'),
        "template markers left in the rendered fixture:\n{out}"
    );
    assert!(out.contains("pam_faillock.so preauth silent"));
    assert!(out.contains("[success=done default=ignore]"));
    out
}

/// Minimal GDM service delegating to `shared-auth` after one in-file gate.
const SIMPLE_GDM_PASSWORD: &str = "\
#%PAM-1.0
auth       requisite   pam_nologin.so
auth       include     shared-auth
auth       optional    pam_gnome_keyring.so
account    include     shared-auth
";

/// Builds a `shared-auth` stack whose auth phase runs one gate, `rule`, then the
/// credential module.
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

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------

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

fn write_disable_flag(f: &Fixture) {
    fs::create_dir_all(f.disable_file.parent().unwrap()).unwrap();
    fs::write(&f.disable_file, "disabled\n").unwrap();
}

fn dir_entries(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

/// Inode, size and modification time (ns) of a file: unchanged means "not written".
fn identity(path: &Path) -> (u64, u64, i64, i64) {
    let m = fs::metadata(path).unwrap();
    (m.ino(), m.size(), m.mtime(), m.mtime_nsec())
}

fn enable(f: &Fixture) -> Result<soos_admin_cli::GdmStatus, soos_admin_cli::AdminCliError> {
    configure_gdm(&GdmAction::Enable, &f.pam_file, &f.disable_file)
}

/// Asserts the IGF14 "nothing written" outcome of a successful enable on a shared stack.
fn assert_enable_writes_nothing(f: &Fixture, original: &str) {
    write_disable_flag(f);
    let before_entries = dir_entries(&f.pam_dir);
    let before_identity = identity(&f.pam_file);

    let status = enable(f).unwrap_or_else(|e| {
        panic!("enable must accept a stack that already reaches a primary pam_soos.so rule: {e}")
    });

    assert_eq!(
        fs::read_to_string(&f.pam_file).unwrap(),
        original,
        "the GDM file must stay byte-identical (no managed block)"
    );
    assert_eq!(
        identity(&f.pam_file),
        before_identity,
        "the GDM file must not be rewritten at all"
    );
    assert!(
        !pam_backup_path(&f.pam_file).exists(),
        "no backup may be created when nothing is inserted"
    );
    assert_eq!(
        dir_entries(&f.pam_dir),
        before_entries,
        "no temporary or backup file may be left in the PAM directory"
    );
    assert!(
        !f.disable_file.exists(),
        "gdm.disable must be removed after a successful enable"
    );
    assert!(
        status.installed,
        "the integration is reached through the shared rule: installed must be true"
    );
}

// ----------------------------------------------------------------------------
// IGF14 — packaged shared rules: no managed block
// ----------------------------------------------------------------------------

#[test]
fn test_igf14_arch_packaged_system_auth_enable_adds_no_block() {
    let f = arch_fixture(ARCH_PACKAGED_SYSTEM_AUTH);
    assert_enable_writes_nothing(&f, ARCH_GDM_PASSWORD);
}

#[test]
fn test_igf14_debian_common_auth_done_enable_adds_no_block() {
    let f = fixture(
        UBUNTU_GDM_PASSWORD,
        &[("common-auth", DEBIAN_COMMON_AUTH_SOOS_DONE)],
    );
    assert_enable_writes_nothing(&f, UBUNTU_GDM_PASSWORD);
}

#[test]
fn test_igf14_debian_common_auth_rewritten_jump_enable_adds_no_block() {
    let f = fixture(
        UBUNTU_GDM_PASSWORD,
        &[("common-auth", DEBIAN_COMMON_AUTH_SOOS_JUMP)],
    );
    assert_enable_writes_nothing(&f, UBUNTU_GDM_PASSWORD);
}

#[test]
fn test_igf14_fedora_soos_password_auth_enable_adds_no_block() {
    let rendered = render_fedora_password_auth();
    let f = fixture(
        FEDORA_GDM_PASSWORD,
        &[
            ("password-auth", &rendered),
            ("postlogin", FEDORA_POSTLOGIN),
        ],
    );
    assert_enable_writes_nothing(&f, FEDORA_GDM_PASSWORD);
}

#[test]
fn test_igf14_hand_written_sufficient_rule_enable_adds_no_block() {
    let shared = shared_auth_with("auth       sufficient                  pam_soos.so");
    let f = fixture(SIMPLE_GDM_PASSWORD, &[("shared-auth", &shared)]);
    assert_enable_writes_nothing(&f, SIMPLE_GDM_PASSWORD);
}

/// Rules after the shared primary rule are never inspected (§2.3): an unknown module there
/// does not cause a refusal.
#[test]
fn test_igf14_rules_after_the_shared_rule_are_not_inspected() {
    let shared = "\
auth       [success=done default=ignore]  pam_soos.so
auth       required                       pam_mystery_after.so
auth       sufficient                     pam_unix.so nullok
auth       required                       pam_deny.so
";
    let f = fixture(SIMPLE_GDM_PASSWORD, &[("shared-auth", shared)]);
    assert_enable_writes_nothing(&f, SIMPLE_GDM_PASSWORD);
}

/// The disable flag removal and status of a repeated enable stay idempotent.
#[test]
fn test_igf14_second_enable_on_shared_stack_still_writes_nothing() {
    let f = arch_fixture(ARCH_PACKAGED_SYSTEM_AUTH);
    assert_enable_writes_nothing(&f, ARCH_GDM_PASSWORD);
    assert_enable_writes_nothing(&f, ARCH_GDM_PASSWORD);
}

// ----------------------------------------------------------------------------
// IGF15 — a redundant managed block is removed; the backup is never altered
// ----------------------------------------------------------------------------

/// Enable on the stock Arch stack (block inserted, backup created), then the package edits
/// `system-auth`: returns the fixture plus the backup bytes, mode and inode.
fn arch_with_block_then_shared_rule() -> (Fixture, Vec<u8>, u32, u64) {
    let f = arch_fixture(ARCH_STOCK_SYSTEM_AUTH);
    enable(&f).expect("enable on the stock Arch stack inserts the managed block");
    let with_block = fs::read_to_string(&f.pam_file).unwrap();
    assert!(with_block.contains(GDM_BLOCK_BEGIN));
    assert!(with_block.contains(GDM_PAM_LINE));
    let backup = pam_backup_path(&f.pam_file);
    let bytes = fs::read(&backup).unwrap();
    assert_eq!(bytes, ARCH_GDM_PASSWORD.as_bytes());
    let meta = fs::metadata(&backup).unwrap();
    let mode = meta.permissions().mode();
    let ino = meta.ino();

    fs::write(f.pam_dir.join("system-auth"), ARCH_PACKAGED_SYSTEM_AUTH).unwrap();
    (f, bytes, mode, ino)
}

#[test]
fn test_igf15_existing_block_is_removed_and_backup_left_identical() {
    let (f, backup_bytes, backup_mode, backup_ino) = arch_with_block_then_shared_rule();
    let mode_before = fs::metadata(&f.pam_file).unwrap().permissions().mode();
    write_disable_flag(&f);

    let status = enable(&f).unwrap_or_else(|e| {
        panic!("enable must remove the redundant managed block, not refuse: {e}")
    });

    let content = fs::read_to_string(&f.pam_file).unwrap();
    assert_eq!(
        content, ARCH_GDM_PASSWORD,
        "the file must equal the pre-soos pristine content once the block is removed"
    );
    assert!(!content.contains("pam_soos.so"));
    assert_eq!(
        fs::metadata(&f.pam_file).unwrap().permissions().mode(),
        mode_before,
        "the rewritten file keeps its mode"
    );
    let backup = pam_backup_path(&f.pam_file);
    assert_eq!(
        fs::read(&backup).unwrap(),
        backup_bytes,
        "backup content untouched"
    );
    let meta = fs::metadata(&backup).unwrap();
    assert_eq!(
        meta.permissions().mode(),
        backup_mode,
        "backup mode untouched"
    );
    assert_eq!(meta.ino(), backup_ino, "backup never rewritten");
    let mut expected = BTreeSet::new();
    for name in [
        "gdm-password",
        "gdm-password.soos-backup",
        "system-auth",
        "system-local-login",
        "system-login",
    ] {
        expected.insert(name.to_string());
    }
    assert_eq!(dir_entries(&f.pam_dir), expected, "no temporary file left");
    assert!(!f.disable_file.exists(), "gdm.disable removed");
    assert!(status.installed);
}

#[test]
fn test_igf15_restore_after_block_removal_succeeds_and_removes_backup() {
    let (f, _, _, _) = arch_with_block_then_shared_rule();
    enable(&f).expect("enable removes the redundant block");

    configure_gdm(&GdmAction::Restore, &f.pam_file, &f.disable_file)
        .expect("restore of an up-to-date backup must succeed");
    assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), ARCH_GDM_PASSWORD);
    assert!(
        !pam_backup_path(&f.pam_file).exists(),
        "restore removes the backup"
    );
}

#[test]
fn test_igf15_second_enable_after_block_removal_writes_nothing() {
    let (f, backup_bytes, _, backup_ino) = arch_with_block_then_shared_rule();
    enable(&f).expect("first enable removes the redundant block");
    let before = identity(&f.pam_file);

    enable(&f).expect("second enable is a no-op");

    assert_eq!(
        identity(&f.pam_file),
        before,
        "inode and mtime unchanged: no rewrite"
    );
    assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), ARCH_GDM_PASSWORD);
    let backup = pam_backup_path(&f.pam_file);
    assert_eq!(fs::read(&backup).unwrap(), backup_bytes);
    assert_eq!(fs::metadata(&backup).unwrap().ino(), backup_ino);
}

#[test]
fn test_igf15_stale_backup_still_needs_force_after_block_removal() {
    let (f, backup_bytes, _, _) = arch_with_block_then_shared_rule();
    let mut edited = fs::read_to_string(&f.pam_file).unwrap();
    edited.push_str("# local administrator edit\n");
    fs::write(&f.pam_file, &edited).unwrap();

    enable(&f).expect("enable removes the redundant block and keeps the local edit");
    let expected = format!("{ARCH_GDM_PASSWORD}# local administrator edit\n");
    assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), expected);
    assert_eq!(
        fs::read(pam_backup_path(&f.pam_file)).unwrap(),
        backup_bytes,
        "the stale backup is kept as is"
    );

    let refused = configure_gdm(&GdmAction::Restore, &f.pam_file, &f.disable_file);
    assert!(
        refused.is_err(),
        "a stale backup must be refused without --force"
    );
    assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), expected);
    assert!(pam_backup_path(&f.pam_file).exists());

    configure_gdm_with_options(
        &GdmAction::Restore,
        &f.pam_file,
        &f.disable_file,
        GdmOptions { force: true },
    )
    .expect("--force restores the stale backup");
    assert_eq!(fs::read(&f.pam_file).unwrap(), backup_bytes);
    assert!(!pam_backup_path(&f.pam_file).exists());
}

/// A legacy (pre-#177) line or a bare managed line is removed too, and no backup is
/// created when none existed.
#[test]
fn test_igf15_legacy_and_bare_lines_removed_without_creating_a_backup() {
    for managed in [LEGACY_GDM_PAM_LINE, GDM_PAM_LINE] {
        let with_line = format!(
            "#%PAM-1.0\n{managed}\n{}",
            &ARCH_GDM_PASSWORD["#%PAM-1.0\n".len()..]
        );
        let f = arch_fixture(ARCH_PACKAGED_SYSTEM_AUTH);
        fs::write(&f.pam_file, &with_line).unwrap();

        enable(&f).unwrap_or_else(|e| {
            panic!("enable must remove the redundant line {managed:?}, not refuse: {e}")
        });

        assert_eq!(
            fs::read_to_string(&f.pam_file).unwrap(),
            ARCH_GDM_PASSWORD,
            "line {managed:?} must be removed"
        );
        assert!(
            !pam_backup_path(&f.pam_file).exists(),
            "RemoveRedundant never creates a backup"
        );
        let mut expected = BTreeSet::new();
        for name in [
            "gdm-password",
            "system-auth",
            "system-local-login",
            "system-login",
        ] {
            expected.insert(name.to_string());
        }
        assert_eq!(dir_entries(&f.pam_dir), expected, "no temporary file left");
    }
}

// ----------------------------------------------------------------------------
// IGF17 — only primary pam_soos.so rules qualify
// ----------------------------------------------------------------------------

/// Non-primary `pam_soos.so` rules before the credential module keep the previous
/// unclassified refusal: nothing written, no backup.
#[test]
fn test_igf17_non_primary_soos_rules_keep_the_refusal() {
    let rules = [
        "auth  optional  pam_soos.so event=password-failed timeout_ms=20",
        "auth  [success=done default=ignore]  pam_soos.so event=password-failed",
        "auth  [success=done default=ignore]  pam_soos.so service=sudo",
        "auth  sufficient  pam_soos.so service=sudo",
        "auth  optional  pam_soos.so",
        "auth  required  pam_soos.so",
        "auth  requisite  pam_soos.so",
        "auth  [success=ok default=ignore]  pam_soos.so",
        "auth  [success=done default=die]  pam_soos.so",
        "auth  [success=0 default=ignore]  pam_soos.so",
        "auth  [success=-1 default=ignore]  pam_soos.so",
        "auth  [success= default=ignore]  pam_soos.so",
        "auth  [default=ignore]  pam_soos.so",
        "auth  [success=done success=bad default=ignore]  pam_soos.so",
    ];
    for rule in rules {
        let shared = shared_auth_with(rule);
        let f = fixture(SIMPLE_GDM_PASSWORD, &[("shared-auth", &shared)]);
        write_disable_flag(&f);
        let before = dir_entries(&f.pam_dir);

        let err = enable(&f).expect_err(&format!("rule {rule:?} must not count as shared"));
        let msg = err.to_string();
        assert!(
            msg.contains("unclassified auth rule") && msg.contains("pam_soos.so"),
            "rule {rule:?}: expected the unclassified refusal, got: {msg}"
        );
        assert_eq!(
            fs::read_to_string(&f.pam_file).unwrap(),
            SIMPLE_GDM_PASSWORD,
            "rule {rule:?}: file unchanged"
        );
        assert!(
            !pam_backup_path(&f.pam_file).exists(),
            "rule {rule:?}: no backup"
        );
        assert_eq!(
            dir_entries(&f.pam_dir),
            before,
            "rule {rule:?}: nothing written"
        );
        assert!(f.disable_file.exists(), "rule {rule:?}: disable flag kept");
    }
}

/// Qualifying edge forms of a primary rule: no block is added.
#[test]
fn test_igf17_qualifying_edge_forms_count_as_shared() {
    let rules = [
        "auth  SUFFICIENT  pam_soos.so",
        "AUTH  sufficient  pam_soos.so",
        "auth  [Success=DONE Default=Ignore]  pam_soos.so",
        "auth  [success=done ignore=ignore default=ignore]  pam_soos.so",
        "auth  [success=1 default=ignore]  pam_soos.so timeout_ms=1500",
        "auth  [success=done default=ignore]  /usr/lib/security/pam_soos.so",
        "-auth  [success=done default=ignore]  pam_soos.so",
    ];
    for rule in rules {
        let shared = shared_auth_with(rule);
        let f = fixture(SIMPLE_GDM_PASSWORD, &[("shared-auth", &shared)]);
        let before = dir_entries(&f.pam_dir);

        enable(&f).unwrap_or_else(|e| panic!("rule {rule:?} must count as shared: {e}"));

        assert_eq!(
            fs::read_to_string(&f.pam_file).unwrap(),
            SIMPLE_GDM_PASSWORD,
            "rule {rule:?}: no managed block"
        );
        assert!(
            !pam_backup_path(&f.pam_file).exists(),
            "rule {rule:?}: no backup"
        );
        assert_eq!(
            dir_entries(&f.pam_dir),
            before,
            "rule {rule:?}: nothing written"
        );
    }
}

/// A primary rule placed after the credential module is not reached first: the managed
/// block is still inserted (unchanged behaviour).
#[test]
fn test_igf17_soos_rule_after_credential_module_still_inserts_block() {
    let shared = "\
auth       required                    pam_faillock.so preauth
auth       sufficient                  pam_unix.so nullok
auth       [success=done default=ignore]  pam_soos.so
auth       required                    pam_deny.so
";
    let f = fixture(SIMPLE_GDM_PASSWORD, &[("shared-auth", shared)]);
    enable(&f).expect("enable inserts the block");
    let content = fs::read_to_string(&f.pam_file).unwrap();
    assert!(content.contains(GDM_BLOCK_BEGIN));
    assert!(content.contains(GDM_PAM_LINE));
}

// ----------------------------------------------------------------------------
// IGF18 — error priority
// ----------------------------------------------------------------------------

/// GDM file whose in-file jump crosses the insertion point.
const JUMPING_GDM_PASSWORD: &str = "\
#%PAM-1.0
auth       [success=1 default=ignore]  pam_succeed_if.so user ingroup nopasswdlogin
auth       requisite                   pam_nologin.so
auth       include                     shared-auth
account    include                     shared-auth
";

#[test]
fn test_igf18_jump_crossing_with_shared_rule_is_accepted_without_edit() {
    let shared = shared_auth_with("auth  [success=done default=ignore]  pam_soos.so");
    let f = fixture(JUMPING_GDM_PASSWORD, &[("shared-auth", &shared)]);
    assert_enable_writes_nothing(&f, JUMPING_GDM_PASSWORD);
}

#[test]
fn test_igf18_jump_crossing_without_shared_rule_keeps_jump_error() {
    let shared = shared_auth_with("auth  required  pam_env.so");
    let f = fixture(JUMPING_GDM_PASSWORD, &[("shared-auth", &shared)]);
    let err = enable(&f).expect_err("jump crossing must still be refused");
    assert!(
        err.to_string()
            .contains("a [...=N] jump before the insertion point would change target"),
        "got: {err}"
    );
    assert_eq!(
        fs::read_to_string(&f.pam_file).unwrap(),
        JUMPING_GDM_PASSWORD
    );
    assert!(!pam_backup_path(&f.pam_file).exists());
}

#[test]
fn test_igf18_unclassified_rule_before_shared_rule_is_refused() {
    let shared = "\
auth       required                       pam_mystery.so
auth       [success=done default=ignore]  pam_soos.so
auth       sufficient                     pam_unix.so nullok
";
    let f = fixture(SIMPLE_GDM_PASSWORD, &[("shared-auth", shared)]);
    let err = enable(&f).expect_err("an unclassified rule before the shared rule refuses");
    let msg = err.to_string();
    assert!(
        msg.contains("unclassified auth rule") && msg.contains("pam_mystery.so"),
        "got: {msg}"
    );
    assert_eq!(
        fs::read_to_string(&f.pam_file).unwrap(),
        SIMPLE_GDM_PASSWORD
    );
    assert!(!pam_backup_path(&f.pam_file).exists());
}

#[test]
fn test_igf18_unclassified_rule_in_gdm_file_before_delegation_is_refused() {
    let gdm = "\
#%PAM-1.0
auth       required    pam_mystery.so
auth       include     shared-auth
";
    let shared = shared_auth_with("auth  [success=done default=ignore]  pam_soos.so");
    let f = fixture(gdm, &[("shared-auth", &shared)]);
    let err = enable(&f).expect_err("an unclassified in-file rule refuses");
    assert!(err.to_string().contains("pam_mystery.so"), "got: {err}");
    assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), gdm);
}

#[test]
fn test_igf18_administrator_rule_in_gdm_file_is_untouched_with_shared_rule() {
    let gdm = "\
#%PAM-1.0
auth       requisite   pam_nologin.so
auth       sufficient  pam_soos.so timeout_ms=3000
auth       include     shared-auth
";
    let shared = shared_auth_with("auth  [success=done default=ignore]  pam_soos.so");
    let f = fixture(gdm, &[("shared-auth", &shared)]);
    assert_enable_writes_nothing(&f, gdm);
}

/// A missing delegated stack keeps its refusal even when the edited file has a jump.
#[test]
fn test_igf18_missing_include_still_refused() {
    let f = fixture(SIMPLE_GDM_PASSWORD, &[]);
    let err = enable(&f).expect_err("a missing include refuses");
    assert!(err.to_string().contains("shared-auth"), "got: {err}");
    assert_eq!(
        fs::read_to_string(&f.pam_file).unwrap(),
        SIMPLE_GDM_PASSWORD
    );
    assert!(!pam_backup_path(&f.pam_file).exists());
}

/// Candid review 2026-10-05 (MAJOR): removing a managed block shifts every `[...=N]` jump that
/// crosses it. A jump written by the administrator with the block rules counted (here it lands
/// on the `include`) would land past the delegation once the block is gone, so `enable` must
/// refuse with the existing jump error, leave the GDM file and its backup byte-identical and
/// keep `gdm.disable`.
#[test]
fn test_igf18_block_removal_with_crossing_jump_is_refused() {
    let plain = shared_auth_with("auth       required                    pam_env.so");
    let f = fixture(SIMPLE_GDM_PASSWORD, &[("shared-auth", &plain)]);
    enable(&f).expect("enable on a stack without soos rule inserts the managed block");
    let with_block = fs::read_to_string(&f.pam_file).unwrap();
    assert!(with_block.contains(GDM_BLOCK_BEGIN));

    // The administrator adds a jump over `pam_nologin` and the two managed rules, landing on
    // the `include` line.
    let edited = with_block.replacen(
        "auth       requisite   pam_nologin.so\n",
        "auth       [success=3 default=ignore]  pam_succeed_if.so user ingroup nopasswdlogin\n\
         auth       requisite   pam_nologin.so\n",
        1,
    );
    assert_ne!(edited, with_block, "fixture edit must apply");
    fs::write(&f.pam_file, &edited).unwrap();

    // The delegated stack now carries a primary soos rule.
    let shared = shared_auth_with("auth  [success=done default=ignore]  pam_soos.so");
    fs::write(f.pam_dir.join("shared-auth"), &shared).unwrap();

    let backup = pam_backup_path(&f.pam_file);
    let backup_bytes = fs::read(&backup).unwrap();
    let backup_identity = identity(&backup);
    write_disable_flag(&f);
    let before_entries = dir_entries(&f.pam_dir);
    let before_identity = identity(&f.pam_file);

    let err = enable(&f).expect_err("removing the block would retarget the jump: refuse");
    assert!(
        err.to_string()
            .contains("a [...=N] jump before the insertion point would change target"),
        "got: {err}"
    );
    assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), edited);
    assert_eq!(identity(&f.pam_file), before_identity, "no rewrite");
    assert_eq!(
        fs::read(&backup).unwrap(),
        backup_bytes,
        "backup bytes unchanged"
    );
    assert_eq!(identity(&backup), backup_identity, "backup not rewritten");
    assert_eq!(dir_entries(&f.pam_dir), before_entries, "no temporary file");
    assert!(f.disable_file.exists(), "gdm.disable is kept on refusal");
}
