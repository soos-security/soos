//! Contract tests of `soos-admin network-lock install|uninstall` against a staging root
//! (GitHub #365, spec `AI/architect_spec_network_lock.md` §5; matrix NLK2, NLK3).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use clap::Parser;
use soos_admin_cli::args::{Cli, Commands, NetworkLockAction};
use soos_admin_cli::network_lock::{
    install_network_lock, may_modify_root, network_lock_status, uninstall_network_lock, FileAction,
    InstallOptions, NetworkLockEnv, NetworkLockState, NetworkServiceProbe, RuleFileState,
    RunningServices, NETWORK_LOCK_RFKILL_DIR, NETWORK_LOCK_RFKILL_FILE, NETWORK_LOCK_RFKILL_RULE,
    NETWORK_LOCK_RULES, NETWORK_LOCK_RULES_DIR, NETWORK_LOCK_RULES_FILE,
};

const PLAIN: InstallOptions = InstallOptions { rfkill: false };
const WITH_RFKILL: InstallOptions = InstallOptions { rfkill: true };

struct NmProbe;

impl NetworkServiceProbe for NmProbe {
    fn running(&self) -> RunningServices {
        RunningServices {
            network_manager: true,
            ..RunningServices::default()
        }
    }
}

struct Root {
    dir: tempfile::TempDir,
}

impl Root {
    /// A staging root with both target directories (0755).
    fn new() -> Self {
        let root = Self::bare();
        root.mkdir(NETWORK_LOCK_RULES_DIR);
        root.mkdir(NETWORK_LOCK_RFKILL_DIR);
        root
    }

    fn bare() -> Self {
        Self {
            dir: tempfile::tempdir().expect("tempdir"),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn join(&self, rel: &str) -> PathBuf {
        self.path().join(rel)
    }

    /// Creates `rel` and every missing component as 0755 (independent of the umask).
    fn mkdir(&self, rel: &str) {
        let mut p = self.path().to_path_buf();
        for part in Path::new(rel).components() {
            p.push(part);
            if !p.exists() {
                fs::create_dir(&p).expect("mkdir");
                fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).expect("chmod");
            }
        }
    }

    fn rules(&self) -> PathBuf {
        self.join(NETWORK_LOCK_RULES_DIR)
            .join(NETWORK_LOCK_RULES_FILE)
    }

    fn rfkill_rule(&self) -> PathBuf {
        self.join(NETWORK_LOCK_RFKILL_DIR)
            .join(NETWORK_LOCK_RFKILL_FILE)
    }

    fn entries(&self, rel: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.join(rel))
            .expect("read dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o7777
}

fn exists_no_follow(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

// ---------------------------------------------------------------------------------------
// NLK2 — install writes the shipped file root-style 0644, atomically, refusing symlinks
// ---------------------------------------------------------------------------------------

#[test]
fn test_nlk_install_writes_identical_0644_rule_without_temp_file() {
    let root = Root::new();
    let report = install_network_lock(root.path(), PLAIN, false).expect("install");
    assert_eq!(report.rules, FileAction::Written);
    assert_eq!(report.rfkill_rule, FileAction::Absent);
    assert_eq!(
        fs::read_to_string(root.rules()).expect("read"),
        NETWORK_LOCK_RULES
    );
    assert_eq!(mode(&root.rules()), 0o644);
    assert_eq!(
        root.entries(NETWORK_LOCK_RULES_DIR),
        vec![NETWORK_LOCK_RULES_FILE.to_string()],
        "no temporary file is left behind"
    );
    assert!(
        !root.rfkill_rule().exists(),
        "rfkill rule is off by default"
    );

    let env = NetworkLockEnv {
        root: root.path().to_path_buf(),
        required_owner_uid: nix::unistd::geteuid().as_raw(),
    };
    let report = network_lock_status(&env, &NmProbe);
    assert_eq!(report.state, NetworkLockState::Installed);
    assert_eq!(report.rfkill_rule, RuleFileState::Missing);
}

#[test]
fn test_nlk_install_replaces_a_modified_rule() {
    let root = Root::new();
    fs::write(
        root.rules(),
        "polkit.addRule(function () { return polkit.Result.YES; });\n",
    )
    .expect("write");
    fs::set_permissions(root.rules(), fs::Permissions::from_mode(0o666)).expect("chmod");
    install_network_lock(root.path(), PLAIN, false).expect("install");
    assert_eq!(
        fs::read_to_string(root.rules()).expect("read"),
        NETWORK_LOCK_RULES
    );
    assert_eq!(mode(&root.rules()), 0o644);
}

#[test]
fn test_nlk_install_refuses_a_symlink_target() {
    let root = Root::new();
    let outside = root.join("outside.txt");
    fs::write(&outside, "keep me").expect("write");
    symlink(&outside, root.rules()).expect("symlink");
    let err = install_network_lock(root.path(), PLAIN, false);
    assert!(err.is_err(), "a symlink at the target name is refused");
    assert_eq!(fs::read_to_string(&outside).expect("read"), "keep me");
    assert!(fs::symlink_metadata(root.rules())
        .expect("still there")
        .file_type()
        .is_symlink());
    assert_eq!(
        root.entries(NETWORK_LOCK_RULES_DIR),
        vec![NETWORK_LOCK_RULES_FILE.to_string()],
        "no temporary file is left behind"
    );
}

#[test]
fn test_nlk_install_refuses_a_dangling_symlink_target() {
    let root = Root::new();
    let outside = root.join("would-be-created.txt");
    symlink(&outside, root.rules()).expect("symlink");
    assert!(install_network_lock(root.path(), PLAIN, false).is_err());
    assert!(!outside.exists(), "nothing is created through the link");
}

#[test]
fn test_nlk_install_refuses_a_symlinked_rules_dir() {
    let root = Root::new();
    let real = root.join("elsewhere");
    fs::create_dir(&real).expect("mkdir");
    fs::remove_dir(root.join(NETWORK_LOCK_RULES_DIR)).expect("rmdir");
    symlink(&real, root.join(NETWORK_LOCK_RULES_DIR)).expect("symlink");
    assert!(install_network_lock(root.path(), PLAIN, false).is_err());
    assert!(
        fs::read_dir(&real).expect("read").next().is_none(),
        "nothing written through a symlinked directory"
    );
}

#[test]
fn test_nlk_install_refuses_a_symlinked_ancestor_dir() {
    let root = Root::bare();
    let real = root.join("elsewhere");
    fs::create_dir_all(real.join("rules.d")).expect("mkdir");
    root.mkdir("etc");
    symlink(&real, root.join("etc/polkit-1")).expect("symlink");
    root.mkdir(NETWORK_LOCK_RFKILL_DIR);
    assert!(install_network_lock(root.path(), PLAIN, false).is_err());
    assert!(
        fs::read_dir(real.join("rules.d"))
            .expect("read")
            .next()
            .is_none(),
        "nothing written through a symlinked ancestor"
    );
}

#[test]
fn test_nlk_install_refuses_a_missing_rules_dir() {
    let root = Root::bare();
    root.mkdir(NETWORK_LOCK_RFKILL_DIR);
    assert!(install_network_lock(root.path(), PLAIN, false).is_err());
    assert!(
        !root.join("etc/polkit-1").exists(),
        "the installer never creates the polkit directory with guessed permissions"
    );
}

#[test]
fn test_nlk_install_refuses_a_directory_at_the_target_name() {
    let root = Root::new();
    fs::create_dir(root.rules()).expect("mkdir");
    assert!(install_network_lock(root.path(), PLAIN, false).is_err());
    assert!(root.rules().is_dir());
}

#[test]
fn test_nlk_install_rfkill_writes_the_udev_rule() {
    let root = Root::new();
    let report = install_network_lock(root.path(), WITH_RFKILL, false).expect("install");
    assert_eq!(report.rules, FileAction::Written);
    assert_eq!(report.rfkill_rule, FileAction::Written);
    assert_eq!(
        fs::read_to_string(root.rfkill_rule()).expect("read"),
        NETWORK_LOCK_RFKILL_RULE
    );
    assert_eq!(mode(&root.rfkill_rule()), 0o644);
    assert_eq!(
        root.entries(NETWORK_LOCK_RFKILL_DIR),
        vec![NETWORK_LOCK_RFKILL_FILE.to_string()]
    );
}

#[test]
fn test_nlk_install_rfkill_with_missing_udev_dir_writes_nothing() {
    let root = Root::bare();
    root.mkdir(NETWORK_LOCK_RULES_DIR);
    assert!(install_network_lock(root.path(), WITH_RFKILL, false).is_err());
    assert!(
        !exists_no_follow(&root.rules()),
        "every target directory is validated before the first write"
    );
}

#[test]
fn test_nlk_install_without_rfkill_removes_only_the_soos_rfkill_rule() {
    let root = Root::new();
    let other = root.join(NETWORK_LOCK_RFKILL_DIR).join("70-local.rules");
    fs::write(&other, "# local\n").expect("write");
    install_network_lock(root.path(), WITH_RFKILL, false).expect("install rfkill");
    let report = install_network_lock(root.path(), PLAIN, false).expect("install plain");
    assert_eq!(report.rfkill_rule, FileAction::Removed);
    assert!(!exists_no_follow(&root.rfkill_rule()));
    assert_eq!(fs::read_to_string(&other).expect("read"), "# local\n");
    assert_eq!(
        fs::read_to_string(root.rules()).expect("read"),
        NETWORK_LOCK_RULES
    );
}

#[test]
fn test_nlk_install_without_rfkill_needs_no_udev_dir() {
    let root = Root::bare();
    root.mkdir(NETWORK_LOCK_RULES_DIR);
    let report = install_network_lock(root.path(), PLAIN, false).expect("install");
    assert_eq!(report.rfkill_rule, FileAction::Absent);
}

// ---------------------------------------------------------------------------------------
// NLK3 — uninstall
// ---------------------------------------------------------------------------------------

#[test]
fn test_nlk_uninstall_removes_both_files_and_is_idempotent() {
    let root = Root::new();
    install_network_lock(root.path(), WITH_RFKILL, false).expect("install");
    let report = uninstall_network_lock(root.path()).expect("uninstall");
    assert_eq!(report.rules, FileAction::Removed);
    assert_eq!(report.rfkill_rule, FileAction::Removed);
    assert!(!exists_no_follow(&root.rules()));
    assert!(!exists_no_follow(&root.rfkill_rule()));
    assert!(
        root.join(NETWORK_LOCK_RULES_DIR).is_dir(),
        "directories are kept"
    );

    let report = uninstall_network_lock(root.path()).expect("second uninstall");
    assert_eq!(report.rules, FileAction::Absent);
    assert_eq!(report.rfkill_rule, FileAction::Absent);
}

#[test]
fn test_nlk_uninstall_on_an_empty_root_is_ok() {
    let root = Root::bare();
    let report = uninstall_network_lock(root.path()).expect("uninstall");
    assert_eq!(report.rules, FileAction::Absent);
    assert_eq!(report.rfkill_rule, FileAction::Absent);
}

#[test]
fn test_nlk_uninstall_unlinks_a_symlink_without_touching_its_target() {
    let root = Root::new();
    let outside = root.join("outside.txt");
    fs::write(&outside, "keep me").expect("write");
    symlink(&outside, root.rules()).expect("symlink");
    let report = uninstall_network_lock(root.path()).expect("uninstall");
    assert_eq!(report.rules, FileAction::Removed);
    assert!(!exists_no_follow(&root.rules()));
    assert_eq!(fs::read_to_string(&outside).expect("read"), "keep me");
}

#[test]
fn test_nlk_uninstall_refuses_a_symlinked_rules_dir() {
    let root = Root::new();
    let real = root.join("elsewhere");
    fs::create_dir(&real).expect("mkdir");
    fs::write(real.join(NETWORK_LOCK_RULES_FILE), "foreign").expect("write");
    fs::remove_dir(root.join(NETWORK_LOCK_RULES_DIR)).expect("rmdir");
    symlink(&real, root.join(NETWORK_LOCK_RULES_DIR)).expect("symlink");
    assert!(uninstall_network_lock(root.path()).is_err());
    assert!(real.join(NETWORK_LOCK_RULES_FILE).exists());
}

#[test]
fn test_nlk_uninstall_refuses_a_directory_at_the_target_name() {
    let root = Root::new();
    fs::create_dir(root.rules()).expect("mkdir");
    assert!(uninstall_network_lock(root.path()).is_err());
    assert!(root.rules().is_dir());
}

// ---------------------------------------------------------------------------------------
// Root guard and CLI surface
// ---------------------------------------------------------------------------------------

#[test]
fn test_nlk_may_modify_root() {
    assert!(may_modify_root(Path::new("/"), 0));
    assert!(!may_modify_root(Path::new("/"), 1000));
    assert!(!may_modify_root(Path::new("/."), 1000));
    assert!(!may_modify_root(Path::new("//"), 1000));
    assert!(may_modify_root(Path::new("/tmp/staging"), 1000));
    assert!(may_modify_root(Path::new("/tmp/staging"), 0));
    assert!(
        !may_modify_root(Path::new(""), 0),
        "an empty root is refused"
    );
    assert!(!may_modify_root(Path::new(""), 1000));
}

#[test]
fn test_nlk_cli_parses_network_lock_actions() {
    let cli = Cli::try_parse_from(["soos-admin", "network-lock", "status"]).expect("parse");
    match cli.command {
        Commands::NetworkLock(args) => assert_eq!(
            args.action,
            NetworkLockAction::Status {
                root: PathBuf::from("/")
            }
        ),
        _ => panic!("expected network-lock"),
    }

    let cli = Cli::try_parse_from([
        "soos-admin",
        "network-lock",
        "install",
        "--rfkill",
        "--root",
        "/tmp/stage",
    ])
    .expect("parse");
    match cli.command {
        Commands::NetworkLock(args) => assert_eq!(
            args.action,
            NetworkLockAction::Install {
                rfkill: true,
                root: PathBuf::from("/tmp/stage")
            }
        ),
        _ => panic!("expected network-lock"),
    }

    let cli = Cli::try_parse_from(["soos-admin", "network-lock", "install"]).expect("parse");
    match cli.command {
        Commands::NetworkLock(args) => assert_eq!(
            args.action,
            NetworkLockAction::Install {
                rfkill: false,
                root: PathBuf::from("/")
            }
        ),
        _ => panic!("expected network-lock"),
    }

    let cli = Cli::try_parse_from(["soos-admin", "network-lock", "uninstall"]).expect("parse");
    match cli.command {
        Commands::NetworkLock(args) => assert_eq!(
            args.action,
            NetworkLockAction::Uninstall {
                root: PathBuf::from("/")
            }
        ),
        _ => panic!("expected network-lock"),
    }

    assert!(Cli::try_parse_from(["soos-admin", "network-lock"]).is_err());
    assert!(Cli::try_parse_from(["soos-admin", "network-lock", "enable"]).is_err());
}

// ---------------------------------------------------------------------------------------
// Plan evaluation round 1: F-6 directory ownership and mode, F-7 atomic replacement
// ---------------------------------------------------------------------------------------

#[test]
fn test_nlk_install_replaces_a_hard_linked_target_with_a_new_inode() {
    use std::os::unix::fs::MetadataExt;
    let root = Root::new();
    let outside = root.join("outside.txt");
    fs::write(&outside, "keep me").expect("write");
    fs::hard_link(&outside, root.rules()).expect("hard link");
    let before = fs::metadata(&outside).expect("metadata").ino();
    install_network_lock(root.path(), PLAIN, false).expect("install");
    assert_eq!(
        fs::read_to_string(&outside).expect("read"),
        "keep me",
        "the file is replaced by rename, never written in place"
    );
    assert_ne!(fs::metadata(root.rules()).expect("metadata").ino(), before);
    assert_eq!(
        fs::read_to_string(root.rules()).expect("read"),
        NETWORK_LOCK_RULES
    );
}

#[test]
fn test_nlk_install_fails_cleanly_on_a_stale_temp_name() {
    let root = Root::new();
    fs::write(root.rules(), "old").expect("write");
    let stale = root.join(NETWORK_LOCK_RULES_DIR).join(format!(
        ".{NETWORK_LOCK_RULES_FILE}.{}.tmp",
        std::process::id()
    ));
    fs::write(&stale, "stale").expect("write");
    assert!(install_network_lock(root.path(), PLAIN, false).is_err());
    assert_eq!(fs::read_to_string(root.rules()).expect("read"), "old");
    assert_eq!(
        fs::read_to_string(&stale).expect("read"),
        "stale",
        "a file the installer did not create is never removed"
    );
}

#[test]
fn test_nlk_install_refuses_group_or_world_writable_directories() {
    for (dir, mode) in [
        (NETWORK_LOCK_RULES_DIR, 0o777),
        (NETWORK_LOCK_RULES_DIR, 0o775),
        ("etc/polkit-1", 0o757),
        ("etc", 0o777),
    ] {
        let root = Root::new();
        fs::set_permissions(root.join(dir), fs::Permissions::from_mode(mode)).expect("chmod");
        assert!(
            install_network_lock(root.path(), PLAIN, false).is_err(),
            "{dir} {mode:o}"
        );
        assert!(
            !exists_no_follow(&root.rules()),
            "{dir} {mode:o}: nothing written"
        );
    }
}

#[test]
fn test_nlk_uninstall_refuses_group_or_world_writable_directories() {
    let root = Root::new();
    install_network_lock(root.path(), PLAIN, false).expect("install");
    fs::set_permissions(
        root.join(NETWORK_LOCK_RULES_DIR),
        fs::Permissions::from_mode(0o777),
    )
    .expect("chmod");
    assert!(uninstall_network_lock(root.path()).is_err());
    assert!(exists_no_follow(&root.rules()));
}

#[test]
fn test_nlk_install_as_root_requires_root_owned_directories() {
    if nix::unistd::geteuid().is_root() {
        return;
    }
    // chown_root = true is the production path: every directory must belong to uid 0.
    let root = Root::new();
    assert!(install_network_lock(root.path(), PLAIN, true).is_err());
    assert!(!exists_no_follow(&root.rules()));
}
