//! Contract tests of `soos-admin network-lock status` (GitHub #365, spec
//! `AI/architect_spec_network_lock.md` §4; matrix NLK4–NLK8).
//!
//! Every state is reached through the injected seams: a temporary root directory
//! (`NetworkLockEnv::root`), the owner the files must have (`required_owner_uid`, the test's
//! own uid instead of root) and a fake `NetworkServiceProbe`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use soos_admin_cli::network_lock::{
    detect_backend, network_lock_status, rfkill_access, NetworkBackend, NetworkLockEnv,
    NetworkLockReport, NetworkLockState, NetworkServiceProbe, ProcServiceProbe, RfkillAccess,
    RuleFileState, RunningServices, MAX_COMM_BYTES, MAX_RULE_FILE_BYTES, NETWORK_LOCK_RFKILL_DIR,
    NETWORK_LOCK_RFKILL_FILE, NETWORK_LOCK_RFKILL_RULE, NETWORK_LOCK_RULES, NETWORK_LOCK_RULES_DIR,
    NETWORK_LOCK_RULES_FILE,
};

const NM: RunningServices = RunningServices {
    network_manager: true,
    networkd: false,
    iwd: false,
};
const IWD_ONLY: RunningServices = RunningServices {
    network_manager: false,
    networkd: false,
    iwd: true,
};
const NETWORKD_ONLY: RunningServices = RunningServices {
    network_manager: false,
    networkd: true,
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

/// A staging root with `etc/polkit-1/rules.d` and `etc/udev/rules.d` (0755).
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let fx = Self { dir };
        fx.mkdir(NETWORK_LOCK_RULES_DIR, 0o755);
        fx.mkdir(NETWORK_LOCK_RFKILL_DIR, 0o755);
        fx
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root().join(rel)
    }

    fn rules_path(&self) -> PathBuf {
        self.path(NETWORK_LOCK_RULES_DIR)
            .join(NETWORK_LOCK_RULES_FILE)
    }

    fn rfkill_rule_path(&self) -> PathBuf {
        self.path(NETWORK_LOCK_RFKILL_DIR)
            .join(NETWORK_LOCK_RFKILL_FILE)
    }

    /// Creates every missing component of `rel` as 0755 (independent of the umask), then
    /// sets `mode` on `rel` itself.
    fn mkdir(&self, rel: &str, mode: u32) {
        let mut p = self.root().to_path_buf();
        for part in Path::new(rel).components() {
            p.push(part);
            if !p.exists() {
                fs::create_dir(&p).expect("mkdir");
                fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).expect("chmod");
            }
        }
        fs::set_permissions(&p, fs::Permissions::from_mode(mode)).expect("chmod dir");
    }

    fn write(&self, path: &Path, content: &[u8], mode: u32) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir parent");
        }
        fs::write(path, content).expect("write");
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
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

    /// `/sys/class/misc/rfkill/dev`, the udev database entry and the `/dev/rfkill` node mode.
    fn rfkill(&self, dev: Option<&str>, udev_db: Option<&str>, node_mode: u32) {
        if let Some(dev) = dev {
            self.write(
                &self.path("sys/class/misc/rfkill/dev"),
                dev.as_bytes(),
                0o444,
            );
        }
        if let Some(db) = udev_db {
            self.write(&self.path("run/udev/data/c10:242"), db.as_bytes(), 0o644);
        }
        self.write(&self.path("dev/rfkill"), b"", node_mode);
    }
}

// ---------------------------------------------------------------------------------------
// NLK4 — installed / missing / modified
// ---------------------------------------------------------------------------------------

#[test]
fn test_nlk_status_installed_when_byte_identical_and_secure() {
    let fx = Fixture::new();
    fx.install_rules();
    let report = fx.status(NM);
    assert_eq!(report.rules, RuleFileState::Installed);
    assert_eq!(report.state, NetworkLockState::Installed);
    assert_eq!(report.backend, NetworkBackend::NetworkManager);
    assert_eq!(report.exit_code(), 0);
    assert!(report.format_table().contains("installed"));
}

#[test]
fn test_nlk_status_installed_in_a_0750_polkitd_style_directory() {
    // Arch ships /etc/polkit-1/rules.d as 0750 root:polkitd: not group- or world-writable.
    let fx = Fixture::new();
    fx.mkdir(NETWORK_LOCK_RULES_DIR, 0o750);
    fx.install_rules();
    assert_eq!(fx.status(NM).rules, RuleFileState::Installed);
}

#[test]
fn test_nlk_status_missing_when_absent() {
    let fx = Fixture::new();
    let report = fx.status(NM);
    assert_eq!(report.rules, RuleFileState::Missing);
    assert_eq!(report.state, NetworkLockState::Missing);
    assert_eq!(
        report.exit_code(),
        0,
        "the lock is opt-in: missing is not an error"
    );
    assert!(report.format_table().contains("missing"));
}

#[test]
fn test_nlk_status_missing_when_polkit_rules_dir_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let env = NetworkLockEnv {
        root: dir.path().to_path_buf(),
        required_owner_uid: euid(),
    };
    let report = network_lock_status(&env, &FakeProbe(NM));
    assert_eq!(report.rules, RuleFileState::Missing);
    assert_eq!(report.rfkill_rule, RuleFileState::Missing);
    assert_eq!(report.state, NetworkLockState::Missing);
    assert_eq!(report.exit_code(), 0);
}

#[test]
fn test_nlk_status_modified_when_one_byte_differs() {
    let fx = Fixture::new();
    let mut content = NETWORK_LOCK_RULES.as_bytes().to_vec();
    let at = NETWORK_LOCK_RULES
        .find("AUTH_ADMIN")
        .expect("rule returns AUTH_ADMIN");
    content[at] = b'X';
    fx.write(&fx.rules_path(), &content, 0o644);
    let report = fx.status(NM);
    assert_eq!(report.rules, RuleFileState::Modified);
    assert_eq!(report.state, NetworkLockState::Modified);
    assert_eq!(report.exit_code(), 1);
    assert!(report.format_table().contains("modified"));
}

#[test]
fn test_nlk_status_modified_when_trailing_byte_added_or_truncated() {
    let fx = Fixture::new();
    let mut longer = NETWORK_LOCK_RULES.as_bytes().to_vec();
    longer.push(b'\n');
    fx.write(&fx.rules_path(), &longer, 0o644);
    assert_eq!(fx.status(NM).rules, RuleFileState::Modified);

    let shorter = &NETWORK_LOCK_RULES.as_bytes()[..NETWORK_LOCK_RULES.len() - 1];
    fx.write(&fx.rules_path(), shorter, 0o644);
    assert_eq!(fx.status(NM).rules, RuleFileState::Modified);
}

#[test]
fn test_nlk_status_modified_when_empty() {
    let fx = Fixture::new();
    fx.write(&fx.rules_path(), b"", 0o644);
    let report = fx.status(NM);
    assert_eq!(report.rules, RuleFileState::Modified);
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn test_nlk_status_modified_when_over_the_read_cap() {
    let fx = Fixture::new();
    let mut big = NETWORK_LOCK_RULES.as_bytes().to_vec();
    big.resize(MAX_RULE_FILE_BYTES as usize + 1, b' ');
    fx.write(&fx.rules_path(), &big, 0o644);
    let report = fx.status(NM);
    assert_eq!(report.rules, RuleFileState::Modified);
    assert_eq!(report.exit_code(), 1);
}

// ---------------------------------------------------------------------------------------
// NLK5 — insecure_permissions / unreadable
// ---------------------------------------------------------------------------------------

#[test]
fn test_nlk_status_insecure_when_group_or_world_writable() {
    for mode in [0o664, 0o646, 0o666, 0o660] {
        let fx = Fixture::new();
        fx.write(&fx.rules_path(), NETWORK_LOCK_RULES.as_bytes(), mode);
        let report = fx.status(NM);
        assert_eq!(
            report.rules,
            RuleFileState::InsecurePermissions,
            "mode {mode:o}"
        );
        assert_eq!(report.state, NetworkLockState::InsecurePermissions);
        assert_eq!(report.exit_code(), 1, "mode {mode:o}");
        assert!(report.format_table().contains("insecure_permissions"));
    }
}

#[test]
fn test_nlk_status_insecure_when_owner_is_not_the_required_owner() {
    let fx = Fixture::new();
    fx.install_rules();
    let env = NetworkLockEnv {
        root: fx.root().to_path_buf(),
        required_owner_uid: euid().wrapping_add(1),
    };
    let report = network_lock_status(&env, &FakeProbe(NM));
    assert_eq!(report.rules, RuleFileState::InsecurePermissions);
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn test_nlk_status_insecure_when_rule_file_is_a_symlink() {
    let fx = Fixture::new();
    let elsewhere = fx.path("elsewhere.rules");
    fx.write(&elsewhere, NETWORK_LOCK_RULES.as_bytes(), 0o644);
    symlink(&elsewhere, fx.rules_path()).expect("symlink");
    let report = fx.status(NM);
    assert_eq!(report.rules, RuleFileState::InsecurePermissions);
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn test_nlk_status_insecure_when_rules_dir_is_a_symlink() {
    let fx = Fixture::new();
    let real = fx.path("real-rules.d");
    fs::create_dir_all(&real).expect("mkdir");
    fs::set_permissions(&real, fs::Permissions::from_mode(0o755)).expect("chmod");
    fx.write(
        &real.join(NETWORK_LOCK_RULES_FILE),
        NETWORK_LOCK_RULES.as_bytes(),
        0o644,
    );
    fs::remove_dir(fx.path(NETWORK_LOCK_RULES_DIR)).expect("rmdir");
    symlink(&real, fx.path(NETWORK_LOCK_RULES_DIR)).expect("symlink dir");
    assert_eq!(fx.status(NM).rules, RuleFileState::InsecurePermissions);
}

#[test]
fn test_nlk_status_insecure_when_rules_dir_is_group_or_world_writable() {
    for mode in [0o777, 0o775, 0o757] {
        let fx = Fixture::new();
        fx.install_rules();
        fx.mkdir(NETWORK_LOCK_RULES_DIR, mode);
        assert_eq!(
            fx.status(NM).rules,
            RuleFileState::InsecurePermissions,
            "dir mode {mode:o}"
        );
    }
}

#[test]
fn test_nlk_status_insecure_takes_precedence_over_unsupported_backend() {
    let fx = Fixture::new();
    fx.write(&fx.rules_path(), NETWORK_LOCK_RULES.as_bytes(), 0o666);
    let report = fx.status(IWD_ONLY);
    assert_eq!(report.state, NetworkLockState::InsecurePermissions);
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn test_nlk_status_unreadable_when_rules_dir_cannot_be_searched() {
    if euid() == 0 {
        // Root bypasses permission bits; this state is reached only by non-root callers.
        return;
    }
    let fx = Fixture::new();
    fx.install_rules();
    fx.mkdir(NETWORK_LOCK_RULES_DIR, 0o000);
    let report = fx.status(NM);
    fx.mkdir(NETWORK_LOCK_RULES_DIR, 0o755);
    assert_eq!(report.rules, RuleFileState::Unreadable);
    assert_eq!(report.state, NetworkLockState::Unreadable);
    assert_eq!(
        report.exit_code(),
        1,
        "an unverifiable lock is not reported as fine"
    );
    let table = report.format_table();
    assert!(table.contains("unreadable"));
    assert!(table.contains("run as root"), "{table}");
}

#[test]
fn test_nlk_status_unreadable_when_rule_file_cannot_be_read() {
    if euid() == 0 {
        return;
    }
    let fx = Fixture::new();
    fx.write(&fx.rules_path(), NETWORK_LOCK_RULES.as_bytes(), 0o000);
    let report = fx.status(NM);
    assert_eq!(report.rules, RuleFileState::Unreadable);
    assert_eq!(report.exit_code(), 1);
}

// ---------------------------------------------------------------------------------------
// NLK6 — backend detection and unsupported_backend
// ---------------------------------------------------------------------------------------

#[test]
fn test_nlk_status_unsupported_backend_on_iwd_only_host() {
    let fx = Fixture::new();
    let report = fx.status(IWD_ONLY);
    assert_eq!(report.backend, NetworkBackend::Iwd);
    assert_eq!(report.state, NetworkLockState::UnsupportedBackend);
    assert_eq!(report.exit_code(), 0);
    assert!(
        report.format_table().contains("unsupported_backend(iwd)"),
        "{}",
        report.format_table()
    );

    fx.install_rules();
    let report = fx.status(IWD_ONLY);
    assert_eq!(report.rules, RuleFileState::Installed);
    assert_eq!(
        report.state,
        NetworkLockState::UnsupportedBackend,
        "an installed rule does not lock an iwd-only host"
    );
}

#[test]
fn test_nlk_status_unsupported_backend_on_networkd_only_host() {
    let fx = Fixture::new();
    fx.install_rules();
    let report = fx.status(NETWORKD_ONLY);
    assert_eq!(report.backend, NetworkBackend::Networkd);
    assert_eq!(report.state, NetworkLockState::UnsupportedBackend);
    assert!(report
        .format_table()
        .contains("unsupported_backend(networkd)"));
}

#[test]
fn test_nlk_status_modified_takes_precedence_over_unsupported_backend() {
    let fx = Fixture::new();
    fx.write(
        &fx.rules_path(),
        b"polkit.addRule(function () {});\n",
        0o644,
    );
    let report = fx.status(IWD_ONLY);
    assert_eq!(report.state, NetworkLockState::Modified);
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn test_nlk_backend_precedence() {
    let all = |nm, nd, iwd| RunningServices {
        network_manager: nm,
        networkd: nd,
        iwd,
    };
    assert_eq!(
        detect_backend(all(true, false, true)),
        NetworkBackend::NetworkManager,
        "NetworkManager with the iwd Wi-Fi backend is still governed by polkit"
    );
    assert_eq!(
        detect_backend(all(true, true, false)),
        NetworkBackend::NetworkManager
    );
    assert_eq!(
        detect_backend(all(true, true, true)),
        NetworkBackend::NetworkManager
    );
    assert_eq!(detect_backend(all(false, true, true)), NetworkBackend::Iwd);
    assert_eq!(
        detect_backend(all(false, true, false)),
        NetworkBackend::Networkd
    );
    assert_eq!(detect_backend(all(false, false, true)), NetworkBackend::Iwd);
    assert_eq!(
        detect_backend(all(false, false, false)),
        NetworkBackend::Unknown
    );
}

#[test]
fn test_nlk_status_unknown_backend_keeps_the_file_verdict() {
    let fx = Fixture::new();
    fx.install_rules();
    let report = fx.status(RunningServices::default());
    assert_eq!(report.backend, NetworkBackend::Unknown);
    assert_eq!(report.state, NetworkLockState::Installed);
    assert_eq!(report.exit_code(), 0);
}

#[test]
fn test_nlk_proc_probe_matches_exact_comm_names() {
    let fx = Fixture::new();
    fx.write(&fx.path("proc/1/comm"), b"systemd\n", 0o444);
    fx.write(&fx.path("proc/31337/comm"), b"NetworkManager\n", 0o444);
    fx.write(&fx.path("proc/4242/comm"), b"systemd-network\n", 0o444);
    fx.write(&fx.path("proc/777/comm"), b"iwd\n", 0o444);
    // Contract amendment (GitHub #370): a match is credited only for a root-owned process.
    fx.write(
        &fx.path("proc/1/status"),
        b"Name:\tsystemd\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    fx.write(
        &fx.path("proc/31337/status"),
        b"Name:\tNetworkManager\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    fx.write(
        &fx.path("proc/4242/status"),
        b"Name:\tsystemd-network\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    fx.write(
        &fx.path("proc/777/status"),
        b"Name:\tiwd\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    let probe = ProcServiceProbe {
        root: fx.root().to_path_buf(),
    };
    assert_eq!(
        probe.running(),
        RunningServices {
            network_manager: true,
            networkd: true,
            iwd: true,
        }
    );
}

#[test]
fn test_nlk_proc_probe_ignores_non_numeric_entries_and_near_names() {
    let fx = Fixture::new();
    // Only numeric /proc entries are processes.
    fx.write(&fx.path("proc/self/comm"), b"NetworkManager\n", 0o444);
    fx.write(&fx.path("proc/net/comm"), b"iwd\n", 0o444);
    // Near names are not the services.
    fx.write(&fx.path("proc/10/comm"), b"nm-dispatcher\n", 0o444);
    fx.write(&fx.path("proc/11/comm"), b"iwdx\n", 0o444);
    fx.write(&fx.path("proc/12/comm"), b"NetworkManagerX\n", 0o444);
    fx.write(&fx.path("proc/13/comm"), b"systemd-resolve\n", 0o444);
    // Contract amendment (GitHub #370): root-owned, so only the names decide.
    fx.write(
        &fx.path("proc/self/status"),
        b"Name:\tNetworkManager\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    fx.write(
        &fx.path("proc/net/status"),
        b"Name:\tiwd\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    fx.write(
        &fx.path("proc/10/status"),
        b"Name:\tnm-dispatcher\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    fx.write(
        &fx.path("proc/11/status"),
        b"Name:\tiwdx\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    fx.write(
        &fx.path("proc/12/status"),
        b"Name:\tNetworkManagerX\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    fx.write(
        &fx.path("proc/13/status"),
        b"Name:\tsystemd-resolve\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    let probe = ProcServiceProbe {
        root: fx.root().to_path_buf(),
    };
    assert_eq!(probe.running(), RunningServices::default());
}

#[test]
fn test_nlk_proc_probe_ignores_oversize_comm_and_missing_proc() {
    let fx = Fixture::new();
    let mut big = b"iwd\n".to_vec();
    big.resize(MAX_COMM_BYTES as usize + 1, b'\n');
    fx.write(&fx.path("proc/20/comm"), &big, 0o444);
    // Contract amendment (GitHub #370): root-owned, so only the comm size decides.
    fx.write(
        &fx.path("proc/20/status"),
        b"Name:\tiwd\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    let probe = ProcServiceProbe {
        root: fx.root().to_path_buf(),
    };
    assert_eq!(probe.running(), RunningServices::default());

    let empty = tempfile::tempdir().expect("tempdir");
    let probe = ProcServiceProbe {
        root: empty.path().to_path_buf(),
    };
    assert_eq!(probe.running(), RunningServices::default());
}

#[test]
fn test_nlk_status_uses_the_probe_result_for_the_backend() {
    let fx = Fixture::new();
    fx.install_rules();
    fx.write(&fx.path("proc/31337/comm"), b"iwd\n", 0o444);
    // Contract amendment (GitHub #370): a match is credited only for a root-owned process.
    fx.write(
        &fx.path("proc/31337/status"),
        b"Name:\tiwd\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    let probe = ProcServiceProbe {
        root: fx.root().to_path_buf(),
    };
    let report = network_lock_status(&fx.env(), &probe);
    assert_eq!(report.backend, NetworkBackend::Iwd);
    assert_eq!(report.state, NetworkLockState::UnsupportedBackend);
}

// ---------------------------------------------------------------------------------------
// NLK7 — rfkill access and the rfkill rule file
// ---------------------------------------------------------------------------------------

#[test]
fn test_nlk_rfkill_current_uaccess_tag_warns() {
    let fx = Fixture::new();
    fx.install_rules();
    fx.rfkill(
        Some("10:242\n"),
        Some("I:123\nG:uaccess\nG:seat\nQ:uaccess\nQ:seat\nV:1\n"),
        0o664,
    );
    assert_eq!(rfkill_access(fx.root()), RfkillAccess::Uaccess);
    let report = fx.status(NM);
    assert_eq!(report.rfkill, RfkillAccess::Uaccess);
    assert_eq!(report.warnings, vec!["rfkill_uaccess"]);
    assert_eq!(
        report.exit_code(),
        0,
        "the documented gap is a warning, not a failure"
    );
    assert!(report.format_table().contains("rfkill_uaccess"));
}

#[test]
fn test_nlk_rfkill_legacy_database_without_current_tags_uses_g_lines() {
    let fx = Fixture::new();
    fx.rfkill(Some("10:242\n"), Some("G:uaccess\nG:seat\n"), 0o664);
    assert_eq!(rfkill_access(fx.root()), RfkillAccess::Uaccess);
}

#[test]
fn test_nlk_rfkill_removed_current_tag_is_restricted() {
    // After `TAG-="uaccess"`, G: (tags ever set) may keep it while Q: (current) does not.
    let fx = Fixture::new();
    fx.rfkill(Some("10:242\n"), Some("G:uaccess\nG:seat\nQ:seat\n"), 0o664);
    assert_eq!(rfkill_access(fx.root()), RfkillAccess::Restricted);
    let report = fx.status(NM);
    assert!(report.warnings.is_empty());
    assert!(!report.format_table().contains("rfkill_uaccess"));
}

#[test]
fn test_nlk_rfkill_tag_match_is_exact() {
    let fx = Fixture::new();
    fx.rfkill(Some("10:242\n"), Some("Q:uaccess-foo\nQ:xuaccess\n"), 0o664);
    assert_eq!(rfkill_access(fx.root()), RfkillAccess::Restricted);
}

#[test]
fn test_nlk_rfkill_world_writable_node_warns() {
    let fx = Fixture::new();
    fx.rfkill(Some("10:242\n"), Some("Q:seat\n"), 0o666);
    assert_eq!(rfkill_access(fx.root()), RfkillAccess::Uaccess);
}

#[test]
fn test_nlk_rfkill_absent_without_sysfs_device() {
    let fx = Fixture::new();
    assert_eq!(rfkill_access(fx.root()), RfkillAccess::Absent);
    assert!(fx.status(NM).warnings.is_empty());
}

#[test]
fn test_nlk_rfkill_unknown_on_malformed_dev_or_missing_database() {
    for dev in [
        "garbage\n",
        "10\n",
        "10:\n",
        ":242\n",
        "10:242:1\n",
        "-1:5\n",
        "",
    ] {
        let fx = Fixture::new();
        fx.rfkill(Some(dev), Some("Q:uaccess\n"), 0o664);
        assert_eq!(
            rfkill_access(fx.root()),
            RfkillAccess::Unknown,
            "dev {dev:?}"
        );
    }
    let fx = Fixture::new();
    fx.rfkill(Some("10:242\n"), None, 0o664);
    assert_eq!(rfkill_access(fx.root()), RfkillAccess::Unknown);
}

#[test]
fn test_nlk_rfkill_rule_file_state_is_reported() {
    let fx = Fixture::new();
    fx.install_rules();
    fx.write(
        &fx.rfkill_rule_path(),
        NETWORK_LOCK_RFKILL_RULE.as_bytes(),
        0o644,
    );
    let report = fx.status(NM);
    assert_eq!(report.rfkill_rule, RuleFileState::Installed);
    assert_eq!(report.exit_code(), 0);
    assert!(report.format_table().contains(NETWORK_LOCK_RFKILL_FILE));
}

#[test]
fn test_nlk_rfkill_rule_modified_or_insecure_fails_the_status() {
    let fx = Fixture::new();
    fx.install_rules();
    fx.write(&fx.rfkill_rule_path(), b"# edited\n", 0o644);
    let report = fx.status(NM);
    assert_eq!(report.rfkill_rule, RuleFileState::Modified);
    assert_eq!(report.state, NetworkLockState::Installed);
    assert_eq!(report.exit_code(), 1);

    fx.write(
        &fx.rfkill_rule_path(),
        NETWORK_LOCK_RFKILL_RULE.as_bytes(),
        0o666,
    );
    let report = fx.status(NM);
    assert_eq!(report.rfkill_rule, RuleFileState::InsecurePermissions);
    assert_eq!(report.exit_code(), 1);
}

// ---------------------------------------------------------------------------------------
// NLK8 — output shape and privacy
// ---------------------------------------------------------------------------------------

#[test]
fn test_nlk_status_json_shape() {
    let fx = Fixture::new();
    fx.install_rules();
    fx.rfkill(Some("10:242\n"), Some("Q:uaccess\n"), 0o664);
    let json: serde_json::Value =
        serde_json::from_str(&fx.status(NM).to_json()).expect("valid JSON");
    assert_eq!(json["state"], "installed");
    assert_eq!(json["backend"], "network_manager");
    assert_eq!(json["rules"], "installed");
    assert_eq!(json["rfkill_rule"], "missing");
    assert_eq!(json["rfkill"], "uaccess");
    assert_eq!(json["warnings"], serde_json::json!(["rfkill_uaccess"]));
    assert_eq!(json.as_object().expect("object").len(), 6);

    let json: serde_json::Value =
        serde_json::from_str(&fx.status(IWD_ONLY).to_json()).expect("valid JSON");
    assert_eq!(json["state"], "unsupported_backend");
    assert_eq!(json["backend"], "iwd");
}

#[test]
fn test_nlk_status_output_has_no_network_names() {
    let fx = Fixture::new();
    fx.install_rules();
    fx.write(
        &fx.path("etc/NetworkManager/system-connections/HomeNet-5G.nmconnection"),
        b"[connection]\nid=HomeNet-5G\nuuid=0b6a4c1e-1111-2222-3333-444455556666\n\
          [wifi]\nssid=HomeNet-5G\n[wifi-security]\npsk=hunter2-secret\n",
        0o600,
    );
    fx.write(&fx.path("proc/31337/comm"), b"NetworkManager\n", 0o444);
    // Contract amendment (GitHub #370): a match is credited only for a root-owned process.
    fx.write(
        &fx.path("proc/31337/status"),
        b"Name:\tNetworkManager\nUid:\t0\t0\t0\t0\n",
        0o444,
    );
    fx.rfkill(Some("10:242\n"), Some("Q:uaccess\n"), 0o664);
    let probe = ProcServiceProbe {
        root: fx.root().to_path_buf(),
    };
    let report = network_lock_status(&fx.env(), &probe);
    assert_eq!(report.backend, NetworkBackend::NetworkManager);
    for out in [report.format_table(), report.to_json()] {
        assert!(!out.is_empty());
        for secret in [
            "HomeNet",
            "hunter2",
            "nmconnection",
            "0b6a4c1e",
            "31337",
            "system-connections",
        ] {
            assert!(!out.contains(secret), "`{secret}` leaked into:\n{out}");
        }
        assert!(
            !out.contains(&fx.root().display().to_string()),
            "the staging root is not printed"
        );
    }
}

#[test]
fn test_nlk_status_table_names_the_installed_rule_path() {
    let fx = Fixture::new();
    fx.install_rules();
    let table = fx.status(NM).format_table();
    assert!(
        table.contains("/etc/polkit-1/rules.d/49-soos-network-lock.rules"),
        "{table}"
    );
    assert!(table.contains("network_manager"), "{table}");
}

// ---------------------------------------------------------------------------------------
// Plan evaluation round 1: F-3 `iwd_dbus_bypass`, F-4 `rfkill_reboot_pending`
// ---------------------------------------------------------------------------------------

const NM_AND_IWD: RunningServices = RunningServices {
    network_manager: true,
    networkd: false,
    iwd: true,
};

#[test]
fn test_nlk_status_warns_iwd_dbus_bypass_next_to_network_manager() {
    let fx = Fixture::new();
    fx.install_rules();
    let report = fx.status(NM_AND_IWD);
    assert_eq!(report.backend, NetworkBackend::NetworkManager);
    assert_eq!(report.state, NetworkLockState::Installed);
    assert_eq!(report.warnings, vec!["iwd_dbus_bypass"]);
    assert_eq!(report.exit_code(), 0);
    assert!(report.format_table().contains("iwd_dbus_bypass"));

    assert!(fx.status(NM).warnings.is_empty(), "no iwd, no warning");
    assert!(
        !fx.status(IWD_ONLY).warnings.contains(&"iwd_dbus_bypass"),
        "an iwd-only host is reported as unsupported_backend instead"
    );
}

#[test]
fn test_nlk_rfkill_reboot_pending_after_rule_written_since_boot() {
    let fx = Fixture::new();
    fx.install_rules();
    fx.write(
        &fx.rfkill_rule_path(),
        NETWORK_LOCK_RFKILL_RULE.as_bytes(),
        0o644,
    );
    // Booted about 11.5 days ago: the rule file written now is newer than the boot.
    fx.write(&fx.path("proc/uptime"), b"1000000.00 1.00\n", 0o444);
    let report = fx.status(NM);
    assert_eq!(report.rfkill_rule, RuleFileState::Installed);
    assert_eq!(report.warnings, vec!["rfkill_reboot_pending"]);
    assert_eq!(report.exit_code(), 0);
    assert!(report.format_table().contains("rfkill_reboot_pending"));
}

#[test]
fn test_nlk_rfkill_reboot_pending_needs_an_installed_rule_and_a_valid_uptime() {
    for uptime in [
        None,
        Some("abc\n"),
        Some("inf 1\n"),
        Some("NaN 1\n"),
        Some("-5 1\n"),
        Some(""),
    ] {
        let fx = Fixture::new();
        fx.write(
            &fx.rfkill_rule_path(),
            NETWORK_LOCK_RFKILL_RULE.as_bytes(),
            0o644,
        );
        if let Some(u) = uptime {
            fx.write(&fx.path("proc/uptime"), u.as_bytes(), 0o444);
        }
        assert!(
            fx.status(NM).warnings.is_empty(),
            "uptime {uptime:?}: unknown boot time adds no warning"
        );
    }
    let fx = Fixture::new();
    fx.write(&fx.path("proc/uptime"), b"1000000.00 1.00\n", 0o444);
    assert!(
        fx.status(NM).warnings.is_empty(),
        "no rfkill rule, no warning"
    );
    fx.write(&fx.rfkill_rule_path(), b"# edited\n", 0o644);
    assert!(
        !fx.status(NM).warnings.contains(&"rfkill_reboot_pending"),
        "a modified rfkill rule is reported as modified, not pending"
    );
}

#[test]
fn test_nlk_rfkill_no_reboot_pending_when_rule_predates_boot() {
    // Plan evaluation round 2, N-1: the rule written before the (simulated) boot gives no
    // warning. Boot = now - uptime; with a 10 ms uptime read 400 ms after the write, the
    // rule's ctime is ~390 ms older than the boot.
    let fx = Fixture::new();
    fx.write(
        &fx.rfkill_rule_path(),
        NETWORK_LOCK_RFKILL_RULE.as_bytes(),
        0o644,
    );
    fx.write(&fx.path("proc/uptime"), b"0.01 0.01\n", 0o444);
    std::thread::sleep(std::time::Duration::from_millis(400));
    let report = fx.status(NM);
    assert_eq!(report.rfkill_rule, RuleFileState::Installed);
    assert!(
        !report.warnings.contains(&"rfkill_reboot_pending"),
        "{:?}",
        report.warnings
    );
}

#[test]
fn test_nlk_status_warnings_have_a_fixed_order() {
    let fx = Fixture::new();
    fx.install_rules();
    fx.write(
        &fx.rfkill_rule_path(),
        NETWORK_LOCK_RFKILL_RULE.as_bytes(),
        0o644,
    );
    fx.write(&fx.path("proc/uptime"), b"1000000.00 1.00\n", 0o444);
    fx.rfkill(Some("10:242\n"), Some("Q:uaccess\n"), 0o664);
    let report = fx.status(NM_AND_IWD);
    assert_eq!(
        report.warnings,
        vec!["rfkill_uaccess", "rfkill_reboot_pending", "iwd_dbus_bypass"]
    );
    let json: serde_json::Value = serde_json::from_str(&report.to_json()).expect("json");
    assert_eq!(
        json["warnings"],
        serde_json::json!(["rfkill_uaccess", "rfkill_reboot_pending", "iwd_dbus_bypass"])
    );
}
