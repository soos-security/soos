//! Bounded, purpose/path/binding-scoped, single-use WebAuthn challenge store (architect
//! spec §4.2).
//!
//! Pools: the authenticated pools `(Tailnet, Register)`, `(Tailnet, Unlock)`,
//! `(Funnel, Unlock)`, `(Tailnet, CameraView)` and `(Funnel, CameraView)` (ADR 2026-10-07)
//! hold at most `MAX_PENDING_CHALLENGES` entries each and never evict;
//! the anonymous `(Funnel, Login)` pool holds at most `MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES`
//! entries, at most `MAX_LOGIN_CHALLENGES_PER_HINT` per client hint, and evicts its oldest
//! entry instead of refusing (eviction can only fail a ceremony, never admit one). An entry
//! is removed on the first `take` that names its bytes, whatever the outcome. Every
//! comparison of secret bytes is constant time.

use std::fmt;
use std::time::Duration;

use subtle::ConstantTimeEq;
use tokio::time::Instant;

use crate::identity::{ClientHint, PathClass};
use crate::{
    CHALLENGE_BYTES, CHALLENGE_TTL_MS, MAX_LOGIN_CHALLENGES_PER_HINT,
    MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES, MAX_PENDING_CHALLENGES, USER_HANDLE_BYTES,
};

/// Ceremony a challenge was issued for. Never rendered as text (invariant RMC-S3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChallengePurpose {
    /// Passkey registration.
    Register,
    /// Funnel login.
    Login,
    /// Remote unlock.
    Unlock,
    /// Start of a live camera view (ADR 2026-10-07); its own pools, never an unlock.
    CameraView,
}

/// What a challenge is bound to besides purpose and path. `Debug` is redacted; the derived
/// `PartialEq` exists for tests only (production compares in constant time).
#[derive(Clone, PartialEq, Eq)]
pub enum ChallengeBinding {
    /// No extra binding (tailnet unlock).
    None,
    /// Anonymous Funnel login bucket (never compared by `take`).
    Login(ClientHint),
    /// SHA-256 of the web-session token (Funnel unlock).
    WebSession([u8; 32]),
    /// SHA-256 of the normalized enrollment code (registration).
    EnrollCode([u8; 32]),
}

impl fmt::Debug for ChallengeBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChallengeBinding(<redacted>)")
    }
}

impl ChallengeBinding {
    /// Constant-time binding comparison; for `Login(_)` only the variant is compared.
    fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, Self::None) | (Self::Login(_), Self::Login(_)) => true,
            (Self::WebSession(a), Self::WebSession(b))
            | (Self::EnrollCode(a), Self::EnrollCode(b)) => bool::from(a.ct_eq(b)),
            _ => false,
        }
    }
}

/// Registration state carried by a `Register` challenge; `Debug` redacted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PendingRegistration {
    /// The user handle sent as `user.id` in the options.
    pub user_handle: [u8; USER_HANDLE_BYTES],
}

impl fmt::Debug for PendingRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingRegistration(<redacted>)")
    }
}

/// What a successful `take` hands back; `Debug` redacted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    /// Login, unlock or camera-view challenge.
    Plain,
    /// Register challenge with its pending user handle.
    Register(PendingRegistration),
}

impl fmt::Debug for Taken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plain => f.write_str("Taken::Plain"),
            Self::Register(_) => f.write_str("Taken::Register(<redacted>)"),
        }
    }
}

/// One pending challenge.
struct Entry {
    bytes: [u8; CHALLENGE_BYTES],
    expires: Instant,
    class: PathClass,
    purpose: ChallengePurpose,
    binding: ChallengeBinding,
    pending: Option<PendingRegistration>,
}

impl Entry {
    fn in_pool(&self, class: PathClass, purpose: ChallengePurpose) -> bool {
        self.class == class && self.purpose == purpose
    }

    fn hint(&self) -> Option<ClientHint> {
        match self.binding {
            ChallengeBinding::Login(hint) => Some(hint),
            _ => None,
        }
    }
}

/// Pending challenges, in issuance order (oldest first). At most
/// `5 × MAX_PENDING_CHALLENGES + MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES` entries.
#[derive(Default)]
pub struct ChallengeStore {
    entries: Vec<Entry>,
}

impl fmt::Debug for ChallengeStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChallengeStore(<redacted>)")
    }
}

/// The pool a `(class, purpose, binding)` triple belongs to.
enum Pool {
    /// Bounded by `MAX_PENDING_CHALLENGES`, never evicts.
    Authenticated,
    /// The anonymous Funnel login pool of one hint.
    Anonymous(ClientHint),
}

fn pool_of(
    class: PathClass,
    purpose: ChallengePurpose,
    binding: &ChallengeBinding,
) -> Option<Pool> {
    match (class, purpose, binding) {
        (PathClass::Funnel, ChallengePurpose::Login, ChallengeBinding::Login(hint)) => {
            Some(Pool::Anonymous(*hint))
        }
        (PathClass::Tailnet, ChallengePurpose::Register, ChallengeBinding::EnrollCode(_))
        | (PathClass::Tailnet, ChallengePurpose::Unlock, ChallengeBinding::None)
        | (PathClass::Funnel, ChallengePurpose::Unlock, ChallengeBinding::WebSession(_))
        | (PathClass::Tailnet, ChallengePurpose::CameraView, ChallengeBinding::None)
        | (PathClass::Funnel, ChallengePurpose::CameraView, ChallengeBinding::WebSession(_)) => {
            Some(Pool::Authenticated)
        }
        _ => None,
    }
}

impl ChallengeStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Removes the first (oldest) entry matching `predicate`; true when one was removed.
    fn remove_oldest(&mut self, predicate: impl Fn(&Entry) -> bool) -> bool {
        match self.entries.iter().position(predicate) {
            Some(index) => {
                self.entries.remove(index);
                true
            }
            None => false,
        }
    }

    /// Purges expired entries of the pool, then stores `bytes` with `expires = now +
    /// CHALLENGE_TTL_MS`. `pending` must be `Some` exactly when `purpose == Register`, and
    /// the binding must fit the pool, else [`ChallengeError::Mismatch`] (nothing stored).
    /// Authenticated pools refuse when full ([`ChallengeError::PoolFull`]); the anonymous
    /// login pool first evicts the hint's oldest entry when the hint is at
    /// `MAX_LOGIN_CHALLENGES_PER_HINT`, then the pool's oldest when the pool is full. Returns
    /// the number of entries evicted (0..=2).
    ///
    /// # Errors
    ///
    /// [`ChallengeError::PoolFull`], [`ChallengeError::Mismatch`].
    pub fn issue(
        &mut self,
        now: Instant,
        class: PathClass,
        purpose: ChallengePurpose,
        binding: ChallengeBinding,
        pending: Option<PendingRegistration>,
        bytes: [u8; CHALLENGE_BYTES],
    ) -> Result<usize, ChallengeError> {
        if pending.is_some() != (purpose == ChallengePurpose::Register) {
            return Err(ChallengeError::Mismatch);
        }
        let pool = pool_of(class, purpose, &binding).ok_or(ChallengeError::Mismatch)?;
        self.entries
            .retain(|e| !(e.in_pool(class, purpose) && e.expires <= now));
        let in_pool = self
            .entries
            .iter()
            .filter(|e| e.in_pool(class, purpose))
            .count();
        let mut evicted = 0usize;
        match pool {
            Pool::Authenticated => {
                if in_pool >= MAX_PENDING_CHALLENGES {
                    return Err(ChallengeError::PoolFull);
                }
            }
            Pool::Anonymous(hint) => {
                let of_hint = self
                    .entries
                    .iter()
                    .filter(|e| e.in_pool(class, purpose) && e.hint() == Some(hint))
                    .count();
                if of_hint >= MAX_LOGIN_CHALLENGES_PER_HINT
                    && self.remove_oldest(|e| e.in_pool(class, purpose) && e.hint() == Some(hint))
                {
                    evicted = evicted.saturating_add(1);
                }
                let in_pool = self
                    .entries
                    .iter()
                    .filter(|e| e.in_pool(class, purpose))
                    .count();
                if in_pool >= MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES
                    && self.remove_oldest(|e| e.in_pool(class, purpose))
                {
                    evicted = evicted.saturating_add(1);
                }
            }
        }
        let expires = now
            .checked_add(Duration::from_millis(CHALLENGE_TTL_MS))
            .unwrap_or(now);
        self.entries.push(Entry {
            bytes,
            expires,
            class,
            purpose,
            binding,
            pending,
        });
        Ok(evicted)
    }

    /// Finds the entry whose bytes equal `bytes` (constant time per entry, every entry
    /// compared), removes it whatever the outcome, then checks expiry, class, purpose and
    /// binding (for `Login(_)` only the variant, never the hint).
    ///
    /// # Errors
    ///
    /// [`ChallengeError::Unknown`], [`ChallengeError::Expired`], [`ChallengeError::Mismatch`].
    pub fn take(
        &mut self,
        now: Instant,
        class: PathClass,
        purpose: ChallengePurpose,
        binding: &ChallengeBinding,
        bytes: &[u8; CHALLENGE_BYTES],
    ) -> Result<Taken, ChallengeError> {
        let mut found: Option<usize> = None;
        for (index, entry) in self.entries.iter().enumerate() {
            let equal = bool::from(entry.bytes.ct_eq(bytes));
            if equal && found.is_none() {
                found = Some(index);
            }
        }
        let index = found.ok_or(ChallengeError::Unknown)?;
        let entry = self.entries.remove(index);
        if now >= entry.expires {
            return Err(ChallengeError::Expired);
        }
        if entry.class != class || entry.purpose != purpose || !entry.binding.matches(binding) {
            return Err(ChallengeError::Mismatch);
        }
        Ok(match entry.pending {
            Some(pending) => Taken::Register(pending),
            None => Taken::Plain,
        })
    }

    /// Entries stored in a pool (test hook; purges nothing).
    #[must_use]
    pub fn pending(&self, class: PathClass, purpose: ChallengePurpose) -> usize {
        self.entries
            .iter()
            .filter(|e| e.in_pool(class, purpose))
            .count()
    }

    /// Anonymous login entries of one hint (test hook; purges nothing).
    #[must_use]
    pub fn pending_for_hint(&self, hint: ClientHint) -> usize {
        self.entries
            .iter()
            .filter(|e| e.in_pool(PathClass::Funnel, ChallengePurpose::Login))
            .filter(|e| e.hint() == Some(hint))
            .count()
    }
}

/// Challenge refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChallengeError {
    /// `429 too_many_challenges`.
    #[error("challenge pool full")]
    PoolFull,
    /// `403 passkey_rejected`.
    #[error("challenge unknown")]
    Unknown,
    /// `403 passkey_rejected`.
    #[error("challenge expired")]
    Expired,
    /// `403 passkey_rejected`.
    #[error("challenge mismatch")]
    Mismatch,
}
