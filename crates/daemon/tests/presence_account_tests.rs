//! Contract tests of GitHub #323 (owner answer Q1, 2026-10-02) for the presence account
//! guard: a presence unlock respects `pam_faillock` and account / password expiry; locked,
//! expired **or undeterminable** ⇒ no unlock (matrix PAU22–PAU27).
//!
//! Every source is a tempdir tree (`faillock.conf`, `pam.d/`, tally files built byte for
//! byte as Linux-PAM `struct tally`, `shadow`) and the wall clock is injected; nothing reads
//! the real `/etc` or `/run`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use proptest::prelude::*;

use soos_daemon::presence::account::{
    decode_tally_records, faillock_denies, find_shadow_entry, parse_faillock_conf,
    read_tally_records, scan_pam_faillock_options, shadow_refusal, AccountGuard, AccountRefusal,
    AccountState, FaillockPolicy, ShadowEntry, SystemAccountGuard, TallyRecord, UserName,
};
use soos_daemon::presence::{
    MAX_FAILLOCK_CONF_BYTES, MAX_FAILLOCK_TIME_INTERVAL, MAX_PAM_DIR_ENTRIES, MAX_PAM_FILE_BYTES,
    MAX_SHADOW_BYTES, MAX_SHADOW_LINES, MAX_TALLY_BYTES, MAX_USER_NAME_LEN, TALLY_RECORD_BYTES,
    TALLY_STATUS_VALID,
};
use soos_daemon::DaemonError;

const DAY_S: u64 = 86_400;
/// Wall clock of the guard tests: day 20 000 (2024-10-04), 12:00 UTC.
const NOW_S: u64 = 20_000 * DAY_S + 12 * 3600;
const TODAY: i64 = 20_000;

fn now_realtime() -> Result<u64, DaemonError> {
    Ok(NOW_S)
}

fn broken_realtime() -> Result<u64, DaemonError> {
    Err(DaemonError::Clock("injected realtime failure".into()))
}

fn name(raw: &str) -> UserName {
    UserName::parse(raw).unwrap()
}

/// Source bytes written into every tally record; they must never be returned.
const SECRET_SOURCE: &[u8] = b"SECRET-SOURCE-TTY";

/// One `struct tally { char source[52]; uint16_t reserved; uint16_t status; uint64_t time; }`
/// in native endianness.
fn tally_record(status: u16, time: u64) -> [u8; 64] {
    let mut record = [0u8; 64];
    record[..SECRET_SOURCE.len()].copy_from_slice(SECRET_SOURCE);
    record[52..54].copy_from_slice(&0xBEEF_u16.to_ne_bytes());
    record[54..56].copy_from_slice(&status.to_ne_bytes());
    record[56..64].copy_from_slice(&time.to_ne_bytes());
    record
}

fn tally_bytes(records: &[(u16, u64)]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|(status, time)| tally_record(*status, *time))
        .collect()
}

fn valid(time: u64) -> TallyRecord {
    TallyRecord {
        status: TALLY_STATUS_VALID,
        time,
    }
}

fn policy(deny: u16, fail_interval: u32, unlock_time: u32) -> FaillockPolicy {
    FaillockPolicy {
        deny,
        fail_interval,
        unlock_time,
        root_unlock_time: unlock_time,
        ..FaillockPolicy::default()
    }
}

/// A hermetic account tree: `etc/security/faillock.conf` (optional), a vendor conf path,
/// a default tally dir, three PAM directories and a shadow file.
struct Accounts {
    temp: tempfile::TempDir,
}

impl Accounts {
    fn new() -> Self {
        let accounts = Self {
            temp: tempfile::tempdir().unwrap(),
        };
        fs::create_dir_all(accounts.default_tally_dir()).unwrap();
        fs::create_dir_all(accounts.conf_tally_dir()).unwrap();
        // Tally directories are never group/other-writable (constraint 13), whatever the
        // umask of the test runner.
        for dir in [accounts.default_tally_dir(), accounts.conf_tally_dir()] {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
        }
        for dir in accounts.pam_dirs() {
            fs::create_dir_all(dir).unwrap();
        }
        fs::create_dir_all(accounts.path("etc/security")).unwrap();
        fs::create_dir_all(accounts.path("usr/etc/security")).unwrap();
        accounts.write_conf(&format!("dir = {}\n", accounts.conf_tally_dir().display()));
        accounts.write_shadow("alice:$6$secrethash:19900:0:99999:7:::\n");
        accounts
    }
    fn path(&self, rel: &str) -> PathBuf {
        self.temp.path().join(rel)
    }
    fn conf(&self) -> PathBuf {
        self.path("etc/security/faillock.conf")
    }
    fn vendor_conf(&self) -> PathBuf {
        self.path("usr/etc/security/faillock.conf")
    }
    fn default_tally_dir(&self) -> PathBuf {
        self.path("run/faillock")
    }
    fn conf_tally_dir(&self) -> PathBuf {
        self.path("var/faillock")
    }
    fn pam_dirs(&self) -> Vec<PathBuf> {
        vec![
            self.path("etc/pam.d"),
            self.path("usr/lib/pam.d"),
            self.path("usr/etc/pam.d"),
        ]
    }
    fn shadow(&self) -> PathBuf {
        self.path("etc/shadow")
    }
    fn write_conf(&self, content: &str) {
        fs::write(self.conf(), content).unwrap();
    }
    fn write_shadow(&self, content: &str) {
        fs::write(self.shadow(), content).unwrap();
    }
    fn write_tally(&self, dir: &Path, user: &str, records: &[(u16, u64)]) {
        fs::write(dir.join(user), tally_bytes(records)).unwrap();
    }
    fn guard(&self) -> SystemAccountGuard {
        SystemAccountGuard::new()
            .with_faillock_conf(self.conf())
            .with_vendor_faillock_conf(self.vendor_conf())
            .with_default_faillock_dir(self.default_tally_dir())
            .with_pam_dirs(self.pam_dirs())
            .with_pam_conf(self.path("etc/pam.conf"))
            .with_shadow(self.shadow())
            .with_realtime_fn(now_realtime)
    }
    fn check(&self, user: &str) -> AccountState {
        self.guard().check(&name(user), 1000)
    }
}

fn refused(kind: AccountRefusal) -> AccountState {
    AccountState::Refused(kind)
}

// ---------------------------------------------------------------------------------------
// PAU27 — user names
// ---------------------------------------------------------------------------------------

/// PAU27: accepted and refused logind `Name` values (they become a tally path component).
#[test]
fn test_pau_user_name_parse_table() {
    let max = "a".repeat(MAX_USER_NAME_LEN);
    for accepted in ["alice", "a.b-c_d", "host$", "A1", "_svc", max.as_str()] {
        let parsed = UserName::parse(accepted).unwrap_or_else(|| panic!("`{accepted}` must parse"));
        assert_eq!(parsed.as_str(), accepted);
    }
    let too_long = "a".repeat(MAX_USER_NAME_LEN + 1);
    for rejected in [
        "",
        ".",
        "..",
        "-x",
        ".x",
        "a/b",
        "../x",
        "a$b",
        "a b",
        "a:b",
        "a\nb",
        "é",
        "ünïcode",
        too_long.as_str(),
    ] {
        assert!(
            UserName::parse(rejected).is_none(),
            "`{rejected:?}` must be refused"
        );
    }
}

/// PAU27: UID 0 is always `RootAccount`, before any file is read.
#[test]
fn test_pau_root_account_is_refused_first() {
    let accounts = Accounts::new();
    fs::remove_file(accounts.shadow()).unwrap();
    assert_eq!(
        accounts.guard().check(&name("root"), 0),
        refused(AccountRefusal::RootAccount)
    );
    let broken = accounts.guard().with_realtime_fn(broken_realtime);
    assert_eq!(
        broken.check(&name("root"), 0),
        refused(AccountRefusal::RootAccount),
        "the root rule comes before the clock"
    );
}

// ---------------------------------------------------------------------------------------
// Guard nominal path and order
// ---------------------------------------------------------------------------------------

/// A healthy account with no failure is usable.
#[test]
fn test_pau_healthy_account_is_usable() {
    let accounts = Accounts::new();
    assert_eq!(accounts.check("alice"), AccountState::Usable);
}

/// A realtime clock failure is `Undeterminable`.
#[test]
fn test_pau_realtime_failure_is_undeterminable() {
    let accounts = Accounts::new();
    let guard = accounts.guard().with_realtime_fn(broken_realtime);
    assert_eq!(
        guard.check(&name("alice"), 1000),
        refused(AccountRefusal::Undeterminable)
    );
}

/// The guard never writes anything: every source is byte-identical after a check.
#[test]
fn test_pau_guard_never_writes_its_sources() {
    let accounts = Accounts::new();
    let now = NOW_S;
    accounts.write_tally(
        &accounts.conf_tally_dir(),
        "alice",
        &[(1, now - 10), (1, now - 20), (1, now - 30)],
    );
    let tally = accounts.conf_tally_dir().join("alice");
    let before = (
        fs::read(&tally).unwrap(),
        fs::read(accounts.shadow()).unwrap(),
        fs::read(accounts.conf()).unwrap(),
    );
    let mtime = fs::metadata(&tally).unwrap().modified().unwrap();
    assert_eq!(accounts.check("alice"), refused(AccountRefusal::Faillocked));
    assert_eq!(
        before,
        (
            fs::read(&tally).unwrap(),
            fs::read(accounts.shadow()).unwrap(),
            fs::read(accounts.conf()).unwrap(),
        ),
        "no tally reset, no write"
    );
    assert_eq!(fs::metadata(&tally).unwrap().modified().unwrap(), mtime);
}

// ---------------------------------------------------------------------------------------
// PAU22 — faillock.conf
// ---------------------------------------------------------------------------------------

/// PAU22: the built-in defaults are `pam_faillock.c`'s.
#[test]
fn test_pau_faillock_defaults() {
    let defaults = FaillockPolicy::default();
    assert_eq!(defaults.dir, PathBuf::from("/run/faillock"));
    assert_eq!(defaults.deny, 3);
    assert_eq!(defaults.fail_interval, 900);
    assert_eq!(defaults.unlock_time, 600);
    assert_eq!(defaults.root_unlock_time, 600);
    assert_eq!(defaults.admin_group, None);
    assert!(!defaults.even_deny_root);
    assert_eq!(parse_faillock_conf(b""), Some(defaults.clone()));
    assert_eq!(
        parse_faillock_conf(b"# all comments\n\n   \n#deny = 1\n"),
        Some(defaults)
    );
}

/// PAU22: `name value`, `name=value`, `name = value`, comments and blank lines.
#[test]
fn test_pau_faillock_conf_grammar() {
    for line in [
        "deny = 5\n",
        "deny=5\n",
        "deny 5\n",
        "  deny\t=  5   # trailing comment\n",
        "deny = 5",
    ] {
        let parsed =
            parse_faillock_conf(line.as_bytes()).unwrap_or_else(|| panic!("`{line}` must parse"));
        assert_eq!(parsed.deny, 5, "`{line}`");
    }
    let parsed = parse_faillock_conf(
        b"dir = /var/lib/faillock\nfail_interval = 60\nunlock_time = never\nadmin_group = wheel\n\
          even_deny_root\naudit\nsilent\nno_log_info\nlocal_users_only\nnodelay\n",
    )
    .unwrap();
    assert_eq!(parsed.dir, PathBuf::from("/var/lib/faillock"));
    assert_eq!(parsed.fail_interval, 60);
    assert_eq!(parsed.unlock_time, 0, "`never` is 0");
    assert_eq!(
        parsed.root_unlock_time, 0,
        "root_unlock_time defaults to unlock_time"
    );
    assert_eq!(parsed.admin_group.as_deref(), Some("wheel"));
    assert!(parsed.even_deny_root);
}

/// PAU22: `root_unlock_time` defaults to `unlock_time` whatever the key order.
#[test]
fn test_pau_root_unlock_time_defaults_to_unlock_time() {
    assert_eq!(
        parse_faillock_conf(b"unlock_time = 100\n")
            .unwrap()
            .root_unlock_time,
        100
    );
    assert_eq!(
        parse_faillock_conf(b"unlock_time = 100\nroot_unlock_time = 50\n")
            .unwrap()
            .root_unlock_time,
        50
    );
    assert_eq!(
        parse_faillock_conf(b"root_unlock_time = 50\nunlock_time = 100\n")
            .unwrap()
            .root_unlock_time,
        50
    );
    assert_eq!(
        parse_faillock_conf(b"root_unlock_time = never\n")
            .unwrap()
            .root_unlock_time,
        0
    );
}

/// PAU22: the bound itself (604800 s) is accepted.
#[test]
fn test_pau_faillock_time_bound_is_accepted() {
    let content = format!(
        "fail_interval = {m}\nunlock_time = {m}\nroot_unlock_time = {m}\ndeny = 65535\n",
        m = MAX_FAILLOCK_TIME_INTERVAL
    );
    let parsed = parse_faillock_conf(content.as_bytes()).unwrap();
    assert_eq!(u64::from(parsed.fail_interval), MAX_FAILLOCK_TIME_INTERVAL);
    assert_eq!(parsed.deny, 65_535);
}

/// PAU22: stricter than PAM: unknown key, malformed / out-of-range value, relative or `..`
/// dir, non-UTF-8 or oversized file ⇒ `Undeterminable` (`None`).
#[test]
fn test_pau_faillock_conf_invalid_inputs_are_undeterminable() {
    let long_dir = format!("dir = /{}\n", "d".repeat(4096));
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("unknown key", b"denyy = 3\n".to_vec()),
        ("unknown flag", b"lenient\n".to_vec()),
        (
            "fail_interval too large",
            b"fail_interval = 604801\n".to_vec(),
        ),
        ("unlock_time too large", b"unlock_time = 604801\n".to_vec()),
        (
            "root_unlock_time too large",
            b"root_unlock_time = 604801\n".to_vec(),
        ),
        ("fail_interval never", b"fail_interval = never\n".to_vec()),
        ("deny too large", b"deny = 65536\n".to_vec()),
        ("deny negative", b"deny = -1\n".to_vec()),
        ("deny not numeric", b"deny = three\n".to_vec()),
        ("deny without value", b"deny\n".to_vec()),
        ("deny hex", b"deny = 0x3\n".to_vec()),
        ("unlock_time negative", b"unlock_time = -5\n".to_vec()),
        ("relative dir", b"dir = run/faillock\n".to_vec()),
        ("dotdot dir", b"dir = /run/../etc\n".to_vec()),
        ("empty dir", b"dir =\n".to_vec()),
        ("dir too long", long_dir.into_bytes()),
        ("empty admin_group", b"admin_group =\n".to_vec()),
        ("non utf-8", b"deny = 3\n\xff\xfe\n".to_vec()),
        (
            "oversized",
            format!("#{}\n", "x".repeat(MAX_FAILLOCK_CONF_BYTES)).into_bytes(),
        ),
    ];
    for (label, content) in cases {
        assert_eq!(
            parse_faillock_conf(&content),
            None,
            "{label} must be undeterminable"
        );
    }
    let at_bound = vec![b'#'; MAX_FAILLOCK_CONF_BYTES];
    assert!(
        parse_faillock_conf(&at_bound).is_some(),
        "exactly MAX_FAILLOCK_CONF_BYTES is accepted"
    );
}

/// PAU22: the `/etc` file wins over the vendor file when it exists.
#[test]
fn test_pau_etc_faillock_conf_wins_over_vendor() {
    let accounts = Accounts::new();
    accounts.write_conf(&format!(
        "dir = {}\ndeny = 10\n",
        accounts.conf_tally_dir().display()
    ));
    fs::write(accounts.vendor_conf(), "deny = 1\n").unwrap();
    accounts.write_tally(
        &accounts.conf_tally_dir(),
        "alice",
        &[(1, NOW_S - 10), (1, NOW_S - 20), (1, NOW_S - 30)],
    );
    assert_eq!(
        accounts.check("alice"),
        AccountState::Usable,
        "deny = 10 from /etc applies; the vendor file is not read"
    );
}

/// PAU22 (F9): without `/etc` the tally is evaluated under both the defaults and the
/// vendor policy; vendor `deny = 10` with 3 failures is refused by the defaults.
#[test]
fn test_pau_vendor_conf_is_evaluated_with_the_defaults_strictest_wins() {
    let accounts = Accounts::new();
    fs::remove_file(accounts.conf()).unwrap();
    fs::write(accounts.vendor_conf(), "deny = 10\n").unwrap();
    accounts.write_tally(
        &accounts.default_tally_dir(),
        "alice",
        &[(1, NOW_S - 10), (1, NOW_S - 20), (1, NOW_S - 30)],
    );
    assert_eq!(accounts.check("alice"), refused(AccountRefusal::Faillocked));

    // And the reverse: vendor stricter than the defaults (deny = 1, own dir).
    let accounts = Accounts::new();
    fs::remove_file(accounts.conf()).unwrap();
    fs::write(
        accounts.vendor_conf(),
        format!("dir = {}\ndeny = 1\n", accounts.conf_tally_dir().display()),
    )
    .unwrap();
    accounts.write_tally(&accounts.conf_tally_dir(), "alice", &[(1, NOW_S - 10)]);
    assert_eq!(accounts.check("alice"), refused(AccountRefusal::Faillocked));
}

/// PAU22: without any file the built-in defaults apply (default tally dir).
#[test]
fn test_pau_absent_conf_files_use_the_defaults() {
    let accounts = Accounts::new();
    fs::remove_file(accounts.conf()).unwrap();
    assert_eq!(accounts.check("alice"), AccountState::Usable);
    accounts.write_tally(
        &accounts.default_tally_dir(),
        "alice",
        &[(1, NOW_S - 10), (1, NOW_S - 20)],
    );
    assert_eq!(accounts.check("alice"), AccountState::Usable, "2 < deny 3");
    accounts.write_tally(
        &accounts.default_tally_dir(),
        "alice",
        &[(1, NOW_S - 10), (1, NOW_S - 20), (1, NOW_S - 30)],
    );
    assert_eq!(accounts.check("alice"), refused(AccountRefusal::Faillocked));
}

/// PAU22: an unreadable conf file (here a directory) or an invalid one is `Undeterminable`.
#[test]
fn test_pau_unreadable_or_invalid_conf_is_undeterminable() {
    let accounts = Accounts::new();
    fs::remove_file(accounts.conf()).unwrap();
    fs::create_dir(accounts.conf()).unwrap();
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable)
    );

    let accounts = Accounts::new();
    fs::remove_file(accounts.conf()).unwrap();
    fs::create_dir(accounts.vendor_conf()).unwrap();
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "a vendor read error is undeterminable"
    );

    let accounts = Accounts::new();
    accounts.write_conf("deny = lots\n");
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable)
    );
}

// ---------------------------------------------------------------------------------------
// PAU23 — tally reading
// ---------------------------------------------------------------------------------------

/// PAU23: records decode byte for byte (native endian, status at 54, time at 56); the
/// source bytes are never returned.
#[test]
fn test_pau_tally_records_decode_like_struct_tally() {
    assert_eq!(TALLY_RECORD_BYTES, 64);
    let bytes = tally_bytes(&[(1, 1_700_000_000), (0, 42), (3, u64::MAX)]);
    let records = decode_tally_records(&bytes).unwrap();
    assert_eq!(
        records,
        vec![
            TallyRecord {
                status: 1,
                time: 1_700_000_000
            },
            TallyRecord {
                status: 0,
                time: 42
            },
            TallyRecord {
                status: 3,
                time: u64::MAX
            },
        ]
    );
    let shown = format!("{records:?}");
    assert!(
        !shown.contains("SECRET"),
        "source bytes are never kept: {shown}"
    );
    assert_eq!(decode_tally_records(&[]), Some(Vec::new()));
}

/// PAU23: a size that is not a multiple of 64 or above `MAX_TALLY_BYTES` is `None`.
#[test]
fn test_pau_tally_size_bounds() {
    for len in [1usize, 63, 65, 127, 1000] {
        assert_eq!(decode_tally_records(&vec![0u8; len]), None, "{len} bytes");
    }
    let at_bound = vec![0u8; MAX_TALLY_BYTES];
    assert_eq!(decode_tally_records(&at_bound).unwrap().len(), 1088);
    assert_eq!(
        decode_tally_records(&vec![0u8; MAX_TALLY_BYTES + TALLY_RECORD_BYTES]),
        None
    );
}

/// PAU23: a missing tally file is zero records (not locked, as PAM).
#[test]
fn test_pau_missing_tally_file_is_zero_records() {
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(
        read_tally_records(&temp.path().join("alice")),
        Some(Vec::new())
    );
}

/// PAU23: a regular tally file is read.
#[test]
fn test_pau_regular_tally_file_is_read() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("alice");
    fs::write(&path, tally_bytes(&[(1, 10), (1, 20)])).unwrap();
    assert_eq!(read_tally_records(&path), Some(vec![valid(10), valid(20)]));
}

/// PAU23: a symlink (`O_NOFOLLOW`), a FIFO, a directory or a bad size is `None`.
#[test]
fn test_pau_non_regular_tally_paths_are_undeterminable() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    fs::write(&target, tally_bytes(&[(1, 10)])).unwrap();
    let link = temp.path().join("link");
    symlink(&target, &link).unwrap();
    assert_eq!(read_tally_records(&link), None, "symlink");

    let fifo = temp.path().join("fifo");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).unwrap();
    assert_eq!(read_tally_records(&fifo), None, "FIFO (must not block)");

    let dir = temp.path().join("dir");
    fs::create_dir(&dir).unwrap();
    assert_eq!(read_tally_records(&dir), None, "directory");

    let odd = temp.path().join("odd");
    fs::write(&odd, vec![0u8; 65]).unwrap();
    assert_eq!(read_tally_records(&odd), None, "size not a multiple of 64");

    let big = temp.path().join("big");
    fs::write(&big, vec![0u8; MAX_TALLY_BYTES + TALLY_RECORD_BYTES]).unwrap();
    assert_eq!(read_tally_records(&big), None, "above MAX_TALLY_BYTES");
}

/// PAU23: `EACCES` is `Undeterminable` (PAM would say "not locked"). Skipped when the test
/// runs as root, which bypasses file modes.
#[test]
fn test_pau_permission_denied_tally_is_undeterminable() {
    if nix::unistd::geteuid().is_root() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("alice");
    fs::write(&path, tally_bytes(&[(1, 10)])).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    assert_eq!(read_tally_records(&path), None);
}

/// PAU23: a tally under an exclusive `flock` (a `pam_faillock` writer mid-update) is `None`.
#[test]
fn test_pau_exclusively_locked_tally_is_undeterminable() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("alice");
    fs::write(&path, tally_bytes(&[(1, 10)])).unwrap();
    let writer = fs::File::open(&path).unwrap();
    let held = nix::fcntl::Flock::lock(writer, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap();
    assert_eq!(read_tally_records(&path), None);
    drop(held);
    assert_eq!(read_tally_records(&path), Some(vec![valid(10)]));
}

/// PAU23: through the guard, a symlinked tally is `Undeterminable`.
#[test]
fn test_pau_guard_refuses_a_symlinked_tally() {
    let accounts = Accounts::new();
    let elsewhere = accounts.path("elsewhere");
    fs::write(&elsewhere, Vec::<u8>::new()).unwrap();
    symlink(&elsewhere, accounts.conf_tally_dir().join("alice")).unwrap();
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable)
    );
}

// ---------------------------------------------------------------------------------------
// PAU24 — check_tally replica
// ---------------------------------------------------------------------------------------

/// Reference port of Linux-PAM 1.7.1 `check_tally` for a non-admin user.
fn reference_denies(policy: &FaillockPolicy, records: &[TallyRecord], now: u64) -> bool {
    let valid: Vec<u64> = records
        .iter()
        .filter(|r| r.status & TALLY_STATUS_VALID != 0)
        .map(|r| r.time)
        .collect();
    let latest = valid.iter().copied().max().unwrap_or(0);
    let failures = valid
        .iter()
        .filter(|&&t| latest.saturating_sub(t) < u64::from(policy.fail_interval))
        .count();
    if policy.deny == 0 || failures < usize::from(policy.deny) {
        return false;
    }
    if policy.unlock_time != 0 {
        if let Some(end) = latest.checked_add(u64::from(policy.unlock_time)) {
            if end < now {
                return false;
            }
        }
    }
    true
}

const T: u64 = 1_000_000;

/// PAU24: `deny - 1` failures are not locked, `deny` are.
#[test]
fn test_pau_faillock_threshold() {
    let p = policy(3, 900, 600);
    assert!(!faillock_denies(&p, &[valid(T - 10), valid(T - 20)], T));
    assert!(faillock_denies(
        &p,
        &[valid(T - 10), valid(T - 20), valid(T - 30)],
        T
    ));
}

/// PAU24: `deny = 0` never locks.
#[test]
fn test_pau_faillock_deny_zero_never_locks() {
    let records: Vec<TallyRecord> = (0..20).map(|i| valid(T - i)).collect();
    assert!(!faillock_denies(&policy(0, 900, 600), &records, T));
}

/// PAU24: a record exactly `fail_interval` older than the latest is not counted.
#[test]
fn test_pau_faillock_fail_interval_boundary() {
    let p = policy(3, 900, 600);
    let latest = T - 5;
    assert!(!faillock_denies(
        &p,
        &[valid(latest), valid(latest - 1), valid(latest - 900)],
        T
    ));
    assert!(faillock_denies(
        &p,
        &[valid(latest), valid(latest - 1), valid(latest - 899)],
        T
    ));
}

/// PAU24: `latest + unlock_time == now` is still locked, `< now` is unlocked.
#[test]
fn test_pau_faillock_unlock_time_boundary() {
    let p = policy(3, 900, 600);
    let records = |latest: u64| vec![valid(latest), valid(latest - 1), valid(latest - 2)];
    assert!(faillock_denies(&p, &records(T - 600), T), "== now: locked");
    assert!(
        !faillock_denies(&p, &records(T - 601), T),
        "< now: unlocked"
    );
}

/// PAU24: `unlock_time = 0` locks permanently.
#[test]
fn test_pau_faillock_unlock_time_zero_is_permanent() {
    let p = policy(3, 900, 0);
    let old = 10u64;
    assert!(faillock_denies(
        &p,
        &[valid(old), valid(old + 1), valid(old + 2)],
        T * 1000
    ));
}

/// PAU24: records without the VALID bit are ignored; any status with the bit counts.
#[test]
fn test_pau_faillock_invalid_status_records_are_ignored() {
    let p = policy(3, 900, 600);
    let invalid = |status: u16, time: u64| TallyRecord { status, time };
    assert!(!faillock_denies(
        &p,
        &[invalid(0, T - 1), invalid(2, T - 2), invalid(4, T - 3)],
        T
    ));
    assert!(faillock_denies(
        &p,
        &[invalid(3, T - 1), invalid(0x8001, T - 2), valid(T - 3)],
        T
    ));
    // An invalid record newer than the valid ones does not move `latest`.
    assert!(!faillock_denies(
        &policy(3, 900, 600),
        &[
            valid(T - 2000),
            valid(T - 2001),
            valid(T - 2002),
            invalid(0, T)
        ],
        T
    ));
}

/// PAU24: future-dated records are counted.
#[test]
fn test_pau_faillock_future_dated_records_count() {
    let p = policy(3, 900, 600);
    assert!(faillock_denies(
        &p,
        &[valid(T + 100), valid(T + 101), valid(T + 102)],
        T
    ));
}

/// PAU24: overflow of `latest + unlock_time` is locked (fail closed).
#[test]
fn test_pau_faillock_overflow_is_locked() {
    let p = policy(3, 900, 600);
    let near = u64::MAX - 10;
    assert!(faillock_denies(
        &p,
        &[valid(near), valid(near - 1), valid(near - 2)],
        T
    ));
}

/// PAU24: with `admin_group` set (no group lookup) the stricter of `unlock_time` and
/// `root_unlock_time` applies; without it `root_unlock_time` is irrelevant.
#[test]
fn test_pau_faillock_admin_group_uses_the_stricter_unlock_time() {
    let records = [valid(T - 100), valid(T - 101), valid(T - 102)];
    let base = FaillockPolicy {
        deny: 3,
        fail_interval: 900,
        unlock_time: 60,
        root_unlock_time: 600,
        ..FaillockPolicy::default()
    };
    assert!(
        !faillock_denies(&base, &records, T),
        "without admin_group: unlock_time 60 elapsed"
    );
    let admin = FaillockPolicy {
        admin_group: Some("wheel".into()),
        ..base.clone()
    };
    assert!(
        faillock_denies(&admin, &records, T),
        "with admin_group: root_unlock_time 600 still running"
    );
    let never = FaillockPolicy {
        admin_group: Some("wheel".into()),
        root_unlock_time: 0,
        ..base.clone()
    };
    assert!(
        faillock_denies(&never, &[valid(10), valid(11), valid(12)], T * 1000),
        "root_unlock_time = never is the strictest"
    );
    let reversed = FaillockPolicy {
        admin_group: Some("wheel".into()),
        unlock_time: 600,
        root_unlock_time: 60,
        ..base
    };
    assert!(faillock_denies(&reversed, &records, T));
}

proptest! {
    /// PAU24: `faillock_denies` equals the reference port of `check_tally` (non-admin).
    #[test]
    fn prop_pau_faillock_denies_matches_check_tally(
        deny in 0u16..6,
        fail_interval in 0u32..2000,
        unlock_time in 0u32..2000,
        records in proptest::collection::vec((0u16..5, 0u64..6000), 0..12),
        now in 0u64..6000,
    ) {
        let p = policy(deny, fail_interval, unlock_time);
        let records: Vec<TallyRecord> = records
            .into_iter()
            .map(|(status, offset)| TallyRecord { status, time: T - 3000 + offset })
            .collect();
        let now = T - 3000 + now;
        prop_assert_eq!(
            faillock_denies(&p, &records, now),
            reference_denies(&p, &records, now)
        );
    }

    /// PAU23: decoding never panics and accepts exactly the 64-byte multiples up to the bound.
    #[test]
    fn prop_pau_decode_tally_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..1024)) {
        let decoded = decode_tally_records(&bytes);
        if bytes.len() % 64 == 0 {
            prop_assert_eq!(decoded.map(|r| r.len()), Some(bytes.len() / 64));
        } else {
            prop_assert!(decoded.is_none());
        }
    }

    /// PAU22: the conf parser never panics on arbitrary bytes.
    #[test]
    fn prop_pau_parse_faillock_conf_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = parse_faillock_conf(&bytes);
    }

    /// PAU25: the PAM option scan never panics on arbitrary text.
    #[test]
    fn prop_pau_scan_pam_options_never_panics(text in ".{0,400}") {
        let _ = scan_pam_faillock_options(&text);
    }
}

// ---------------------------------------------------------------------------------------
// PAU25 — PAM-stack option scan
// ---------------------------------------------------------------------------------------

/// PAU25: a `pam_faillock.so` line carrying a policy option makes the account
/// undeterminable, in every syntax Linux-PAM accepts.
#[test]
fn test_pau_pam_lines_with_faillock_policy_options_are_detected() {
    for line in [
        "auth required pam_faillock.so preauth deny=5\n",
        "auth required pam_faillock.so authfail dir=/var/faillock\n",
        "auth required pam_faillock.so authfail fail_interval=60\n",
        "auth required pam_faillock.so authfail unlock_time=10\n",
        "auth required pam_faillock.so authfail root_unlock_time=10\n",
        "auth required pam_faillock.so authfail admin_group=wheel\n",
        "auth required pam_faillock.so authfail conf=/etc/other.conf\n",
        "auth required pam_faillock.so authfail even_deny_root\n",
        "auth [success=1 default=bad] pam_faillock.so authfail deny=1\n",
        "-auth required pam_faillock.so preauth deny=1\n",
        "auth required /usr/lib/security/pam_faillock.so preauth deny=1\n",
        "auth\trequired\tpam_faillock.so\tpreauth\tdeny=1\n",
        "auth required pam_faillock.so preauth \\\n    deny=1\n",
        "account required pam_faillock.so even_deny_root # trailing comment\n",
    ] {
        assert!(
            scan_pam_faillock_options(line),
            "`{line}` sets faillock policy and must be detected"
        );
    }
}

/// PAU25: harmless arguments, commented lines and other modules have no effect.
#[test]
fn test_pau_pam_lines_without_policy_options_are_ignored() {
    for content in [
        "",
        "auth required pam_faillock.so preauth\nauth [default=die] pam_faillock.so authfail\naccount required pam_faillock.so\n",
        "auth required pam_faillock.so preauth silent audit no_log_info local_users_only nodelay\n",
        "auth sufficient pam_faillock.so authsucc\n",
        "# auth required pam_faillock.so deny=1\n",
        "#auth required pam_faillock.so preauth deny=1\n",
        "auth required pam_tally2.so deny=3 unlock_time=10\n",
        "auth required pam_unix.so deny=1\n",
        "auth required pam_faillock.so preauth # deny=1 commented out\n",
        "auth required pam_deny.so\n",
    ] {
        assert!(
            !scan_pam_faillock_options(content),
            "`{content}` must not be flagged"
        );
    }
}

/// PAU25: through the guard, any of the three directories is scanned.
#[test]
fn test_pau_guard_scans_every_pam_directory() {
    for (i, dir_index) in [0usize, 1, 2].into_iter().enumerate() {
        let accounts = Accounts::new();
        assert_eq!(accounts.check("alice"), AccountState::Usable);
        let dir = &accounts.pam_dirs()[dir_index];
        fs::write(
            dir.join(format!("svc{i}")),
            "auth required pam_faillock.so preauth deny=10\n",
        )
        .unwrap();
        assert_eq!(
            accounts.check("alice"),
            refused(AccountRefusal::Undeterminable),
            "an option line in {} must be seen",
            dir.display()
        );
    }
}

/// PAU25: missing directories are skipped; sub-directories are not files of the stack.
#[test]
fn test_pau_guard_skips_missing_pam_directories_and_subdirectories() {
    let accounts = Accounts::new();
    fs::remove_dir_all(&accounts.pam_dirs()[1]).unwrap();
    let sub = accounts.pam_dirs()[0].join("nested");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("svc"), "auth required pam_faillock.so deny=1\n").unwrap();
    fs::write(
        accounts.pam_dirs()[0].join("system-auth"),
        "auth required pam_faillock.so preauth\n",
    )
    .unwrap();
    assert_eq!(accounts.check("alice"), AccountState::Usable);
}

/// PAU25: an unreadable directory, more than 512 entries, a file above 64 KiB or a
/// non-UTF-8 file ⇒ `Undeterminable`.
#[test]
fn test_pau_guard_pam_scan_bounds_are_undeterminable() {
    let accounts = Accounts::new();
    let first = accounts.pam_dirs()[0].clone();
    fs::remove_dir_all(&first).unwrap();
    fs::write(&first, "not a directory").unwrap();
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "unreadable PAM directory"
    );

    let accounts = Accounts::new();
    for i in 0..=MAX_PAM_DIR_ENTRIES {
        fs::write(accounts.pam_dirs()[1].join(format!("svc{i}")), "").unwrap();
    }
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "more than MAX_PAM_DIR_ENTRIES"
    );

    let accounts = Accounts::new();
    for i in 0..MAX_PAM_DIR_ENTRIES {
        fs::write(accounts.pam_dirs()[1].join(format!("svc{i}")), "").unwrap();
    }
    assert_eq!(
        accounts.check("alice"),
        AccountState::Usable,
        "exactly MAX_PAM_DIR_ENTRIES is scanned"
    );

    let accounts = Accounts::new();
    fs::write(
        accounts.pam_dirs()[0].join("big"),
        format!("#{}\n", "x".repeat(MAX_PAM_FILE_BYTES)),
    )
    .unwrap();
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "PAM file above MAX_PAM_FILE_BYTES"
    );

    let accounts = Accounts::new();
    fs::write(accounts.pam_dirs()[2].join("bin"), b"auth \xff\xfe\n").unwrap();
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "non-UTF-8 PAM file"
    );
}

// ---------------------------------------------------------------------------------------
// PAU26 — shadow
// ---------------------------------------------------------------------------------------

fn entry(line: &str, user: &str) -> Option<ShadowEntry> {
    find_shadow_entry(line.as_bytes(), &name(user))
}

fn shadow_state(line: &str) -> Option<AccountRefusal> {
    let e = entry(line, "alice").unwrap_or_else(|| panic!("`{line}` must parse"));
    shadow_refusal(&e, NOW_S)
}

/// PAU26: account expiry (`today >= expire`, `expire = 0` included).
#[test]
fn test_pau_shadow_account_expiry() {
    assert_eq!(
        shadow_state(&format!("alice:$6$h:19900:0:99999:7::{TODAY}:")),
        Some(AccountRefusal::AccountExpired)
    );
    assert_eq!(
        shadow_state(&format!("alice:$6$h:19900:0:99999:7::{}:", TODAY - 1)),
        Some(AccountRefusal::AccountExpired)
    );
    assert_eq!(
        shadow_state("alice:$6$h:19900:0:99999:7::0:"),
        Some(AccountRefusal::AccountExpired),
        "expire = 0 is expired (conservative reading)"
    );
    assert_eq!(
        shadow_state(&format!("alice:$6$h:19900:0:99999:7::{}:", TODAY + 1)),
        None
    );
}

/// PAU26: `lastchg = 0` forces a password change.
#[test]
fn test_pau_shadow_forced_password_change() {
    assert_eq!(
        shadow_state("alice:$6$h:0:0:99999:7:::"),
        Some(AccountRefusal::PasswordChangeForced)
    );
}

/// PAU26: password expiry and inactivity, with the `today == lastchg + max` boundary usable.
#[test]
fn test_pau_shadow_password_expiry_and_inactivity() {
    let lastchg = TODAY - 100;
    assert_eq!(
        shadow_state(&format!("alice:$6$h:{lastchg}:0:100:7:::")),
        None,
        "today == lastchg + max is still usable"
    );
    assert_eq!(
        shadow_state(&format!("alice:$6$h:{lastchg}:0:99:7:::")),
        Some(AccountRefusal::PasswordExpired)
    );
    assert_eq!(
        shadow_state(&format!("alice:$6$h:{lastchg}:0:50:7:50::")),
        Some(AccountRefusal::PasswordExpired),
        "today == lastchg + max + inactive is expired, not yet inactive"
    );
    assert_eq!(
        shadow_state(&format!("alice:$6$h:{lastchg}:0:50:7:49::")),
        Some(AccountRefusal::AccountInactive)
    );
}

/// PAU26: locked password fields.
#[test]
fn test_pau_shadow_locked_password() {
    for password in ["!$6$hash", "!", "!!", "*", "!*"] {
        assert_eq!(
            shadow_state(&format!("alice:{password}:19900:0:99999:7:::")),
            Some(AccountRefusal::PasswordLocked),
            "`{password}`"
        );
    }
    for password in ["$6$hash", "", "x"] {
        assert_eq!(
            shadow_state(&format!("alice:{password}:19900:0:99999:7:::")),
            None,
            "`{password}` is not a locked marker"
        );
    }
}

/// PAU26: empty numeric fields mean "not set".
#[test]
fn test_pau_shadow_empty_fields_are_not_set() {
    assert_eq!(shadow_state("alice:$6$h:19900::::::"), None);
    let e = entry("alice:$6$h:19900::::::", "alice").unwrap();
    assert_eq!(e.max, None);
    assert_eq!(e.inactive, None);
    assert_eq!(e.expire, None);
    assert_eq!(e.lastchg, Some(19_900));
    assert_eq!(
        shadow_state("alice:$6$h:::::::"),
        None,
        "no lastchg: nothing expires"
    );
}

/// PAU26: absent, duplicated, wrong field count, negative or non-numeric ⇒ `None`.
#[test]
fn test_pau_shadow_malformed_inputs_are_undeterminable() {
    let good = "alice:$6$h:19900:0:99999:7:::";
    for (label, content) in [
        ("absent", "bob:$6$h:19900:0:99999:7:::\n".to_string()),
        ("duplicated", format!("{good}\n{good}\n")),
        ("8 fields", "alice:$6$h:19900:0:99999:7::".to_string()),
        ("10 fields", "alice:$6$h:19900:0:99999:7::::".to_string()),
        ("negative", "alice:$6$h:-1:0:99999:7:::".to_string()),
        ("non numeric", "alice:$6$h:abc:0:99999:7:::".to_string()),
        (
            "non numeric max",
            "alice:$6$h:19900:0:lots:7:::".to_string(),
        ),
        ("prefix user", "alice2:$6$h:19900:0:99999:7:::".to_string()),
    ] {
        assert_eq!(entry(&content, "alice"), None, "{label}");
    }
    assert!(entry(&format!("root:*:19000:0:99999:7:::\n{good}\n"), "alice").is_some());
    assert_eq!(
        find_shadow_entry(b"alice:\xff:1:::::::\n", &name("alice")),
        None
    );
}

/// PAU26: the password hash is never copied: neither the entry nor the result shows it.
#[test]
fn test_pau_shadow_hash_never_leaves_the_parser() {
    let e = entry("alice:$6$TOPSECRETHASH:19900:0:99999:7:::", "alice").unwrap();
    let shown = format!("{e:?} {:?}", shadow_refusal(&e, NOW_S));
    assert!(!shown.contains("TOPSECRET"), "{shown}");
    assert!(!e.password_locked);
}

/// PAU26: through the guard: expiry refusals, missing user, oversized or non-UTF-8 file.
#[test]
fn test_pau_guard_shadow_refusals() {
    let accounts = Accounts::new();
    accounts.write_shadow(&format!("alice:$6$h:19900:0:99999:7::{TODAY}:\n"));
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::AccountExpired)
    );

    accounts.write_shadow("alice:!$6$h:19900:0:99999:7:::\n");
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::PasswordLocked)
    );

    accounts.write_shadow("bob:$6$h:19900:0:99999:7:::\n");
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "NSS-only users (no shadow line) never get a presence unlock"
    );

    fs::remove_file(accounts.shadow()).unwrap();
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable)
    );

    let mut big = String::from("alice:$6$h:19900:0:99999:7:::\n");
    big.push_str(&"#".repeat(MAX_SHADOW_BYTES));
    accounts.write_shadow(&big);
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "shadow above MAX_SHADOW_BYTES"
    );

    let mut many = String::from("alice:$6$h:19900:0:99999:7:::\n");
    many.push_str(&"x\n".repeat(MAX_SHADOW_LINES));
    accounts.write_shadow(&many);
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "shadow above MAX_SHADOW_LINES"
    );

    fs::write(accounts.shadow(), b"alice:$6$h:19900:0:99999:7:::\n\xff\n").unwrap();
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable)
    );
}

/// Order: a faillocked account is refused as `Faillocked` before the shadow is consulted.
#[test]
fn test_pau_guard_faillock_and_shadow_order() {
    let accounts = Accounts::new();
    accounts.write_tally(
        &accounts.conf_tally_dir(),
        "alice",
        &[(1, NOW_S - 10), (1, NOW_S - 20), (1, NOW_S - 30)],
    );
    accounts.write_shadow(&format!("alice:$6$h:19900:0:99999:7::{TODAY}:\n"));
    assert_eq!(accounts.check("alice"), refused(AccountRefusal::Faillocked));

    // Once unlock_time (600 s) has passed the faillock no longer denies.
    accounts.write_tally(
        &accounts.conf_tally_dir(),
        "alice",
        &[(1, NOW_S - 700), (1, NOW_S - 701), (1, NOW_S - 702)],
    );
    accounts.write_shadow("alice:$6$h:19900:0:99999:7:::\n");
    assert_eq!(accounts.check("alice"), AccountState::Usable);
}

// ---------------------------------------------------------------------------------------
// Round 2 (auditor T1, T2, T8, constraint 13)
// ---------------------------------------------------------------------------------------

/// T1 / A5: PAM stack files that are symlinks (authselect on Fedora/RHEL) are followed and
/// scanned like regular files.
#[test]
fn test_pau_guard_follows_symlinked_pam_stack_files() {
    let accounts = Accounts::new();
    let authselect = accounts.path("etc/authselect");
    fs::create_dir_all(&authselect).unwrap();
    fs::write(
        authselect.join("system-auth"),
        "auth required pam_faillock.so preauth deny=1\n",
    )
    .unwrap();
    symlink(
        authselect.join("system-auth"),
        accounts.pam_dirs()[0].join("system-auth"),
    )
    .unwrap();
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "a deny= line behind a symlinked stack file must be seen"
    );

    let accounts = Accounts::new();
    let authselect = accounts.path("etc/authselect");
    fs::create_dir_all(&authselect).unwrap();
    fs::write(
        authselect.join("system-auth"),
        "auth required pam_faillock.so preauth\n",
    )
    .unwrap();
    symlink(
        authselect.join("system-auth"),
        accounts.pam_dirs()[0].join("system-auth"),
    )
    .unwrap();
    assert_eq!(
        accounts.check("alice"),
        AccountState::Usable,
        "a symlinked stack file without policy options is harmless"
    );
}

/// T1 / A5: a dangling symlink, or one resolving to something other than a regular file or
/// a directory, in a PAM directory ⇒ `Undeterminable`; a symlink to a directory is skipped.
#[test]
fn test_pau_guard_refuses_dangling_or_special_pam_symlinks() {
    let accounts = Accounts::new();
    symlink(
        accounts.path("nowhere"),
        accounts.pam_dirs()[0].join("system-auth"),
    )
    .unwrap();
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "dangling symlink"
    );

    let accounts = Accounts::new();
    let fifo = accounts.path("fifo");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).unwrap();
    symlink(&fifo, accounts.pam_dirs()[1].join("login")).unwrap();
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "symlink to a FIFO (must not block either)"
    );

    let accounts = Accounts::new();
    fs::create_dir_all(accounts.path("elsewhere")).unwrap();
    fs::write(
        accounts.path("elsewhere/svc"),
        "auth required pam_faillock.so deny=1\n",
    )
    .unwrap();
    symlink(
        accounts.path("elsewhere"),
        accounts.pam_dirs()[2].join("dirlink"),
    )
    .unwrap();
    assert_eq!(
        accounts.check("alice"),
        AccountState::Usable,
        "a symlink to a directory is skipped like a sub-directory"
    );
}

/// Constraint 13: a tally directory that is a symlink, or group/other-writable without
/// the root-owned sticky exception, ⇒ `Undeterminable`.
#[test]
fn test_pau_guard_refuses_an_unsafe_tally_directory() {
    let accounts = Accounts::new();
    let real = accounts.path("real-faillock");
    fs::create_dir_all(&real).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o755)).unwrap();
    let link = accounts.path("linked-faillock");
    symlink(&real, &link).unwrap();
    accounts.write_conf(&format!("dir = {}\n", link.display()));
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "a symlinked tally directory"
    );

    for mode in [0o777u32, 0o775, 0o757, 0o1777] {
        let accounts = Accounts::new();
        fs::set_permissions(accounts.conf_tally_dir(), fs::Permissions::from_mode(mode)).unwrap();
        // The sticky exception only applies to a root-owned directory; tests never run the
        // directory as root-owned, so every mode here is refused.
        if nix::unistd::geteuid().is_root() && mode & 0o1000 != 0 {
            continue;
        }
        assert_eq!(
            accounts.check("alice"),
            refused(AccountRefusal::Undeterminable),
            "tally directory mode {mode:o}"
        );
    }

    let accounts = Accounts::new();
    accounts.write_conf(&format!("dir = {}\n", accounts.shadow().display()));
    assert_eq!(
        accounts.check("alice"),
        refused(AccountRefusal::Undeterminable),
        "a tally directory that is a regular file (ENOTDIR, root-proof)"
    );
}

/// T8: a root-proof unreadable tally (`ENOTDIR`: a parent is a regular file).
#[test]
fn test_pau_tally_under_a_regular_file_is_undeterminable_even_as_root() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("plain");
    fs::write(&file, b"x").unwrap();
    assert_eq!(read_tally_records(&file.join("alice")), None);
}

/// T2: one guard instance re-reads every source on every call (no caching), in both
/// directions.
#[test]
fn test_pau_guard_rereads_every_source_on_every_check() {
    let accounts = Accounts::new();
    let guard = accounts.guard();
    let alice = name("alice");
    assert_eq!(guard.check(&alice, 1000), AccountState::Usable);

    // Tally: usable -> faillocked -> usable again.
    accounts.write_tally(
        &accounts.conf_tally_dir(),
        "alice",
        &[(1, NOW_S - 10), (1, NOW_S - 20), (1, NOW_S - 30)],
    );
    assert_eq!(
        guard.check(&alice, 1000),
        refused(AccountRefusal::Faillocked),
        "three failures typed after the first check must be seen"
    );
    fs::remove_file(accounts.conf_tally_dir().join("alice")).unwrap();
    assert_eq!(guard.check(&alice, 1000), AccountState::Usable);

    // Shadow: expiry appears, then is lifted.
    accounts.write_shadow(&format!("alice:$6$h:19900:0:99999:7::{}:\n", TODAY - 1));
    assert_eq!(
        guard.check(&alice, 1000),
        refused(AccountRefusal::AccountExpired)
    );
    accounts.write_shadow("alice:$6$h:19900:0:99999:7:::\n");
    assert_eq!(guard.check(&alice, 1000), AccountState::Usable);

    // PAM stack: a policy option appears, then is removed.
    let stack = accounts.pam_dirs()[0].join("system-auth");
    fs::write(&stack, "auth required pam_faillock.so preauth deny=10\n").unwrap();
    assert_eq!(
        guard.check(&alice, 1000),
        refused(AccountRefusal::Undeterminable)
    );
    fs::remove_file(&stack).unwrap();
    assert_eq!(guard.check(&alice, 1000), AccountState::Usable);

    // faillock.conf: an unknown key appears, then the conf is restored with deny = 1.
    accounts.write_conf(&format!(
        "dir = {}\nnot_a_key = 1\n",
        accounts.conf_tally_dir().display()
    ));
    assert_eq!(
        guard.check(&alice, 1000),
        refused(AccountRefusal::Undeterminable)
    );
    accounts.write_conf(&format!(
        "dir = {}\ndeny = 1\n",
        accounts.conf_tally_dir().display()
    ));
    accounts.write_tally(&accounts.conf_tally_dir(), "alice", &[(1, NOW_S - 10)]);
    assert_eq!(
        guard.check(&alice, 1000),
        refused(AccountRefusal::Faillocked),
        "a stricter deny in faillock.conf applies on the very next check"
    );
}
