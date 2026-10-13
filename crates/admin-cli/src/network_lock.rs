//! Opt-in polkit Wi-Fi lock and its read-only diagnostic (GitHub #365; ADR "[2026-10-13]
//! Opt-In polkit Wi-Fi Lock and `soos-admin network-lock`"; spec
//! `AI/architect_spec_network_lock.md`).
//!
//! - [`network_lock_status`] reads the installed rule files, the running network services and
//!   the rfkill access below a root directory, and never changes anything. Its output uses a
//!   fixed vocabulary: no SSID, key, connection name, address or process id is read or printed.
//! - [`install_network_lock`] / [`uninstall_network_lock`] write or remove
//!   `/etc/polkit-1/rules.d/49-soos-network-lock.rules` and, with the rfkill option,
//!   `/etc/udev/rules.d/72-soos-network-lock-rfkill.rules`. Every directory on the path is
//!   checked (real directory, owner, not group- or world-writable) before the first write, and
//!   the file is replaced atomically relative to the verified directory descriptor. Stale
//!   temp files of an interrupted install (dead pid) are removed first (GitHub #370).

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nix::errno::Errno;
use nix::fcntl::{renameat, AtFlags};
use nix::sys::stat::{fstat, fstatat, lstat, FileStat};
use nix::unistd::{unlinkat, UnlinkatFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::AdminCliError;

/// Shipped polkit rule, single source: `packaging/polkit/49-soos-network-lock.rules`.
pub const NETWORK_LOCK_RULES: &str =
    include_str!("../../../packaging/polkit/49-soos-network-lock.rules");
/// Shipped udev rule of the rfkill option: `packaging/udev/72-soos-network-lock-rfkill.rules`.
pub const NETWORK_LOCK_RFKILL_RULE: &str =
    include_str!("../../../packaging/udev/72-soos-network-lock-rfkill.rules");

/// Directory of the polkit rule, relative to the root directory (`/` in production).
pub const NETWORK_LOCK_RULES_DIR: &str = "etc/polkit-1/rules.d";
/// File name of the polkit rule.
pub const NETWORK_LOCK_RULES_FILE: &str = "49-soos-network-lock.rules";
/// Directory of the rfkill udev rule, relative to the root directory.
pub const NETWORK_LOCK_RFKILL_DIR: &str = "etc/udev/rules.d";
/// File name of the rfkill udev rule.
pub const NETWORK_LOCK_RFKILL_FILE: &str = "72-soos-network-lock-rfkill.rules";

/// The NetworkManager polkit actions the rule answers with `AUTH_ADMIN`, in file order
/// (`Docs/NETWORK_LOCK.md` §2; pinned by `soos-invariants::network_lock_contract`).
pub const NETWORK_LOCK_ACTION_IDS: [&str; 5] = [
    "org.freedesktop.NetworkManager.network-control",
    "org.freedesktop.NetworkManager.enable-disable-wifi",
    "org.freedesktop.NetworkManager.enable-disable-network",
    "org.freedesktop.NetworkManager.settings.modify.system",
    "org.freedesktop.NetworkManager.settings.modify.own",
];

/// Read cap of an installed rule file; a longer file is `modified`.
pub const MAX_RULE_FILE_BYTES: u64 = 16_384;
/// `/proc` directory entries examined by [`ProcServiceProbe`].
pub const MAX_PROC_ENTRIES: usize = 65_536;
/// Read cap of `/proc/<pid>/comm`; a longer file is ignored.
pub const MAX_COMM_BYTES: u64 = 64;
/// Read cap of `/sys/class/misc/rfkill/dev`.
pub const MAX_SYSFS_DEV_BYTES: u64 = 32;
/// Read cap of the udev database entry of `/dev/rfkill`.
pub const MAX_UDEV_DB_BYTES: u64 = 65_536;
/// Read cap of `/proc/uptime`.
pub const MAX_UPTIME_BYTES: u64 = 64;
/// Prefix of `/proc/<pid>/status` examined for the `Name:` and `Uid:` lines (the first and
/// ninth lines, about 200 bytes in); only complete lines inside it are parsed.
pub const MAX_PROC_STATUS_BYTES: u64 = 4096;
/// Prefix of `/etc/passwd` examined for the [`SYSTEMD_NETWORK_USER`] entry; only complete
/// lines inside it are parsed.
pub const MAX_PASSWD_BYTES: u64 = 1_048_576;
/// Account systemd-networkd runs as (its `comm` is `systemd-network`).
pub const SYSTEMD_NETWORK_USER: &str = "systemd-network";
/// Directory entries examined when looking for stale temp files.
pub const MAX_RULES_DIR_ENTRIES: usize = 4096;

const WARN_RFKILL_UACCESS: &str = "rfkill_uaccess";
const WARN_RFKILL_REBOOT_PENDING: &str = "rfkill_reboot_pending";
const WARN_IWD_DBUS_BYPASS: &str = "iwd_dbus_bypass";

/// Network stack detected from the running processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkBackend {
    /// NetworkManager runs (it checks polkit, whatever its Wi-Fi backend).
    NetworkManager,
    /// systemd-networkd without NetworkManager or iwd.
    Networkd,
    /// iwd without NetworkManager (iwd does not use polkit).
    Iwd,
    /// None of them was seen.
    Unknown,
}

impl NetworkBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::NetworkManager => "network_manager",
            Self::Networkd => "networkd",
            Self::Iwd => "iwd",
            Self::Unknown => "unknown",
        }
    }
}

/// Which network services run (output of a [`NetworkServiceProbe`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunningServices {
    pub network_manager: bool,
    pub networkd: bool,
    pub iwd: bool,
}

/// Injectable service-detection seam.
pub trait NetworkServiceProbe {
    /// The network services currently running.
    fn running(&self) -> RunningServices;
}

/// Production probe: matches the exact `comm` names of the processes under `<root>/proc`
/// (`NetworkManager`, `systemd-network` — the 15-byte truncation of `systemd-networkd` — and
/// `iwd`), then credits a match only when `<root>/proc/<pid>/status` names the same process
/// (`Name:`) and all four uids of its `Uid:` line are trusted: root, or for networkd also the
/// `systemd-network` account of `<root>/etc/passwd` (GitHub #370). Any user can name a
/// process; only root can give it these uids. Read-only and bounded; it reads nothing else of
/// a process.
#[derive(Debug, Clone)]
pub struct ProcServiceProbe {
    pub root: PathBuf,
}

impl NetworkServiceProbe for ProcServiceProbe {
    fn running(&self) -> RunningServices {
        let mut found = RunningServices::default();
        let Ok(entries) = fs::read_dir(self.root.join("proc")) else {
            return found;
        };
        // Resolved at most once per call, and only when a `systemd-network` comm matches.
        let mut networkd_uid: Option<Option<u32>> = None;
        for entry in entries.take(MAX_PROC_ENTRIES).flatten() {
            let name = entry.file_name();
            let is_pid = name
                .to_str()
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            if !is_pid {
                continue;
            }
            let Ok(Bounded::Data(comm)) = read_bounded(&entry.path().join("comm"), MAX_COMM_BYTES)
            else {
                continue;
            };
            let comm = comm.strip_suffix(b"\n").unwrap_or(&comm);
            let service_uid = match comm {
                b"NetworkManager" | b"iwd" => None,
                b"systemd-network" => *networkd_uid
                    .get_or_insert_with(|| account_uid(&self.root, SYSTEMD_NETWORK_USER)),
                _ => continue,
            };
            let Some(uids) = trusted_process_uids(&entry.path(), comm) else {
                continue;
            };
            if !uids.iter().all(|&uid| uid == 0 || Some(uid) == service_uid) {
                continue;
            }
            match comm {
                b"NetworkManager" => found.network_manager = true,
                b"systemd-network" => found.networkd = true,
                b"iwd" => found.iwd = true,
                _ => {}
            }
        }
        found
    }
}

/// The four uids (real, effective, saved, filesystem) of `<proc_pid>/status` when its first
/// `Name:` line equals `comm` (the same process, even if the pid was reused after `comm` was
/// read) and its first `Uid:` line holds exactly four decimal values; else `None`. Only the
/// complete lines of the first [`MAX_PROC_STATUS_BYTES`] bytes are parsed. The `status`
/// content is used, not the owner of `/proc/<pid>`, which is root for any non-dumpable
/// process.
fn trusted_process_uids(proc_pid: &Path, comm: &[u8]) -> Option<[u32; 4]> {
    let status = read_complete_lines(&proc_pid.join("status"), MAX_PROC_STATUS_BYTES).ok()?;
    let mut lines = status.split(|&b| b == b'\n');
    let name = lines.clone().find_map(|line| line.strip_prefix(b"Name:"))?;
    if name.trim_ascii_start() != comm {
        return None;
    }
    let uid_line = lines.find_map(|line| line.strip_prefix(b"Uid:"))?;
    let text = std::str::from_utf8(uid_line).ok()?;
    let mut uids = [0_u32; 4];
    let mut fields = text.split_ascii_whitespace();
    for slot in &mut uids {
        *slot = parse_decimal_u32(fields.next()?)?;
    }
    fields.next().is_none().then_some(uids)
}

/// The uid of `user` from the complete lines of the first [`MAX_PASSWD_BYTES`] bytes of
/// `<root>/etc/passwd`. NSS is not used: it may block on a network directory (LDAP, SSSD)
/// and cannot follow a test root.
fn account_uid(root: &Path, user: &str) -> Option<u32> {
    let passwd = read_complete_lines(&root.join("etc/passwd"), MAX_PASSWD_BYTES).ok()?;
    passwd.split(|&b| b == b'\n').find_map(|line| {
        let mut fields = line.split(|&b| b == b':');
        if fields.next()? != user.as_bytes() {
            return None;
        }
        let _password_field = fields.next()?;
        parse_decimal_u32(std::str::from_utf8(fields.next()?).ok()?)
    })
}

/// A `u32` written with ASCII digits only (no sign, no space).
fn parse_decimal_u32(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// State of one installed rule file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleFileState {
    /// Byte-identical to the shipped copy, owned by the required owner, not group- or
    /// world-writable, in a directory with the same properties.
    Installed,
    /// The file (or its directory) does not exist.
    Missing,
    /// The content differs from the shipped copy (or is over [`MAX_RULE_FILE_BYTES`]).
    Modified,
    /// Wrong owner, group- or world-writable, a symbolic link or not a regular file.
    InsecurePermissions,
    /// The file could not be examined (for example `/etc/polkit-1/rules.d` is `0750` on Arch).
    Unreadable,
}

impl RuleFileState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Missing => "missing",
            Self::Modified => "modified",
            Self::InsecurePermissions => "insecure_permissions",
            Self::Unreadable => "unreadable",
        }
    }

    fn fails(self) -> bool {
        matches!(
            self,
            Self::Modified | Self::InsecurePermissions | Self::Unreadable
        )
    }
}

/// Seat-user access to `/dev/rfkill`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RfkillAccess {
    /// Tagged `uaccess` (or world-writable): the seat user can soft-block radios.
    Uaccess,
    /// Not tagged `uaccess` and not world-writable.
    Restricted,
    /// No rfkill device.
    Absent,
    /// Could not be determined.
    Unknown,
}

impl RfkillAccess {
    fn as_str(self) -> &'static str {
        match self {
            Self::Uaccess => "uaccess",
            Self::Restricted => "restricted",
            Self::Absent => "absent",
            Self::Unknown => "unknown",
        }
    }
}

/// Overall verdict of `network-lock status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkLockState {
    Installed,
    Missing,
    Modified,
    InsecurePermissions,
    Unreadable,
    /// The host has no NetworkManager (iwd-only or networkd): polkit does not govern Wi-Fi.
    UnsupportedBackend,
}

impl NetworkLockState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Missing => "missing",
            Self::Modified => "modified",
            Self::InsecurePermissions => "insecure_permissions",
            Self::Unreadable => "unreadable",
            Self::UnsupportedBackend => "unsupported_backend",
        }
    }
}

/// Filesystem seam of the status: everything is read below `root`; the rule files and their
/// directories must belong to `required_owner_uid` (0 in production).
#[derive(Debug, Clone)]
pub struct NetworkLockEnv {
    pub root: PathBuf,
    pub required_owner_uid: u32,
}

impl NetworkLockEnv {
    /// The real system: root `/`, files owned by root.
    #[must_use]
    pub fn system() -> Self {
        Self {
            root: PathBuf::from("/"),
            required_owner_uid: 0,
        }
    }
}

/// Result of `soos-admin network-lock status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkLockReport {
    pub state: NetworkLockState,
    pub backend: NetworkBackend,
    pub rules: RuleFileState,
    pub rfkill_rule: RuleFileState,
    pub rfkill: RfkillAccess,
    /// Fixed words, fixed order: `rfkill_uaccess`, `rfkill_reboot_pending`, `iwd_dbus_bypass`.
    pub warnings: Vec<&'static str>,
}

impl NetworkLockReport {
    /// `1` when a rule file is modified, insecure or could not be verified; else `0`.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        i32::from(self.rules.fails() || self.rfkill_rule.fails())
    }

    /// Pretty-printed JSON of the report.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| String::from("{}"))
    }

    /// Human-readable table (fixed vocabulary and the two production paths only).
    #[must_use]
    pub fn format_table(&self) -> String {
        let state = match self.state {
            NetworkLockState::UnsupportedBackend => {
                format!("unsupported_backend({})", self.backend.as_str())
            }
            other => other.as_str().to_string(),
        };
        let mut out = String::from("soos network lock (opt-in polkit Wi-Fi lock):\n");
        out.push_str(&format!("  State:        {state}\n"));
        out.push_str(&format!("  Backend:      {}\n", self.backend.as_str()));
        out.push_str(&format!(
            "  Polkit rule:  {}  (/{NETWORK_LOCK_RULES_DIR}/{NETWORK_LOCK_RULES_FILE})\n",
            self.rules.as_str()
        ));
        out.push_str(&format!(
            "  rfkill rule:  {}  (/{NETWORK_LOCK_RFKILL_DIR}/{NETWORK_LOCK_RFKILL_FILE})\n",
            self.rfkill_rule.as_str()
        ));
        out.push_str(&format!("  rfkill:       {}\n", self.rfkill.as_str()));
        for warning in &self.warnings {
            let text = match *warning {
                WARN_RFKILL_UACCESS => {
                    "the seat user can turn the radio off through /dev/rfkill \
                     (see `install --rfkill`)"
                }
                WARN_RFKILL_REBOOT_PENDING => {
                    "the rfkill rule was written after the last boot; reboot to apply it"
                }
                WARN_IWD_DBUS_BYPASS => {
                    "iwd runs next to NetworkManager; root and the netdev group can drive it \
                     without polkit"
                }
                _ => "",
            };
            out.push_str(&format!("  Warning:      {warning}: {text}\n"));
        }
        if self.state == NetworkLockState::UnsupportedBackend {
            out.push_str("  Note:         polkit does not govern Wi-Fi on this host; the rule has no effect\n");
        }
        if self.rules == RuleFileState::Unreadable || self.rfkill_rule == RuleFileState::Unreadable
        {
            out.push_str("  Note:         a rule file could not be examined; run as root (sudo)\n");
        }
        out
    }
}

/// Backend precedence: NetworkManager, then iwd, then networkd.
#[must_use]
pub fn detect_backend(services: RunningServices) -> NetworkBackend {
    if services.network_manager {
        NetworkBackend::NetworkManager
    } else if services.iwd {
        NetworkBackend::Iwd
    } else if services.networkd {
        NetworkBackend::Networkd
    } else {
        NetworkBackend::Unknown
    }
}

/// Read-only status of the network lock below `env.root`.
#[must_use]
pub fn network_lock_status(
    env: &NetworkLockEnv,
    probe: &dyn NetworkServiceProbe,
) -> NetworkLockReport {
    let rules = rule_file_state(
        env,
        NETWORK_LOCK_RULES_DIR,
        NETWORK_LOCK_RULES_FILE,
        NETWORK_LOCK_RULES,
    );
    let rfkill_rule = rule_file_state(
        env,
        NETWORK_LOCK_RFKILL_DIR,
        NETWORK_LOCK_RFKILL_FILE,
        NETWORK_LOCK_RFKILL_RULE,
    );
    let services = probe.running();
    let backend = detect_backend(services);
    let rfkill = rfkill_access(&env.root);

    let mut warnings = Vec::new();
    if rfkill == RfkillAccess::Uaccess {
        warnings.push(WARN_RFKILL_UACCESS);
    }
    if rfkill_rule == RuleFileState::Installed && rfkill_rule_newer_than_boot(&env.root) {
        warnings.push(WARN_RFKILL_REBOOT_PENDING);
    }
    if services.network_manager && services.iwd {
        warnings.push(WARN_IWD_DBUS_BYPASS);
    }

    let state = match rules {
        RuleFileState::InsecurePermissions => NetworkLockState::InsecurePermissions,
        RuleFileState::Modified => NetworkLockState::Modified,
        RuleFileState::Unreadable => NetworkLockState::Unreadable,
        _ if matches!(backend, NetworkBackend::Iwd | NetworkBackend::Networkd) => {
            NetworkLockState::UnsupportedBackend
        }
        RuleFileState::Installed => NetworkLockState::Installed,
        RuleFileState::Missing => NetworkLockState::Missing,
    };

    NetworkLockReport {
        state,
        backend,
        rules,
        rfkill_rule,
        rfkill,
        warnings,
    }
}

fn is_secure(uid: u32, mode: u32, owner: u32) -> bool {
    uid == owner && mode & 0o022 == 0
}

/// State of `<root>/<dir>/<file>` against the shipped `expected` content.
#[must_use]
pub fn rule_file_state(
    env: &NetworkLockEnv,
    dir: &str,
    file: &str,
    expected: &str,
) -> RuleFileState {
    let dir_path = match walk_dirs(&env.root, dir, env.required_owner_uid) {
        DirWalk::Secure(_, path) => path,
        DirWalk::Missing => return RuleFileState::Missing,
        DirWalk::NotRealDir(_) | DirWalk::Insecure(_) => return RuleFileState::InsecurePermissions,
        DirWalk::Error(_, _) => return RuleFileState::Unreadable,
    };
    let path = dir_path.join(file);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == ErrorKind::NotFound => return RuleFileState::Missing,
        Err(_) => return RuleFileState::Unreadable,
        Ok(meta) => {
            if !meta.file_type().is_file()
                || !is_secure(meta.uid(), meta.mode(), env.required_owner_uid)
            {
                return RuleFileState::InsecurePermissions;
            }
        }
    }
    match read_bounded(&path, MAX_RULE_FILE_BYTES) {
        Err(_) => RuleFileState::Unreadable,
        Ok(Bounded::TooLarge) => RuleFileState::Modified,
        Ok(Bounded::Data(bytes)) => {
            if Sha256::digest(&bytes) == Sha256::digest(expected.as_bytes()) {
                RuleFileState::Installed
            } else {
                RuleFileState::Modified
            }
        }
    }
}

/// Seat-user access to `<root>/dev/rfkill` from sysfs, the node mode and the udev database.
#[must_use]
pub fn rfkill_access(root: &Path) -> RfkillAccess {
    let dev = match read_bounded(&root.join("sys/class/misc/rfkill/dev"), MAX_SYSFS_DEV_BYTES) {
        Err(e) if e.kind() == ErrorKind::NotFound => return RfkillAccess::Absent,
        Err(_) | Ok(Bounded::TooLarge) => return RfkillAccess::Unknown,
        Ok(Bounded::Data(bytes)) => bytes,
    };
    let Some((major, minor)) = parse_major_minor(&dev) else {
        return RfkillAccess::Unknown;
    };
    if let Ok(meta) = fs::symlink_metadata(root.join("dev/rfkill")) {
        if meta.mode() & 0o002 != 0 {
            return RfkillAccess::Uaccess;
        }
    }
    let db_path = root.join(format!("run/udev/data/c{major}:{minor}"));
    let Ok(Bounded::Data(db)) = read_bounded(&db_path, MAX_UDEV_DB_BYTES) else {
        return RfkillAccess::Unknown;
    };
    let text = String::from_utf8_lossy(&db);
    let current: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix("Q:")).collect();
    let tags: Vec<&str> = if current.is_empty() {
        // systemd < 247 has no current-tag lines.
        text.lines().filter_map(|l| l.strip_prefix("G:")).collect()
    } else {
        current
    };
    if tags.contains(&"uaccess") {
        RfkillAccess::Uaccess
    } else {
        RfkillAccess::Restricted
    }
}

fn parse_major_minor(bytes: &[u8]) -> Option<(u32, u32)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    let (major, minor) = text.split_once(':')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// `true` when the installed rfkill rule changed after the last boot (`now - uptime`).
/// An unreadable or malformed uptime gives `false` (the warning is informational).
fn rfkill_rule_newer_than_boot(root: &Path) -> bool {
    let Ok(Bounded::Data(bytes)) = read_bounded(&root.join("proc/uptime"), MAX_UPTIME_BYTES) else {
        return false;
    };
    let Some(uptime) = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|t| t.split_whitespace().next())
        .and_then(|t| t.parse::<f64>().ok())
        .and_then(|secs| Duration::try_from_secs_f64(secs).ok())
    else {
        return false;
    };
    let Some(boot) = SystemTime::now().checked_sub(uptime) else {
        return false;
    };
    let path = root
        .join(NETWORK_LOCK_RFKILL_DIR)
        .join(NETWORK_LOCK_RFKILL_FILE);
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    let (Ok(secs), Ok(nanos)) = (
        u64::try_from(meta.ctime()),
        u32::try_from(meta.ctime_nsec()),
    ) else {
        return false;
    };
    UNIX_EPOCH
        .checked_add(Duration::new(secs, nanos))
        .is_some_and(|changed| changed > boot)
}

enum Bounded {
    Data(Vec<u8>),
    TooLarge,
}

/// Reads at most `cap` bytes of `path` (`O_NOFOLLOW | O_CLOEXEC`); longer is `TooLarge`.
fn read_bounded(path: &Path, cap: u64) -> io::Result<Bounded> {
    let bytes = read_at_most(path, cap.saturating_add(1))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > cap {
        Ok(Bounded::TooLarge)
    } else {
        Ok(Bounded::Data(bytes))
    }
}

/// At most `cap` bytes of `path` (`O_NOFOLLOW | O_CLOEXEC`). A file longer than `cap` is
/// cut to its first `cap` bytes, then after their last newline, so a line cut by the cap is
/// never parsed (no newline: nothing is kept).
fn read_complete_lines(path: &Path, cap: u64) -> io::Result<Vec<u8>> {
    let mut bytes = read_at_most(path, cap.saturating_add(1))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > cap {
        let keep = usize::try_from(cap).unwrap_or(usize::MAX);
        bytes.truncate(keep);
        let complete = bytes
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |last| last.saturating_add(1));
        bytes.truncate(complete);
    }
    Ok(bytes)
}

/// The first `cap` bytes (at most) of `path` (`O_NOFOLLOW | O_CLOEXEC`).
fn read_at_most(path: &Path, cap: u64) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let mut bytes = Vec::new();
    file.take(cap).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Outcome of checking every component of `<root>/<rel>`, in path order.
enum DirWalk {
    /// Every component is a real directory owned by the required owner and not group- or
    /// world-writable: the `lstat` of the last component and its path.
    Secure(FileStat, PathBuf),
    /// A component does not exist.
    Missing,
    /// A component is a symbolic link or not a directory.
    NotRealDir(PathBuf),
    /// A component has another owner or is group- or world-writable.
    Insecure(PathBuf),
    /// A component could not be examined.
    Error(PathBuf, Errno),
}

/// Checks every component of `<root>/<rel>` below `root` (the root itself is not checked):
/// shared by `status` and by install/uninstall so both apply the same rule (GitHub #370).
fn walk_dirs(root: &Path, rel: &str, owner: u32) -> DirWalk {
    let mut path = root.to_path_buf();
    let mut last: Option<FileStat> = None;
    for part in Path::new(rel).components() {
        path.push(part);
        let st = match lstat(&path) {
            Ok(st) => st,
            Err(Errno::ENOENT) => return DirWalk::Missing,
            Err(e) => return DirWalk::Error(path, e),
        };
        if st.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return DirWalk::NotRealDir(path);
        }
        if !is_secure(st.st_uid, st.st_mode, owner) {
            return DirWalk::Insecure(path);
        }
        last = Some(st);
    }
    match last {
        Some(st) => DirWalk::Secure(st, path),
        // An empty `rel` names no directory below the root.
        None => DirWalk::NotRealDir(path),
    }
}

// ---------------------------------------------------------------------------------------
// Install / uninstall
// ---------------------------------------------------------------------------------------

/// Options of `network-lock install`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InstallOptions {
    /// Also install the rfkill udev rule (off by default).
    pub rfkill: bool,
}

/// What install or uninstall did to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    Written,
    Removed,
    Absent,
}

/// Per-file result of install or uninstall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    pub rules: FileAction,
    pub rfkill_rule: FileAction,
}

/// `true` when install/uninstall may act on `root`: a staging directory, or `/` as euid 0.
/// An empty root is refused. This is a usability guard; file permissions are the control.
#[must_use]
pub fn may_modify_root(root: &Path, euid: u32) -> bool {
    if root.as_os_str().is_empty() {
        return false;
    }
    let is_system_root = root
        .components()
        .all(|c| matches!(c, Component::RootDir | Component::CurDir));
    !is_system_root || euid == 0
}

/// A target directory whose every component below the root was checked, held open.
struct CheckedDir {
    file: File,
    path: PathBuf,
}

fn lock_error(message: String) -> AdminCliError {
    AdminCliError::NetworkLock(message)
}

fn same_inode(a: &FileStat, b: &FileStat) -> bool {
    a.st_dev == b.st_dev && a.st_ino == b.st_ino
}

/// Checks every component of `<root>/<rel>`: a real directory (not a symbolic link), owned by
/// `owner`, not group- or world-writable. `Ok(None)` when a component does not exist.
fn checked_dir(root: &Path, rel: &str, owner: u32) -> Result<Option<CheckedDir>, AdminCliError> {
    let (expected, path) = match walk_dirs(root, rel, owner) {
        DirWalk::Secure(st, path) => (st, path),
        DirWalk::Missing => return Ok(None),
        DirWalk::Error(path, e) => {
            return Err(lock_error(format!(
                "cannot examine {}: {e}",
                path.display()
            )));
        }
        DirWalk::NotRealDir(path) => {
            return Err(lock_error(format!(
                "{} is not a real directory (symbolic links are refused)",
                path.display()
            )));
        }
        DirWalk::Insecure(path) => {
            return Err(lock_error(format!(
                "{} must be owned by uid {owner} and not writable by group or others",
                path.display()
            )));
        }
    };
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|e| lock_error(format!("cannot open {}: {e}", path.display())))?;
    let opened = fstat(file.as_raw_fd())
        .map_err(|e| lock_error(format!("cannot examine {}: {e}", path.display())))?;
    if !same_inode(&opened, &expected) {
        return Err(lock_error(format!(
            "{} changed while it was checked",
            path.display()
        )));
    }
    Ok(Some(CheckedDir { file, path }))
}

/// Type of `name` inside `dir`, without following a symbolic link (`None` when absent).
fn entry_type(dir: &CheckedDir, name: &str) -> Result<Option<u32>, AdminCliError> {
    match fstatat(
        Some(dir.file.as_raw_fd()),
        name,
        AtFlags::AT_SYMLINK_NOFOLLOW,
    ) {
        Ok(st) => Ok(Some(st.st_mode & libc::S_IFMT)),
        Err(Errno::ENOENT) => Ok(None),
        Err(e) => Err(lock_error(format!(
            "cannot examine {}: {e}",
            dir.path.join(name).display()
        ))),
    }
}

/// Atomically replaces `<dir>/<name>` with `content` (mode 0644, root:root when `chown_root`).
fn write_rule_file(
    dir: &CheckedDir,
    name: &str,
    content: &str,
    chown_root: bool,
) -> Result<FileAction, AdminCliError> {
    let target = dir.path.join(name);
    match entry_type(dir, name)? {
        None => {}
        Some(kind) if kind == libc::S_IFREG => {}
        Some(_) => {
            return Err(lock_error(format!(
                "{} is a symbolic link or not a regular file; refusing to replace it",
                target.display()
            )));
        }
    }
    // The temporary name starts with a dot and does not end in `.rules`: neither polkitd nor
    // udev loads it.
    let tmp_name = format!(".{name}.{}.tmp", std::process::id());
    let tmp_path = dir.path.join(&tmp_name);
    let mut tmp = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&tmp_path)
        .map_err(|e| lock_error(format!("cannot create {}: {e}", tmp_path.display())))?;
    // Prove that the file created by path is the one inside the verified directory before
    // using any descriptor-relative call on that name (`openat` would hand back a raw
    // descriptor, which this crate cannot wrap without `unsafe`).
    let dirfd = dir.file.as_raw_fd();
    let ours = fstat(tmp.as_raw_fd()).ok();
    let there = fstatat(Some(dirfd), tmp_name.as_str(), AtFlags::AT_SYMLINK_NOFOLLOW).ok();
    match (ours, there) {
        (Some(a), Some(b)) if same_inode(&a, &b) => {}
        _ => {
            return Err(lock_error(format!(
                "{} changed while the rule was written",
                dir.path.display()
            )));
        }
    }
    let written = write_and_publish(&mut tmp, dir, &tmp_name, name, content, chown_root);
    if written.is_err() {
        let _ = unlinkat(Some(dirfd), tmp_name.as_str(), UnlinkatFlags::NoRemoveDir);
    }
    written.map_err(|e| lock_error(format!("cannot write {}: {e}", target.display())))?;
    Ok(FileAction::Written)
}

fn write_and_publish(
    tmp: &mut File,
    dir: &CheckedDir,
    tmp_name: &str,
    name: &str,
    content: &str,
    chown_root: bool,
) -> io::Result<()> {
    tmp.write_all(content.as_bytes())?;
    tmp.set_permissions(fs::Permissions::from_mode(0o644))?;
    if chown_root {
        std::os::unix::fs::fchown(&*tmp, Some(0), Some(0))?;
    }
    tmp.sync_all()?;
    let dirfd = dir.file.as_raw_fd();
    renameat(Some(dirfd), tmp_name, Some(dirfd), name).map_err(io::Error::from)?;
    dir.file.sync_all()
}

/// Removes `<dir>/<name>` (a symbolic link itself, never its target).
fn remove_rule_file(dir: &CheckedDir, name: &str) -> Result<FileAction, AdminCliError> {
    let target = dir.path.join(name);
    match entry_type(dir, name)? {
        None => return Ok(FileAction::Absent),
        Some(kind) if kind == libc::S_IFDIR => {
            return Err(lock_error(format!(
                "{} is a directory; refusing to remove it",
                target.display()
            )));
        }
        Some(_) => {}
    }
    unlinkat(Some(dir.file.as_raw_fd()), name, UnlinkatFlags::NoRemoveDir)
        .map_err(|e| lock_error(format!("cannot remove {}: {e}", target.display())))?;
    dir.file
        .sync_all()
        .map_err(|e| lock_error(format!("cannot flush {}: {e}", dir.path.display())))?;
    Ok(FileAction::Removed)
}

/// `true` when `entry` is `.<name>.<pid>.tmp` (1 to 10 ASCII digits without a leading zero,
/// pid in `1..=i32::MAX`, as `write_rule_file` names it) and no process has that pid: the
/// temp file of an interrupted install (GitHub #370).
fn is_stale_temp_name(entry: &str, name: &str) -> bool {
    let Some(pid) = entry
        .strip_prefix('.')
        .and_then(|rest| rest.strip_prefix(name))
        .and_then(|rest| rest.strip_prefix('.'))
        .and_then(|rest| rest.strip_suffix(".tmp"))
    else {
        return false;
    };
    if pid.len() > 10 || pid.starts_with('0') {
        return false;
    }
    let Some(pid) = parse_decimal_u32(pid).and_then(|p| i32::try_from(p).ok()) else {
        return false;
    };
    if pid <= 0 || u32::try_from(pid).ok() == Some(std::process::id()) {
        return false;
    }
    // `ESRCH`: no such process. `Ok` or `EPERM`: it runs (perhaps a concurrent install).
    matches!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
        Err(Errno::ESRCH)
    )
}

/// Removes the stale temp files of `name` (regular files only) from the verified `dir`,
/// examining at most [`MAX_RULES_DIR_ENTRIES`] entries; removals are descriptor-relative.
fn remove_stale_temp_files(dir: &CheckedDir, name: &str) -> Result<(), AdminCliError> {
    let entries = fs::read_dir(&dir.path)
        .map_err(|e| lock_error(format!("cannot list {}: {e}", dir.path.display())))?;
    let mut removed = false;
    for entry in entries.take(MAX_RULES_DIR_ENTRIES) {
        let entry =
            entry.map_err(|e| lock_error(format!("cannot list {}: {e}", dir.path.display())))?;
        let file_name = entry.file_name();
        let Some(entry_name) = file_name.to_str() else {
            continue;
        };
        if !is_stale_temp_name(entry_name, name) {
            continue;
        }
        if entry_type(dir, entry_name)? != Some(libc::S_IFREG) {
            continue;
        }
        match unlinkat(
            Some(dir.file.as_raw_fd()),
            entry_name,
            UnlinkatFlags::NoRemoveDir,
        ) {
            Ok(()) => removed = true,
            Err(Errno::ENOENT) => {}
            Err(e) => {
                return Err(lock_error(format!(
                    "cannot remove {}: {e}",
                    dir.path.join(entry_name).display()
                )));
            }
        }
    }
    if removed {
        dir.file
            .sync_all()
            .map_err(|e| lock_error(format!("cannot flush {}: {e}", dir.path.display())))?;
    }
    Ok(())
}

fn required_owner(chown_root: bool) -> u32 {
    if chown_root {
        0
    } else {
        nix::unistd::geteuid().as_raw()
    }
}

/// Writes the polkit rule below `root`; with `options.rfkill` also the udev rule, without it
/// removes a soos rfkill rule (the files match the options). Every directory is validated
/// before the first write. `chown_root` (the caller is root) makes the files `root:root` and
/// requires root-owned directories; otherwise they must belong to the caller.
pub fn install_network_lock(
    root: &Path,
    options: InstallOptions,
    chown_root: bool,
) -> Result<InstallReport, AdminCliError> {
    let owner = required_owner(chown_root);
    let rules_dir = checked_dir(root, NETWORK_LOCK_RULES_DIR, owner)?.ok_or_else(|| {
        lock_error(format!(
            "{} does not exist; is polkit installed?",
            root.join(NETWORK_LOCK_RULES_DIR).display()
        ))
    })?;
    let rfkill_dir = checked_dir(root, NETWORK_LOCK_RFKILL_DIR, owner)?;
    if options.rfkill && rfkill_dir.is_none() {
        return Err(lock_error(format!(
            "{} does not exist; is udev installed?",
            root.join(NETWORK_LOCK_RFKILL_DIR).display()
        )));
    }
    remove_stale_temp_files(&rules_dir, NETWORK_LOCK_RULES_FILE)?;
    if let Some(dir) = &rfkill_dir {
        remove_stale_temp_files(dir, NETWORK_LOCK_RFKILL_FILE)?;
    }
    let rules = write_rule_file(
        &rules_dir,
        NETWORK_LOCK_RULES_FILE,
        NETWORK_LOCK_RULES,
        chown_root,
    )?;
    let rfkill_rule = match rfkill_dir {
        None => FileAction::Absent,
        Some(dir) if options.rfkill => write_rule_file(
            &dir,
            NETWORK_LOCK_RFKILL_FILE,
            NETWORK_LOCK_RFKILL_RULE,
            chown_root,
        )?,
        Some(dir) => remove_rule_file(&dir, NETWORK_LOCK_RFKILL_FILE)?,
    };
    Ok(InstallReport { rules, rfkill_rule })
}

/// Removes both rule files below `root` (directories are kept).
pub fn uninstall_network_lock(root: &Path) -> Result<InstallReport, AdminCliError> {
    let owner = required_owner(nix::unistd::geteuid().is_root());
    let rules = match checked_dir(root, NETWORK_LOCK_RULES_DIR, owner)? {
        Some(dir) => {
            remove_stale_temp_files(&dir, NETWORK_LOCK_RULES_FILE)?;
            remove_rule_file(&dir, NETWORK_LOCK_RULES_FILE)?
        }
        None => FileAction::Absent,
    };
    let rfkill_rule = match checked_dir(root, NETWORK_LOCK_RFKILL_DIR, owner)? {
        Some(dir) => {
            remove_stale_temp_files(&dir, NETWORK_LOCK_RFKILL_FILE)?;
            remove_rule_file(&dir, NETWORK_LOCK_RFKILL_FILE)?
        }
        None => FileAction::Absent,
    };
    Ok(InstallReport { rules, rfkill_rule })
}
