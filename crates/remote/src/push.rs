//! Web Push runtime of `soos-remote` (ADR 2026-10-06 "Web Push Notifications for
//! Failed-Password Alerts Through a Separate Sender Unit", architect spec
//! `AI/architect_spec_remote_web_push.md` §5): the `0600` subscription store, the pure
//! coalescing scheduler, the notification payload, the transport seam with its production
//! Unix-socket client of `soos-push-sender`, the dispatcher task and the route helpers.
//!
//! Privacy (O-2): a notification is built only from enums and counts through a fixed
//! vocabulary; no journal text, no account name and never a typed password reaches it.
//! Nothing here logs through a `tracing` macro: the five push audit lines are fixed-text
//! events of [`crate::audit`]. No endpoint (a capability URL), key, JWT or ciphertext is
//! ever logged, echoed or returned.
//!
//! Lock order: the only nesting is *alerts runtime mutex → scheduler mutex* (the live sink).
//! The dispatcher takes the scheduler mutex alone and briefly; no mutex is held across an
//! `.await`, a store `flock` or a transport call.

use std::collections::VecDeque;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64ct::{Base64UrlUnpadded, Encoding};
use serde::{Deserialize, Deserializer, Serialize};
use soos_push_protocol::{
    decode_reply, encode_request, frame_len, DeliveryReply, DeliveryRequest, EndpointError,
    Outcome, PushEndpoint, PushHost, MAX_PUSH_REPLY_BYTES, PUSH_FRAME_IO_TIMEOUT_MS,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::Notify;
use tokio::time::{sleep, sleep_until, timeout, Instant};
use zeroize::Zeroizing;

use crate::alerts::{Attempt, AttemptKind, LiveAttemptSink};
use crate::audit;
use crate::auth::RandomSource;
use crate::config::PushPreviews;
use crate::credentials::{
    check_parent_dir, read_owned_file_with_capacity, try_lock_file, write_atomic, StoreError,
    StoreLock, STORE_LOCK_RETRY_MS,
};
use crate::journal::{AccountClass, BoxFuture, SourceClass};
use crate::server::UnixClock;
use crate::webpush::{
    encode_subscription_keys, encrypt, parse_subscription_keys, JwtCache, UaKeys, VapidKey,
    WebPushError,
};
use crate::{
    MAX_PUSH_PLAINTEXT_BYTES, MAX_PUSH_STORE_BYTES, MAX_PUSH_SUBSCRIBE_BODY_BYTES,
    MAX_PUSH_SUBSCRIPTIONS, PUSH_COALESCE_MS, PUSH_CONNECT_UNIX_TIMEOUT_MS,
    PUSH_EXCHANGE_TIMEOUT_MS, PUSH_MAX_PER_HOUR, PUSH_MIN_INTERVAL_MS, PUSH_RETRY_DELAYS_MS,
    PUSH_ROUTE_MIN_INTERVAL_MS, PUSH_TEST_MIN_INTERVAL_MS, PUSH_TEST_TOPIC, PUSH_TOPIC, PUSH_TTL_S,
    PUSH_URGENCY, STORE_LOCK_TIMEOUT_MS,
};

/// The rolling window of `PUSH_MAX_PER_HOUR`.
const HOUR_MS: u64 = 3_600_000;
/// Version of the store file.
const STORE_VERSION: u32 = 1;
/// The Declarative Web Push marker (`"web_push": 8030`).
const DECLARATIVE_WEB_PUSH: u32 = 8030;
/// Version of the `soos` payload member.
const PAYLOAD_VERSION: u32 = 1;

// ---------------------------------------------------------------------------------------
// Store (spec §5.1)
// ---------------------------------------------------------------------------------------

/// One stored subscription. No `Debug`.
pub struct PushSubscription {
    /// The validated endpoint.
    pub endpoint: PushEndpoint,
    /// The phone's keys.
    pub keys: UaKeys,
    /// When it was added (Unix seconds).
    pub created_unix_s: u64,
}

impl PushSubscription {
    fn duplicate(&self) -> Self {
        Self {
            endpoint: self.endpoint.clone(),
            keys: self.keys.clone(),
            created_unix_s: self.created_unix_s,
        }
    }
}

/// The parsed store. No `Debug`.
pub struct PushStoreFile {
    /// The VAPID key.
    pub key: VapidKey,
    /// At most `MAX_PUSH_SUBSCRIPTIONS` subscriptions, in insertion order.
    pub subscriptions: Vec<PushSubscription>,
}

/// Store failure. Fixed texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PushStoreError {
    /// Insecure (symlink, mode, owner, not a regular file) or oversized.
    #[error("push store refused")]
    Refused,
    /// Not a valid store (never repaired).
    #[error("push store malformed")]
    Malformed,
    /// An I/O operation failed.
    #[error("push store input/output failed")]
    Io,
    /// Another writer holds the store lock.
    #[error("push store busy")]
    Locked,
    /// The random source failed.
    #[error("random source failed")]
    Random,
    /// The store already holds `MAX_PUSH_SUBSCRIPTIONS` subscriptions.
    #[error("too many push subscriptions")]
    TooMany,
    /// The store file does not exist (only service start and `push reset` create it).
    #[error("push store missing")]
    Missing,
}

/// Result of [`PushStore::upsert`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertResult {
    /// A new endpoint was stored.
    Added,
    /// An existing endpoint got new keys (creation time kept).
    Replaced,
}

/// A secret string that is wiped on drop, even when parsing fails half-way.
struct SecretText(Zeroizing<String>);

impl<'de> Deserialize<'de> for SecretText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|text| Self(Zeroizing::new(text)))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreIn {
    version: u32,
    vapid_private_key: SecretText,
    subscriptions: Vec<SubscriptionIn>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubscriptionIn {
    endpoint: String,
    p256dh: String,
    auth: SecretText,
    created_unix_s: u64,
}

#[derive(Serialize)]
struct StoreOut<'a> {
    version: u32,
    vapid_private_key: &'a str,
    subscriptions: Vec<SubscriptionOut<'a>>,
}

#[derive(Serialize)]
struct SubscriptionOut<'a> {
    endpoint: &'a str,
    p256dh: &'a str,
    auth: &'a str,
    created_unix_s: u64,
}

fn parse_store(bytes: &[u8]) -> Result<PushStoreFile, PushStoreError> {
    let parsed: StoreIn = serde_json::from_slice(bytes).map_err(|_| PushStoreError::Malformed)?;
    if parsed.version != STORE_VERSION || parsed.subscriptions.len() > MAX_PUSH_SUBSCRIPTIONS {
        return Err(PushStoreError::Malformed);
    }
    let raw = Zeroizing::new(
        Base64UrlUnpadded::decode_vec(&parsed.vapid_private_key.0)
            .map_err(|_| PushStoreError::Malformed)?,
    );
    let scalar: Zeroizing<[u8; 32]> = Zeroizing::new(
        raw.as_slice()
            .try_into()
            .map_err(|_| PushStoreError::Malformed)?,
    );
    let key = VapidKey::from_bytes(&scalar).map_err(|_| PushStoreError::Malformed)?;
    let mut subscriptions: Vec<PushSubscription> = Vec::with_capacity(parsed.subscriptions.len());
    for entry in &parsed.subscriptions {
        let endpoint =
            PushEndpoint::parse(&entry.endpoint).map_err(|_| PushStoreError::Malformed)?;
        if subscriptions.iter().any(|s| s.endpoint == endpoint) {
            return Err(PushStoreError::Malformed);
        }
        let keys = parse_subscription_keys(&entry.p256dh, &entry.auth.0)
            .map_err(|_| PushStoreError::Malformed)?;
        subscriptions.push(PushSubscription {
            endpoint,
            keys,
            created_unix_s: entry.created_unix_s,
        });
    }
    Ok(PushStoreFile { key, subscriptions })
}

fn serialize_store(file: &PushStoreFile) -> Result<Zeroizing<Vec<u8>>, PushStoreError> {
    let private = Zeroizing::new(Base64UrlUnpadded::encode_string(&file.key.to_bytes()[..]));
    let encoded: Vec<(String, Zeroizing<String>)> = file
        .subscriptions
        .iter()
        .map(|s| encode_subscription_keys(&s.keys))
        .collect();
    let out = StoreOut {
        version: STORE_VERSION,
        vapid_private_key: &private,
        subscriptions: file
            .subscriptions
            .iter()
            .zip(&encoded)
            .map(|(s, (p256dh, auth))| SubscriptionOut {
                endpoint: s.endpoint.as_str(),
                p256dh,
                auth,
                created_unix_s: s.created_unix_s,
            })
            .collect(),
    };
    let bytes = Zeroizing::new(serde_json::to_vec(&out).map_err(|_| PushStoreError::Io)?);
    if bytes.len() > MAX_PUSH_STORE_BYTES {
        return Err(PushStoreError::Io);
    }
    Ok(bytes)
}

fn from_store_error(err: StoreError) -> PushStoreError {
    match err {
        StoreError::Insecure | StoreError::TooLarge => PushStoreError::Refused,
        StoreError::Malformed => PushStoreError::Malformed,
        StoreError::Busy => PushStoreError::Locked,
        _ => PushStoreError::Io,
    }
}

/// The Web Push store file (`remote-push.json`, `0600`, owner-checked, ≤ 16 KiB). Every use
/// re-reads the file; only [`PushStore::load_or_create`] (service start) and
/// [`PushStore::reset`] (`soos-remote push reset`) ever create it.
#[derive(Clone)]
pub struct PushStore {
    path: PathBuf,
    owner_uid: u32,
}

impl std::fmt::Debug for PushStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PushStore")
    }
}

impl PushStore {
    /// Store at `path`, owned by `owner_uid`.
    #[must_use]
    pub fn new(path: PathBuf, owner_uid: u32) -> Self {
        Self { path, owner_uid }
    }

    /// The store path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Absent → `Ok(None)`. Present → strict checks (regular file, owner, no group/other
    /// bits, ≤ `MAX_PUSH_STORE_BYTES`, `O_NOFOLLOW`), then the JSON, every entry re-validated.
    ///
    /// # Errors
    /// [`PushStoreError::Refused`], [`PushStoreError::Malformed`], [`PushStoreError::Io`].
    pub fn load(&self) -> Result<Option<PushStoreFile>, PushStoreError> {
        let read = read_owned_file_with_capacity(
            &self.path,
            self.owner_uid,
            MAX_PUSH_STORE_BYTES,
            MAX_PUSH_STORE_BYTES.saturating_add(1),
        )
        .map_err(from_store_error)?;
        match read {
            None => Ok(None),
            Some((bytes, _)) => {
                let bytes = Zeroizing::new(bytes);
                parse_store(&bytes).map(Some)
            }
        }
    }

    fn lock(&self) -> Result<StoreLock, PushStoreError> {
        match try_lock_file(&self.path, self.owner_uid) {
            Ok(Some(lock)) => Ok(lock),
            Ok(None) => Err(PushStoreError::Locked),
            Err(err) => Err(from_store_error(err)),
        }
    }

    fn write(&self, file: &PushStoreFile, random: &RandomSource) -> Result<(), PushStoreError> {
        let dir = check_parent_dir(&self.path, self.owner_uid).map_err(from_store_error)?;
        let bytes = serialize_store(file)?;
        write_atomic(&self.path, &dir, &bytes, random).map_err(from_store_error)
    }

    /// Service start only: an absent file is created with a fresh key and no subscription
    /// (temp file + `fsync` + `rename` under the lock); a present file is never written.
    ///
    /// # Errors
    /// The errors of [`PushStore::load`], [`PushStoreError::Random`] (nothing created),
    /// [`PushStoreError::Locked`], [`PushStoreError::Io`].
    pub fn load_or_create(&self, random: &RandomSource) -> Result<PushStoreFile, PushStoreError> {
        if let Some(file) = self.load()? {
            return Ok(file);
        }
        let key = VapidKey::generate(random).map_err(|_| PushStoreError::Random)?;
        let _lock = self.lock()?;
        if let Some(file) = self.load()? {
            return Ok(file);
        }
        let file = PushStoreFile {
            key,
            subscriptions: Vec::new(),
        };
        self.write(&file, random)?;
        Ok(file)
    }

    /// Locked read-modify-write; an absent file is [`PushStoreError::Missing`] (nothing
    /// created).
    fn update<T>(
        &self,
        random: &RandomSource,
        f: impl FnOnce(&mut PushStoreFile) -> Result<Option<T>, PushStoreError>,
        unchanged: T,
    ) -> Result<T, PushStoreError> {
        let _lock = self.lock()?;
        let mut file = self.load()?.ok_or(PushStoreError::Missing)?;
        match f(&mut file)? {
            Some(result) => {
                self.write(&file, random)?;
                Ok(result)
            }
            None => Ok(unchanged),
        }
    }

    /// Adds `sub`, or replaces the keys of its endpoint (creation time kept).
    ///
    /// # Errors
    /// [`PushStoreError::TooMany`] for a new endpoint on a full store (nothing written),
    /// [`PushStoreError::Missing`], and the load/lock/write errors.
    pub fn upsert(
        &self,
        sub: PushSubscription,
        random: &RandomSource,
    ) -> Result<UpsertResult, PushStoreError> {
        self.upsert_ref(&sub, random)
    }

    fn upsert_ref(
        &self,
        sub: &PushSubscription,
        random: &RandomSource,
    ) -> Result<UpsertResult, PushStoreError> {
        self.update(
            random,
            |file| {
                if let Some(existing) = file
                    .subscriptions
                    .iter_mut()
                    .find(|s| s.endpoint == sub.endpoint)
                {
                    existing.keys = sub.keys.clone();
                    return Ok(Some(UpsertResult::Replaced));
                }
                if file.subscriptions.len() >= MAX_PUSH_SUBSCRIPTIONS {
                    return Err(PushStoreError::TooMany);
                }
                file.subscriptions.push(sub.duplicate());
                Ok(Some(UpsertResult::Added))
            },
            UpsertResult::Replaced,
        )
    }

    /// Removes `endpoint`; `Ok(false)` when absent (nothing written).
    ///
    /// # Errors
    /// [`PushStoreError::Missing`] and the load/lock/write errors.
    pub fn remove_endpoint(
        &self,
        endpoint: &PushEndpoint,
        random: &RandomSource,
    ) -> Result<bool, PushStoreError> {
        self.update(
            random,
            |file| {
                let before = file.subscriptions.len();
                file.subscriptions.retain(|s| s.endpoint != *endpoint);
                Ok((file.subscriptions.len() != before).then_some(true))
            },
            false,
        )
    }

    /// CLI: removes subscription `index` (1-based); `Ok(false)` when out of range.
    ///
    /// # Errors
    /// [`PushStoreError::Missing`] and the load/lock/write errors.
    pub fn remove_index(
        &self,
        index: usize,
        random: &RandomSource,
    ) -> Result<bool, PushStoreError> {
        self.update(
            random,
            |file| {
                let Some(position) = index.checked_sub(1) else {
                    return Ok(None);
                };
                if position >= file.subscriptions.len() {
                    return Ok(None);
                }
                file.subscriptions.remove(position);
                Ok(Some(true))
            },
            false,
        )
    }

    /// The path must be absent or a regular file owned by the uid (never followed).
    fn check_resettable(&self) -> Result<(), PushStoreError> {
        match fs::symlink_metadata(&self.path) {
            Ok(meta) if meta.file_type().is_file() && meta.uid() == self.owner_uid => Ok(()),
            Ok(_) => Err(PushStoreError::Refused),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(_) => Err(PushStoreError::Io),
        }
    }

    /// CLI `push reset`: replaces the file (any mode, any content, as long as it is a regular
    /// file of the uid) with a fresh key and no subscription.
    ///
    /// # Errors
    /// [`PushStoreError::Refused`] (symlink, foreign owner, not a file),
    /// [`PushStoreError::Random`] (old file untouched), lock and write errors.
    pub fn reset(&self, random: &RandomSource) -> Result<PushStoreFile, PushStoreError> {
        self.check_resettable()?;
        let key = VapidKey::generate(random).map_err(|_| PushStoreError::Random)?;
        let _lock = self.lock()?;
        self.check_resettable()?;
        let file = PushStoreFile {
            key,
            subscriptions: Vec::new(),
        };
        self.write(&file, random)?;
        Ok(file)
    }
}

/// Runs `op` until it is not [`PushStoreError::Locked`], sleeping `STORE_LOCK_RETRY_MS`
/// between attempts for at most `STORE_LOCK_TIMEOUT_MS` (never a blocking `flock`).
///
/// Each attempt (open, read, `fsync`, `rename` of one local file of at most
/// `MAX_PUSH_STORE_BYTES`) runs on the blocking pool through `spawn_blocking`, so a slow
/// disk never stalls the other connections of the current-thread runtime (candid review
/// 2026-10-06). A failed join (the attempt panicked or was cancelled) is
/// [`PushStoreError::Io`], fail-closed.
async fn with_store_lock<T: Send + 'static>(
    op: impl Fn() -> Result<T, PushStoreError> + Send + Sync + 'static,
) -> Result<T, PushStoreError> {
    let op = Arc::new(op);
    let started = Instant::now();
    loop {
        let attempt = Arc::clone(&op);
        let result = tokio::task::spawn_blocking(move || attempt())
            .await
            .unwrap_or(Err(PushStoreError::Io));
        match result {
            Err(PushStoreError::Locked)
                if started.elapsed() < Duration::from_millis(STORE_LOCK_TIMEOUT_MS) =>
            {
                sleep(Duration::from_millis(STORE_LOCK_RETRY_MS)).await;
            }
            other => return other,
        }
    }
}

/// One `soos-remote push list` line: `<index>  <host>  created <unix_s>` (never the endpoint
/// path or a key).
#[must_use]
pub fn subscription_line(index: usize, sub: &PushSubscription) -> String {
    format!(
        "{index}  {}  created {}",
        sub.endpoint.host(),
        sub.created_unix_s
    )
}

// ---------------------------------------------------------------------------------------
// Scheduler (spec §5.2)
// ---------------------------------------------------------------------------------------

/// Accumulated live attempts since the last send (enums, counts and times only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PushSummary {
    /// Wrong passwords (saturating).
    pub wrong_password: u32,
    /// Attempts while locked out (saturating).
    pub locked_out: u32,
    /// Source class of the newest attempt.
    pub newest_source: SourceClass,
    /// Account class of the newest attempt.
    pub newest_account: AccountClass,
    /// Kind of the newest attempt.
    pub newest_kind: AttemptKind,
    /// Journal time (ms) of the oldest attempt.
    pub first_unix_ms: u64,
    /// Journal time (ms) of the newest attempt.
    pub last_unix_ms: u64,
}

impl PushSummary {
    fn of(attempt: Attempt) -> Self {
        let ms = attempt.at_us / 1000;
        let (wrong_password, locked_out) = match attempt.kind {
            AttemptKind::WrongPassword => (1, 0),
            AttemptKind::LockedOut => (0, 1),
        };
        Self {
            wrong_password,
            locked_out,
            newest_source: attempt.class,
            newest_account: attempt.account,
            newest_kind: attempt.kind,
            first_unix_ms: ms,
            last_unix_ms: ms,
        }
    }

    /// Pure fold of an undelivered (older) summary into a newer one: counts add
    /// (saturating), the time span widens, `newest_*` come from the operand with the greater
    /// `last_unix_ms` (ties: `newer`).
    #[must_use]
    pub fn merge(older: &PushSummary, newer: &PushSummary) -> PushSummary {
        let newest = if older.last_unix_ms > newer.last_unix_ms {
            older
        } else {
            newer
        };
        PushSummary {
            wrong_password: older.wrong_password.saturating_add(newer.wrong_password),
            locked_out: older.locked_out.saturating_add(newer.locked_out),
            newest_source: newest.newest_source,
            newest_account: newest.newest_account,
            newest_kind: newest.newest_kind,
            first_unix_ms: older.first_unix_ms.min(newer.first_unix_ms),
            last_unix_ms: older.last_unix_ms.max(newer.last_unix_ms),
        }
    }
}

/// The pending summary and its scheduling facts.
#[derive(Debug, Clone, Copy)]
struct Pending {
    summary: PushSummary,
    first_noted_ms: u64,
    newest_at_us: u64,
}

/// Pure coalescing scheduler: the first live attempt after a quiet period is sent
/// `PUSH_COALESCE_MS` later, then at most one summary per `PUSH_MIN_INTERVAL_MS` and
/// `PUSH_MAX_PER_HOUR` per rolling hour (monotonic milliseconds).
#[derive(Debug, Default)]
pub struct PushScheduler {
    pending: Option<Pending>,
    last_sent_ms: Option<u64>,
    sent: VecDeque<u64>,
}

impl PushScheduler {
    /// An idle scheduler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one live attempt noted at monotonic `now_ms`.
    pub fn note(&mut self, attempt: Attempt, now_ms: u64) {
        match self.pending.as_mut() {
            None => {
                self.pending = Some(Pending {
                    summary: PushSummary::of(attempt),
                    first_noted_ms: now_ms,
                    newest_at_us: attempt.at_us,
                });
            }
            Some(pending) => {
                let s = &mut pending.summary;
                match attempt.kind {
                    AttemptKind::WrongPassword => {
                        s.wrong_password = s.wrong_password.saturating_add(1);
                    }
                    AttemptKind::LockedOut => s.locked_out = s.locked_out.saturating_add(1),
                }
                let ms = attempt.at_us / 1000;
                s.first_unix_ms = s.first_unix_ms.min(ms);
                s.last_unix_ms = s.last_unix_ms.max(ms);
                if attempt.at_us >= pending.newest_at_us {
                    pending.newest_at_us = attempt.at_us;
                    s.newest_source = attempt.class;
                    s.newest_account = attempt.account;
                    s.newest_kind = attempt.kind;
                }
            }
        }
    }

    /// When the pending summary may be sent; `None` without a pending summary.
    #[must_use]
    pub fn due_at(&self) -> Option<u64> {
        let pending = self.pending.as_ref()?;
        let mut due = pending.first_noted_ms.saturating_add(PUSH_COALESCE_MS);
        if let Some(last) = self.last_sent_ms {
            due = due.max(last.saturating_add(PUSH_MIN_INTERVAL_MS));
        }
        if self.sent.len() >= usize::try_from(PUSH_MAX_PER_HOUR).unwrap_or(usize::MAX) {
            if let Some(oldest) = self.sent.front() {
                due = due.max(oldest.saturating_add(HOUR_MS));
            }
        }
        Some(due)
    }

    /// The pending summary when it is due at `now_ms` (the send is recorded).
    pub fn take(&mut self, now_ms: u64) -> Option<PushSummary> {
        let due = self.due_at()?;
        if due > now_ms {
            return None;
        }
        let pending = self.pending.take()?;
        while self
            .sent
            .front()
            .is_some_and(|t| t.saturating_add(HOUR_MS) <= now_ms)
        {
            self.sent.pop_front();
        }
        let cap = usize::try_from(PUSH_MAX_PER_HOUR).unwrap_or(usize::MAX);
        while self.sent.len() >= cap {
            self.sent.pop_front();
        }
        self.sent.push_back(now_ms);
        self.last_sent_ms = Some(now_ms);
        Some(pending.summary)
    }
}

// ---------------------------------------------------------------------------------------
// Payload (spec §5.4)
// ---------------------------------------------------------------------------------------

#[derive(Serialize)]
struct Payload<'a> {
    web_push: u32,
    notification: NotificationText<'a>,
    soos: PayloadSoos,
}

#[derive(Serialize)]
struct NotificationText<'a> {
    title: &'a str,
    body: &'a str,
    navigate: String,
    lang: &'a str,
}

#[derive(Serialize)]
struct PayloadSoos {
    v: u32,
    kind: &'static str,
    wrong_password: u32,
    locked_out: u32,
    source: Option<SourceClass>,
    account: Option<AccountClass>,
    last_unix_ms: Option<u64>,
}

fn source_label(source: SourceClass) -> &'static str {
    match source {
        SourceClass::LockScreen => "lock screen",
        SourceClass::Sudo => "sudo",
        SourceClass::Login => "login",
        SourceClass::Other => "other",
    }
}

fn account_label(account: AccountClass) -> &'static str {
    match account {
        AccountClass::Owner => "your account",
        AccountClass::Root => "root",
        AccountClass::Other => "another account",
    }
}

fn counted(n: u32, singular: &str, plural: &str) -> String {
    if n == 1 {
        format!("{n} {singular}")
    } else {
        format!("{n} {plural}")
    }
}

fn render(payload: &Payload<'_>) -> Result<Vec<u8>, WebPushError> {
    let bytes = serde_json::to_vec(payload).map_err(|_| WebPushError::Encode)?;
    if bytes.len() > MAX_PUSH_PLAINTEXT_BYTES {
        return Err(WebPushError::PlaintextTooLarge);
    }
    Ok(bytes)
}

/// The Declarative Web Push JSON of an alert summary (≤ `MAX_PUSH_PLAINTEXT_BYTES`), built
/// only from enums and counts.
///
/// # Errors
/// [`WebPushError::Encode`], [`WebPushError::PlaintextTooLarge`].
pub fn alert_payload(
    summary: &PushSummary,
    previews: PushPreviews,
    rp_id: &str,
) -> Result<Vec<u8>, WebPushError> {
    let (title, body, source, account) = match previews {
        PushPreviews::Generic => (
            "Security alert on your PC",
            "Open soos for details".to_string(),
            None,
            None,
        ),
        PushPreviews::Detailed => {
            let place = format!(
                "{}, {}",
                source_label(summary.newest_source),
                account_label(summary.newest_account)
            );
            let locked = counted(
                summary.locked_out,
                "attempt while locked out",
                "attempts while locked out",
            );
            let body = if summary.wrong_password == 0 && summary.locked_out > 0 {
                format!("{locked} \u{2014} {place}")
            } else {
                let wrong = counted(summary.wrong_password, "wrong password", "wrong passwords");
                if summary.locked_out > 0 {
                    format!("{wrong} \u{2014} {place} and {locked}")
                } else {
                    format!("{wrong} \u{2014} {place}")
                }
            };
            (
                "Failed password on your PC",
                body,
                Some(summary.newest_source),
                Some(summary.newest_account),
            )
        }
    };
    render(&Payload {
        web_push: DECLARATIVE_WEB_PUSH,
        notification: NotificationText {
            title,
            body: &body,
            navigate: format!("https://{rp_id}/"),
            lang: "en",
        },
        soos: PayloadSoos {
            v: PAYLOAD_VERSION,
            kind: "alerts",
            wrong_password: summary.wrong_password,
            locked_out: summary.locked_out,
            source,
            account,
            last_unix_ms: Some(summary.last_unix_ms),
        },
    })
}

/// The test notification payload.
///
/// # Errors
/// [`WebPushError::Encode`], [`WebPushError::PlaintextTooLarge`].
pub fn test_payload(rp_id: &str) -> Result<Vec<u8>, WebPushError> {
    render(&Payload {
        web_push: DECLARATIVE_WEB_PUSH,
        notification: NotificationText {
            title: "soos test notification",
            body: "Notifications from your PC work",
            navigate: format!("https://{rp_id}/"),
            lang: "en",
        },
        soos: PayloadSoos {
            v: PAYLOAD_VERSION,
            kind: "test",
            wrong_password: 0,
            locked_out: 0,
            source: None,
            account: None,
            last_unix_ms: None,
        },
    })
}

// ---------------------------------------------------------------------------------------
// Transport (spec §5.5)
// ---------------------------------------------------------------------------------------

/// Transport failure. Fixed texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// The sender is not reachable (socket missing, not ours, connect failed, peer uid).
    #[error("push sender unavailable")]
    Unavailable,
    /// The exchange exceeded its bound.
    #[error("push sender timed out")]
    Timeout,
    /// The reply is malformed.
    #[error("push sender reply refused")]
    Protocol,
}

/// One delivery through the sender.
pub trait PushTransport: Send + Sync + 'static {
    /// Hands `request` to the sender and returns its reply.
    fn deliver(
        &self,
        request: DeliveryRequest,
    ) -> BoxFuture<'_, Result<DeliveryReply, TransportError>>;
}

/// Production transport: one connection to the `soos-push-sender` socket per request.
pub struct UnixPushTransport {
    socket_path: PathBuf,
    owner_uid: u32,
}

impl std::fmt::Debug for UnixPushTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UnixPushTransport")
    }
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

impl UnixPushTransport {
    /// The sender socket at `socket_path`, which must be a socket owned by `owner_uid`.
    #[must_use]
    pub fn new(socket_path: PathBuf, owner_uid: u32) -> Self {
        Self {
            socket_path,
            owner_uid,
        }
    }

    async fn exchange(&self, request: DeliveryRequest) -> Result<DeliveryReply, TransportError> {
        let meta =
            fs::symlink_metadata(&self.socket_path).map_err(|_| TransportError::Unavailable)?;
        if !meta.file_type().is_socket() || meta.uid() != self.owner_uid {
            return Err(TransportError::Unavailable);
        }
        let frame = encode_request(&request).map_err(|_| TransportError::Protocol)?;
        drop(request);
        let mut stream = timeout(
            ms(PUSH_CONNECT_UNIX_TIMEOUT_MS),
            UnixStream::connect(&self.socket_path),
        )
        .await
        .map_err(|_| TransportError::Unavailable)?
        .map_err(|_| TransportError::Unavailable)?;
        let peer = stream
            .peer_cred()
            .map_err(|_| TransportError::Unavailable)?;
        if peer.uid() != self.owner_uid {
            return Err(TransportError::Unavailable);
        }
        timeout(ms(PUSH_FRAME_IO_TIMEOUT_MS), stream.write_all(&frame))
            .await
            .map_err(|_| TransportError::Timeout)?
            .map_err(|_| TransportError::Unavailable)?;
        drop(frame);
        let mut prefix = [0u8; 4];
        stream
            .read_exact(&mut prefix)
            .await
            .map_err(|_| TransportError::Protocol)?;
        let len = frame_len(prefix, MAX_PUSH_REPLY_BYTES).map_err(|_| TransportError::Protocol)?;
        let mut payload = vec![0u8; len];
        timeout(
            ms(PUSH_FRAME_IO_TIMEOUT_MS),
            stream.read_exact(&mut payload),
        )
        .await
        .map_err(|_| TransportError::Timeout)?
        .map_err(|_| TransportError::Protocol)?;
        decode_reply(&payload).map_err(|_| TransportError::Protocol)
    }
}

impl PushTransport for UnixPushTransport {
    fn deliver(
        &self,
        request: DeliveryRequest,
    ) -> BoxFuture<'_, Result<DeliveryReply, TransportError>> {
        Box::pin(async move {
            timeout(ms(PUSH_EXCHANGE_TIMEOUT_MS), self.exchange(request))
                .await
                .unwrap_or(Err(TransportError::Timeout))
        })
    }
}

// ---------------------------------------------------------------------------------------
// Runtime and view (spec §5.7)
// ---------------------------------------------------------------------------------------

/// `state` of `GET /api/push`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushState {
    /// `push_notifications` is off.
    Disabled,
    /// Working.
    Active,
    /// The store cannot be used.
    Unavailable,
}

/// `reason` of an unavailable push view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushUnavailableReason {
    /// The store is invalid or insecure (never overwritten).
    StoreFailed,
    /// The key could not be drawn.
    RngFailed,
    /// The store was deleted while running (never recreated by the service).
    StoreMissing,
}

/// The outcome of the last delivery attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LastDelivery {
    /// Accepted by the push service.
    Delivered,
    /// The subscription no longer exists (removed).
    Gone,
    /// Refused by the push service or the sender (kept, not retried).
    Rejected,
    /// Not sent or retries exhausted.
    Failed,
}

/// Reachability of `soos-push-sender`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SenderState {
    /// No exchange yet.
    Unknown,
    /// The last exchange got a reply.
    Reachable,
    /// The last connection failed.
    Unavailable,
}

/// The push service of a subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushService {
    /// `web.push.apple.com` (Safari, iOS / iPadOS 16.4+ home-screen apps).
    Apple,
    /// `fcm.googleapis.com` (Chrome, Samsung Internet and other Chromium browsers, Android
    /// included).
    Google,
    /// `updates.push.services.mozilla.com` (Firefox, Android included).
    Mozilla,
}

impl PushService {
    /// The service of a validated endpoint; exhaustive, each variant from its own host only.
    #[must_use]
    pub fn of(endpoint: &PushEndpoint) -> Self {
        match endpoint.push_host() {
            PushHost::Apple => Self::Apple,
            PushHost::Google => Self::Google,
            PushHost::Mozilla => Self::Mozilla,
        }
    }
}

/// One registered device: service and creation time only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PushDeviceView {
    /// The push service.
    pub service: PushService,
    /// When it was added (Unix seconds).
    pub created_unix_s: u64,
}

/// JSON of `GET /api/push`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushView {
    /// State.
    pub state: PushState,
    /// `Some` iff `Unavailable`.
    pub reason: Option<PushUnavailableReason>,
    /// The VAPID public key (87 characters), only when `Active`.
    pub public_key: Option<String>,
    /// Stored subscriptions (0 unless `Active`).
    pub subscriptions: u32,
    /// One entry per stored subscription (empty unless `Active`).
    pub devices: Vec<PushDeviceView>,
    /// `None` until the first attempt.
    pub last_delivery: Option<LastDelivery>,
    /// Sender reachability.
    pub sender: SenderState,
}

impl PushView {
    /// The view of an unusable push runtime (no key, no device).
    fn unavailable(
        reason: PushUnavailableReason,
        last_delivery: Option<LastDelivery>,
        sender: SenderState,
    ) -> Self {
        Self {
            state: PushState::Unavailable,
            reason: Some(reason),
            public_key: None,
            subscriptions: 0,
            devices: Vec::new(),
            last_delivery,
            sender,
        }
    }

    /// The view while `push_notifications` is off.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            state: PushState::Disabled,
            reason: None,
            public_key: None,
            subscriptions: 0,
            devices: Vec::new(),
            last_delivery: None,
            sender: SenderState::Unknown,
        }
    }
}

/// What `main` resolves for the runtime.
#[derive(Debug, Clone)]
pub struct PushSettings {
    /// `remote-push.json` next to the credential store.
    pub store_path: PathBuf,
    /// VAPID `sub` (`vapid_subject`, else `https://<rp_id>`).
    pub subject: String,
    /// Lock-screen previews.
    pub previews: PushPreviews,
    /// The relying party (notification `navigate` origin).
    pub rp_id: String,
}

/// Mutable status shown by the view.
struct PushStatus {
    state: PushState,
    reason: Option<PushUnavailableReason>,
    last_delivery: Option<LastDelivery>,
    sender: SenderState,
    delivery_failing: bool,
}

/// A route answer: status and fixed result code.
pub(crate) type RouteAnswer = (u16, &'static str);

/// The push state shared by the routes, the live sink and the dispatcher.
pub(crate) struct PushRuntime {
    settings: PushSettings,
    store: PushStore,
    transport: Arc<dyn PushTransport>,
    random: RandomSource,
    clock: UnixClock,
    base: Instant,
    /// Set once by [`PushRuntime::start`] when the store is usable.
    active: AtomicBool,
    status: Mutex<PushStatus>,
    scheduler: Mutex<PushScheduler>,
    wake: Notify,
    test_queued: AtomicBool,
    /// Shared gate of subscribe and unsubscribe.
    pub(crate) route_gate: tokio::sync::Mutex<Option<Instant>>,
    /// Gate of the test notification.
    pub(crate) test_gate: tokio::sync::Mutex<Option<Instant>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubscribeKeys {
    p256dh: String,
    auth: SecretText,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct SubscribeBody {
    endpoint: String,
    #[serde(default)]
    #[allow(dead_code, reason = "Accepted and ignored (spec §6.2 step 5)")]
    expiration_time: Option<serde::de::IgnoredAny>,
    keys: SubscribeKeys,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnsubscribeBody {
    endpoint: String,
}

/// What one dispatch sends.
#[derive(Clone, Copy)]
enum Message {
    Alert(PushSummary),
    Test,
}

/// A scheduled retry of one alert summary for one subscription.
struct Retry {
    endpoint: PushEndpoint,
    summary: PushSummary,
    /// Index into `PUSH_RETRY_DELAYS_MS` of the next retry after this one fails.
    next: usize,
    due_ms: u64,
}

/// Dispatcher-local state (never shared, never behind a mutex).
#[derive(Default)]
struct DispatchState {
    retries: Vec<Retry>,
    carry: Vec<(PushEndpoint, PushSummary)>,
    known: Vec<PushEndpoint>,
    jwt: JwtCache,
    public_key: Option<String>,
}

impl DispatchState {
    fn fold_carry(&mut self, endpoint: &PushEndpoint, summary: PushSummary) {
        if let Some((_, existing)) = self.carry.iter_mut().find(|(e, _)| e == endpoint) {
            *existing = PushSummary::merge(existing, &summary);
        } else if self.carry.len() < MAX_PUSH_SUBSCRIPTIONS {
            self.carry.push((endpoint.clone(), summary));
        }
    }

    fn take_carry(&mut self, endpoint: &PushEndpoint) -> Option<PushSummary> {
        let position = self.carry.iter().position(|(e, _)| e == endpoint)?;
        Some(self.carry.remove(position).1)
    }

    fn after_load(&mut self, file: &PushStoreFile) {
        let public_key = file.key.public_key_b64();
        if self.public_key.as_deref() != Some(public_key.as_str()) {
            self.jwt.clear();
            self.public_key = Some(public_key);
        }
        self.known = file
            .subscriptions
            .iter()
            .map(|s| s.endpoint.clone())
            .collect();
        let known = &self.known;
        self.carry.retain(|(e, _)| known.contains(e));
        self.retries.retain(|r| known.contains(&r.endpoint));
    }
}

impl PushRuntime {
    /// The runtime before its store is opened (no file access, no random draw): `serve`
    /// creates it before the alerts runtime so that the live sink can be handed over, then
    /// calls [`PushRuntime::start`] after the alerts set-up.
    pub(crate) fn new(
        settings: PushSettings,
        transport: Arc<dyn PushTransport>,
        random: RandomSource,
        clock: UnixClock,
        file_owner_uid: u32,
    ) -> Arc<Self> {
        let store = PushStore::new(settings.store_path.clone(), file_owner_uid);
        Arc::new(Self {
            settings,
            store,
            transport,
            random,
            clock,
            base: Instant::now(),
            active: AtomicBool::new(false),
            status: Mutex::new(PushStatus {
                state: PushState::Unavailable,
                reason: Some(PushUnavailableReason::StoreFailed),
                last_delivery: None,
                sender: SenderState::Unknown,
                delivery_failing: false,
            }),
            scheduler: Mutex::new(PushScheduler::new()),
            wake: Notify::new(),
            test_queued: AtomicBool::new(false),
            route_gate: tokio::sync::Mutex::new(None),
            test_gate: tokio::sync::Mutex::new(None),
        })
    }

    /// Start: the store is loaded or created (`Active`), else the runtime is `Unavailable`
    /// (`store_failed` / `rng_failed`, the file untouched) with one audit line.
    pub(crate) fn start(&self) {
        let (state, reason) = match self.store.load_or_create(&self.random) {
            Ok(_) => (PushState::Active, None),
            Err(PushStoreError::Random) => (
                PushState::Unavailable,
                Some(PushUnavailableReason::RngFailed),
            ),
            Err(_) => (
                PushState::Unavailable,
                Some(PushUnavailableReason::StoreFailed),
            ),
        };
        self.with_status(|status| {
            status.state = state;
            status.reason = reason;
        });
        if state == PushState::Active {
            self.active.store(true, Ordering::SeqCst);
        } else {
            audit::push_unavailable();
        }
    }

    /// `true` when the store was usable at start.
    pub(crate) fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// The live-attempt sink handed to the alerts runtime: notes the attempt in the
    /// scheduler and wakes the dispatcher (no I/O, no await).
    pub(crate) fn live_sink(self: &Arc<Self>) -> LiveAttemptSink {
        let push = Arc::clone(self);
        Arc::new(move |attempt: Attempt| {
            if !push.is_active() {
                return;
            }
            let now = push.now_ms();
            if let Ok(mut scheduler) = push.scheduler.lock() {
                scheduler.note(attempt, now);
                drop(scheduler);
                push.wake.notify_one();
            }
        })
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(
            Instant::now()
                .saturating_duration_since(self.base)
                .as_millis(),
        )
        .unwrap_or(u64::MAX)
    }

    fn now_unix_s(&self) -> u64 {
        (self.clock)() / 1000
    }

    /// Reads the subscription store on the blocking pool (never on a Tokio worker thread),
    /// like the write paths of [`with_store_lock`].
    async fn load_store(&self) -> Result<Option<PushStoreFile>, PushStoreError> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.load())
            .await
            .unwrap_or(Err(PushStoreError::Io))
    }

    /// The status fields of the view (a poisoned status mutex reads as a store failure).
    fn status_snapshot(
        &self,
    ) -> (
        PushState,
        Option<PushUnavailableReason>,
        Option<LastDelivery>,
        SenderState,
    ) {
        match self.status.lock() {
            Ok(status) => (
                status.state,
                status.reason,
                status.last_delivery,
                status.sender,
            ),
            Err(_) => (
                PushState::Unavailable,
                Some(PushUnavailableReason::StoreFailed),
                None,
                SenderState::Unknown,
            ),
        }
    }

    /// `GET /api/push`. The store is read first (on the blocking pool) and the delivery
    /// status afterwards, so a device removal seen in the store is never paired with a
    /// status snapshot taken before that removal was recorded.
    pub(crate) async fn view(&self) -> PushView {
        let (state, reason, last_delivery, sender) = self.status_snapshot();
        if state != PushState::Active {
            return PushView::unavailable(
                reason.unwrap_or(PushUnavailableReason::StoreFailed),
                last_delivery,
                sender,
            );
        }
        let loaded = self.load_store().await;
        let (_, _, last_delivery, sender) = self.status_snapshot();
        match loaded {
            Ok(Some(file)) => PushView {
                state: PushState::Active,
                reason: None,
                public_key: Some(file.key.public_key_b64()),
                subscriptions: u32::try_from(file.subscriptions.len()).unwrap_or(u32::MAX),
                devices: file
                    .subscriptions
                    .iter()
                    .map(|s| PushDeviceView {
                        service: PushService::of(&s.endpoint),
                        created_unix_s: s.created_unix_s,
                    })
                    .collect(),
                last_delivery,
                sender,
            },
            Ok(None) => {
                PushView::unavailable(PushUnavailableReason::StoreMissing, last_delivery, sender)
            }
            Err(_) => {
                PushView::unavailable(PushUnavailableReason::StoreFailed, last_delivery, sender)
            }
        }
    }

    /// `503 unavailable` when the store was unusable at start.
    pub(crate) fn availability(&self) -> Result<(), RouteAnswer> {
        if self.is_active() {
            Ok(())
        } else {
            Err((503, "unavailable"))
        }
    }

    /// The shared subscribe/unsubscribe gate (`429` within `PUSH_ROUTE_MIN_INTERVAL_MS`).
    pub(crate) async fn pass_route_gate(&self) -> bool {
        pass_gate(&self.route_gate, PUSH_ROUTE_MIN_INTERVAL_MS).await
    }

    /// The test gate (`429` within `PUSH_TEST_MIN_INTERVAL_MS`).
    pub(crate) async fn pass_test_gate(&self) -> bool {
        pass_gate(&self.test_gate, PUSH_TEST_MIN_INTERVAL_MS).await
    }

    /// `POST /api/push/subscribe` after the gates and the bounded body read (§6.2 steps
    /// 4–8).
    pub(crate) async fn subscribe(&self, body: &[u8]) -> RouteAnswer {
        if body.len() > MAX_PUSH_SUBSCRIBE_BODY_BYTES {
            return (413, "body_too_large");
        }
        let Ok(parsed) = serde_json::from_slice::<SubscribeBody>(body) else {
            return (400, "bad_request");
        };
        let endpoint = match PushEndpoint::parse(&parsed.endpoint) {
            Ok(endpoint) => endpoint,
            Err(EndpointError::HostNotAllowed) => return (400, "unsupported_push_service"),
            Err(_) => return (400, "bad_request"),
        };
        let Ok(keys) = parse_subscription_keys(&parsed.keys.p256dh, &parsed.keys.auth.0) else {
            return (400, "bad_request");
        };
        let sub = PushSubscription {
            endpoint,
            keys,
            created_unix_s: self.now_unix_s(),
        };
        let (store, random) = (self.store.clone(), Arc::clone(&self.random));
        match with_store_lock(move || store.upsert_ref(&sub, &random)).await {
            Ok(UpsertResult::Added) => {
                audit::push_subscription_added();
                (200, "subscribed")
            }
            Ok(UpsertResult::Replaced) => (200, "subscribed"),
            Err(PushStoreError::TooMany) => (409, "too_many_subscriptions"),
            Err(_) => (503, "store_unavailable"),
        }
    }

    /// `POST /api/push/unsubscribe` after the gates and the bounded body read (§6.3): present
    /// and absent endpoints alike answer `200 unsubscribed` (no oracle).
    pub(crate) async fn unsubscribe(&self, body: &[u8]) -> RouteAnswer {
        if body.len() > MAX_PUSH_SUBSCRIBE_BODY_BYTES {
            return (413, "body_too_large");
        }
        let Ok(parsed) = serde_json::from_slice::<UnsubscribeBody>(body) else {
            return (400, "bad_request");
        };
        let Ok(endpoint) = PushEndpoint::parse(&parsed.endpoint) else {
            return (400, "bad_request");
        };
        let (store, random) = (self.store.clone(), Arc::clone(&self.random));
        match with_store_lock(move || store.remove_endpoint(&endpoint, &random)).await {
            Ok(true) => {
                audit::push_subscription_removed();
                (200, "unsubscribed")
            }
            Ok(false) => (200, "unsubscribed"),
            Err(_) => (503, "store_unavailable"),
        }
    }

    /// `POST /api/push/test` after the gates (§6.4 steps 4–5).
    pub(crate) async fn queue_test(&self) -> RouteAnswer {
        match self.load_store().await {
            Ok(Some(file)) if file.subscriptions.is_empty() => (409, "no_subscriptions"),
            Ok(Some(_)) => {
                self.test_queued.store(true, Ordering::SeqCst);
                self.wake.notify_one();
                (202, "test_queued")
            }
            Ok(None) | Err(_) => (503, "store_unavailable"),
        }
    }

    // --- status transitions ------------------------------------------------------------

    fn with_status(&self, f: impl FnOnce(&mut PushStatus)) {
        if let Ok(mut status) = self.status.lock() {
            f(&mut status);
        }
    }

    fn record_delivered(&self) {
        self.with_status(|s| {
            s.last_delivery = Some(LastDelivery::Delivered);
            s.delivery_failing = false;
        });
    }

    fn record_gone(&self) {
        self.with_status(|s| s.last_delivery = Some(LastDelivery::Gone));
    }

    fn record_failure(&self, outcome: LastDelivery) {
        let mut transition = false;
        self.with_status(|s| {
            s.last_delivery = Some(outcome);
            transition = !s.delivery_failing;
            s.delivery_failing = true;
        });
        if transition {
            audit::push_delivery_failed();
        }
    }

    fn sender_reachable(&self) {
        self.with_status(|s| s.sender = SenderState::Reachable);
    }

    fn sender_unavailable(&self) {
        let mut transition = false;
        self.with_status(|s| {
            transition = s.sender != SenderState::Unavailable;
            s.sender = SenderState::Unavailable;
        });
        if transition {
            audit::push_sender_unavailable();
        }
    }

    // --- dispatch ----------------------------------------------------------------------

    fn build_request(
        &self,
        state: &mut DispatchState,
        key: &VapidKey,
        sub: &PushSubscription,
        message: Message,
    ) -> Result<DeliveryRequest, WebPushError> {
        let (payload, topic) = match message {
            Message::Alert(summary) => (
                alert_payload(&summary, self.settings.previews, &self.settings.rp_id)?,
                PUSH_TOPIC,
            ),
            Message::Test => (test_payload(&self.settings.rp_id)?, PUSH_TEST_TOPIC),
        };
        let payload = Zeroizing::new(payload);
        let body = encrypt(&payload, &sub.keys, &self.random)?;
        let authorization = state.jwt.authorization(
            key,
            &sub.endpoint.origin(),
            &self.settings.subject,
            self.now_unix_s(),
        )?;
        Ok(DeliveryRequest {
            endpoint: sub.endpoint.clone(),
            authorization,
            ttl_s: PUSH_TTL_S,
            urgency: PUSH_URGENCY,
            topic: Some(topic.to_owned()),
            body,
        })
    }

    /// One bounded exchange (the outer bound also covers a transport without its own).
    async fn exchange(&self, request: DeliveryRequest) -> Result<DeliveryReply, TransportError> {
        let bound = ms(PUSH_EXCHANGE_TIMEOUT_MS.saturating_add(PUSH_CONNECT_UNIX_TIMEOUT_MS));
        timeout(bound, self.transport.deliver(request))
            .await
            .unwrap_or(Err(TransportError::Timeout))
    }

    async fn remove_gone(&self, endpoint: &PushEndpoint) {
        let (store, random) = (self.store.clone(), Arc::clone(&self.random));
        let endpoint = endpoint.clone();
        if let Ok(true) = with_store_lock(move || store.remove_endpoint(&endpoint, &random)).await {
            audit::push_subscription_removed();
        }
        self.record_gone();
    }

    /// Schedules the next retry of `summary` for `endpoint`, or gives up (carry kept).
    fn retry_or_fail(
        &self,
        state: &mut DispatchState,
        endpoint: &PushEndpoint,
        summary: PushSummary,
        next: usize,
        retry_after_s: Option<u32>,
    ) {
        match PUSH_RETRY_DELAYS_MS.get(next) {
            Some(delay) => {
                let wait = retry_after_s.map_or(*delay, |s| u64::from(s).saturating_mul(1000));
                state.retries.retain(|r| r.endpoint != *endpoint);
                state.retries.push(Retry {
                    endpoint: endpoint.clone(),
                    summary,
                    next: next.saturating_add(1),
                    due_ms: self.now_ms().saturating_add(wait),
                });
            }
            None => {
                state.fold_carry(endpoint, summary);
                self.record_failure(LastDelivery::Failed);
            }
        }
    }

    /// Sends `effective` to `sub` and applies the outcome table of spec §5.6.
    async fn deliver_alert(
        &self,
        state: &mut DispatchState,
        key: &VapidKey,
        sub: &PushSubscription,
        effective: PushSummary,
        next: usize,
    ) {
        let request = match self.build_request(state, key, sub, Message::Alert(effective)) {
            Ok(request) => request,
            Err(_) => {
                state.fold_carry(&sub.endpoint, effective);
                self.record_failure(LastDelivery::Failed);
                return;
            }
        };
        match self.exchange(request).await {
            Ok(reply) => {
                self.sender_reachable();
                match reply.outcome {
                    Outcome::Delivered => self.record_delivered(),
                    Outcome::Gone => self.remove_gone(&sub.endpoint).await,
                    Outcome::Rejected | Outcome::Refused => {
                        state.fold_carry(&sub.endpoint, effective);
                        self.record_failure(LastDelivery::Rejected);
                    }
                    Outcome::Retry => self.retry_or_fail(
                        state,
                        &sub.endpoint,
                        effective,
                        next,
                        reply.retry_after_s,
                    ),
                }
            }
            Err(error) => {
                if error == TransportError::Unavailable {
                    self.sender_unavailable();
                }
                self.retry_or_fail(state, &sub.endpoint, effective, next, None);
            }
        }
    }

    /// A due alert summary: cancels pending retries (folded in), merges carries, sends to
    /// every subscription sequentially.
    ///
    /// The dispatcher (`send_summary`, `send_retry`, `send_test`) reads the store inline: the
    /// read is one `O_NOFOLLOW` open of a file capped at `MAX_PUSH_STORE_BYTES` (16 KiB), done
    /// at most once per due delivery, and keeping it on the dispatcher task keeps each
    /// delivery round a single deterministic step (the HTTP routes, which run per request,
    /// read through [`PushRuntime::load_store`] on the blocking pool instead).
    async fn send_summary(&self, state: &mut DispatchState, summary: PushSummary) {
        let file = match self.store.load() {
            Ok(Some(file)) => file,
            Ok(None) | Err(_) => {
                for endpoint in state.known.clone() {
                    state.fold_carry(&endpoint, summary);
                }
                self.record_failure(LastDelivery::Failed);
                return;
            }
        };
        state.after_load(&file);
        for sub in &file.subscriptions {
            if let Some(position) = state
                .retries
                .iter()
                .position(|r| r.endpoint == sub.endpoint)
            {
                let cancelled = state.retries.remove(position);
                state.fold_carry(&sub.endpoint, cancelled.summary);
            }
            let effective = match state.take_carry(&sub.endpoint) {
                Some(carried) => PushSummary::merge(&carried, &summary),
                None => summary,
            };
            self.deliver_alert(state, &file.key, sub, effective, 0)
                .await;
        }
    }

    /// A due retry.
    async fn send_retry(&self, state: &mut DispatchState, retry: Retry) {
        let file = match self.store.load() {
            Ok(Some(file)) => file,
            Ok(None) | Err(_) => {
                state.fold_carry(&retry.endpoint, retry.summary);
                self.record_failure(LastDelivery::Failed);
                return;
            }
        };
        state.after_load(&file);
        let Some(sub) = file
            .subscriptions
            .iter()
            .find(|s| s.endpoint == retry.endpoint)
        else {
            return;
        };
        let effective = match state.take_carry(&sub.endpoint) {
            Some(carried) => PushSummary::merge(&carried, &retry.summary),
            None => retry.summary,
        };
        self.deliver_alert(state, &file.key, sub, effective, retry.next)
            .await;
    }

    /// The queued test notification: one attempt per subscription, never touching alert
    /// retries or carries.
    async fn send_test(&self, state: &mut DispatchState) {
        let file = match self.store.load() {
            Ok(Some(file)) => file,
            Ok(None) | Err(_) => {
                self.record_failure(LastDelivery::Failed);
                return;
            }
        };
        state.after_load(&file);
        for sub in &file.subscriptions {
            let Ok(request) = self.build_request(state, &file.key, sub, Message::Test) else {
                self.record_failure(LastDelivery::Failed);
                continue;
            };
            match self.exchange(request).await {
                Ok(reply) => {
                    self.sender_reachable();
                    match reply.outcome {
                        Outcome::Delivered => self.record_delivered(),
                        Outcome::Gone => self.remove_gone(&sub.endpoint).await,
                        Outcome::Rejected | Outcome::Refused => {
                            self.record_failure(LastDelivery::Rejected);
                        }
                        Outcome::Retry => self.record_failure(LastDelivery::Failed),
                    }
                }
                Err(error) => {
                    if error == TransportError::Unavailable {
                        self.sender_unavailable();
                    }
                    self.record_failure(LastDelivery::Failed);
                }
            }
        }
    }

    fn next_wake(&self, state: &DispatchState) -> Option<u64> {
        let summary = self.scheduler.lock().ok().and_then(|s| s.due_at());
        let retry = state.retries.iter().map(|r| r.due_ms).min();
        match (summary, retry) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// One gate: refused when the last pass is younger than `interval_ms`.
async fn pass_gate(gate: &tokio::sync::Mutex<Option<Instant>>, interval_ms: u64) -> bool {
    let mut last = gate.lock().await;
    let now = Instant::now();
    if last.is_some_and(|t| now.saturating_duration_since(t) < ms(interval_ms)) {
        return false;
    }
    *last = Some(now);
    true
}

/// The dispatcher task (spawned by `serve` when push is active): a queued test first, then
/// due retries, then a due summary; deliveries are sequential. No push error ends it.
pub(crate) async fn run_dispatcher(push: Arc<PushRuntime>) {
    let mut state = DispatchState::default();
    loop {
        let now = push.now_ms();
        if push.test_queued.swap(false, Ordering::SeqCst) {
            push.send_test(&mut state).await;
            continue;
        }
        if let Some(position) = state.retries.iter().position(|r| r.due_ms <= now) {
            let retry = state.retries.remove(position);
            push.send_retry(&mut state, retry).await;
            continue;
        }
        let taken = push.scheduler.lock().ok().and_then(|mut s| s.take(now));
        if let Some(summary) = taken {
            push.send_summary(&mut state, summary).await;
            continue;
        }
        match push.next_wake(&state) {
            Some(at) => {
                let deadline = push.base.checked_add(ms(at)).unwrap_or(push.base);
                tokio::select! {
                    () = sleep_until(deadline) => {}
                    () = push.wake.notified() => {}
                }
            }
            None => push.wake.notified().await,
        }
    }
}
