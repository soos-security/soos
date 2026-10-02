//! Contract tests of GitHub #325 item 3 (matrix PFU3): the account guard and `/etc/pam.conf`.
//!
//! libpam (v1.7.1 `_pam_init_handlers`, `pam.conf(5)`) reads `/etc/pam.conf` only when none of
//! its PAM directories is a directory (`stat`, symlinks followed); builds with the non-default
//! `read-both-confs` option also consult it next to the directories. The guard therefore:
//! - `pam.conf` present and no configured PAM directory is a directory ⇒ `Undeterminable`;
//! - `pam.conf` present next to a PAM directory ⇒ scanned like a stack file (fail closed);
//! - `pam.conf` absent ⇒ unchanged behaviour.
//!
//! Every path lives in a tempdir (`with_pam_conf`, `with_pam_dirs`, ...); nothing under `/etc`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use soos_daemon::presence::account::{
    AccountGuard, AccountRefusal, AccountState, SystemAccountGuard, UserName,
};
use soos_daemon::presence::{DEFAULT_PAM_CONF, MAX_PAM_FILE_BYTES};
use soos_daemon::DaemonError;

const UNDETERMINABLE: AccountState = AccountState::Refused(AccountRefusal::Undeterminable);

fn now() -> Result<u64, DaemonError> {
    Ok(20_000 * 86_400)
}

/// A hermetic account tree: valid shadow line for `alice`, built-in faillock defaults (no
/// `faillock.conf`), an empty safe tally directory, three PAM directories.
struct Tree {
    temp: tempfile::TempDir,
}

impl Tree {
    fn new() -> Self {
        let tree = Self {
            temp: tempfile::tempdir().unwrap(),
        };
        let tally = tree.path("run/faillock");
        fs::create_dir_all(&tally).unwrap();
        fs::set_permissions(&tally, fs::Permissions::from_mode(0o755)).unwrap();
        for dir in tree.pam_dirs() {
            fs::create_dir_all(dir).unwrap();
        }
        fs::create_dir_all(tree.path("etc")).unwrap();
        fs::write(tree.path("etc/shadow"), "alice:$6$h:19900:0:99999:7:::\n").unwrap();
        tree
    }
    fn path(&self, rel: &str) -> PathBuf {
        self.temp.path().join(rel)
    }
    fn pam_dirs(&self) -> Vec<PathBuf> {
        vec![
            self.path("etc/pam.d"),
            self.path("usr/lib/pam.d"),
            self.path("usr/etc/pam.d"),
        ]
    }
    fn pam_conf(&self) -> PathBuf {
        self.path("etc/pam.conf")
    }
    fn remove_all_pam_dirs(&self) {
        for dir in self.pam_dirs() {
            fs::remove_dir_all(dir).unwrap();
        }
    }
    fn guard(&self) -> SystemAccountGuard {
        self.guard_with_dirs(self.pam_dirs())
    }
    fn guard_with_dirs(&self, dirs: Vec<PathBuf>) -> SystemAccountGuard {
        SystemAccountGuard::new()
            .with_faillock_conf(self.path("etc/security/faillock.conf"))
            .with_vendor_faillock_conf(self.path("usr/etc/security/faillock.conf"))
            .with_default_faillock_dir(self.path("run/faillock"))
            .with_pam_dirs(dirs)
            .with_pam_conf(self.pam_conf())
            .with_shadow(self.path("etc/shadow"))
            .with_realtime_fn(now)
    }
    fn check(&self) -> AccountState {
        self.guard().check(&UserName::parse("alice").unwrap(), 1000)
    }
    fn check_with(&self, guard: &SystemAccountGuard) -> AccountState {
        guard.check(&UserName::parse("alice").unwrap(), 1000)
    }
}

const DEBIAN_PAM_CONF: &str =
    "# ---------------------------------------------------------------------------#\n\
# /etc/pam.conf                                                              #\n\
# ---------------------------------------------------------------------------#\n\
#\n\
# NOTE\n\
# ----\n\
#\n\
# NOTE: Most program use a file under the /etc/pam.d/ directory to setup their\n\
# PAM service modules. This file is used only if that directory does not exist.\n\
# ---------------------------------------------------------------------------#\n\
\n\
# Format:\n\
# serv.\tmodule\t   ctrl\t      module [path]\t...[args..]\t\t     #\n\
# name\ttype\t   flag\n";

#[test]
fn test_pfu_default_pam_conf_is_etc_pam_conf() {
    assert_eq!(DEFAULT_PAM_CONF, "/etc/pam.conf");
}

/// PFU3: baseline — no `pam.conf`, option-free PAM directories ⇒ usable.
#[test]
fn test_pfu_absent_pam_conf_keeps_the_account_usable() {
    let tree = Tree::new();
    assert_eq!(tree.check(), AccountState::Usable);
    tree.remove_all_pam_dirs();
    assert_eq!(
        tree.check(),
        AccountState::Usable,
        "no PAM directory and no pam.conf: nothing configures faillock options"
    );
}

/// PFU3: `pam.conf` present while no PAM directory exists ⇒ libpam reads only `pam.conf`,
/// which the guard does not model ⇒ `Undeterminable`, whatever its content.
#[test]
fn test_pfu_pam_conf_without_any_pam_directory_is_undeterminable() {
    for content in ["", DEBIAN_PAM_CONF, "login auth required pam_unix.so\n"] {
        let tree = Tree::new();
        tree.remove_all_pam_dirs();
        fs::write(tree.pam_conf(), content).unwrap();
        assert_eq!(tree.check(), UNDETERMINABLE, "content {content:?}");
    }
    // An empty list of PAM directories means "no PAM directory".
    let tree = Tree::new();
    fs::write(tree.pam_conf(), "").unwrap();
    assert_eq!(
        tree.check_with(&tree.guard_with_dirs(Vec::new())),
        UNDETERMINABLE
    );
}

/// PFU3: PAM paths that exist but are not directories count as absent directories
/// (`stat` + `S_ISDIR`); a `pam.conf` that is a dangling symlink is still "present".
#[test]
fn test_pfu_non_directory_pam_paths_do_not_hide_pam_conf() {
    let tree = Tree::new();
    tree.remove_all_pam_dirs();
    fs::write(tree.pam_conf(), DEBIAN_PAM_CONF).unwrap();
    let file_dir = tree.path("etc/pam.d");
    fs::write(&file_dir, "").unwrap();
    assert_eq!(
        tree.check_with(&tree.guard_with_dirs(vec![file_dir])),
        UNDETERMINABLE,
        "a regular file at the PAM directory path is no PAM directory"
    );

    let tree = Tree::new();
    tree.remove_all_pam_dirs();
    symlink(tree.path("nowhere"), tree.pam_conf()).unwrap();
    assert_eq!(
        tree.check(),
        UNDETERMINABLE,
        "a dangling pam.conf symlink is present (libpam cannot open it)"
    );

    let tree = Tree::new();
    tree.remove_all_pam_dirs();
    symlink(tree.path("no-such-dir"), tree.path("etc/pam.d")).unwrap();
    fs::write(tree.pam_conf(), DEBIAN_PAM_CONF).unwrap();
    assert_eq!(
        tree.check(),
        UNDETERMINABLE,
        "a dangling symlink at the PAM directory path is no PAM directory (stat fails)"
    );
}

/// PFU3: one PAM directory is enough for libpam to ignore `pam.conf` in its default build
/// (the guard then scans `pam.conf` instead of refusing); a symlinked directory counts.
#[test]
fn test_pfu_any_pam_directory_switches_to_the_scan() {
    for keep in 0..3 {
        let tree = Tree::new();
        for (index, dir) in tree.pam_dirs().into_iter().enumerate() {
            if index != keep {
                fs::remove_dir_all(dir).unwrap();
            }
        }
        fs::write(tree.pam_conf(), DEBIAN_PAM_CONF).unwrap();
        assert_eq!(
            tree.check(),
            AccountState::Usable,
            "PAM directory #{keep} present: the comment-only pam.conf is scanned, not refused"
        );
    }
    let tree = Tree::new();
    tree.remove_all_pam_dirs();
    let target = tree.path("real-pam.d");
    fs::create_dir_all(&target).unwrap();
    symlink(&target, tree.path("etc/pam.d")).unwrap();
    fs::write(tree.pam_conf(), DEBIAN_PAM_CONF).unwrap();
    assert_eq!(
        tree.check(),
        AccountState::Usable,
        "a symlink to a directory is a PAM directory (stat follows it)"
    );
}

/// PFU3: next to a PAM directory, `pam.conf` is scanned like a stack file: a
/// `pam_faillock.so` policy option makes the state undeterminable (the leading service field
/// is just one more token); option-free lines stay usable.
#[test]
fn test_pfu_pam_conf_next_to_pam_d_is_scanned_for_faillock_options() {
    for (content, expected) in [
        (
            "login auth required pam_faillock.so preauth deny=3\n",
            UNDETERMINABLE,
        ),
        (
            "sudo auth [default=die] pam_faillock.so authfail [unlock_time=60]\n",
            UNDETERMINABLE,
        ),
        (
            "login auth required pam_faillock.so preauth \\\n    even_deny_root\n",
            UNDETERMINABLE,
        ),
        (
            "login auth required pam_faillock.so preauth silent\n",
            AccountState::Usable,
        ),
        (
            "# login auth required pam_faillock.so deny=3\n",
            AccountState::Usable,
        ),
        (DEBIAN_PAM_CONF, AccountState::Usable),
        ("", AccountState::Usable),
    ] {
        let tree = Tree::new();
        fs::write(tree.pam_conf(), content).unwrap();
        assert_eq!(tree.check(), expected, "pam.conf {content:?}");
    }
}

/// PFU3: next to a PAM directory, a `pam.conf` that cannot be scanned is undeterminable:
/// directory, FIFO, oversized, non-UTF-8, unreadable target.
#[test]
fn test_pfu_unscannable_pam_conf_is_undeterminable() {
    let tree = Tree::new();
    fs::create_dir_all(tree.pam_conf()).unwrap();
    assert_eq!(tree.check(), UNDETERMINABLE, "pam.conf is a directory");

    let tree = Tree::new();
    nix::unistd::mkfifo(
        &tree.pam_conf(),
        nix::sys::stat::Mode::from_bits_truncate(0o600),
    )
    .unwrap();
    assert_eq!(tree.check(), UNDETERMINABLE, "pam.conf is a FIFO");

    let tree = Tree::new();
    fs::write(tree.pam_conf(), vec![b'#'; MAX_PAM_FILE_BYTES + 1]).unwrap();
    assert_eq!(tree.check(), UNDETERMINABLE, "pam.conf is oversized");

    let tree = Tree::new();
    fs::write(tree.pam_conf(), b"login auth required pam_unix.so \xff\n").unwrap();
    assert_eq!(tree.check(), UNDETERMINABLE, "pam.conf is not UTF-8");

    let tree = Tree::new();
    symlink(tree.path("nowhere"), tree.pam_conf()).unwrap();
    assert_eq!(
        tree.check(),
        UNDETERMINABLE,
        "pam.conf is a dangling symlink"
    );
}

/// PFU3 (auditor, optional case): a `pam.conf` path that cannot be examined (its parent is a
/// regular file, `ENOTDIR`) is undeterminable, never "absent".
#[test]
fn test_pfu_pam_conf_path_under_a_regular_file_is_undeterminable() {
    let tree = Tree::new();
    let parent = tree.path("etc/not-a-dir");
    fs::write(&parent, "").unwrap();
    let guard = tree.guard().with_pam_conf(parent.join("pam.conf"));
    assert_eq!(tree.check_with(&guard), UNDETERMINABLE);
}

/// PFU3: a symlinked `pam.conf` is followed and scanned (like PAM stack files).
#[test]
fn test_pfu_symlinked_pam_conf_is_followed() {
    let tree = Tree::new();
    let target = tree.path("pam.conf.real");
    fs::write(
        &target,
        "login auth required pam_faillock.so fail_interval=60\n",
    )
    .unwrap();
    symlink(&target, tree.pam_conf()).unwrap();
    assert_eq!(tree.check(), UNDETERMINABLE);
    fs::write(&target, DEBIAN_PAM_CONF).unwrap();
    assert_eq!(tree.check(), AccountState::Usable);
}

/// PFU3: UID 0 is still refused first (no file is consulted for root).
#[test]
fn test_pfu_root_is_refused_before_pam_conf() {
    let tree = Tree::new();
    tree.remove_all_pam_dirs();
    fs::write(tree.pam_conf(), "").unwrap();
    assert_eq!(
        tree.guard().check(&UserName::parse("root").unwrap(), 0),
        AccountState::Refused(AccountRefusal::RootAccount)
    );
}

/// PFU3: the guard never writes `pam.conf` (content and mode unchanged after checks).
#[test]
fn test_pfu_guard_never_writes_pam_conf() {
    let tree = Tree::new();
    fs::write(tree.pam_conf(), DEBIAN_PAM_CONF).unwrap();
    fs::set_permissions(tree.pam_conf(), fs::Permissions::from_mode(0o644)).unwrap();
    for _ in 0..3 {
        let _ = tree.check();
    }
    assert_eq!(
        fs::read_to_string(tree.pam_conf()).unwrap(),
        DEBIAN_PAM_CONF
    );
    let mode = fs::metadata(tree.pam_conf()).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o644);
    assert!(Path::new(&tree.pam_conf()).is_file());
}
