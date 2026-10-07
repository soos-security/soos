//! Web Push fixtures of the push contract tests (ADR 2026-10-06 "Web Push Notifications for
//! Failed-Password Alerts Through a Separate Sender Unit", architect spec
//! `AI/architect_spec_remote_web_push.md` §12.5).
//!
//! - [`UaFixture`]: a subscription key pair held by the test (the phone's side): P-256
//!   private key + 16-byte auth secret, rendered as the browser's `toJSON` strings.
//! - [`decrypt_aes128gcm`]: RFC 8291 / RFC 8188 receiver side (test-only), so a test reads
//!   the exact plaintext the service encrypted.
//! - [`verify_vapid`]: splits `vapid t=<jwt>, k=<key>`, checks the ES256 raw signature and
//!   returns the decoded header and claims.
//! - [`FakeTransport`]: a `PushTransport` recording every `DeliveryRequest` (with the virtual
//!   time of the call) and answering scripted replies or errors, or holding forever.
//! - Endpoint and subscription body builders.
//!
//! Nothing here opens a network socket or contacts a push service.
//!
//! Include with `#[path = "common/push.rs"] mod push;` (needs `passkey.rs` for `sha256`,
//! `b64url`, `b64url_decode`).

#![allow(
    dead_code,
    unused_imports,
    reason = "Shared test fixtures library used conditionally across test modules"
)]

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::elliptic_curve::point::AffineCoordinates;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::time::Instant;

use soos_push_protocol::{DeliveryReply, DeliveryRequest, Outcome};
use soos_remote::push::{PushTransport, TransportError};

use super::passkey::{b64url, b64url_decode, sha256};

// ---------------------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------------------

/// An Apple endpoint with a distinct token per `n`.
pub fn apple_endpoint(n: u32) -> String {
    format!("https://web.push.apple.com/QGuQyavXutnMH8l2ce1fmbpFBjgLv9SQK3-ASsYlvuDH_tok{n}")
}

/// A Mozilla endpoint with a distinct token per `n`.
pub fn mozilla_endpoint(n: u32) -> String {
    format!("https://updates.push.services.mozilla.com/wpush/v2/gAAAAABl3n0yK1b2Qz8sP-tok{n}")
}

/// An FCM endpoint with a distinct token per `n`.
pub fn fcm_endpoint(n: u32) -> String {
    format!("https://fcm.googleapis.com/fcm/send/dpH5lCsTSSM:APA91bHqjZxM0VImWW-tok{n}")
}

/// The path part of an endpoint (the capability token: a log or response needle).
pub fn endpoint_token(endpoint: &str) -> String {
    let rest = endpoint.strip_prefix("https://").unwrap();
    let slash = rest.find('/').unwrap();
    rest[slash + 1..].to_string()
}

// ---------------------------------------------------------------------------------------
// HMAC-SHA-256 and HKDF of RFC 8291 (test-only reference, built on sha2)
// ---------------------------------------------------------------------------------------

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&sha256(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0u8; 64];
    let mut opad = [0u8; 64];
    for i in 0..64 {
        ipad[i] = k[i] ^ 0x36;
        opad[i] = k[i] ^ 0x5c;
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner);
    outer.finalize().into()
}

// ---------------------------------------------------------------------------------------
// The phone's subscription keys
// ---------------------------------------------------------------------------------------

/// One subscription key pair held by the test.
#[derive(Clone)]
pub struct UaFixture {
    pub secret: SecretKey,
    pub auth: [u8; 16],
}

impl UaFixture {
    /// A deterministic key pair per `seed` (scalar = SHA-256 of the seed text).
    pub fn new(seed: u8) -> Self {
        let mut n = 0u32;
        loop {
            let digest = sha256(format!("soos-push-ua-{seed}-{n}").as_bytes());
            if let Ok(secret) = SecretKey::from_slice(&digest) {
                let auth_digest = sha256(format!("soos-push-auth-{seed}").as_bytes());
                let mut auth = [0u8; 16];
                auth.copy_from_slice(&auth_digest[..16]);
                return Self { secret, auth };
            }
            n += 1;
        }
    }

    /// The RFC 8291 Appendix A receiver.
    pub fn rfc8291() -> Self {
        let secret = SecretKey::from_slice(&b64url_decode(
            "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94",
        ))
        .unwrap();
        let mut auth = [0u8; 16];
        auth.copy_from_slice(&b64url_decode("BTBZMqHH6r4Tts7J_aSIgg"));
        Self { secret, auth }
    }

    /// 65-byte uncompressed public key.
    pub fn public_bytes(&self) -> Vec<u8> {
        self.secret
            .public_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec()
    }

    /// `keys.p256dh` of `PushSubscription.toJSON()` (base64url, no padding, 87 chars).
    pub fn p256dh_b64(&self) -> String {
        b64url(&self.public_bytes())
    }

    /// `keys.auth` (base64url, no padding, 22 chars).
    pub fn auth_b64(&self) -> String {
        b64url(&self.auth)
    }

    /// The parsed keys through the crate's validator.
    pub fn ua_keys(&self) -> soos_remote::webpush::UaKeys {
        soos_remote::webpush::parse_subscription_keys(&self.p256dh_b64(), &self.auth_b64())
            .unwrap_or_else(|e| panic!("fixture keys rejected: {e:?}"))
    }
}

/// The fixed subscription key pair of most tests (spec §12.5 `ua_keys()`).
pub fn ua_keys() -> UaFixture {
    UaFixture::new(1)
}

/// `PushSubscription.toJSON()` as Safari sends it.
pub fn subscribe_body(endpoint: &str, ua: &UaFixture) -> String {
    format!(
        "{{\"endpoint\":\"{endpoint}\",\"expirationTime\":null,\"keys\":{{\"p256dh\":\"{}\",\"auth\":\"{}\"}}}}",
        ua.p256dh_b64(),
        ua.auth_b64()
    )
}

/// `{"endpoint":"…"}` of the unsubscribe route.
pub fn unsubscribe_body(endpoint: &str) -> String {
    format!("{{\"endpoint\":\"{endpoint}\"}}")
}

// ---------------------------------------------------------------------------------------
// RFC 8291 receiver
// ---------------------------------------------------------------------------------------

/// Decrypts one `aes128gcm` body (RFC 8188 header + one record) with the receiver keys;
/// checks `rs`, `idlen = 65` and the last-record delimiter `0x02`; returns the plaintext.
pub fn decrypt_aes128gcm(body: &[u8], ua: &UaFixture) -> Vec<u8> {
    assert!(body.len() > 86 + 16, "body too short: {}", body.len());
    let salt = &body[..16];
    let rs = u32::from_be_bytes(body[16..20].try_into().unwrap());
    assert_eq!(rs, 4096, "record size");
    assert_eq!(body[20], 65, "idlen");
    let as_public = &body[21..86];
    let ciphertext = &body[86..];
    assert!(
        ciphertext.len() <= rs as usize,
        "exactly one record ({} bytes)",
        ciphertext.len()
    );
    let as_key = PublicKey::from_sec1_bytes(as_public).expect("sender key on the curve");
    let shared = (as_key.to_projective() * *ua.secret.to_nonzero_scalar()).to_affine();
    let ecdh = shared.x();
    let ua_public = ua.public_bytes();
    let prk_key = hmac_sha256(&ua.auth, &ecdh);
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(&ua_public);
    key_info.extend_from_slice(as_public);
    key_info.push(1);
    let ikm = hmac_sha256(&prk_key, &key_info);
    let prk = hmac_sha256(salt, &ikm);
    let cek = hmac_sha256(&prk, b"Content-Encoding: aes128gcm\0\x01");
    let nonce = hmac_sha256(&prk, b"Content-Encoding: nonce\0\x01");
    let cipher = Aes128Gcm::new_from_slice(&cek[..16]).unwrap();
    let mut plain = cipher
        .decrypt(Nonce::from_slice(&nonce[..12]), ciphertext)
        .expect("AES-128-GCM tag verifies");
    while plain.last() == Some(&0) {
        plain.pop();
    }
    assert_eq!(plain.pop(), Some(2), "last-record delimiter 0x02");
    plain
}

/// The decrypted payload as JSON.
pub fn decrypt_json(body: &[u8], ua: &UaFixture) -> Value {
    serde_json::from_slice(&decrypt_aes128gcm(body, ua)).expect("payload is JSON")
}

// ---------------------------------------------------------------------------------------
// VAPID
// ---------------------------------------------------------------------------------------

/// What a `vapid t=<jwt>, k=<key>` value carries.
pub struct Vapid {
    pub jwt: String,
    pub k: String,
    pub header: String,
    pub claims: String,
}

/// Splits and verifies an `Authorization` value: exact `vapid t=…, k=…` shape, ES256 raw
/// 64-byte signature over `header.claims` verifying with `k`; `k` equals `public_key` when
/// given.
pub fn verify_vapid(authorization: &str, public_key: Option<&str>) -> Vapid {
    let rest = authorization
        .strip_prefix("vapid t=")
        .unwrap_or_else(|| panic!("not a vapid value"));
    let (jwt, k) = rest.split_once(", k=").expect("`, k=` separator");
    assert_eq!(k.len(), 87, "k is 65 bytes in base64url");
    let k_bytes = b64url_decode(k);
    assert_eq!(k_bytes.len(), 65);
    assert_eq!(k_bytes[0], 0x04, "uncompressed point");
    if let Some(expected) = public_key {
        assert_eq!(k, expected, "k is the service's public key");
    }
    let parts: Vec<&str> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3, "JWS compact");
    let header = String::from_utf8(b64url_decode(parts[0])).unwrap();
    let claims = String::from_utf8(b64url_decode(parts[1])).unwrap();
    let sig = b64url_decode(parts[2]);
    assert_eq!(sig.len(), 64, "raw r||s, not DER");
    let signature = Signature::from_slice(&sig).expect("r||s");
    let key = VerifyingKey::from_sec1_bytes(&k_bytes).unwrap();
    key.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .expect("ES256 signature verifies with k");
    Vapid {
        jwt: jwt.to_string(),
        k: k.to_string(),
        header,
        claims,
    }
}

// ---------------------------------------------------------------------------------------
// Fake transport
// ---------------------------------------------------------------------------------------

/// What one call answers.
#[derive(Clone, Copy, Debug)]
pub enum Scripted {
    Reply(DeliveryReply),
    Fail(TransportError),
    /// Never completes.
    Hold,
}

pub const DELIVERED: DeliveryReply = DeliveryReply {
    outcome: Outcome::Delivered,
    status: Some(201),
    retry_after_s: None,
};

pub const fn reply(outcome: Outcome, status: Option<u16>, retry_after_s: Option<u32>) -> Scripted {
    Scripted::Reply(DeliveryReply {
        outcome,
        status,
        retry_after_s,
    })
}

/// One recorded call: virtual ms since the transport was made, and the request.
#[derive(Clone)]
pub struct Call {
    pub at_ms: u64,
    pub request: DeliveryRequest,
}

impl Call {
    pub fn endpoint(&self) -> &str {
        self.request.endpoint.as_str()
    }

    pub fn topic(&self) -> Option<&str> {
        self.request.topic.as_deref()
    }
}

pub struct FakeInner {
    pub start: Instant,
    pub calls: Mutex<Vec<Call>>,
    /// Answers handed out before `default` (front first, one per call).
    pub queue: Mutex<VecDeque<Scripted>>,
    pub default: Mutex<Scripted>,
}

/// A `PushTransport` driven by the test.
#[derive(Clone)]
pub struct FakeTransport(pub Arc<FakeInner>);

impl FakeTransport {
    /// Every call answers `Delivered` (201) unless scripted.
    pub fn new() -> Self {
        Self(Arc::new(FakeInner {
            start: Instant::now(),
            calls: Mutex::new(Vec::new()),
            queue: Mutex::new(VecDeque::new()),
            default: Mutex::new(Scripted::Reply(DELIVERED)),
        }))
    }

    pub fn set_default(&self, answer: Scripted) {
        *self.0.default.lock().unwrap() = answer;
    }

    pub fn queue(&self, answers: &[Scripted]) {
        self.0.queue.lock().unwrap().extend(answers.iter().copied());
    }

    pub fn calls(&self) -> Vec<Call> {
        self.0.calls.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.0.calls.lock().unwrap().len()
    }

    /// Calls whose topic is `topic`.
    pub fn calls_with_topic(&self, topic: &str) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| c.topic() == Some(topic))
            .collect()
    }

    /// Calls to `endpoint`.
    pub fn calls_to(&self, endpoint: &str) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| c.endpoint() == endpoint)
            .collect()
    }

    /// The `Arc<dyn PushTransport>` handed to `ServerState::with_push`.
    pub fn transport(&self) -> Arc<dyn PushTransport> {
        Arc::new(self.clone())
    }
}

impl Default for FakeTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl PushTransport for FakeTransport {
    fn deliver(
        &self,
        request: DeliveryRequest,
    ) -> Pin<Box<dyn Future<Output = Result<DeliveryReply, TransportError>> + Send + '_>> {
        let at_ms = self.0.start.elapsed().as_millis() as u64;
        self.0.calls.lock().unwrap().push(Call { at_ms, request });
        let answer = self
            .0
            .queue
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(*self.0.default.lock().unwrap());
        Box::pin(async move {
            match answer {
                Scripted::Reply(reply) => Ok(reply),
                Scripted::Fail(error) => Err(error),
                Scripted::Hold => std::future::pending().await,
            }
        })
    }
}
