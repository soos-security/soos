//! Pure Web Push cryptography (ADR 2026-10-06 "Web Push Notifications for Failed-Password
//! Alerts Through a Separate Sender Unit", architect spec `AI/architect_spec_remote_web_push.md`
//! §4): the VAPID key (RFC 8292), the ES256 VAPID JWT and its bounded cache, the subscription
//! keys and the RFC 8291 `aes128gcm` message encryption (RFC 8188, one record).
//!
//! RustCrypto only (`p256` with its existing `ecdsa` feature, `hmac`, `sha2`, `aes-gcm`);
//! every random byte comes from the injected [`RandomSource`]; time is an argument (no clock
//! is read here); every intermediate secret is wiped on drop. Nothing here logs.

use std::fmt;

use aes_gcm::aead::{AeadInPlace, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use base64ct::{Base64UrlUnpadded, Encoding};
use hmac::{Hmac, Mac};
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::point::AffineCoordinates;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};
use sha2::Sha256;
use soos_push_protocol::PUSH_HOSTS;
use zeroize::Zeroizing;

use crate::auth::RandomSource;
use crate::{
    MAX_PUSH_PLAINTEXT_BYTES, MIN_PLAUSIBLE_UNIX_S, PUSH_RECORD_SIZE, VAPID_JWT_LIFETIME_S,
    VAPID_JWT_REUSE_S,
};

/// Draws of a scalar before giving up (zero or ≥ n is redrawn).
const MAX_SCALAR_DRAWS: usize = 4;
/// Length of an uncompressed SEC1 P-256 point.
const UNCOMPRESSED_POINT_LEN: usize = 65;
/// Length of the subscription authentication secret.
const AUTH_SECRET_LEN: usize = 16;
/// Length of the RFC 8188 salt.
const SALT_LEN: usize = 16;
/// RFC 8188 last-record delimiter.
const LAST_RECORD_DELIMITER: u8 = 0x02;
/// The base64url JOSE header of every VAPID JWT (`{"typ":"JWT","alg":"ES256"}`).
const JWT_HEADER: &str = "{\"typ\":\"JWT\",\"alg\":\"ES256\"}";

/// Web Push failure. Fixed texts: never a key, a value or a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WebPushError {
    /// The random source failed or kept returning invalid scalars.
    #[error("random source failed")]
    Random,
    /// The private key is not a valid P-256 scalar.
    #[error("invalid push key")]
    InvalidKey,
    /// The subscription keys are malformed.
    #[error("invalid subscription keys")]
    InvalidSubscriptionKeys,
    /// The notification payload is too large.
    #[error("notification payload too large")]
    PlaintextTooLarge,
    /// Encryption failed.
    #[error("notification encryption failed")]
    Encrypt,
    /// The clock is implausible.
    #[error("clock not plausible")]
    Clock,
    /// Encoding failed.
    #[error("notification encoding failed")]
    Encode,
}

/// Draws a valid non-zero P-256 scalar (at most [`MAX_SCALAR_DRAWS`] draws).
fn draw_secret(random: &RandomSource) -> Result<SecretKey, WebPushError> {
    for _ in 0..MAX_SCALAR_DRAWS {
        let mut bytes = Zeroizing::new([0u8; 32]);
        random(&mut bytes[..]).map_err(|_| WebPushError::Random)?;
        if let Ok(secret) = SecretKey::from_slice(&bytes[..]) {
            return Ok(secret);
        }
    }
    Err(WebPushError::Random)
}

/// The 65-byte uncompressed SEC1 encoding of `key`.
fn uncompressed(key: &PublicKey) -> Vec<u8> {
    key.to_encoded_point(false).as_bytes().to_vec()
}

/// VAPID P-256 key. The secret zeroizes on drop; `Debug` is redacted.
pub struct VapidKey(SecretKey);

impl fmt::Debug for VapidKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VapidKey(<redacted>)")
    }
}

impl VapidKey {
    /// Draws a fresh key (zero or ≥ n redrawn, at most 4 draws).
    ///
    /// # Errors
    /// [`WebPushError::Random`].
    pub fn generate(random: &RandomSource) -> Result<Self, WebPushError> {
        draw_secret(random).map(Self)
    }

    /// The key of a stored 32-byte scalar.
    ///
    /// # Errors
    /// [`WebPushError::InvalidKey`] for zero or a value ≥ n.
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self, WebPushError> {
        SecretKey::from_slice(&bytes[..])
            .map(Self)
            .map_err(|_| WebPushError::InvalidKey)
    }

    /// The 32-byte scalar (wiped on drop).
    #[must_use]
    pub fn to_bytes(&self) -> Zeroizing<[u8; 32]> {
        let field = Zeroizing::new(self.0.to_bytes());
        let mut out = Zeroizing::new([0u8; 32]);
        for (dst, src) in out.iter_mut().zip(field.iter()) {
            *dst = *src;
        }
        out
    }

    /// The public key as `applicationServerKey` and VAPID `k`: 65-byte uncompressed point in
    /// base64url without padding (87 characters).
    #[must_use]
    pub fn public_key_b64(&self) -> String {
        Base64UrlUnpadded::encode_string(&uncompressed(&self.0.public_key()))
    }
}

/// The keys of one subscription (the phone's side). No `Debug`.
#[derive(Clone)]
pub struct UaKeys {
    /// `keys.p256dh`: the user agent's ECDH public key.
    pub p256dh: PublicKey,
    /// `keys.auth`: the 16-byte authentication secret.
    pub auth: Zeroizing<[u8; 16]>,
}

/// Parses `keys.p256dh` (base64url without padding of exactly 65 bytes, `0x04`, on the
/// curve) and `keys.auth` (base64url without padding of exactly 16 bytes).
///
/// # Errors
/// [`WebPushError::InvalidSubscriptionKeys`].
pub fn parse_subscription_keys(p256dh_b64: &str, auth_b64: &str) -> Result<UaKeys, WebPushError> {
    let invalid = WebPushError::InvalidSubscriptionKeys;
    let point = Base64UrlUnpadded::decode_vec(p256dh_b64).map_err(|_| invalid)?;
    if point.len() != UNCOMPRESSED_POINT_LEN || point.first() != Some(&0x04) {
        return Err(invalid);
    }
    let p256dh = PublicKey::from_sec1_bytes(&point).map_err(|_| invalid)?;
    let auth_bytes = Zeroizing::new(Base64UrlUnpadded::decode_vec(auth_b64).map_err(|_| invalid)?);
    if auth_bytes.len() != AUTH_SECRET_LEN {
        return Err(invalid);
    }
    let mut auth = Zeroizing::new([0u8; 16]);
    for (dst, src) in auth.iter_mut().zip(auth_bytes.iter()) {
        *dst = *src;
    }
    Ok(UaKeys { p256dh, auth })
}

/// The base64url renderings of `keys` (`p256dh`, `auth`), as stored and as the browser
/// sends them.
#[must_use]
pub fn encode_subscription_keys(keys: &UaKeys) -> (String, Zeroizing<String>) {
    (
        Base64UrlUnpadded::encode_string(&uncompressed(&keys.p256dh)),
        Zeroizing::new(Base64UrlUnpadded::encode_string(&keys.auth[..])),
    )
}

/// HMAC-SHA-256 of the concatenated `parts` under `key`.
fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> Result<Zeroizing<[u8; 32]>, WebPushError> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).map_err(|_| WebPushError::Encrypt)?;
    for part in parts {
        mac.update(part);
    }
    let digest = Zeroizing::new(mac.finalize().into_bytes());
    let mut out = Zeroizing::new([0u8; 32]);
    for (dst, src) in out.iter_mut().zip(digest.iter()) {
        *dst = *src;
    }
    Ok(out)
}

/// The x-coordinate of `public · secret` (P-256 ECDH, RFC 8291 §3.1): constant-time group
/// arithmetic of `p256`, not integer arithmetic.
fn ecdh_x(public: &PublicKey, secret: &SecretKey) -> p256::FieldBytes {
    std::ops::Mul::mul(public.to_projective(), *secret.to_nonzero_scalar())
        .to_affine()
        .x()
}

/// The (CEK, NONCE) pair of RFC 8291 §3.4.
pub type ContentKeys = (Zeroizing<[u8; 16]>, Zeroizing<[u8; 12]>);

/// The application-server key pair of one message and its RFC 8291 §3.4 key schedule.
struct MessageKeys {
    as_public: Vec<u8>,
    cek: Zeroizing<[u8; 16]>,
    nonce: Zeroizing<[u8; 12]>,
}

fn message_keys(
    ua: &UaKeys,
    as_secret: &[u8; 32],
    salt: &[u8; 16],
) -> Result<MessageKeys, WebPushError> {
    let secret = SecretKey::from_slice(&as_secret[..]).map_err(|_| WebPushError::InvalidKey)?;
    let as_public = uncompressed(&secret.public_key());
    let ua_public = uncompressed(&ua.p256dh);
    let ecdh = Zeroizing::new(ecdh_x(&ua.p256dh, &secret));
    if ecdh.iter().all(|b| *b == 0) {
        return Err(WebPushError::Encrypt);
    }
    let prk_key = hmac_sha256(&ua.auth[..], &[&ecdh[..]])?;
    let ikm = hmac_sha256(
        &prk_key[..],
        &[b"WebPush: info\0", &ua_public, &as_public, &[0x01]],
    )?;
    let prk = hmac_sha256(&salt[..], &[&ikm[..]])?;
    let cek_full = hmac_sha256(&prk[..], &[b"Content-Encoding: aes128gcm\0\x01"])?;
    let nonce_full = hmac_sha256(&prk[..], &[b"Content-Encoding: nonce\0\x01"])?;
    let mut cek = Zeroizing::new([0u8; 16]);
    for (dst, src) in cek.iter_mut().zip(cek_full.iter()) {
        *dst = *src;
    }
    let mut nonce = Zeroizing::new([0u8; 12]);
    for (dst, src) in nonce.iter_mut().zip(nonce_full.iter()) {
        *dst = *src;
    }
    Ok(MessageKeys {
        as_public,
        cek,
        nonce,
    })
}

/// Test seam: the (CEK, NONCE) of RFC 8291 §3.4.
///
/// # Errors
/// [`WebPushError::InvalidKey`], [`WebPushError::Encrypt`].
#[doc(hidden)]
pub fn derive_keys(
    ua: &UaKeys,
    as_secret: &[u8; 32],
    salt: &[u8; 16],
) -> Result<ContentKeys, WebPushError> {
    let keys = message_keys(ua, as_secret, salt)?;
    Ok((keys.cek, keys.nonce))
}

/// RFC 8291 + RFC 8188 `aes128gcm`, one record, `rs = PUSH_RECORD_SIZE`, delimiter `0x02`,
/// no padding: `salt(16) ‖ rs(u32 BE) ‖ idlen(1) = 65 ‖ as_public(65) ‖ ciphertext ‖ tag`.
///
/// # Errors
/// [`WebPushError::PlaintextTooLarge`], [`WebPushError::InvalidKey`],
/// [`WebPushError::Encrypt`].
pub fn encrypt_with(
    plaintext: &[u8],
    ua: &UaKeys,
    as_secret: &[u8; 32],
    salt: &[u8; 16],
) -> Result<Zeroizing<Vec<u8>>, WebPushError> {
    if plaintext.len() > MAX_PUSH_PLAINTEXT_BYTES {
        return Err(WebPushError::PlaintextTooLarge);
    }
    let keys = message_keys(ua, as_secret, salt)?;
    let cipher = Aes128Gcm::new_from_slice(&keys.cek[..]).map_err(|_| WebPushError::Encrypt)?;
    let nonce = Nonce::from(*keys.nonce);
    let mut record = Zeroizing::new(Vec::with_capacity(plaintext.len().saturating_add(17)));
    record.extend_from_slice(plaintext);
    record.push(LAST_RECORD_DELIMITER);
    cipher
        .encrypt_in_place(&nonce, b"", &mut *record)
        .map_err(|_| WebPushError::Encrypt)?;
    let idlen = u8::try_from(keys.as_public.len()).map_err(|_| WebPushError::Encrypt)?;
    let mut body = Zeroizing::new(Vec::with_capacity(
        SALT_LEN
            .saturating_add(5)
            .saturating_add(keys.as_public.len())
            .saturating_add(record.len()),
    ));
    body.extend_from_slice(&salt[..]);
    body.extend_from_slice(&PUSH_RECORD_SIZE.to_be_bytes());
    body.push(idlen);
    body.extend_from_slice(&keys.as_public);
    body.extend_from_slice(&record);
    Ok(body)
}

/// Draws the application-server secret (retried like [`VapidKey::generate`]) and the salt
/// from `random`, then [`encrypt_with`].
///
/// # Errors
/// [`WebPushError::PlaintextTooLarge`] (before any draw), [`WebPushError::Random`] and the
/// errors of [`encrypt_with`].
pub fn encrypt(
    plaintext: &[u8],
    ua: &UaKeys,
    random: &RandomSource,
) -> Result<Zeroizing<Vec<u8>>, WebPushError> {
    if plaintext.len() > MAX_PUSH_PLAINTEXT_BYTES {
        return Err(WebPushError::PlaintextTooLarge);
    }
    let secret = draw_secret(random)?;
    let field = Zeroizing::new(secret.to_bytes());
    let mut as_secret = Zeroizing::new([0u8; 32]);
    for (dst, src) in as_secret.iter_mut().zip(field.iter()) {
        *dst = *src;
    }
    let mut salt = [0u8; SALT_LEN];
    random(&mut salt).map_err(|_| WebPushError::Random)?;
    encrypt_with(plaintext, ua, &as_secret, &salt)
}

/// VAPID claims, serialized in this order.
#[derive(serde::Serialize)]
struct Claims<'a> {
    aud: &'a str,
    exp: u64,
    sub: &'a str,
}

/// `vapid t=<jwt>, k=<public key>` (RFC 8292): ES256 (deterministic, RFC 6979) over the
/// header `{"typ":"JWT","alg":"ES256"}` and the claims `aud` (the endpoint origin), `exp =
/// now_s + VAPID_JWT_LIFETIME_S`, `sub`; raw 64-byte `r‖s` signature.
///
/// # Errors
/// [`WebPushError::Clock`] for `now_s < MIN_PLAUSIBLE_UNIX_S` or an overflowing `exp`;
/// [`WebPushError::Encode`].
pub fn vapid_authorization(
    key: &VapidKey,
    origin: &str,
    subject: &str,
    now_s: u64,
) -> Result<Zeroizing<String>, WebPushError> {
    if now_s < MIN_PLAUSIBLE_UNIX_S {
        return Err(WebPushError::Clock);
    }
    let exp = now_s
        .checked_add(VAPID_JWT_LIFETIME_S)
        .ok_or(WebPushError::Clock)?;
    let claims = serde_json::to_vec(&Claims {
        aud: origin,
        exp,
        sub: subject,
    })
    .map_err(|_| WebPushError::Encode)?;
    let mut signing_input = Zeroizing::new(Base64UrlUnpadded::encode_string(JWT_HEADER.as_bytes()));
    signing_input.push('.');
    signing_input.push_str(&Base64UrlUnpadded::encode_string(&claims));
    let signing_key = SigningKey::from(&key.0);
    let signature: Signature = signing_key
        .try_sign(signing_input.as_bytes())
        .map_err(|_| WebPushError::Encode)?;
    let raw = signature.to_bytes();
    let mut value = Zeroizing::new(String::with_capacity(
        signing_input.len().saturating_add(200),
    ));
    value.push_str("vapid t=");
    value.push_str(&signing_input);
    value.push('.');
    value.push_str(&Base64UrlUnpadded::encode_string(&raw));
    value.push_str(", k=");
    value.push_str(&key.public_key_b64());
    Ok(value)
}

/// One cached token.
struct CachedToken {
    origin: String,
    public_key: String,
    issued_s: u64,
    token: Zeroizing<String>,
}

/// At most one token per allowlisted origin (≤ `PUSH_HOSTS.len()` entries), reused while
/// `issued ≤ now < issued + VAPID_JWT_REUSE_S` with the same key.
#[derive(Default)]
pub struct JwtCache {
    entries: Vec<CachedToken>,
}

impl fmt::Debug for JwtCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JwtCache(<redacted>)")
    }
}

impl JwtCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached value for `origin` when still reusable, else a fresh one (cached).
    ///
    /// # Errors
    /// The errors of [`vapid_authorization`] (nothing is cached then).
    pub fn authorization(
        &mut self,
        key: &VapidKey,
        origin: &str,
        subject: &str,
        now_s: u64,
    ) -> Result<Zeroizing<String>, WebPushError> {
        let public_key = key.public_key_b64();
        if let Some(entry) = self.entries.iter().find(|entry| {
            entry.origin == origin
                && entry.public_key == public_key
                && entry.issued_s <= now_s
                && now_s < entry.issued_s.saturating_add(VAPID_JWT_REUSE_S)
        }) {
            return Ok(entry.token.clone());
        }
        let token = vapid_authorization(key, origin, subject, now_s)?;
        self.entries.retain(|entry| entry.origin != origin);
        while self.entries.len() >= PUSH_HOSTS.len() {
            self.entries.remove(0);
        }
        self.entries.push(CachedToken {
            origin: origin.to_owned(),
            public_key,
            issued_s: now_s,
            token: token.clone(),
        });
        Ok(token)
    }

    /// Drops every cached token (the key was replaced).
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
