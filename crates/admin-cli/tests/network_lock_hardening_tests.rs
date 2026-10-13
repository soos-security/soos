//! Contract tests of the network lock hardening of GitHub #370 (spec
//! `AI/architect_spec_network_lock.md` §12; matrix NLK13–NLK17):
//!
//! - G-1: the backend probe credits a service only when every uid of the matched process is
//!   trusted (root, or the `systemd-network` account for networkd);
//! - G-2: `status` checks every directory on the path, like `install`;
//! - G-5: install and uninstall remove stale soos temp files left by a dead process.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use soos_admin_cli::network_lock::{
    install_network_lock, network_lock_status, uninstall_network_lock, InstallOptions,
    NetworkBackend, NetworkLockEnv, NetworkLockReport, NetworkLockState, NetworkServiceProbe,
    ProcServiceProbe, RuleFileState, RunningServices, MAX_PASSWD_BYTES, MAX_PROC_STATUS_BYTES,
    NETWORK_LOCK_RFKILL_DIR, NETWORK_LOCK_RFKILL_FILE, NETWORK_LOCK_RFKILL_RULE,
    NETWORK_LOCK_RULES, NETWORK_LOCK_RULES_DIR, NETWORK_LOCK_RULES_FILE, SYSTEMD_NETWORK_USER,
};

const NM: RunningServices = RunningServices {
    network_manager: true,
    networkd: false,
    iwd: false,
};

struct FakeProbe(RunningServices);

impl NetworkServiceProbe for FakeProbe {
    fn running(&self) -> RunningServices {
        self.0
    }
}

fn euid() -> u32 {
    nix::unistd::geteuid().as_raw()
}

/// The uid used for the fake `systemd-network` account (not Arch's real 192, so a lookup of
/// the host account instead of the fixture `etc/passwd` is caught).
const NETWORKD_UID: u32 = 61_234;
/// A regular user.
const USER_UID: u32 = 1000;

/// A staging root with both rule directories, every component 0755.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fx = Self {
            dir: tempfile::tempdir().expect("tempdir"),
        };
        fx.mkdir(NETWORK_LOCK_RULES_DIR);
        fx.mkdir(NETWORK_LOCK_RFKILL_DIR);
        fx
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root().join(rel)
    }

    fn mkdir(&self, rel: &str) {
        let mut p = self.root().to_path_buf();
        for part in Path::new(rel).components() {
            p.push(part);
            if !p.exists() {
                fs::create_dir(&p).expect("mkdir");
                fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).expect("chmod");
            }
        }
    }

    fn chmod(&self, rel: &str, mode: u32) {
        fs::set_permissions(self.path(rel), fs::Permissions::from_mode(mode)).expect("chmod");
    }

    fn write(&self, path: &Path, content: &[u8], mode: u32) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir parent");
        }
        fs::write(path, content).expect("write");
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
    }

    fn rules_path(&self) -> PathBuf {
        self.path(NETWORK_LOCK_RULES_DIR)
            .join(NETWORK_LOCK_RULES_FILE)
    }

    fn rfkill_rule_path(&self) -> PathBuf {
        self.path(NETWORK_LOCK_RFKILL_DIR)
            .join(NETWORK_LOCK_RFKILL_FILE)
    }

    fn install_rules(&self) {
        self.write(&self.rules_path(), NETWORK_LOCK_RULES.as_bytes(), 0o644);
    }

    fn env(&self) -> NetworkLockEnv {
        NetworkLockEnv {
            root: self.root().to_path_buf(),
            required_owner_uid: euid(),
        }
    }

    fn status(&self, services: RunningServices) -> NetworkLockReport {
        network_lock_status(&self.env(), &FakeProbe(services))
    }

    /// A fake process: `proc/<pid>/comm` and, when given, `proc/<pid>/status`.
    fn process(&self, pid: u32, comm: &str, status: Option<&str>) {
        self.write(
            &self.path(&format!("proc/{pid}/comm")),
            format!("{comm}\n").as_bytes(),
            0o444,
        );
        if let Some(status) = status {
            self.write(
                &self.path(&format!("proc/{pid}/status")),
                status.as_bytes(),
                0o444,
            );
        }
    }

    fn passwd(&self, content: &str) {
        self.write(&self.path("etc/passwd"), content.as_bytes(), 0o644);
    }

    fn probe(&self) -> RunningServices {
        ProcServiceProbe {
            root: self.root().to_path_buf(),
        }
        .running()
    }
}

/// A `/proc/<pid>/status` prefix as the kernel writes it, with the four uids.
fn status_with_uids(name: &str, uids: [u32; 4]) -> String {
    format!(
        "Name:\t{name}\nUmask:\t0022\nState:\tS (sleeping)\nTgid:\t4242\nNgid:\t0\n\
         Pid:\t4242\nPPid:\t1\nTracerPid:\t0\nUid:\t{}\t{}\t{}\t{}\nGid:\t0\t0\t0\t0\n\
         FDSize:\t64\nGroups:\t\n",
        uids[0], uids[1], uids[2], uids[3]
    )
}

fn root_status(name: &str) -> String {
    status_with_uids(name, [0; 4])
}

fn passwd_with_systemd_network() -> String {
    format!(
        "root:x:0:0::/root:/bin/bash\n\
         {SYSTEMD_NETWORK_USER}:x:{NETWORKD_UID}:{NETWORKD_UID}:systemd Network Management:/:/usr/bin/nologin\n\
         alice:x:{USER_UID}:{USER_UID}::/home/alice:/bin/bash\n"
    )
}

// ---------------------------------------------------------------------------------------
// NLK13 — G-1: the backend probe trusts only system-owned processes
// ---------------------------------------------------------------------------------------

#[test]
fn test_nlk_proc_probe_ignores_user_owned_services() {
    let fx = Fixture::new();
    fx.passwd(&passwd_with_systemd_network());
    fx.process(
        100,
        "NetworkManager",
        Some(&status_with_uids("NetworkManager", [USER_UID; 4])),
    );
    fx.process(101, "iwd", Some(&status_with_uids("iwd", [USER_UID; 4])));
    fx.process(
        102,
        "systemd-network",
        Some(&status_with_uids("systemd-network", [USER_UID; 4])),
    );
    assert_eq!(
        fx.probe(),
        RunningServices::default(),
        "a process any user can name is never credited"
    );

    // Positive control: the same tree with root-owned processes is detected.
    let fx = Fixture::new();
    fx.process(100, "NetworkManager", Some(&root_status("NetworkManager")));
    fx.process(101, "iwd", Some(&root_status("iwd")));
    fx.process(
        102,
        "systemd-network",
        Some(&root_status("systemd-network")),
    );
    assert_eq!(
        fx.probe(),
        RunningServices {
            network_manager: true,
            networkd: true,
            iwd: true,
        }
    );
}

#[test]
fn test_nlk_proc_probe_requires_all_four_uids_trusted() {
    // A setuid-root binary started by a user through a symlink named `NetworkManager` has
    // effective uid 0 but the user's real uid; any untrusted uid position rejects it.
    for position in 0..4 {
        let mut uids = [0; 4];
        uids[position] = USER_UID;
        let fx = Fixture::new();
        fx.process(
            100,
            "NetworkManager",
            Some(&status_with_uids("NetworkManager", uids)),
        );
        assert_eq!(
            fx.probe(),
            RunningServices::default(),
            "untrusted uid at position {position}: {uids:?}"
        );
    }
}

#[test]
fn test_nlk_proc_probe_ignores_missing_or_malformed_status() {
    for status in [
        None,
        Some(""),
        Some("Name:\tiwd\nGid:\t0\t0\t0\t0\n"),
        Some("Name:\tiwd\nUid:\t0\t0\t0\n"),
        Some("Name:\tiwd\nUid:\t0\t0\t0\t0\t0\n"),
        Some("Name:\tiwd\nUid:\t0\t0\tx\t0\n"),
        Some("Name:\tiwd\nUid:\t-1\t0\t0\t0\n"),
        Some("Name:\tiwd\nUid:\t4294967296\t0\t0\t0\n"),
        Some("Name:\tiwd\nXUid:\t0\t0\t0\t0\n"),
    ] {
        let fx = Fixture::new();
        fx.process(100, "iwd", status);
        assert_eq!(
            fx.probe(),
            RunningServices::default(),
            "status {status:?} must not credit iwd"
        );
    }
}

#[test]
fn test_nlk_proc_probe_trusts_systemd_network_only_for_networkd() {
    let networkd = |status: String| {
        let fx = Fixture::new();
        fx.passwd(&passwd_with_systemd_network());
        fx.process(4242, "systemd-network", Some(&status));
        fx.probe()
    };
    assert_eq!(
        networkd(status_with_uids("systemd-network", [NETWORKD_UID; 4])),
        RunningServices {
            networkd: true,
            ..RunningServices::default()
        },
        "systemd-networkd runs as the systemd-network account"
    );
    assert_eq!(
        networkd(status_with_uids("systemd-network", [USER_UID; 4])),
        RunningServices::default()
    );
    assert_eq!(
        networkd(status_with_uids(
            "systemd-network",
            [NETWORKD_UID, NETWORKD_UID, NETWORKD_UID, USER_UID]
        )),
        RunningServices::default()
    );

    // The systemd-network account does not make NetworkManager or iwd trusted.
    let fx = Fixture::new();
    fx.passwd(&passwd_with_systemd_network());
    fx.process(
        100,
        "NetworkManager",
        Some(&status_with_uids("NetworkManager", [NETWORKD_UID; 4])),
    );
    fx.process(
        101,
        "iwd",
        Some(&status_with_uids("iwd", [NETWORKD_UID; 4])),
    );
    assert_eq!(fx.probe(), RunningServices::default());

    // Without a systemd-network entry (or with a malformed one), only root is trusted.
    for passwd in [
        None,
        Some(format!(
            "alice:x:{NETWORKD_UID}:{NETWORKD_UID}::/:/bin/sh\n"
        )),
        Some(format!(
            "{SYSTEMD_NETWORK_USER}x:x:{NETWORKD_UID}:{NETWORKD_UID}::/:/bin/sh\n"
        )),
        Some(format!(
            "{SYSTEMD_NETWORK_USER}:x:abc:{NETWORKD_UID}::/:/bin/sh\n"
        )),
    ] {
        let fx = Fixture::new();
        if let Some(passwd) = &passwd {
            fx.passwd(passwd);
        }
        fx.process(
            4242,
            "systemd-network",
            Some(&status_with_uids("systemd-network", [NETWORKD_UID; 4])),
        );
        assert_eq!(fx.probe(), RunningServices::default(), "passwd {passwd:?}");
        fx.process(
            4243,
            "systemd-network",
            Some(&root_status("systemd-network")),
        );
        assert!(fx.probe().networkd, "root-owned networkd is trusted");
    }
}

#[test]
fn test_nlk_proc_probe_ignores_uid_line_beyond_the_read_cap() {
    let fx = Fixture::new();
    let mut status = String::from("Name:\tiwd\n");
    while status.len() as u64 <= MAX_PROC_STATUS_BYTES {
        status.push_str("Padding:\t0\n");
    }
    status.push_str("Uid:\t0\t0\t0\t0\n");
    fx.process(100, "iwd", Some(&status));
    assert_eq!(fx.probe(), RunningServices::default());

    // Positive control: a long status whose Uid line sits within the cap.
    let fx = Fixture::new();
    let mut status = root_status("iwd");
    while status.len() as u64 <= 2 * MAX_PROC_STATUS_BYTES {
        status.push_str("Padding:\t0\n");
    }
    fx.process(100, "iwd", Some(&status));
    assert!(
        fx.probe().iwd,
        "only the prefix is examined, a long status is fine"
    );
}

#[test]
fn test_nlk_proc_probe_requires_the_status_name_to_match_comm() {
    // `comm` and `status` are read separately: a pid reused in between must not pair the
    // name of one process with the uids of another.
    let fx = Fixture::new();
    fx.process(100, "NetworkManager", Some(&root_status("bash")));
    fx.process(101, "iwd", Some(&root_status("iwdx")));
    fx.process(
        102,
        "systemd-network",
        Some(&root_status("systemd-resolve")),
    );
    assert_eq!(fx.probe(), RunningServices::default());

    let fx = Fixture::new();
    fx.process(100, "NetworkManager", Some(&root_status("NetworkManager")));
    assert!(fx.probe().network_manager, "positive control");
}

#[test]
fn test_nlk_proc_probe_ignores_systemd_network_entry_cut_by_the_read_cap() {
    // The cap falls inside `...:x:61234:...` right after `612`: a reader that parses the cut
    // line would trust uid 612.
    let entry =
        format!("{SYSTEMD_NETWORK_USER}:x:{NETWORKD_UID}:{NETWORKD_UID}::/:/usr/bin/nologin\n");
    let head = format!("{SYSTEMD_NETWORK_USER}:x:612");
    let mut passwd = String::new();
    let filler = "user:x:5000:5000::/home/user:/bin/sh\n";
    let target = MAX_PASSWD_BYTES as usize - head.len();
    while passwd.len() + filler.len() <= target {
        passwd.push_str(filler);
    }
    while passwd.len() < target - 1 {
        passwd.push('#');
    }
    passwd.push('\n');
    assert_eq!(passwd.len() + head.len(), MAX_PASSWD_BYTES as usize);
    passwd.push_str(&entry);

    for uid in [612, NETWORKD_UID] {
        let fx = Fixture::new();
        fx.passwd(&passwd);
        fx.process(
            4242,
            "systemd-network",
            Some(&status_with_uids("systemd-network", [uid; 4])),
        );
        assert_eq!(
            fx.probe(),
            RunningServices::default(),
            "uid {uid}: an entry cut by the cap is not resolved"
        );
    }

    // Positive control: the same entry inside the cap resolves.
    let fx = Fixture::new();
    fx.passwd(&format!("{filler}{entry}"));
    fx.process(
        4242,
        "systemd-network",
        Some(&status_with_uids("systemd-network", [NETWORKD_UID; 4])),
    );
    assert!(fx.probe().networkd);
}

#[test]
fn test_nlk_status_spoofed_network_manager_keeps_unsupported_backend() {
    // The #370 scenario: an iwd-only host where a user names a process `NetworkManager`.
    let fx = Fixture::new();
    fx.install_rules();
    fx.process(100, "iwd", Some(&root_status("iwd")));
    fx.process(
        31337,
        "NetworkManager",
        Some(&status_with_uids("NetworkManager", [USER_UID; 4])),
    );
    let probe = ProcServiceProbe {
        root: fx.root().to_path_buf(),
    };
    let report = network_lock_status(&fx.env(), &probe);
    assert_eq!(report.backend, NetworkBackend::Iwd);
    assert_eq!(report.state, NetworkLockState::UnsupportedBackend);
    assert!(
        !report.warnings.contains(&"iwd_dbus_bypass"),
        "the fake NetworkManager does not count"
    );
}

// ---------------------------------------------------------------------------------------
// NLK14 — G-2: status checks every directory on the path
// ---------------------------------------------------------------------------------------

#[test]
fn test_nlk_status_insecure_when_an_ancestor_dir_is_group_or_world_writable() {
    for (dir, mode) in [
        ("etc/polkit-1", 0o777),
        ("etc/polkit-1", 0o775),
        ("etc/polkit-1", 0o757),
        ("etc", 0o777),
        ("etc", 0o775),
        ("etc", 0o757),
    ] {
        let fx = Fixture::new();
        fx.install_rules();
        fx.chmod(dir, mode);
        let report = fx.status(NM);
        assert_eq!(
            report.rules,
            RuleFileState::InsecurePermissions,
            "{dir} {mode:o}"
        );
        assert_eq!(report.state, NetworkLockState::InsecurePermissions);
        assert_eq!(report.exit_code(), 1, "{dir} {mode:o}");
    }

    // Positive control: the same tree with 0755 ancestors is installed.
    let fx = Fixture::new();
    fx.install_rules();
    assert_eq!(fx.status(NM).state, NetworkLockState::Installed);
}

#[test]
fn test_nlk_status_insecure_when_an_ancestor_dir_is_a_symlink() {
    let fx = Fixture::new();
    // A real polkit-1 tree elsewhere, holding a byte-identical rule.
    let real = fx.path("elsewhere/polkit-1");
    fx.mkdir("elsewhere/polkit-1/rules.d");
    fx.write(
        &real.join("rules.d").join(NETWORK_LOCK_RULES_FILE),
        NETWORK_LOCK_RULES.as_bytes(),
        0o644,
    );
    fs::remove_dir(fx.path(NETWORK_LOCK_RULES_DIR)).expect("rmdir rules.d");
    fs::remove_dir(fx.path("etc/polkit-1")).expect("rmdir polkit-1");
    symlink(&real, fx.path("etc/polkit-1")).expect("symlink");
    let report = fx.status(NM);
    assert_eq!(report.rules, RuleFileState::InsecurePermissions);
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn test_nlk_status_rfkill_rule_insecure_when_its_ancestor_is_writable() {
    let fx = Fixture::new();
    fx.install_rules();
    fx.write(
        &fx.rfkill_rule_path(),
        NETWORK_LOCK_RFKILL_RULE.as_bytes(),
        0o644,
    );
    assert_eq!(fx.status(NM).rfkill_rule, RuleFileState::Installed);
    for dir in ["etc/udev", "etc"] {
        fx.chmod(dir, 0o777);
        let report = fx.status(NM);
        assert_eq!(
            report.rfkill_rule,
            RuleFileState::InsecurePermissions,
            "{dir}"
        );
        assert_eq!(report.exit_code(), 1, "{dir}");
        fx.chmod(dir, 0o755);
    }
    // A writable etc/udev does not affect the polkit rule.
    fx.chmod("etc/udev", 0o777);
    assert_eq!(fx.status(NM).rules, RuleFileState::Installed);
}

// ---------------------------------------------------------------------------------------
// NLK16 — G-5: stale temp files of dead processes are removed by install and uninstall
// ---------------------------------------------------------------------------------------

/// Pids no process can have: `PID_MAX_LIMIT` (pids are below `pid_max` <= 4194304) and
/// `i32::MAX`.
const DEAD_PID: u32 = 4_194_304;
const DEAD_PID_MAX: u32 = 2_147_483_647;

/// A live child of the test (`kill(pid, 0)` returns `Ok`), killed on drop.
struct LiveChild(std::process::Child);

impl LiveChild {
    fn spawn() -> Self {
        Self(
            Command::new("sleep")
                .arg("60")
                .spawn()
                .expect("spawn sleep"),
        )
    }

    fn pid(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for LiveChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn temp_name(file: &str, pid: u32) -> String {
    format!(".{file}.{pid}.tmp")
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn test_nlk_install_removes_stale_temp_files_of_dead_processes() {
    let fx = Fixture::new();
    let pid = DEAD_PID;
    let rules_dir = fx.path(NETWORK_LOCK_RULES_DIR);
    let rfkill_dir = fx.path(NETWORK_LOCK_RFKILL_DIR);
    fx.write(
        &rules_dir.join(temp_name(NETWORK_LOCK_RULES_FILE, pid)),
        b"half",
        0o600,
    );
    fx.write(
        &rfkill_dir.join(temp_name(NETWORK_LOCK_RFKILL_FILE, pid)),
        b"half",
        0o600,
    );
    install_network_lock(fx.root(), InstallOptions { rfkill: true }, false).expect("install");
    assert_eq!(
        names(&rules_dir),
        vec![NETWORK_LOCK_RULES_FILE.to_string()],
        "the stale polkit temp file is removed"
    );
    assert_eq!(
        names(&rfkill_dir),
        vec![NETWORK_LOCK_RFKILL_FILE.to_string()],
        "the stale udev temp file is removed"
    );

    // Without --rfkill the udev directory is still cleaned (the rfkill rule is removed there).
    let fx = Fixture::new();
    let rfkill_dir = fx.path(NETWORK_LOCK_RFKILL_DIR);
    fx.write(
        &rfkill_dir.join(temp_name(NETWORK_LOCK_RFKILL_FILE, pid)),
        b"half",
        0o600,
    );
    install_network_lock(fx.root(), InstallOptions { rfkill: false }, false).expect("install");
    assert!(names(&rfkill_dir).is_empty(), "{:?}", names(&rfkill_dir));
}

#[test]
fn test_nlk_uninstall_removes_stale_temp_files_of_dead_processes() {
    let fx = Fixture::new();
    install_network_lock(fx.root(), InstallOptions { rfkill: true }, false).expect("install");
    let pid = DEAD_PID_MAX;
    let rules_dir = fx.path(NETWORK_LOCK_RULES_DIR);
    let rfkill_dir = fx.path(NETWORK_LOCK_RFKILL_DIR);
    fx.write(
        &rules_dir.join(temp_name(NETWORK_LOCK_RULES_FILE, pid)),
        b"half",
        0o600,
    );
    fx.write(
        &rfkill_dir.join(temp_name(NETWORK_LOCK_RFKILL_FILE, pid)),
        b"half",
        0o600,
    );
    uninstall_network_lock(fx.root()).expect("uninstall");
    assert!(names(&rules_dir).is_empty(), "{:?}", names(&rules_dir));
    assert!(names(&rfkill_dir).is_empty(), "{:?}", names(&rfkill_dir));

    // Also when the rule itself is already gone.
    fx.write(
        &rules_dir.join(temp_name(NETWORK_LOCK_RULES_FILE, pid)),
        b"half",
        0o600,
    );
    uninstall_network_lock(fx.root()).expect("uninstall");
    assert!(names(&rules_dir).is_empty(), "{:?}", names(&rules_dir));
}

#[test]
fn test_nlk_stale_temp_cleanup_keeps_live_foreign_and_non_regular_entries() {
    let fx = Fixture::new();
    let rules_dir = fx.path(NETWORK_LOCK_RULES_DIR);
    let dead = DEAD_PID;
    let child = LiveChild::spawn();
    let kept_files = [
        // pid 1 always runs (`EPERM` for a non-root caller, `Ok` for root).
        temp_name(NETWORK_LOCK_RULES_FILE, 1),
        // A live child of the test (`Ok`): perhaps a concurrent install.
        temp_name(NETWORK_LOCK_RULES_FILE, child.pid()),
        // Near names are not soos temp names.
        format!(".{NETWORK_LOCK_RULES_FILE}.{dead}x.tmp"),
        format!(".{NETWORK_LOCK_RULES_FILE}..tmp"),
        format!(".{NETWORK_LOCK_RULES_FILE}.{dead}.tmp.bak"),
        format!("{NETWORK_LOCK_RULES_FILE}.{dead}.tmp"),
        format!(".{NETWORK_LOCK_RFKILL_FILE}.{dead}.tmp"),
        format!(".10-other.rules.{dead}.tmp"),
        format!(".{NETWORK_LOCK_RULES_FILE}.0.tmp"),
        format!(".{NETWORK_LOCK_RULES_FILE}.0{dead}.tmp"),
        format!(".{NETWORK_LOCK_RULES_FILE}.99999999999.tmp"),
        format!(".{NETWORK_LOCK_RULES_FILE}.+{dead}.tmp"),
    ];
    for name in &kept_files {
        fx.write(&rules_dir.join(name), b"keep", 0o600);
    }
    // A symlink and a directory carrying a stale temp name are not regular files.
    let outside = fx.path("outside");
    fx.write(&outside, b"outside", 0o644);
    let link_name = temp_name(NETWORK_LOCK_RULES_FILE, DEAD_PID_MAX);
    symlink(&outside, rules_dir.join(&link_name)).expect("symlink");
    let dir_name = temp_name(NETWORK_LOCK_RULES_FILE, DEAD_PID + 1);
    fs::create_dir(rules_dir.join(&dir_name)).expect("mkdir");
    // The one stale regular file is removed.
    let stale = temp_name(NETWORK_LOCK_RULES_FILE, dead);
    fx.write(&rules_dir.join(&stale), b"half", 0o600);

    install_network_lock(fx.root(), InstallOptions::default(), false).expect("install");

    let after = names(&rules_dir);
    assert!(
        !after.contains(&stale),
        "stale file of a dead pid removed: {after:?}"
    );
    for name in kept_files.iter().chain([&link_name, &dir_name]) {
        assert!(after.contains(name), "`{name}` must be kept: {after:?}");
    }
    assert_eq!(fs::read(&outside).expect("read"), b"outside");
    assert!(fs::symlink_metadata(rules_dir.join(&link_name))
        .expect("link")
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_to_string(fx.rules_path()).expect("read"),
        NETWORK_LOCK_RULES
    );
}
