//! Funnel web sessions: cookie format and parsing, hashed tokens, TTLs, cap (architect spec
//! §4.3). Named `websession` because `session.rs` already means logind sessions.
//!
//! Only SHA-256 of a token is ever stored; the token itself exists once, in the
//! `Set-Cookie` of the login that created it. At most `MAX_WEB_SESSIONS` records, never
//! evicted; a record expires `WEB_SESSION_IDLE_MS` after its last request-level use or
//! `WEB_SESSION_ABSOLUTE_MS` after its creation, whichever comes first. Every lookup compares
//! every record in constant time.

use std::fmt;
use std::time::Duration;

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::time::Instant;
use zeroize::Zeroizing;

use crate::webauthn::{b64url_decode, b64url_encode};
use crate::{
    MAX_WEB_SESSIONS, SESSION_COOKIE_ATTRIBUTES, SESSION_COOKIE_NAME, SESSION_TOKEN_BYTES,
    WEB_SESSION_ABSOLUTE_MS, WEB_SESSION_IDLE_MS,
};

/// Length of the base64url (unpadded) cookie value of a `SESSION_TOKEN_BYTES` token.
const COOKIE_VALUE_LEN: usize = 43;

/// One live session (private): hashes only.
struct WebSessionRecord {
    token_hash: [u8; 32],
    credential_hash: [u8; 32],
    created: Instant,
    last_used: Instant,
}

impl WebSessionRecord {
    fn is_live(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_used) < Duration::from_millis(WEB_SESSION_IDLE_MS)
            && now.saturating_duration_since(self.created)
                < Duration::from_millis(WEB_SESSION_ABSOLUTE_MS)
    }
}

/// Live web sessions (at most `MAX_WEB_SESSIONS`).
#[derive(Default)]
pub struct WebSessionStore {
    records: Vec<WebSessionRecord>,
}

impl fmt::Debug for WebSessionStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WebSessionStore(<redacted>)")
    }
}

/// A freshly issued token (43-char base64url), zeroized on drop; `Debug` redacted.
pub struct IssuedToken(Zeroizing<String>);

impl fmt::Debug for IssuedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IssuedToken(<redacted>)")
    }
}

impl IssuedToken {
    /// The cookie value.
    #[must_use]
    pub fn set_cookie_value(&self) -> String {
        self.0.as_str().to_string()
    }
}

/// Whether a validation counts as activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Touch {
    /// Request-level validation: refreshes the idle time.
    Refresh,
    /// SSE keep-alive re-validation: never refreshes the idle time.
    Keep,
}

/// SHA-256 of `bytes`.
fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

impl WebSessionStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Purges expired records; when the store is still full, [`WebSessionError::TooMany`]
    /// (never evicts); otherwise stores the token hash and returns the cookie value.
    ///
    /// # Errors
    ///
    /// [`WebSessionError::TooMany`].
    pub fn create(
        &mut self,
        now: Instant,
        token: [u8; SESSION_TOKEN_BYTES],
        credential_hash: [u8; 32],
    ) -> Result<IssuedToken, WebSessionError> {
        let token = Zeroizing::new(token);
        self.records.retain(|r| r.is_live(now));
        if self.records.len() >= MAX_WEB_SESSIONS {
            return Err(WebSessionError::TooMany);
        }
        self.records.push(WebSessionRecord {
            token_hash: sha256(token.as_slice()),
            credential_hash,
            created: now,
            last_used: now,
        });
        Ok(IssuedToken(Zeroizing::new(b64url_encode(token.as_slice()))))
    }

    /// Index of the record with `token_hash` (every record compared in constant time).
    fn position(&self, token_hash: &[u8; 32]) -> Option<usize> {
        let mut found = None;
        for (index, record) in self.records.iter().enumerate() {
            if bool::from(record.token_hash.ct_eq(token_hash)) && found.is_none() {
                found = Some(index);
            }
        }
        found
    }

    /// Valid ⇔ a record with an equal hash exists, idle for less than
    /// `WEB_SESSION_IDLE_MS` and younger than `WEB_SESSION_ABSOLUTE_MS`. A valid lookup returns
    /// the token hash and, only with [`Touch::Refresh`], sets `last_used = now`; an expired
    /// record is removed.
    #[must_use]
    pub fn validate(
        &mut self,
        now: Instant,
        token_hash: &[u8; 32],
        touch: Touch,
    ) -> Option<[u8; 32]> {
        let index = self.position(token_hash)?;
        let record = self.records.get_mut(index)?;
        if !record.is_live(now) {
            self.records.remove(index);
            return None;
        }
        if touch == Touch::Refresh {
            record.last_used = now;
        }
        Some(record.token_hash)
    }

    /// Removes the record (logout); idempotent.
    pub fn remove(&mut self, token_hash: &[u8; 32]) {
        if let Some(index) = self.position(token_hash) {
            self.records.remove(index);
        }
    }

    /// Removes every record whose credential is not in `live` (passkey removal), or all of
    /// them when `live` is `None` (store unreadable).
    pub fn retain_credentials(&mut self, live: Option<&[[u8; 32]]>) {
        match live {
            None => self.records.clear(),
            Some(live) => self.records.retain(|record| {
                live.iter().fold(false, |any, hash| {
                    any | bool::from(hash.ct_eq(&record.credential_hash))
                })
            }),
        }
    }

    /// Number of records (expired ones included until purged).
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// No record.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Pure. Scans every `cookie` header (several are allowed: HTTP/2 crumbs re-joined by the
/// proxy), splits on `;`, trims optional whitespace; exactly one pair named
/// `SESSION_COOKIE_NAME` across all headers (zero or several ⇒ `None`); its value must be
/// exactly 43 base64url characters decoding to `SESSION_TOKEN_BYTES`. Returns SHA-256 of the
/// decoded token.
#[must_use]
pub fn session_token_hash(headers: &[(&str, &[u8])]) -> Option<[u8; 32]> {
    let mut found: Option<&str> = None;
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("cookie") {
            continue;
        }
        let text = std::str::from_utf8(value).ok()?;
        for pair in text.split(';') {
            let pair = pair.trim_matches([' ', '\t']);
            let Some((key, cookie)) = pair.split_once('=') else {
                continue;
            };
            if key == SESSION_COOKIE_NAME {
                if found.is_some() {
                    return None;
                }
                found = Some(cookie);
            }
        }
    }
    let value = found?;
    if value.len() != COOKIE_VALUE_LEN {
        return None;
    }
    let token = Zeroizing::new(b64url_decode(value, SESSION_TOKEN_BYTES).ok()?);
    if token.len() != SESSION_TOKEN_BYTES {
        return None;
    }
    Some(sha256(token.as_slice()))
}

/// `"__Host-soos_session=<token>; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=28800"`.
#[must_use]
pub fn set_cookie_header(token: &IssuedToken) -> String {
    format!(
        "{SESSION_COOKIE_NAME}={}; {SESSION_COOKIE_ATTRIBUTES}; Max-Age={}",
        token.0.as_str(),
        WEB_SESSION_ABSOLUTE_MS / 1000
    )
}

/// `"__Host-soos_session=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"`.
#[must_use]
pub fn clear_cookie_header() -> String {
    format!("{SESSION_COOKIE_NAME}=; {SESSION_COOKIE_ATTRIBUTES}; Max-Age=0")
}

/// Session refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WebSessionError {
    /// `429 too_many_sessions`.
    #[error("too many web sessions")]
    TooMany,
}
