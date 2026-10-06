//! System-journal reading of the failed-password alerts (ADR 2026-10-06 "Failed-Password
//! Alerts in `soos-remote` From the System Journal", architect spec
//! `AI/architect_spec_remote_auth_alerts.md` §2).
//!
//! - [`probe_args`] / [`follow_args`]: the exact, pure `journalctl` argument vectors.
//! - [`parse_entry`]: bounded parsing of one `journalctl --output=json` line through a
//!   hand-written visitor (a repeated read key is refused, unknown keys are skipped without
//!   being materialised).
//! - [`classify_entry`]: the trust allow-list on journald-set fields and the three
//!   prefix-anchored message grammars; the account name is mapped to [`AccountClass`] here
//!   and never leaves this function.
//! - [`JournalSource`] / [`JournalLines`]: the injectable source seam; [`JournalctlSource`]
//!   is the production source (a `journalctl` child process, environment cleared, no shell).
//! - [`BoundedLineReader`]: a fixed-capacity, wiped line reader over the child's pipe.
//!
//! Every journal field is untrusted input. Nothing here logs: a malformed, overlong,
//! foreign or untrusted line is skipped silently. The typed password never reaches the
//! journal (research §4), and no type here is designed to carry it.

use std::borrow::Cow;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use serde::de::{self, Deserializer as _, IgnoredAny, MapAccess, SeqAccess, Visitor};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, ChildStdout, Command};
use zeroize::{Zeroize, Zeroizing};

use crate::{
    JOURNAL_PROBE_TIMEOUT_MS, JOURNAL_READ_CHUNK_BYTES, MAX_CURSOR_LEN, MAX_FIELD_BYTES,
    MAX_JOURNAL_LINE_BYTES, MAX_MESSAGE_BYTES,
};

/// Absolute path; never resolved through `PATH` (the environment is cleared).
pub const JOURNALCTL_PATH: &str = "/usr/bin/journalctl";

/// The kernel's suffix for an executable unlinked or replaced while running.
pub const EXE_DELETED_SUFFIX: &str = " (deleted)";

/// `unix_chkpwd` locations accepted as `_EXE` of a password-check line.
pub const UNIX_CHKPWD_PATHS: [&str; 2] = ["/usr/bin/unix_chkpwd", "/usr/sbin/unix_chkpwd"];

/// Setuid authenticators whose owner-UID `pam_unix` lines are root-side and trusted.
pub const SETUID_AUTH_PROGRAMS: [&str; 2] = ["/usr/bin/sudo", "/usr/bin/su"];

/// PAM services of the lock-screen class.
pub const LOCK_SCREEN_SERVICES: [&str; 7] = [
    "swaylock",
    "hyprlock",
    "gtklock",
    "waylock",
    "i3lock",
    "xscreensaver",
    "kde",
];

/// PAM services of the `sudo` class.
pub const SUDO_SERVICES: [&str; 2] = ["sudo", "sudo-i"];

/// PAM services of the login class.
pub const LOGIN_SERVICES: [&str; 5] = ["gdm-password", "login", "sddm", "lightdm", "greetd"];

/// Longest account name of a password-check line (bytes).
const MAX_CHECK_NAME_BYTES: usize = 256;
/// Longest PAM service name (bytes).
const MAX_SERVICE_BYTES: usize = 64;
/// Most digits of the `pam_faillock` failure count.
const MAX_FAILLOCK_DIGITS: usize = 5;
/// Longest owner login (bytes).
const MAX_OWNER_LOGIN_BYTES: usize = 32;
/// Most decimal digits of `__REALTIME_TIMESTAMP`.
const MAX_TIMESTAMP_DIGITS: usize = 20;
/// Most decimal digits of `_UID`.
const MAX_UID_DIGITS: usize = 10;
/// Most decimal digits of `SYSLOG_FACILITY`.
const MAX_FACILITY_DIGITS: usize = 3;
/// The `authpriv` syslog facility.
const AUTHPRIV_FACILITY: u8 = 10;

/// Where a follower starts.
#[derive(Clone, PartialEq, Eq)]
pub enum FollowStart {
    /// `--since=@<unix_s>` (first start, or after a refused cursor).
    Since {
        /// Unix time in seconds.
        unix_s: u64,
    },
    /// `--after-cursor=<cursor>` (restart without a gap).
    AfterCursor(JournalCursor),
}

/// Pure. The probe argv (without the program):
/// `--no-pager --quiet --lines=1 --output=json --output-fields=_UID _UID=0`.
#[must_use]
pub fn probe_args() -> Vec<String> {
    [
        "--no-pager",
        "--quiet",
        "--lines=1",
        "--output=json",
        "--output-fields=_UID",
        "_UID=0",
    ]
    .iter()
    .map(|arg| (*arg).to_string())
    .collect()
}

/// Pure. The follower argv (without the program), in this exact order:
/// `--follow --no-pager --quiet --lines=all --output=json
///  --output-fields=MESSAGE,SYSLOG_IDENTIFIER,SYSLOG_FACILITY,_UID,_COMM,_EXE,_TRANSPORT`,
/// then `--since=@<unix_s>` or `--after-cursor=<cursor>` (one element, never split), then
/// the match `SYSLOG_FACILITY=10`.
#[must_use]
pub fn follow_args(start: &FollowStart) -> Vec<String> {
    let mut args: Vec<String> = [
        "--follow",
        "--no-pager",
        "--quiet",
        "--lines=all",
        "--output=json",
        "--output-fields=MESSAGE,SYSLOG_IDENTIFIER,SYSLOG_FACILITY,_UID,_COMM,_EXE,_TRANSPORT",
    ]
    .iter()
    .map(|arg| (*arg).to_string())
    .collect();
    match start {
        FollowStart::Since { unix_s } => args.push(format!("--since=@{unix_s}")),
        FollowStart::AfterCursor(cursor) => {
            args.push(format!("--after-cursor={}", cursor.as_str()));
        }
    }
    args.push("SYSLOG_FACILITY=10".to_string());
    args
}

/// A `__CURSOR` value: 1..=`MAX_CURSOR_LEN` bytes of `[A-Za-z0-9=;_-]`. Anything else is not
/// kept (the next restart falls back to `Since`).
#[derive(Clone, PartialEq, Eq)]
pub struct JournalCursor(String);

impl JournalCursor {
    /// `None` unless 1..=`MAX_CURSOR_LEN` bytes of `[A-Za-z0-9=;_-]`.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.is_empty() || raw.len() > MAX_CURSOR_LEN {
            return None;
        }
        raw.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b';' | b'_' | b'-'))
            .then(|| Self(raw.to_string()))
    }

    /// The cursor text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A cursor is an opaque position, not a secret, but it is never logged.
impl fmt::Debug for JournalCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JournalCursor(<redacted>)")
    }
}

/// One accepted journal entry. No `Debug` (`MESSAGE` may hold typed text in the account
/// field). Every owned string is `Zeroizing`: wiped when the entry is dropped.
pub struct JournalEntry {
    /// `__REALTIME_TIMESTAMP` (µs since the Unix epoch).
    pub realtime_us: u64,
    /// `__CURSOR`, when valid.
    pub cursor: Option<JournalCursor>,
    /// `MESSAGE`, 1..=`MAX_MESSAGE_BYTES`, no byte below 0x20 and no 0x7F.
    pub message: Zeroizing<String>,
    /// `SYSLOG_IDENTIFIER` (grammar input only, never trusted).
    pub identifier: Option<Zeroizing<String>>,
    /// `SYSLOG_FACILITY`; [`parse_entry`] only accepts 10.
    pub facility: u8,
    /// `_UID`.
    pub uid: u32,
    /// `_COMM`.
    pub comm: Option<Zeroizing<String>>,
    /// `_EXE` (raw; the ` (deleted)` suffix is stripped at comparison only).
    pub exe: Option<Zeroizing<String>>,
    /// `_TRANSPORT`.
    pub transport: Zeroizing<String>,
}

/// Why a journal line is not an entry (fixed text; never echoes input).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EntryError {
    /// Longer than `MAX_JOURNAL_LINE_BYTES` (checked before JSON).
    #[error("journal line too long")]
    TooLong,
    /// Not exactly one JSON object.
    #[error("journal line is not a JSON object")]
    NotJson,
    /// `__REALTIME_TIMESTAMP`, `MESSAGE`, `_UID`, `_TRANSPORT` or `SYSLOG_FACILITY` absent.
    #[error("journal field missing")]
    MissingField,
    /// A read field with an unexpected shape, bound or repetition.
    #[error("journal field has an unexpected shape")]
    Shape,
    /// Not on the `authpriv` facility.
    #[error("journal entry is not on the authpriv facility")]
    Facility,
}

/// The nine read keys, in the order of their slot.
const READ_KEYS: [&str; 9] = [
    "__REALTIME_TIMESTAMP",
    "__CURSOR",
    "MESSAGE",
    "SYSLOG_IDENTIFIER",
    "SYSLOG_FACILITY",
    "_UID",
    "_COMM",
    "_EXE",
    "_TRANSPORT",
];

/// Slot of `MESSAGE` (its own bound).
const MESSAGE_SLOT: usize = 2;
/// Slot of `_UID` (read by the probe).
const UID_SLOT: usize = 5;

/// The bound of the field in `slot`.
fn field_bound(slot: usize) -> usize {
    if slot == MESSAGE_SLOT {
        MAX_MESSAGE_BYTES
    } else {
        MAX_FIELD_BYTES
    }
}

/// One read field value: the text, or a refused shape. No `Debug`.
enum FieldValue {
    Text(Zeroizing<String>),
    Bad,
}

/// Deserialises one read field with its bound enforced while visiting.
struct FieldSeed {
    bound: usize,
}

impl<'de> de::DeserializeSeed<'de> for FieldSeed {
    type Value = FieldValue;

    fn deserialize<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<FieldValue, D::Error> {
        deserializer.deserialize_any(FieldVisitor { bound: self.bound })
    }
}

struct FieldVisitor {
    bound: usize,
}

impl<'de> Visitor<'de> for FieldVisitor {
    type Value = FieldValue;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a journal field value")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<FieldValue, E> {
        if value.len() > self.bound {
            return Ok(FieldValue::Bad);
        }
        Ok(FieldValue::Text(Zeroizing::new(value.to_owned())))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<FieldValue, A::Error> {
        let mut bytes: Zeroizing<Vec<u8>> =
            Zeroizing::new(Vec::with_capacity(self.bound.min(JOURNAL_READ_CHUNK_BYTES)));
        let mut bad = false;
        while let Some(element) = seq.next_element::<ByteElement>()? {
            if bad {
                continue;
            }
            match element.0 {
                Some(byte) if bytes.len() < self.bound => bytes.push(byte),
                _ => {
                    bad = true;
                    bytes.zeroize();
                }
            }
        }
        if bad {
            return Ok(FieldValue::Bad);
        }
        match std::str::from_utf8(&bytes) {
            Ok(text) => Ok(FieldValue::Text(Zeroizing::new(text.to_owned()))),
            Err(_) => Ok(FieldValue::Bad),
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<FieldValue, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(FieldValue::Bad)
    }

    fn visit_unit<E: de::Error>(self) -> Result<FieldValue, E> {
        Ok(FieldValue::Bad)
    }

    fn visit_bool<E: de::Error>(self, _value: bool) -> Result<FieldValue, E> {
        Ok(FieldValue::Bad)
    }

    fn visit_i64<E: de::Error>(self, _value: i64) -> Result<FieldValue, E> {
        Ok(FieldValue::Bad)
    }

    fn visit_u64<E: de::Error>(self, _value: u64) -> Result<FieldValue, E> {
        Ok(FieldValue::Bad)
    }

    fn visit_f64<E: de::Error>(self, _value: f64) -> Result<FieldValue, E> {
        Ok(FieldValue::Bad)
    }
}

/// One element of a byte-array field: `Some(byte)` for an integer 0..=255, `None` for any
/// other JSON value (skipped without being materialised).
struct ByteElement(Option<u8>);

impl<'de> de::Deserialize<'de> for ByteElement {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ByteElementVisitor)
    }
}

struct ByteElementVisitor;

impl<'de> Visitor<'de> for ByteElementVisitor {
    type Value = ByteElement;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a byte")
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<ByteElement, E> {
        Ok(ByteElement(u8::try_from(value).ok()))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<ByteElement, E> {
        Ok(ByteElement(u8::try_from(value).ok()))
    }

    fn visit_f64<E: de::Error>(self, _value: f64) -> Result<ByteElement, E> {
        Ok(ByteElement(None))
    }

    fn visit_str<E: de::Error>(self, _value: &str) -> Result<ByteElement, E> {
        Ok(ByteElement(None))
    }

    fn visit_bool<E: de::Error>(self, _value: bool) -> Result<ByteElement, E> {
        Ok(ByteElement(None))
    }

    fn visit_unit<E: de::Error>(self) -> Result<ByteElement, E> {
        Ok(ByteElement(None))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<ByteElement, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(ByteElement(None))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ByteElement, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(ByteElement(None))
    }
}

/// The read fields of one object, by slot, or the first shape error. No `Debug`.
struct ReadFields(Result<[Option<Zeroizing<String>>; 9], EntryError>);

struct ObjectVisitor;

impl<'de> Visitor<'de> for ObjectVisitor {
    type Value = ReadFields;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a journal object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ReadFields, A::Error> {
        let mut values: [Option<Zeroizing<String>>; 9] = Default::default();
        let mut seen = [false; 9];
        let mut error = None;
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            let Some(slot) = READ_KEYS.iter().position(|k| *k == key) else {
                map.next_value::<IgnoredAny>()?;
                continue;
            };
            let value = map.next_value_seed(FieldSeed {
                bound: field_bound(slot),
            })?;
            let (Some(seen_slot), Some(value_slot)) = (seen.get_mut(slot), values.get_mut(slot))
            else {
                error.get_or_insert(EntryError::Shape);
                continue;
            };
            if *seen_slot {
                error.get_or_insert(EntryError::Shape);
                continue;
            }
            *seen_slot = true;
            match value {
                FieldValue::Text(text) => *value_slot = Some(text),
                FieldValue::Bad => {
                    error.get_or_insert(EntryError::Shape);
                }
            }
        }
        Ok(ReadFields(match error {
            Some(err) => Err(err),
            None => Ok(values),
        }))
    }
}

/// Bounded read of the nine fields of one JSON line (length checked first, trailing bytes
/// refused).
fn read_fields(line: &[u8]) -> Result<[Option<Zeroizing<String>>; 9], EntryError> {
    if line.len() > MAX_JOURNAL_LINE_BYTES {
        return Err(EntryError::TooLong);
    }
    let mut deserializer = serde_json::Deserializer::from_slice(line);
    let fields = (&mut deserializer)
        .deserialize_map(ObjectVisitor)
        .map_err(|_| EntryError::NotJson)?;
    deserializer.end().map_err(|_| EntryError::NotJson)?;
    fields.0
}

/// Strict decimal: 1..=`max_digits` ASCII digits, in range of `T`.
fn strict_decimal<T: std::str::FromStr>(text: &str, max_digits: usize) -> Option<T> {
    if text.is_empty() || text.len() > max_digits || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Pure; never panics on any input. Field value rules: a JSON string → the text; a JSON
/// array whose every element is an integer 0..=255 → those bytes, which must be valid UTF-8;
/// anything else → [`EntryError::Shape`] for a read field, ignored for any other key. A read
/// key present twice → `Shape`.
///
/// # Errors
///
/// [`EntryError`].
pub fn parse_entry(line: &[u8]) -> Result<JournalEntry, EntryError> {
    let [timestamp, cursor, message, identifier, facility, uid, comm, exe, transport] =
        read_fields(line)?;
    let (Some(timestamp), Some(message), Some(facility), Some(uid), Some(transport)) =
        (timestamp, message, facility, uid, transport)
    else {
        return Err(EntryError::MissingField);
    };
    let realtime_us: u64 =
        strict_decimal(&timestamp, MAX_TIMESTAMP_DIGITS).ok_or(EntryError::Shape)?;
    let uid: u32 = strict_decimal(&uid, MAX_UID_DIGITS).ok_or(EntryError::Shape)?;
    let facility: u8 = strict_decimal(&facility, MAX_FACILITY_DIGITS).ok_or(EntryError::Shape)?;
    if message.is_empty()
        || message.len() > MAX_MESSAGE_BYTES
        || message.bytes().any(|b| b < 0x20 || b == 0x7f)
    {
        return Err(EntryError::Shape);
    }
    if facility != AUTHPRIV_FACILITY {
        return Err(EntryError::Facility);
    }
    Ok(JournalEntry {
        realtime_us,
        cursor: cursor.and_then(|c| JournalCursor::parse(&c)),
        message,
        identifier,
        facility,
        uid,
        comm,
        exe,
        transport,
    })
}

/// Where the attempt was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceClass {
    /// A lock screen.
    LockScreen,
    /// `sudo`.
    Sudo,
    /// A login manager or console login.
    Login,
    /// Any other PAM service (`polkit-1`, `su`, `sshd`, …).
    Other,
}

/// Account class; the raw account name never leaves [`classify_entry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountClass {
    /// The service owner's account.
    Owner,
    /// `root`.
    Root,
    /// Any other name (another account, or text typed into a user-name box).
    Other,
}

/// UID the password helper runs as for this attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HelperSide {
    /// A root-side PAM client (`sudo`, a login manager, polkit).
    Root,
    /// An owner-side PAM client (a lock screen).
    Owner,
}

/// One classified journal signal; carries no text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// `unix_chkpwd`: one real password check.
    Check {
        /// Helper side.
        side: HelperSide,
        /// Account class.
        account: AccountClass,
        /// Journal time (µs).
        at_us: u64,
    },
    /// `pam_unix(<svc>:auth): authentication failure; …`.
    Failure {
        /// Source class of the service.
        class: SourceClass,
        /// Helper side.
        side: HelperSide,
        /// Account class.
        account: AccountClass,
        /// Journal time (µs).
        at_us: u64,
        /// Whether the trust allow-list accepted the line.
        trusted: bool,
    },
    /// `pam_faillock(<svc>:auth): User <n> is temporarily locked out due to <d> consecutive
    /// failed login attempts` (trusted lines only).
    LockedOut {
        /// Source class of the service.
        class: SourceClass,
        /// Account class.
        account: AccountClass,
        /// Journal time (µs).
        at_us: u64,
    },
}

/// What [`classify_entry`] needs besides the entry.
#[derive(Clone)]
pub struct TrustContext {
    /// The service owner's uid (the process uid).
    pub owner_uid: u32,
    /// The service owner's login.
    pub owner_login: OwnerLogin,
    /// Absolute paths of the trusted lock-screen programs (configuration).
    pub lock_screen_programs: Vec<String>,
}

/// The owner's local login name (from the passwd entry of the process uid at startup):
/// 1..=32 bytes, `[a-z_][a-z0-9_-]*` with an optional final `$`. `Debug` is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnerLogin(String);

impl OwnerLogin {
    /// `None` unless 1..=32 bytes of `[a-z_][a-z0-9_-]*` with an optional final `$`.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.is_empty() || raw.len() > MAX_OWNER_LOGIN_BYTES {
            return None;
        }
        let body = raw.strip_suffix('$').unwrap_or(raw);
        let mut bytes = body.bytes();
        let first = bytes.next()?;
        if !(first.is_ascii_lowercase() || first == b'_') {
            return None;
        }
        bytes
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
            .then(|| Self(raw.to_string()))
    }
}

/// The login is an identity: never printed.
impl fmt::Debug for OwnerLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OwnerLogin(<redacted>)")
    }
}

/// Pure. Strips exactly one trailing [`EXE_DELETED_SUFFIX`]; everything else byte-for-byte
/// unchanged.
#[must_use]
pub fn exe_for_comparison(exe: &str) -> &str {
    exe.strip_suffix(EXE_DELETED_SUFFIX).unwrap_or(exe)
}

/// A syntactically valid PAM service name: 1..=64 bytes of `[A-Za-z0-9._-]`.
fn valid_service(service: &str) -> bool {
    !service.is_empty()
        && service.len() <= MAX_SERVICE_BYTES
        && service
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The source class of a PAM service.
fn service_class(service: &str) -> SourceClass {
    if LOCK_SCREEN_SERVICES.contains(&service) {
        SourceClass::LockScreen
    } else if SUDO_SERVICES.contains(&service) {
        SourceClass::Sudo
    } else if LOGIN_SERVICES.contains(&service) {
        SourceClass::Login
    } else {
        SourceClass::Other
    }
}

/// Maps a borrowed account name to its class (byte equality, case-sensitive); the name is
/// never copied, stored or compared after this.
fn account_class(name: &str, ctx: &TrustContext) -> AccountClass {
    if name == ctx.owner_login.0 {
        AccountClass::Owner
    } else if name == "root" {
        AccountClass::Root
    } else {
        AccountClass::Other
    }
}

/// Trust of a `pam_unix` / `pam_faillock` line (spec §2.4 trust table).
fn line_trust(entry: &JournalEntry, ctx: &TrustContext, class: SourceClass) -> (bool, HelperSide) {
    if entry.uid == 0 {
        return (true, HelperSide::Root);
    }
    let exe = entry
        .exe
        .as_deref()
        .map(|exe| exe_for_comparison(exe.as_str()));
    if exe.is_some_and(|exe| SETUID_AUTH_PROGRAMS.contains(&exe)) {
        return (true, HelperSide::Root);
    }
    if class == SourceClass::LockScreen
        && exe.is_some_and(|exe| ctx.lock_screen_programs.iter().any(|p| p == exe))
    {
        return (true, HelperSide::Owner);
    }
    (false, HelperSide::Owner)
}

/// `password check failed for user (NAME)` from `unix_chkpwd`.
fn classify_check(entry: &JournalEntry, ctx: &TrustContext, rest: &str) -> Option<Signal> {
    if entry.identifier.as_deref().map(String::as_str) != Some("unix_chkpwd") {
        return None;
    }
    if entry
        .comm
        .as_deref()
        .is_some_and(|comm| comm.as_str() != "unix_chkpwd")
    {
        return None;
    }
    if entry
        .exe
        .as_deref()
        .is_some_and(|exe| !UNIX_CHKPWD_PATHS.contains(&exe_for_comparison(exe.as_str())))
    {
        return None;
    }
    let name = rest.strip_suffix(')')?;
    if name.is_empty() || name.len() > MAX_CHECK_NAME_BYTES {
        return None;
    }
    let side = if entry.uid == 0 {
        HelperSide::Root
    } else {
        HelperSide::Owner
    };
    Some(Signal::Check {
        side,
        account: account_class(name, ctx),
        at_us: entry.realtime_us,
    })
}

/// `pam_unix(SVC:auth): authentication failure; … rhost=… user=NAME`.
fn classify_failure(entry: &JournalEntry, ctx: &TrustContext, rest: &str) -> Option<Signal> {
    let (service, details) = rest.split_once(":auth): authentication failure;")?;
    if !valid_service(service) {
        return None;
    }
    let class = service_class(service);
    let account = details
        .find("rhost=")
        .and_then(|at| details.get(at..))
        .and_then(|after_rhost| {
            // pam_unix writes ` user=` exactly once after `rhost=`; a second occurrence can
            // only come from typed text (a user or remote host name), so the name is then
            // ambiguous and the account stays `Other` rather than trusting either copy.
            let user_at = after_rhost.find(" user=")?;
            let name = after_rhost.get(user_at.checked_add(" user=".len())?..)?;
            if name.contains(" user=") {
                return None;
            }
            Some(name)
        })
        .map_or(AccountClass::Other, |name| account_class(name, ctx));
    let (trusted, side) = line_trust(entry, ctx, class);
    Some(Signal::Failure {
        class,
        side,
        account,
        at_us: entry.realtime_us,
        trusted,
    })
}

/// `pam_faillock(SVC:auth): User NAME is temporarily locked out due to DIGITS consecutive
/// failed login attempts` (trusted lines only).
fn classify_locked_out(entry: &JournalEntry, ctx: &TrustContext, rest: &str) -> Option<Signal> {
    let (service, details) = rest.split_once(":auth): User ")?;
    if !valid_service(service) {
        return None;
    }
    let details = details.strip_suffix(" consecutive failed login attempts")?;
    let (name, digits) = details.rsplit_once(" is temporarily locked out due to ")?;
    if digits.is_empty()
        || digits.len() > MAX_FAILLOCK_DIGITS
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let class = service_class(service);
    let (trusted, _) = line_trust(entry, ctx, class);
    if !trusted {
        return None;
    }
    Some(Signal::LockedOut {
        class,
        account: account_class(name, ctx),
        at_us: entry.realtime_us,
    })
}

/// Pure; `None` for every entry that is not a signal. Common gate: `_TRANSPORT=syslog`,
/// facility 10, `_UID` 0 or the owner's uid; then the three prefix-anchored grammars.
#[must_use]
pub fn classify_entry(entry: &JournalEntry, ctx: &TrustContext) -> Option<Signal> {
    if entry.transport.as_str() != "syslog"
        || entry.facility != AUTHPRIV_FACILITY
        || (entry.uid != 0 && entry.uid != ctx.owner_uid)
    {
        return None;
    }
    let message = entry.message.as_str();
    if let Some(rest) = message.strip_prefix("password check failed for user (") {
        return classify_check(entry, ctx, rest);
    }
    if let Some(rest) = message.strip_prefix("pam_unix(") {
        return classify_failure(entry, ctx, rest);
    }
    if let Some(rest) = message.strip_prefix("pam_faillock(") {
        return classify_locked_out(entry, ctx, rest);
    }
    None
}

/// A boxed, sendable future (object-safe source seam).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Journal reader failure (fixed text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    /// `journalctl` is not installed (spawn `ENOENT`).
    #[error("journal reader not found")]
    NotFound,
    /// Any other spawn error.
    #[error("journal reader failed to start")]
    Spawn,
    /// The probe saw no `_UID=0` entry: the account cannot read the system journal.
    #[error("system journal not readable")]
    NoAccess,
    /// The probe exceeded `JOURNAL_PROBE_TIMEOUT_MS`.
    #[error("journal probe timed out")]
    ProbeTimeout,
}

/// One read of a follower. No `Debug`.
pub enum LineRead {
    /// One complete line without its `\n`, at most `MAX_JOURNAL_LINE_BYTES`; wiped on drop.
    Line(Zeroizing<Vec<u8>>),
    /// A line longer than the bound: discarded up to and including its `\n`.
    Overlong,
    /// EOF or a read error: the follower ended.
    End,
}

/// The lines of one follower.
pub trait JournalLines: Send {
    /// The next line; cancel-safe.
    fn next_line(&mut self) -> BoxFuture<'_, LineRead>;

    /// Test seam only: the PID of the child process behind these lines, if any.
    #[doc(hidden)]
    fn child_pid(&self) -> Option<u32> {
        None
    }
}

/// Where journal lines come from (production: [`JournalctlSource`]).
pub trait JournalSource: Send + Sync + 'static {
    /// Bounded probe: `Ok(())` iff one line with `_UID` `"0"` was read within
    /// `JOURNAL_PROBE_TIMEOUT_MS`.
    fn probe(&self) -> BoxFuture<'_, Result<(), JournalError>>;

    /// Starts a follower.
    fn follow(
        &self,
        start: FollowStart,
    ) -> BoxFuture<'_, Result<Box<dyn JournalLines>, JournalError>>;
}

/// Production source: `journalctl` at [`JOURNALCTL_PATH`], environment cleared, working
/// directory `/`, stdin and stderr closed, stdout piped, killed when its lines are dropped.
#[derive(Debug, Default)]
pub struct JournalctlSource;

/// The single spawn helper of the crate.
fn spawn_journalctl(args: Vec<String>) -> Result<(Child, ChildStdout), JournalError> {
    let mut child = Command::new(JOURNALCTL_PATH)
        .args(args)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                JournalError::NotFound
            } else {
                JournalError::Spawn
            }
        })?;
    let stdout = child.stdout.take().ok_or(JournalError::Spawn)?;
    Ok((child, stdout))
}

/// Whether a probe line names `_UID` `"0"` (bounded parse of the read fields).
fn probe_line_is_root(line: &[u8]) -> bool {
    read_fields(line).is_ok_and(|fields| {
        fields
            .get(UID_SLOT)
            .and_then(Option::as_ref)
            .is_some_and(|uid| uid.as_str() == "0")
    })
}

/// The lines of a `journalctl` child; dropping it kills the child (tokio reaps it).
struct ChildLines {
    child: Child,
    reader: BoundedLineReader<ChildStdout>,
}

impl JournalLines for ChildLines {
    fn next_line(&mut self) -> BoxFuture<'_, LineRead> {
        Box::pin(self.reader.next_line())
    }

    fn child_pid(&self) -> Option<u32> {
        self.child.id()
    }
}

impl JournalSource for JournalctlSource {
    fn probe(&self) -> BoxFuture<'_, Result<(), JournalError>> {
        Box::pin(async {
            let (child, stdout) = spawn_journalctl(probe_args())?;
            let mut reader = BoundedLineReader::new(stdout);
            let outcome = tokio::time::timeout(
                Duration::from_millis(JOURNAL_PROBE_TIMEOUT_MS),
                reader.next_line(),
            )
            .await;
            drop(reader);
            drop(child);
            match outcome {
                Err(_) => Err(JournalError::ProbeTimeout),
                Ok(LineRead::Line(line)) if probe_line_is_root(&line) => Ok(()),
                Ok(_) => Err(JournalError::NoAccess),
            }
        })
    }

    fn follow(
        &self,
        start: FollowStart,
    ) -> BoxFuture<'_, Result<Box<dyn JournalLines>, JournalError>> {
        Box::pin(async move {
            let (child, stdout) = spawn_journalctl(follow_args(&start))?;
            Ok(Box::new(ChildLines {
                child,
                reader: BoundedLineReader::new(stdout),
            }) as Box<dyn JournalLines>)
        })
    }
}

/// Bounded, wiped line reader. It owns a fixed buffer of
/// `MAX_JOURNAL_LINE_BYTES + JOURNAL_READ_CHUNK_BYTES` bytes allocated once (never
/// reallocated, wiped on drop); consumed and discarded bytes are zeroized before the buffer
/// is reused. No `BufReader` (whose buffer could not be wiped).
pub struct BoundedLineReader<R: AsyncRead + Unpin> {
    inner: R,
    /// Fixed-length storage; only `..filled` holds pending bytes.
    buf: Zeroizing<Vec<u8>>,
    filled: usize,
    /// Inside a line already reported as too long: skip up to its `\n`.
    discarding: bool,
    eof: bool,
}

impl<R: AsyncRead + Unpin> BoundedLineReader<R> {
    /// A reader over `inner` with its buffer allocated once.
    #[must_use]
    pub fn new(inner: R) -> Self {
        let size = MAX_JOURNAL_LINE_BYTES.saturating_add(JOURNAL_READ_CHUNK_BYTES);
        Self {
            inner,
            buf: Zeroizing::new(vec![0u8; size]),
            filled: 0,
            discarding: false,
            eof: false,
        }
    }

    /// Test seam: the buffer capacity (constant after construction).
    #[doc(hidden)]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// Drops the first `count` pending bytes: wipes them, moves the rest to the front and
    /// wipes the stale tail.
    fn consume(&mut self, count: usize) {
        let count = count.min(self.filled);
        let filled = self.filled;
        if let Some(head) = self.buf.get_mut(..count) {
            head.zeroize();
        }
        self.buf.copy_within(count..filled, 0);
        let remaining = filled.saturating_sub(count);
        if let Some(stale) = self.buf.get_mut(remaining..filled) {
            stale.zeroize();
        }
        self.filled = remaining;
    }

    /// Wipes every pending byte.
    fn clear(&mut self) {
        let filled = self.filled;
        if let Some(pending) = self.buf.get_mut(..filled) {
            pending.zeroize();
        }
        self.filled = 0;
    }

    /// Next line. Cancel-safe: the pending state changes only right after a completed read,
    /// with no await in between. A line longer than `MAX_JOURNAL_LINE_BYTES` is `Overlong`
    /// (once, at its `\n`); a partial last line at EOF is wiped and dropped (`End`); a read
    /// error is `End`.
    pub async fn next_line(&mut self) -> LineRead {
        loop {
            let newline = self
                .buf
                .get(..self.filled)
                .and_then(|pending| pending.iter().position(|b| *b == b'\n'));
            if let Some(at) = newline {
                let line_end = at.saturating_add(1);
                if self.discarding || at > MAX_JOURNAL_LINE_BYTES {
                    self.discarding = false;
                    self.consume(line_end);
                    return LineRead::Overlong;
                }
                let line = Zeroizing::new(self.buf.get(..at).unwrap_or_default().to_vec());
                self.consume(line_end);
                return LineRead::Line(line);
            }
            if self.filled > MAX_JOURNAL_LINE_BYTES {
                self.clear();
                self.discarding = true;
            }
            if self.eof {
                self.clear();
                self.discarding = false;
                return LineRead::End;
            }
            let start = self.filled;
            let end = start
                .saturating_add(JOURNAL_READ_CHUNK_BYTES)
                .min(self.buf.len());
            let Some(window) = self.buf.get_mut(start..end) else {
                self.eof = true;
                continue;
            };
            match self.inner.read(window).await {
                Ok(0) | Err(_) => self.eof = true,
                Ok(read) => self.filled = start.saturating_add(read).min(end),
            }
        }
    }
}
