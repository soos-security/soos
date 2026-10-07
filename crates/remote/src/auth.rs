//! Passkey ceremony state, limiters, class capacity and the CSPRNG seam (architect spec §4.6,
//! §5.2, §6; ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey Authentication for
//! `soos-remote`").
//!
//! [`AuthState`] holds every mutable authentication structure (challenges, web sessions, the
//! credential store, the fixed-window limiters keyed by [`LimitKey`], the enrollment-code
//! attempt counter) behind one async mutex in the server; it is never held across a logind
//! call nor a body read. [`Capacity`] holds the per-class connection permits outside that
//! mutex so that releasing a slot never awaits. Every collection is bounded.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use zeroize::Zeroizing;

use subtle::ConstantTimeEq;

use crate::challenge::{ChallengeBinding, ChallengePurpose, ChallengeStore, Taken};
use crate::credentials::{credential_hash, CredentialStore, PasskeyFile, StoreError};
use crate::identity::{ClientHint, PathClass};
use crate::webauthn::{
    b64url_decode, parse_client_data, verify_assertion, AssertionOutcome, CeremonyType,
    RelyingParty, StoredCredential,
};
use crate::websession::WebSessionStore;
use crate::{
    AUTH_FAILURE_WINDOW_MS, MAX_ANONYMOUS_BODY_READS, MAX_ANONYMOUS_BODY_READS_PER_HINT,
    MAX_ANONYMOUS_FUNNEL_CONNECTIONS, MAX_AUTH_FAILURES, MAX_CLIENT_DATA_JSON_BYTES,
    MAX_CLIENT_HINTS, MAX_CREDENTIAL_ID_BYTES, MAX_FUNNEL_CONNECTIONS, MAX_FUNNEL_SSE_STREAMS,
    MAX_OPTIONS_PER_WINDOW, MAX_SIGNATURE_BYTES, OPTIONS_WINDOW_MS,
};

/// CSPRNG seam. Production: `getrandom::fill`; tests inject a failing or scripted source.
pub type RandomSource = Arc<dyn Fn(&mut [u8]) -> Result<(), RandomError> + Send + Sync>;

/// The production CSPRNG: `getrandom::fill` (the only random source of the crate).
#[must_use]
pub fn system_random() -> RandomSource {
    Arc::new(|buf: &mut [u8]| getrandom::fill(buf).map_err(|_| RandomError::Failed))
}

/// RNG failure (`503 unavailable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RandomError {
    /// The random source failed.
    #[error("random source failed")]
    Failed,
}

// ---------------------------------------------------------------------------------------
// Limiters
// ---------------------------------------------------------------------------------------

/// Fixed-window counter used by both limiters: the window starts at the first recorded
/// event and both the start and the count reset once it has lasted its length.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Window {
    started: Option<Instant>,
    count: u32,
}

impl Window {
    /// Resets the window when it has ended.
    fn roll(&mut self, now: Instant, length: Duration) {
        if let Some(started) = self.started {
            if now.saturating_duration_since(started) >= length {
                *self = Self::default();
            }
        }
    }

    /// Whether `max` events were already recorded in the current window.
    fn reached(&mut self, now: Instant, length: Duration, max: u32) -> bool {
        self.roll(now, length);
        self.count >= max
    }

    /// Records one event.
    fn record(&mut self, now: Instant, length: Duration) {
        self.roll(now, length);
        if self.started.is_none() {
            self.started = Some(now);
        }
        self.count = self.count.saturating_add(1);
    }
}

/// The options window length.
fn options_window() -> Duration {
    Duration::from_millis(OPTIONS_WINDOW_MS)
}

/// The failure window length.
fn failure_window() -> Duration {
    Duration::from_millis(AUTH_FAILURE_WINDOW_MS)
}

/// Limiter key (spec §4.6, F-2). `Debug` redacted (the hint is an address prefix).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitKey {
    /// Tailnet caller (register, unlock options, unlock).
    Tailnet,
    /// Funnel caller holding a valid web session (unlock options, unlock).
    FunnelSession,
    /// Anonymous Funnel caller (login options, login verify), one bucket per client hint.
    FunnelAnonymous(ClientHint),
}

impl fmt::Debug for LimitKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tailnet => f.write_str("LimitKey::Tailnet"),
            Self::FunnelSession => f.write_str("LimitKey::FunnelSession"),
            Self::FunnelAnonymous(_) => f.write_str("LimitKey::FunnelAnonymous(<redacted>)"),
        }
    }
}

/// Per-hint anonymous limiter windows.
#[derive(Default)]
struct HintBucket {
    options: Window,
    failures: Window,
}

impl HintBucket {
    /// Start of the oldest running window (`None`: no running window).
    fn oldest(&mut self, now: Instant) -> Option<Instant> {
        self.options.roll(now, options_window());
        self.failures.roll(now, failure_window());
        match (self.options.started, self.failures.started) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// Which window of a key.
#[derive(Clone, Copy)]
enum Limiter {
    Options,
    Failures,
}

/// Every mutable authentication structure (one async mutex in the server).
pub struct AuthState {
    /// Pending WebAuthn challenges.
    pub challenges: ChallengeStore,
    /// Funnel web sessions.
    pub sessions: WebSessionStore,
    /// The credential store (`None` when no path was resolved: every passkey route then
    /// answers `503 store_unavailable`).
    pub store: Option<CredentialStore>,
    /// `Tailnet` and `FunnelSession` windows (exactly 2 keys).
    options_windows: HashMap<LimitKey, Window>,
    failure_windows: HashMap<LimitKey, Window>,
    /// Anonymous buckets, at most `MAX_CLIENT_HINTS`.
    hints: HashMap<ClientHint, HintBucket>,
    /// Wrong-code count against the current code file, keyed by its code hash.
    code_attempts: Option<([u8; 32], u32)>,
}

impl fmt::Debug for AuthState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AuthState(<redacted>)")
    }
}

impl AuthState {
    /// Fresh state over `store`.
    #[must_use]
    pub fn new(store: Option<CredentialStore>) -> Self {
        Self {
            challenges: ChallengeStore::new(),
            sessions: WebSessionStore::new(),
            store,
            options_windows: HashMap::new(),
            failure_windows: HashMap::new(),
            hints: HashMap::new(),
            code_attempts: None,
        }
    }

    /// Number of anonymous limiter buckets (test hook).
    #[must_use]
    pub fn hint_buckets(&self) -> usize {
        self.hints.len()
    }

    /// The window of `key` for `limiter`, creating it when `create` (an anonymous bucket is
    /// created under the `MAX_CLIENT_HINTS` bound, evicting the bucket with the oldest
    /// window that has no body read in flight; `None` when nothing can be evicted).
    fn window(
        &mut self,
        key: LimitKey,
        limiter: Limiter,
        now: Instant,
        create: bool,
        in_flight: &[ClientHint],
    ) -> Option<&mut Window> {
        let hint = match key {
            LimitKey::FunnelAnonymous(hint) => hint,
            LimitKey::Tailnet | LimitKey::FunnelSession => {
                let map = match limiter {
                    Limiter::Options => &mut self.options_windows,
                    Limiter::Failures => &mut self.failure_windows,
                };
                return Some(map.entry(key).or_default());
            }
        };
        if !self.hints.contains_key(&hint) {
            if !create {
                return None;
            }
            if self.hints.len() >= MAX_CLIENT_HINTS {
                self.evict_hint(now, in_flight);
            }
            if self.hints.len() >= MAX_CLIENT_HINTS {
                return None;
            }
            self.hints.insert(hint, HintBucket::default());
        }
        let bucket = self.hints.get_mut(&hint)?;
        Some(match limiter {
            Limiter::Options => &mut bucket.options,
            Limiter::Failures => &mut bucket.failures,
        })
    }

    /// Purges buckets whose windows all ended; if the table is still full, evicts the
    /// bucket with the oldest window start that has no body read in flight.
    fn evict_hint(&mut self, now: Instant, in_flight: &[ClientHint]) {
        self.hints
            .retain(|hint, bucket| in_flight.contains(hint) || bucket.oldest(now).is_some());
        if self.hints.len() < MAX_CLIENT_HINTS {
            return;
        }
        let victim = self
            .hints
            .iter_mut()
            .filter(|(hint, _)| !in_flight.contains(hint))
            .map(|(hint, bucket)| (*hint, bucket.oldest(now)))
            .min_by_key(|(_, oldest)| *oldest)
            .map(|(hint, _)| hint);
        if let Some(hint) = victim {
            self.hints.remove(&hint);
        }
    }

    /// Whether `key` is locked out by `MAX_AUTH_FAILURES` failures in its window.
    pub fn locked_out(&mut self, key: LimitKey, now: Instant) -> bool {
        self.window(key, Limiter::Failures, now, false, &[])
            .is_some_and(|w| w.reached(now, failure_window(), MAX_AUTH_FAILURES))
    }

    /// Whether `key` already issued `MAX_OPTIONS_PER_WINDOW` challenges in its window.
    pub fn options_exhausted(&mut self, key: LimitKey, now: Instant) -> bool {
        self.window(key, Limiter::Options, now, false, &[])
            .is_some_and(|w| w.reached(now, options_window(), MAX_OPTIONS_PER_WINDOW))
    }

    /// Records one counted failure of `key`.
    pub fn record_failure(&mut self, key: LimitKey, now: Instant, in_flight: &[ClientHint]) {
        if let Some(window) = self.window(key, Limiter::Failures, now, true, in_flight) {
            window.record(now, failure_window());
        }
    }

    /// Records one challenge issuance of `key`.
    pub fn record_issuance(&mut self, key: LimitKey, now: Instant, in_flight: &[ClientHint]) {
        if let Some(window) = self.window(key, Limiter::Options, now, true, in_flight) {
            window.record(now, options_window());
        }
    }

    /// Counts one wrong enrollment code against the code file whose hash is `code_hash`;
    /// returns true when `MAX_ENROLL_CODE_ATTEMPTS` is reached (the caller deletes the file
    /// and the counter resets).
    pub fn wrong_code(&mut self, code_hash: &[u8; 32]) -> bool {
        let count = match self.code_attempts {
            Some((hash, count)) if bool::from(hash.ct_eq(code_hash)) => count.saturating_add(1),
            _ => 1,
        };
        if count >= crate::MAX_ENROLL_CODE_ATTEMPTS {
            self.code_attempts = None;
            true
        } else {
            self.code_attempts = Some((*code_hash, count));
            false
        }
    }

    /// Forgets the wrong-code counter (code file removed or consumed).
    pub fn reset_code_attempts(&mut self) {
        self.code_attempts = None;
    }

    /// Session revocation (spec §4.3, F-6): drops every session whose passkey is no longer
    /// stored, or every session when the store cannot be read.
    ///
    /// Accepted bound (candid review 2026-10-06): this runs synchronously on the
    /// current-thread runtime for every Funnel request. It is one `open`, `fstat` and read of
    /// at most `MAX_CREDENTIAL_STORE_BYTES` (16 KiB) of a local `0600` file, parsed again only
    /// when the file stamp changed (the parsed copy is cached by stamp otherwise).
    pub fn revoke(&mut self) {
        let live = self
            .store
            .as_mut()
            .and_then(CredentialStore::live_credential_hashes);
        self.sessions.retain_credentials(live.as_deref());
    }

    /// The fail-closed reaction to a store error: every error except `Busy` and the logical
    /// refusals (`Full`, `Duplicate`, `NoSuchPasskey`, `UserHandleConflict`) drops every web
    /// session.
    pub fn store_failed(&mut self, err: StoreError) {
        if matches!(
            err,
            StoreError::Io | StoreError::Insecure | StoreError::TooLarge | StoreError::Malformed
        ) {
            self.sessions.retain_credentials(None);
        }
    }

    /// A copy of the current store file (`None`: no file, zero passkeys).
    ///
    /// # Errors
    ///
    /// [`StoreError`] (sessions dropped by [`AuthState::store_failed`]).
    pub fn store_snapshot(&mut self) -> Result<Option<PasskeyFile>, StoreError> {
        let result = match self.store.as_mut() {
            Some(store) => store.load().map(|file| file.cloned()),
            None => Err(StoreError::Io),
        };
        if let Err(err) = result {
            self.store_failed(err);
        }
        result
    }

    /// The assertion check of login and unlock (spec §5.1 step 9a, constraint C9), in this
    /// order: `parse_client_data(Get)` → `take` (the challenge is removed whatever the
    /// outcome) → credential lookup → `verify_assertion` (UV unconditional). The store is
    /// only read for a challenge that `take` accepted.
    ///
    /// # Errors
    ///
    /// [`AssertionFailure::Rejected`] (a counted failure) or [`AssertionFailure::Store`].
    pub fn check_assertion(
        &mut self,
        rp: &RelyingParty,
        class: PathClass,
        purpose: ChallengePurpose,
        binding: &ChallengeBinding,
        assertion: &DecodedAssertion,
        now: Instant,
    ) -> Result<Verified, AssertionFailure> {
        let client = parse_client_data(rp, &assertion.client_data_json, CeremonyType::Get)
            .map_err(|_| AssertionFailure::Rejected)?;
        match self
            .challenges
            .take(now, class, purpose, binding, &client.challenge)
        {
            Ok(Taken::Plain) => {}
            Ok(Taken::Register(_)) | Err(_) => return Err(AssertionFailure::Rejected),
        }
        let file = match self.store_snapshot() {
            Ok(Some(file)) => file,
            Ok(None) => return Err(AssertionFailure::Rejected),
            Err(err) => return Err(AssertionFailure::Store(err)),
        };
        let record = file
            .passkeys
            .iter()
            .find(|r| {
                r.credential_id.len() == assertion.id.len()
                    && bool::from(r.credential_id.as_slice().ct_eq(assertion.id.as_slice()))
            })
            .ok_or(AssertionFailure::Rejected)?;
        let stored = StoredCredential {
            credential_id: &record.credential_id,
            public_key: &record.public_key,
            sign_count: record.sign_count,
            backup_eligible: record.backup_eligible,
        };
        let outcome = verify_assertion(
            rp,
            &stored,
            &file.user_handle,
            &assertion.client_data_json,
            &assertion.authenticator_data,
            &assertion.signature,
            assertion.user_handle.as_ref().map(|h| h.as_slice()),
        )
        .map_err(|_| AssertionFailure::Rejected)?;
        Ok(Verified {
            credential_id: record.credential_id.clone(),
            credential_hash: credential_hash(&record.credential_id),
            changed: outcome.sign_count != record.sign_count
                || outcome.backup_state != record.backup_state,
            outcome,
        })
    }
}

/// A verified assertion (no `Debug`: it carries the credential id).
pub struct Verified {
    /// The credential that signed.
    pub credential_id: Vec<u8>,
    /// SHA-256 of the credential id (session revocation).
    pub credential_hash: [u8; 32],
    /// Counter and backup state to persist.
    pub outcome: AssertionOutcome,
    /// Whether `outcome` differs from the stored record.
    pub changed: bool,
}

/// Why an assertion was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertionFailure {
    /// Any verification failure (`403 passkey_rejected`, counted).
    Rejected,
    /// The store could not be read (`503 store_unavailable`, not counted).
    Store(StoreError),
}

// ---------------------------------------------------------------------------------------
// Class capacity
// ---------------------------------------------------------------------------------------

/// Anonymous body reads in flight (never held across an await).
#[derive(Default)]
struct BodyReads {
    total: usize,
    per_hint: HashMap<ClientHint, usize>,
}

/// Per-class connection capacity (spec S-11, §6), outside the auth mutex.
pub struct Capacity {
    funnel: Arc<Semaphore>,
    anonymous: Arc<Semaphore>,
    body_reads: Arc<Mutex<BodyReads>>,
    funnel_streams: Arc<AtomicUsize>,
}

impl fmt::Debug for Capacity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Capacity")
    }
}

impl Default for Capacity {
    fn default() -> Self {
        Self::new()
    }
}

/// Locks the body-read table, recovering a poisoned mutex (never a panic).
fn lock_reads(reads: &Mutex<BodyReads>) -> MutexGuard<'_, BodyReads> {
    reads
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Holds one anonymous body-read reservation until dropped.
pub struct BodyReadGuard {
    reads: Arc<Mutex<BodyReads>>,
    hint: ClientHint,
}

impl fmt::Debug for BodyReadGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BodyReadGuard")
    }
}

impl Drop for BodyReadGuard {
    fn drop(&mut self) {
        let mut reads = lock_reads(&self.reads);
        reads.total = reads.total.saturating_sub(1);
        if let Some(count) = reads.per_hint.get_mut(&self.hint) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                reads.per_hint.remove(&self.hint);
            }
        }
    }
}

/// Holds one Funnel SSE stream slot until dropped.
pub struct FunnelStreamGuard(Arc<AtomicUsize>);

impl Drop for FunnelStreamGuard {
    fn drop(&mut self) {
        let _ = self
            .0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                Some(n.saturating_sub(1))
            });
    }
}

impl Capacity {
    /// Fresh capacity with every permit free.
    #[must_use]
    pub fn new() -> Self {
        Self {
            funnel: Arc::new(Semaphore::new(MAX_FUNNEL_CONNECTIONS)),
            anonymous: Arc::new(Semaphore::new(MAX_ANONYMOUS_FUNNEL_CONNECTIONS)),
            body_reads: Arc::new(Mutex::new(BodyReads::default())),
            funnel_streams: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// One of the `MAX_FUNNEL_CONNECTIONS` Funnel permits.
    ///
    /// # Errors
    ///
    /// [`CapacityError::Busy`].
    pub fn enter_funnel(&self) -> Result<OwnedSemaphorePermit, CapacityError> {
        Arc::clone(&self.funnel)
            .try_acquire_owned()
            .map_err(|_| CapacityError::Busy)
    }

    /// One of the `MAX_ANONYMOUS_FUNNEL_CONNECTIONS` anonymous permits.
    ///
    /// # Errors
    ///
    /// [`CapacityError::Busy`].
    pub fn enter_anonymous(&self) -> Result<OwnedSemaphorePermit, CapacityError> {
        Arc::clone(&self.anonymous)
            .try_acquire_owned()
            .map_err(|_| CapacityError::Busy)
    }

    /// One anonymous body-read reservation under the global (`MAX_ANONYMOUS_BODY_READS`) and
    /// per-hint (`MAX_ANONYMOUS_BODY_READS_PER_HINT`) caps; the guard releases both.
    ///
    /// # Errors
    ///
    /// [`CapacityError::Busy`].
    pub fn reserve_anonymous_body_read(
        &self,
        hint: ClientHint,
    ) -> Result<BodyReadGuard, CapacityError> {
        let mut reads = lock_reads(&self.body_reads);
        let of_hint = reads.per_hint.get(&hint).copied().unwrap_or(0);
        if reads.total >= MAX_ANONYMOUS_BODY_READS || of_hint >= MAX_ANONYMOUS_BODY_READS_PER_HINT {
            return Err(CapacityError::Busy);
        }
        reads.total = reads.total.saturating_add(1);
        reads.per_hint.insert(hint, of_hint.saturating_add(1));
        drop(reads);
        Ok(BodyReadGuard {
            reads: Arc::clone(&self.body_reads),
            hint,
        })
    }

    /// Hints with a body read in flight.
    #[must_use]
    pub fn hints_in_flight(&self) -> Vec<ClientHint> {
        lock_reads(&self.body_reads)
            .per_hint
            .keys()
            .copied()
            .collect()
    }

    /// One of the `MAX_FUNNEL_SSE_STREAMS` Funnel stream slots.
    ///
    /// # Errors
    ///
    /// [`CapacityError::Busy`].
    pub fn enter_funnel_stream(&self) -> Result<FunnelStreamGuard, CapacityError> {
        self.funnel_streams
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < MAX_FUNNEL_SSE_STREAMS).then(|| n.saturating_add(1))
            })
            .map_err(|_| CapacityError::Busy)?;
        Ok(FunnelStreamGuard(Arc::clone(&self.funnel_streams)))
    }
}

/// Capacity refusal (`503 busy`, never counted as an authentication failure).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CapacityError {
    /// A class cap is reached.
    #[error("capacity reached")]
    Busy,
}

// ---------------------------------------------------------------------------------------
// Request bodies
// ---------------------------------------------------------------------------------------

/// Longest accepted enrollment code input.
const MAX_CODE_FIELD_BYTES: usize = 32;
/// Largest decoded assertion `authenticatorData` accepted before the exact-length check.
const MAX_AUTH_DATA_FIELD_BYTES: usize = 512;
/// Largest decoded `userHandle` accepted before the exact-length check.
const MAX_USER_HANDLE_FIELD_BYTES: usize = 64;

/// `POST /api/auth/register/options` body; `Debug` redacted.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterOptionsBody {
    /// The enrollment code as typed.
    pub code: String,
}

impl fmt::Debug for RegisterOptionsBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegisterOptionsBody(<redacted>)")
    }
}

/// `POST /api/auth/register/verify` body (base64url fields); `Debug` redacted.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterVerifyBody {
    /// Credential id.
    pub id: String,
    /// `clientDataJSON`.
    pub client_data_json: String,
    /// `attestationObject`.
    pub attestation_object: String,
}

impl fmt::Debug for RegisterVerifyBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegisterVerifyBody(<redacted>)")
    }
}

/// Login verify and unlock body (base64url fields); `Debug` redacted.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssertionBody {
    /// Credential id.
    pub id: String,
    /// `clientDataJSON`.
    pub client_data_json: String,
    /// `authenticatorData`.
    pub authenticator_data: String,
    /// DER signature.
    pub signature: String,
    /// `userHandle`.
    pub user_handle: Option<String>,
}

impl fmt::Debug for AssertionBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AssertionBody(<redacted>)")
    }
}

/// A decoded, bounded assertion (zeroized on drop).
pub struct DecodedAssertion {
    /// Credential id.
    pub id: Zeroizing<Vec<u8>>,
    /// Exact received `clientDataJSON`.
    pub client_data_json: Zeroizing<Vec<u8>>,
    /// `authenticatorData`.
    pub authenticator_data: Zeroizing<Vec<u8>>,
    /// DER signature.
    pub signature: Zeroizing<Vec<u8>>,
    /// `userHandle`.
    pub user_handle: Option<Zeroizing<Vec<u8>>>,
}

/// A decoded, bounded registration (zeroized on drop).
pub struct DecodedRegistration {
    /// Credential id.
    pub id: Zeroizing<Vec<u8>>,
    /// Exact received `clientDataJSON`.
    pub client_data_json: Zeroizing<Vec<u8>>,
    /// `attestationObject`.
    pub attestation_object: Zeroizing<Vec<u8>>,
}

/// Malformed body (`400 bad_request`, counted as a failure).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("request body malformed")]
pub struct BodyMalformed;

/// One bounded base64url field.
fn field(value: &str, max: usize) -> Result<Zeroizing<Vec<u8>>, BodyMalformed> {
    b64url_decode(value, max)
        .map(Zeroizing::new)
        .map_err(|()| BodyMalformed)
}

/// Decodes an [`AssertionBody`] from a bounded body.
///
/// # Errors
///
/// [`BodyMalformed`].
pub fn decode_assertion(body: &[u8]) -> Result<DecodedAssertion, BodyMalformed> {
    let raw: AssertionBody = serde_json::from_slice(body).map_err(|_| BodyMalformed)?;
    let user_handle = match &raw.user_handle {
        Some(handle) => Some(field(handle, MAX_USER_HANDLE_FIELD_BYTES)?),
        None => None,
    };
    let decoded = DecodedAssertion {
        id: field(&raw.id, MAX_CREDENTIAL_ID_BYTES)?,
        client_data_json: field(&raw.client_data_json, MAX_CLIENT_DATA_JSON_BYTES)?,
        authenticator_data: field(&raw.authenticator_data, MAX_AUTH_DATA_FIELD_BYTES)?,
        signature: field(&raw.signature, MAX_SIGNATURE_BYTES)?,
        user_handle,
    };
    if decoded.id.is_empty() {
        return Err(BodyMalformed);
    }
    Ok(decoded)
}

/// Decodes a [`RegisterVerifyBody`] from a bounded body.
///
/// # Errors
///
/// [`BodyMalformed`].
pub fn decode_registration(body: &[u8]) -> Result<DecodedRegistration, BodyMalformed> {
    let raw: RegisterVerifyBody = serde_json::from_slice(body).map_err(|_| BodyMalformed)?;
    let decoded = DecodedRegistration {
        id: field(&raw.id, MAX_CREDENTIAL_ID_BYTES)?,
        client_data_json: field(&raw.client_data_json, MAX_CLIENT_DATA_JSON_BYTES)?,
        attestation_object: field(&raw.attestation_object, crate::MAX_ATTESTATION_OBJECT_BYTES)?,
    };
    if decoded.id.is_empty() {
        return Err(BodyMalformed);
    }
    Ok(decoded)
}

/// Decodes a [`RegisterOptionsBody`]; the code is at most 32 bytes.
///
/// # Errors
///
/// [`BodyMalformed`].
pub fn decode_register_options(body: &[u8]) -> Result<Zeroizing<String>, BodyMalformed> {
    let raw: RegisterOptionsBody = serde_json::from_slice(body).map_err(|_| BodyMalformed)?;
    let code = Zeroizing::new(raw.code);
    if code.len() > MAX_CODE_FIELD_BYTES {
        return Err(BodyMalformed);
    }
    Ok(code)
}
