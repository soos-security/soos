//! Passkey credential store file and its in-memory cache (architect spec §4.4).
//!
//! On-disk format (JSON, `deny_unknown_fields`, at most `MAX_CREDENTIAL_STORE_BYTES`):
//! `{"version":1,"user_handle":"<b64url>","passkeys":[{"credential_id":…,"public_key":…,
//! "sign_count":…,"backup_eligible":…,"backup_state":…,"created_unix_s":…}]}`.
//!
//! Reads open the file `O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC` and check type,
//! owner, mode and size on the same descriptor before a bounded read. Writes are
//! read-modify-write under an exclusive `flock` on `<path>.lock`, through a `0600` temp file
//! created exclusively in the same directory, `fsync`, `rename` and a directory `fsync`; the
//! parent directory is never created. Nothing here logs.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg, OFlag};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::auth::RandomSource;
use crate::webauthn::{b64url_decode, b64url_encode};
use crate::{
    MAX_CREDENTIAL_ID_BYTES, MAX_CREDENTIAL_STORE_BYTES, MAX_PASSKEYS, STORE_LOCK_TIMEOUT_MS,
    USER_HANDLE_BYTES,
};

/// Interval between two non-blocking lock attempts.
pub const STORE_LOCK_RETRY_MS: u64 = 25;
/// The only supported store format version.
const STORE_VERSION: u32 = 1;

/// One stored passkey; `Debug` redacted (credential id and key are identifiers, D-H).
#[derive(Clone, PartialEq, Eq)]
pub struct PasskeyRecord {
    /// Credential id.
    pub credential_id: Vec<u8>,
    /// SEC1 uncompressed point.
    pub public_key: [u8; 65],
    /// Signature counter.
    pub sign_count: u32,
    /// BE flag.
    pub backup_eligible: bool,
    /// BS flag.
    pub backup_state: bool,
    /// Registration time (Unix s).
    pub created_unix_s: u64,
}

impl fmt::Debug for PasskeyRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PasskeyRecord(<redacted>)")
    }
}

/// The whole store file; `Debug` redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct PasskeyFile {
    /// The single owner's user handle.
    pub user_handle: [u8; USER_HANDLE_BYTES],
    /// 0..=`MAX_PASSKEYS` records.
    pub passkeys: Vec<PasskeyRecord>,
}

impl fmt::Debug for PasskeyFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PasskeyFile(<redacted>)")
    }
}

/// SHA-256 of a credential id (session revocation, CLI fingerprints).
#[must_use]
pub fn credential_hash(credential_id: &[u8]) -> [u8; 32] {
    Sha256::digest(credential_id).into()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreJson {
    version: u32,
    user_handle: String,
    passkeys: Vec<RecordJson>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordJson {
    credential_id: String,
    public_key: String,
    sign_count: u32,
    backup_eligible: bool,
    backup_state: bool,
    created_unix_s: u64,
}

/// Validates one record.
fn record_from(json: &RecordJson) -> Result<PasskeyRecord, StoreError> {
    let credential_id = b64url_decode(&json.credential_id, MAX_CREDENTIAL_ID_BYTES)
        .map_err(|()| StoreError::Malformed)?;
    if credential_id.is_empty() {
        return Err(StoreError::Malformed);
    }
    let key = b64url_decode(&json.public_key, 65).map_err(|()| StoreError::Malformed)?;
    let public_key: [u8; 65] = key
        .as_slice()
        .try_into()
        .map_err(|_| StoreError::Malformed)?;
    if public_key.first() != Some(&0x04)
        || p256::ecdsa::VerifyingKey::from_sec1_bytes(&public_key).is_err()
    {
        return Err(StoreError::Malformed);
    }
    if json.backup_state && !json.backup_eligible {
        return Err(StoreError::Malformed);
    }
    Ok(PasskeyRecord {
        credential_id,
        public_key,
        sign_count: json.sign_count,
        backup_eligible: json.backup_eligible,
        backup_state: json.backup_state,
        created_unix_s: json.created_unix_s,
    })
}

/// Pure deserialisation; every record re-validated (key on the curve, id length, unique
/// ids, at most `MAX_PASSKEYS`).
///
/// # Errors
///
/// [`StoreError::TooLarge`], [`StoreError::Malformed`].
pub fn parse_store(bytes: &[u8]) -> Result<PasskeyFile, StoreError> {
    if bytes.len() > MAX_CREDENTIAL_STORE_BYTES {
        return Err(StoreError::TooLarge);
    }
    let json: StoreJson = serde_json::from_slice(bytes).map_err(|_| StoreError::Malformed)?;
    if json.version != STORE_VERSION || json.passkeys.len() > MAX_PASSKEYS {
        return Err(StoreError::Malformed);
    }
    let handle =
        b64url_decode(&json.user_handle, USER_HANDLE_BYTES).map_err(|()| StoreError::Malformed)?;
    let user_handle: [u8; USER_HANDLE_BYTES] = handle
        .as_slice()
        .try_into()
        .map_err(|_| StoreError::Malformed)?;
    let mut passkeys: Vec<PasskeyRecord> = Vec::with_capacity(json.passkeys.len());
    for raw in &json.passkeys {
        let record = record_from(raw)?;
        if passkeys
            .iter()
            .any(|existing| existing.credential_id == record.credential_id)
        {
            return Err(StoreError::Malformed);
        }
        passkeys.push(record);
    }
    Ok(PasskeyFile {
        user_handle,
        passkeys,
    })
}

/// Pure serialisation in the spec §4.4 format.
#[must_use]
pub fn encode_store(file: &PasskeyFile) -> Vec<u8> {
    let json = StoreJson {
        version: STORE_VERSION,
        user_handle: b64url_encode(&file.user_handle),
        passkeys: file
            .passkeys
            .iter()
            .map(|r| RecordJson {
                credential_id: b64url_encode(&r.credential_id),
                public_key: b64url_encode(&r.public_key),
                sign_count: r.sign_count,
                backup_eligible: r.backup_eligible,
                backup_state: r.backup_state,
                created_unix_s: r.created_unix_s,
            })
            .collect(),
    };
    serde_json::to_vec(&json).unwrap_or_default()
}

/// `(dev, ino, len, mtime_sec, mtime_nsec)` of the last read; a different stamp ⇒ reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
}

impl FileStamp {
    fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            len: metadata.len(),
            mtime: metadata.mtime(),
            mtime_nsec: metadata.mtime_nsec(),
        }
    }
}

/// Opens `path` for a bounded read under the store's file rules and returns the bytes and
/// the stamp; `Ok(None)` when the file does not exist. Shared with the enrollment code file.
pub(crate) fn read_owned_file(
    path: &Path,
    uid: u32,
    bound: usize,
) -> Result<Option<(Vec<u8>, FileStamp)>, StoreError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC).bits())
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(match e.raw_os_error().map(Errno::from_raw) {
                Some(Errno::ELOOP) => StoreError::Insecure,
                _ => StoreError::Io,
            })
        }
    };
    let metadata = file.metadata().map_err(|_| StoreError::Io)?;
    if !metadata.is_file() {
        return Err(StoreError::Insecure);
    }
    if metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(StoreError::Insecure);
    }
    let limit = u64::try_from(bound).unwrap_or(u64::MAX);
    if metadata.len() > limit {
        return Err(StoreError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(bound.min(4096));
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| StoreError::Io)?;
    if bytes.len() > bound {
        return Err(StoreError::TooLarge);
    }
    Ok(Some((bytes, FileStamp::of(&metadata))))
}

/// The parent directory must exist, be owned by `uid` and not be group- or world-writable.
pub(crate) fn check_parent_dir(path: &Path, uid: u32) -> Result<PathBuf, StoreError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or(StoreError::Io)?;
    let metadata = std::fs::metadata(parent).map_err(|_| StoreError::Io)?;
    if !metadata.is_dir() {
        return Err(StoreError::Io);
    }
    if metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
        return Err(StoreError::Insecure);
    }
    Ok(parent.to_path_buf())
}

/// Writes `bytes` to `path` atomically: a temp file `<name>.tmp-<8 hex>` created exclusively
/// with mode `0600` (never `chmod` afterwards) and `O_NOFOLLOW` in the same directory,
/// `write_all`, `sync_all`, `rename` over `path`, then a best-effort directory `fsync`. On
/// any failure the temp file is removed and `path` is untouched.
pub(crate) fn write_atomic(
    path: &Path,
    dir: &Path,
    bytes: &[u8],
    random: &RandomSource,
) -> Result<(), StoreError> {
    let mut suffix = [0u8; 4];
    random(&mut suffix).map_err(|_| StoreError::Io)?;
    let hex: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(StoreError::Io)?;
    let temp = dir.join(format!("{name}.tmp-{hex}"));
    let written = (|| -> Result<(), StoreError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC).bits())
            .open(&temp)
            .map_err(|_| StoreError::Io)?;
        file.write_all(bytes).map_err(|_| StoreError::Io)?;
        file.sync_all().map_err(|_| StoreError::Io)?;
        std::fs::rename(&temp, path).map_err(|_| StoreError::Io)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
        return written;
    }
    // The new bytes are in place; a failing directory fsync cannot undo the rename.
    if let Ok(directory) = OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_DIRECTORY | OFlag::O_CLOEXEC).bits())
        .open(dir)
    {
        let _ = directory.sync_all();
    }
    Ok(())
}

/// An exclusive lock on `<path>.lock`, released on drop.
pub struct StoreLock(#[allow(dead_code, reason = "Held only for its Drop (unlock)")] Flock<File>);

impl fmt::Debug for StoreLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StoreLock")
    }
}

/// File-backed store with a stamp-checked cache.
pub struct CredentialStore {
    path: PathBuf,
    uid: u32,
    cache: Option<(FileStamp, PasskeyFile)>,
}

impl fmt::Debug for CredentialStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CredentialStore(<redacted>)")
    }
}

impl CredentialStore {
    /// Store at `path`, owned by `uid`.
    #[must_use]
    pub fn new(path: PathBuf, uid: u32) -> Self {
        Self {
            path,
            uid,
            cache: None,
        }
    }

    /// Reads the file (or returns the cached copy when the stamp is unchanged). A missing
    /// file is `Ok(None)` (zero passkeys); an empty file is malformed.
    ///
    /// # Errors
    ///
    /// [`StoreError::Io`], [`StoreError::Insecure`], [`StoreError::TooLarge`],
    /// [`StoreError::Malformed`].
    pub fn load(&mut self) -> Result<Option<&PasskeyFile>, StoreError> {
        let read = match read_owned_file(&self.path, self.uid, MAX_CREDENTIAL_STORE_BYTES) {
            Ok(read) => read,
            Err(err) => {
                self.cache = None;
                return Err(err);
            }
        };
        let Some((bytes, stamp)) = read else {
            self.cache = None;
            return Ok(None);
        };
        let fresh = !matches!(&self.cache, Some((cached, _)) if *cached == stamp);
        if fresh {
            match parse_store(&bytes) {
                Ok(file) => self.cache = Some((stamp, file)),
                Err(err) => {
                    self.cache = None;
                    return Err(err);
                }
            }
        }
        Ok(self.cache.as_ref().map(|(_, file)| file))
    }

    /// The lock file path `<path>.lock`.
    fn lock_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_os_string();
        name.push(".lock");
        PathBuf::from(name)
    }

    /// Checks the parent directory, then one non-blocking attempt at the exclusive lock:
    /// `Ok(None)` when another holder has it. The lock file is opened
    /// `O_CREAT | O_NOFOLLOW | O_CLOEXEC` with mode `0600`; a symlink is refused.
    ///
    /// # Errors
    ///
    /// [`StoreError::Io`], [`StoreError::Insecure`].
    pub fn try_lock(&self) -> Result<Option<StoreLock>, StoreError> {
        check_parent_dir(&self.path, self.uid)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK).bits())
            .open(self.lock_path())
            .map_err(|e| match e.raw_os_error().map(Errno::from_raw) {
                Some(Errno::ELOOP) => StoreError::Insecure,
                _ => StoreError::Io,
            })?;
        let metadata = file.metadata().map_err(|_| StoreError::Io)?;
        if !metadata.is_file() || metadata.uid() != self.uid {
            return Err(StoreError::Insecure);
        }
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(lock) => Ok(Some(StoreLock(lock))),
            Err((_, Errno::EWOULDBLOCK)) => Ok(None),
            Err(_) => Err(StoreError::Io),
        }
    }

    /// Read-modify-write while `lock` is held: loads fresh from disk (the cache is
    /// ignored), applies `f`, then writes atomically. The cache is dropped either way.
    ///
    /// # Errors
    ///
    /// Every [`StoreError`] (including those returned by `f`).
    pub fn update_locked<T>(
        &mut self,
        _lock: &StoreLock,
        random: &RandomSource,
        f: impl FnOnce(Option<PasskeyFile>) -> Result<(PasskeyFile, T), StoreError>,
    ) -> Result<T, StoreError> {
        self.cache = None;
        let dir = check_parent_dir(&self.path, self.uid)?;
        let current = match read_owned_file(&self.path, self.uid, MAX_CREDENTIAL_STORE_BYTES)? {
            Some((bytes, _)) => Some(parse_store(&bytes)?),
            None => None,
        };
        let (next, out) = f(current)?;
        if next.passkeys.len() > MAX_PASSKEYS {
            return Err(StoreError::Full);
        }
        let bytes = encode_store(&next);
        if bytes.is_empty() {
            return Err(StoreError::Malformed);
        }
        if bytes.len() > MAX_CREDENTIAL_STORE_BYTES {
            return Err(StoreError::TooLarge);
        }
        write_atomic(&self.path, &dir, &bytes, random)?;
        Ok(out)
    }

    /// Blocking read-modify-write for the CLI: non-blocking lock attempts every
    /// `STORE_LOCK_RETRY_MS` for at most `STORE_LOCK_TIMEOUT_MS` (then
    /// [`StoreError::Busy`]), then [`CredentialStore::update_locked`]. The server never calls
    /// this (it waits with `tokio::time::sleep`).
    ///
    /// # Errors
    ///
    /// Every [`StoreError`].
    pub fn update<T>(
        &mut self,
        random: &RandomSource,
        f: impl FnOnce(Option<PasskeyFile>) -> Result<(PasskeyFile, T), StoreError>,
    ) -> Result<T, StoreError> {
        let start = std::time::Instant::now();
        let budget = Duration::from_millis(STORE_LOCK_TIMEOUT_MS);
        let lock = loop {
            if let Some(lock) = self.try_lock()? {
                break lock;
            }
            if start.elapsed() >= budget {
                return Err(StoreError::Busy);
            }
            std::thread::sleep(Duration::from_millis(STORE_LOCK_RETRY_MS));
        };
        self.update_locked(&lock, random, f)
    }

    /// SHA-256 of every stored credential id (session revocation); `None` on any error.
    #[must_use]
    pub fn live_credential_hashes(&mut self) -> Option<Vec<[u8; 32]>> {
        match self.load() {
            Ok(None) => Some(Vec::new()),
            Ok(Some(file)) => Some(
                file.passkeys
                    .iter()
                    .map(|r| credential_hash(&r.credential_id))
                    .collect(),
            ),
            Err(_) => None,
        }
    }
}

/// Store failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// The file or its directory cannot be read or written.
    #[error("credential store unreadable")]
    Io,
    /// Owner, mode, file type or a symlink.
    #[error("credential store permissions are insecure")]
    Insecure,
    /// Larger than `MAX_CREDENTIAL_STORE_BYTES`.
    #[error("credential store too large")]
    TooLarge,
    /// Not the spec §4.4 format.
    #[error("credential store malformed")]
    Malformed,
    /// The lock could not be taken within `STORE_LOCK_TIMEOUT_MS`.
    #[error("credential store busy")]
    Busy,
    /// `MAX_PASSKEYS` reached.
    #[error("credential store full")]
    Full,
    /// The credential id is already stored.
    #[error("passkey already registered")]
    Duplicate,
    /// No record at that index (CLI).
    #[error("no such passkey")]
    NoSuchPasskey,
    /// The stored user handle differs from the registration's.
    #[error("registration conflicts with the stored user")]
    UserHandleConflict,
}
