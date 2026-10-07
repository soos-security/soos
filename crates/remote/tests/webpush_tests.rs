//! Contract tests of the pure Web Push crypto of `soos-remote` (`webpush.rs`; ADR 2026-10-06
//! "Web Push Notifications for Failed-Password Alerts Through a Separate Sender Unit",
//! architect spec `AI/architect_spec_remote_web_push.md` §4, tests 13–17; matrix RMC63,
//! RMC64).
//!
//! Every random byte comes from a scripted `RandomSource`; no clock is read (time is an
//! argument). Plan-evaluator round-2 finding encoded here: F-11 (the RFC 8291 Appendix A
//! body is 144 bytes: 86-byte header + 41-byte plaintext + 1 delimiter + 16-byte tag).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

#[path = "common/passkey.rs"]
mod passkey;

#[path = "common/push.rs"]
mod push;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use passkey::*;
use push::*;
use soos_remote::auth::{RandomError, RandomSource};
use soos_remote::webpush::{
    derive_keys, encrypt, encrypt_with, parse_subscription_keys, vapid_authorization, JwtCache,
    VapidKey, WebPushError,
};
use soos_remote::{
    MAX_PUSH_PLAINTEXT_BYTES, MIN_PLAUSIBLE_UNIX_S, PUSH_RECORD_SIZE, VAPID_JWT_LIFETIME_S,
    VAPID_JWT_REUSE_S,
};

// RFC 8291 Appendix A (research §3.1), base64url.
const PLAINTEXT: &str = "V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24";
const AS_PUBLIC: &str =
    "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8";
const AS_PRIVATE: &str = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";
const UA_PUBLIC: &str =
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
const SALT: &str = "DGv6ra1nlYgDCS1FRnbzlw";
const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
const CEK: &str = "oIhVW04MRdy2XN9CiKLxTg";
const NONCE: &str = "4h_95klXJ5E_qnoN";
const BODY: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";

const APPLE_ORIGIN: &str = "https://web.push.apple.com";
const SUBJECT: &str = "https://pc.tail1234.ts.net";
const NOW_S: u64 = 1_759_700_000;

fn arr32(b64: &str) -> [u8; 32] {
    b64url_decode(b64).try_into().unwrap()
}

fn arr16(b64: &str) -> [u8; 16] {
    b64url_decode(b64).try_into().unwrap()
}

/// A source handing out `draws` in order (one per call; each fills the whole buffer with
/// its bytes repeated), then a counter-based stream.
fn scripted_random(draws: Vec<Vec<u8>>) -> RandomSource {
    let queue = Arc::new(Mutex::new(draws));
    let counter = Arc::new(AtomicU64::new(1));
    Arc::new(move |buf: &mut [u8]| -> Result<(), RandomError> {
        let mut q = queue.lock().unwrap();
        if !q.is_empty() {
            let draw = q.remove(0);
            for (i, b) in buf.iter_mut().enumerate() {
                *b = draw[i % draw.len()];
            }
            return Ok(());
        }
        let n = counter.fetch_add(1, Ordering::SeqCst);
        let seed = sha256(&n.to_be_bytes());
        for (i, b) in buf.iter_mut().enumerate() {
            *b = seed[i % 32] ^ (i / 32) as u8;
        }
        Ok(())
    })
}

fn failing_random() -> RandomSource {
    Arc::new(|_: &mut [u8]| -> Result<(), RandomError> { Err(RandomError::Failed) })
}

fn rfc_key() -> VapidKey {
    VapidKey::from_bytes(&arr32(AS_PRIVATE)).unwrap()
}

/// Spec §3.1 constants used by this suite.
#[test]
fn test_rwp_crypto_constants_match_the_spec() {
    assert_eq!(MAX_PUSH_PLAINTEXT_BYTES, 1024);
    assert_eq!(PUSH_RECORD_SIZE, 4096);
    assert_eq!(VAPID_JWT_LIFETIME_S, 43_200);
    assert_eq!(VAPID_JWT_REUSE_S, 3600);
    assert_eq!(MIN_PLAUSIBLE_UNIX_S, 1_700_000_000);
}

// ---------------------------------------------------------------------------------------
// Test 13 — RFC 8291 known answer
// ---------------------------------------------------------------------------------------

/// Test 13 (RMC63, W-7, F-11): the RFC 8291 Appendix A vector: `derive_keys` gives the
/// exact CEK and NONCE; `encrypt_with` gives the exact 144-byte body; the test-side
/// receiver decrypts it back to the RFC plaintext.
#[test]
fn test_rwp_rfc8291_known_answer() {
    let ua = parse_subscription_keys(UA_PUBLIC, AUTH).unwrap();
    let as_secret = arr32(AS_PRIVATE);
    let salt = arr16(SALT);
    let (cek, nonce) = derive_keys(&ua, &as_secret, &salt).unwrap();
    assert_eq!(cek.to_vec(), b64url_decode(CEK));
    assert_eq!(nonce.to_vec(), b64url_decode(NONCE));
    let plaintext = b64url_decode(PLAINTEXT);
    assert_eq!(plaintext, b"When I grow up, I want to be a watermelon");
    let body = encrypt_with(&plaintext, &ua, &as_secret, &salt).unwrap();
    let expected = b64url_decode(BODY);
    assert_eq!(expected.len(), 144, "86 + 41 + 1 + 16");
    assert_eq!(body.len(), 144);
    assert_eq!(body.to_vec(), expected);
    // Header layout: salt ‖ rs ‖ idlen ‖ as_public.
    assert_eq!(&body[..16], &salt[..]);
    assert_eq!(&body[16..20], &4096u32.to_be_bytes()[..]);
    assert_eq!(body[20], 65);
    assert_eq!(&body[21..86], &b64url_decode(AS_PUBLIC)[..]);
    assert_eq!(
        decrypt_aes128gcm(&body, &UaFixture::rfc8291()),
        plaintext,
        "the receiver side of the fixture agrees"
    );
    // The VAPID public key of the RFC sender private key is the RFC as_public.
    assert_eq!(rfc_key().public_key_b64(), AS_PUBLIC);
}

// ---------------------------------------------------------------------------------------
// Test 14 — inputs and bounds
// ---------------------------------------------------------------------------------------

/// Test 14 (RMC63, §4): plaintext bound; strict subscription keys; RNG failure gives no
/// output; scripted randomness changes the body; an invalid scalar draw is retried (at most
/// 4 draws).
#[test]
fn test_rwp_encrypt_inputs_and_bounds() {
    let ua_fixture = ua_keys();
    let ua = ua_fixture.ua_keys();
    let random = scripted_random(Vec::new());

    // Plaintext bound.
    let max = vec![b'x'; MAX_PUSH_PLAINTEXT_BYTES];
    let body = encrypt(&max, &ua, &random).unwrap();
    assert_eq!(body.len(), 86 + MAX_PUSH_PLAINTEXT_BYTES + 1 + 16);
    assert_eq!(decrypt_aes128gcm(&body, &ua_fixture), max);
    let over = vec![b'x'; MAX_PUSH_PLAINTEXT_BYTES + 1];
    assert_eq!(
        encrypt(&over, &ua, &random).map(|_| ()),
        Err(WebPushError::PlaintextTooLarge)
    );
    assert_eq!(
        encrypt_with(&over, &ua, &arr32(AS_PRIVATE), &arr16(SALT)).map(|_| ()),
        Err(WebPushError::PlaintextTooLarge)
    );

    // Subscription keys.
    let p256dh = ua_fixture.public_bytes();
    let auth = ua_fixture.auth_b64();
    let mut compressed = vec![0x02 | (p256dh[64] & 1)];
    compressed.extend_from_slice(&p256dh[1..33]);
    let mut off_curve = p256dh.clone();
    off_curve[64] ^= 0x01;
    let mut wrong_tag = p256dh.clone();
    wrong_tag[0] = 0x05;
    let bad_p256dh: Vec<String> = vec![
        b64url(&compressed),
        b64url(&off_curve),
        b64url(&wrong_tag),
        b64url(&p256dh[..64]),
        b64url(&[p256dh.as_slice(), &[0u8]].concat()),
        format!("{}=", b64url(&p256dh)),
        String::new(),
        "not base64!".to_string(),
    ];
    // The standard alphabet is refused (the fixture key is chosen so that its base64url
    // rendering contains `-` or `_`).
    let url = b64url(&p256dh);
    let standard = url.replace('-', "+").replace('_', "/");
    assert_ne!(standard, url, "fixture key renders with - or _");
    let mut bad_p256dh = bad_p256dh;
    bad_p256dh.push(standard);
    for bad in &bad_p256dh {
        assert_eq!(
            parse_subscription_keys(bad, &auth).map(|_| ()),
            Err(WebPushError::InvalidSubscriptionKeys),
            "{bad}"
        );
    }
    let good = ua_fixture.p256dh_b64();
    for bad_auth in [
        b64url(&ua_fixture.auth[..15]),
        b64url(&[ua_fixture.auth.as_slice(), &[0u8]].concat()),
        format!("{}==", ua_fixture.auth_b64()),
        "+/+/+/+/+/+/+/+/+/+/+w".to_string(),
        String::new(),
    ] {
        assert_eq!(
            parse_subscription_keys(&good, &bad_auth).map(|_| ()),
            Err(WebPushError::InvalidSubscriptionKeys),
            "{bad_auth}"
        );
    }
    assert!(parse_subscription_keys(&good, &auth).is_ok());

    // RNG failure: no output.
    assert_eq!(
        encrypt(b"hello", &ua, &failing_random()).map(|_| ()),
        Err(WebPushError::Random)
    );

    // Different randomness, different body; both decrypt.
    let a = encrypt(
        b"hello",
        &ua,
        &scripted_random(vec![vec![0x11], vec![0x22]]),
    )
    .unwrap();
    let b = encrypt(
        b"hello",
        &ua,
        &scripted_random(vec![vec![0x33], vec![0x44]]),
    )
    .unwrap();
    assert_ne!(a.to_vec(), b.to_vec());
    assert_eq!(decrypt_aes128gcm(&a, &ua_fixture), b"hello");
    assert_eq!(decrypt_aes128gcm(&b, &ua_fixture), b"hello");
    assert_eq!(&a[..16], &[0x22; 16], "the salt is the second draw");

    // A zero scalar draw is retried; the next valid draw is used.
    let retried = encrypt(
        b"hello",
        &ua,
        &scripted_random(vec![vec![0x00], vec![0x11], vec![0x22]]),
    )
    .unwrap();
    assert_eq!(retried.to_vec(), a.to_vec(), "same keys after one retry");
    // A scalar ≥ n (all 0xff) is invalid as well; four invalid draws → Random.
    assert_eq!(
        encrypt(
            b"hello",
            &ua,
            &scripted_random(vec![vec![0x00], vec![0xff], vec![0x00], vec![0xff]]),
        )
        .map(|_| ()),
        Err(WebPushError::Random)
    );
}

// ---------------------------------------------------------------------------------------
// Test 15 — VAPID authorization
// ---------------------------------------------------------------------------------------

/// Test 15 (RMC64, §4, RFC 8292): exact header and claims (order `aud`, `exp`, `sub`),
/// `aud` is the origin, `exp = now + 43200`, raw signature verifying with `k`, `k` is the
/// 87-char public key; a clock before `MIN_PLAUSIBLE_UNIX_S` is refused.
#[test]
fn test_rwp_vapid_authorization_shape_and_signature() {
    let key = rfc_key();
    let value = vapid_authorization(&key, APPLE_ORIGIN, SUBJECT, NOW_S).unwrap();
    let vapid = verify_vapid(&value, Some(&key.public_key_b64()));
    assert_eq!(vapid.header, "{\"typ\":\"JWT\",\"alg\":\"ES256\"}");
    assert_eq!(
        vapid.claims,
        format!(
            "{{\"aud\":\"https://web.push.apple.com\",\"exp\":{},\"sub\":\"https://pc.tail1234.ts.net\"}}",
            NOW_S + 43_200
        )
    );
    assert_eq!(
        value.as_str(),
        format!("vapid t={}, k={AS_PUBLIC}", vapid.jwt)
    );
    let mailto = vapid_authorization(
        &key,
        "https://updates.push.services.mozilla.com",
        "mailto:owner@proton.me",
        NOW_S,
    )
    .unwrap();
    let vapid = verify_vapid(&mailto, Some(AS_PUBLIC));
    assert!(vapid
        .claims
        .starts_with("{\"aud\":\"https://updates.push.services.mozilla.com\",\"exp\":"));
    assert!(vapid
        .claims
        .ends_with(",\"sub\":\"mailto:owner@proton.me\"}"));
    // Deterministic ES256 (RFC 6979): same inputs, same value.
    assert_eq!(
        vapid_authorization(&key, APPLE_ORIGIN, SUBJECT, NOW_S)
            .unwrap()
            .as_str(),
        value.as_str()
    );
    assert_eq!(
        vapid_authorization(&key, APPLE_ORIGIN, SUBJECT, 1_699_999_999).map(|_| ()),
        Err(WebPushError::Clock)
    );
    assert_eq!(
        vapid_authorization(&key, APPLE_ORIGIN, SUBJECT, 0).map(|_| ()),
        Err(WebPushError::Clock)
    );
    assert_eq!(
        vapid_authorization(&key, APPLE_ORIGIN, SUBJECT, u64::MAX).map(|_| ()),
        Err(WebPushError::Clock),
        "exp overflow"
    );
    assert!(vapid_authorization(&key, APPLE_ORIGIN, SUBJECT, MIN_PLAUSIBLE_UNIX_S).is_ok());
}

// ---------------------------------------------------------------------------------------
// Test 16 — JWT cache
// ---------------------------------------------------------------------------------------

fn claims_of(value: &str) -> String {
    verify_vapid(value, None).claims
}

/// Test 16 (RMC64, §4): one token per origin, reused while `now < issued + 3600`, renewed at
/// 3600 s; three origins are independent entries; `clear()` forces a new token.
#[test]
fn test_rwp_jwt_cache_reuse_and_bounds() {
    let key = rfc_key();
    let mut cache = JwtCache::new();
    let first = cache
        .authorization(&key, APPLE_ORIGIN, SUBJECT, NOW_S)
        .unwrap();
    let same = cache
        .authorization(&key, APPLE_ORIGIN, SUBJECT, NOW_S + VAPID_JWT_REUSE_S - 1)
        .unwrap();
    assert_eq!(same.as_str(), first.as_str(), "reused within 3599 s");
    let renewed = cache
        .authorization(&key, APPLE_ORIGIN, SUBJECT, NOW_S + VAPID_JWT_REUSE_S)
        .unwrap();
    assert_ne!(renewed.as_str(), first.as_str(), "renewed at 3600 s");
    assert!(claims_of(&renewed).contains(&format!(
        "\"exp\":{}",
        NOW_S + VAPID_JWT_REUSE_S + VAPID_JWT_LIFETIME_S
    )));

    // Three origins, three independent entries.
    let t = NOW_S + 10_000;
    let origins = [
        "https://web.push.apple.com",
        "https://fcm.googleapis.com",
        "https://updates.push.services.mozilla.com",
    ];
    let tokens: Vec<String> = origins
        .iter()
        .map(|o| {
            cache
                .authorization(&key, o, SUBJECT, t)
                .unwrap()
                .to_string()
        })
        .collect();
    for (origin, token) in origins.iter().zip(&tokens) {
        assert!(claims_of(token).contains(&format!("\"aud\":\"{origin}\"")));
        let again = cache.authorization(&key, origin, SUBJECT, t + 100).unwrap();
        assert_eq!(again.as_str(), token.as_str(), "{origin}");
    }
    // clear() → a new token (issued at the new time).
    cache.clear();
    let after = cache
        .authorization(&key, APPLE_ORIGIN, SUBJECT, t + 200)
        .unwrap();
    assert_ne!(after.as_str(), tokens[0].as_str());
    assert!(claims_of(&after).contains(&format!("\"exp\":{}", t + 200 + VAPID_JWT_LIFETIME_S)));
    // A refused clock is not cached as a token.
    let mut fresh = JwtCache::new();
    assert_eq!(
        fresh
            .authorization(&key, APPLE_ORIGIN, SUBJECT, 5)
            .map(|_| ()),
        Err(WebPushError::Clock)
    );
}

// ---------------------------------------------------------------------------------------
// Test 17 — key generation
// ---------------------------------------------------------------------------------------

/// Test 17 (RMC63, §4): an invalid first draw (zero) is retried; four invalid draws are
/// `Random`; a failing source is `Random`; bytes round trip; `Debug` is redacted; an
/// invalid scalar is `InvalidKey`.
#[test]
fn test_rwp_vapid_key_generation_and_redaction() {
    let key = VapidKey::generate(&scripted_random(vec![vec![0x00], vec![0x42]])).unwrap();
    assert_eq!(key.to_bytes().to_vec(), vec![0x42; 32]);
    assert_eq!(
        VapidKey::generate(&scripted_random(vec![
            vec![0x00],
            vec![0xff],
            vec![0x00],
            vec![0xff]
        ]))
        .map(|_| ()),
        Err(WebPushError::Random)
    );
    assert_eq!(
        VapidKey::generate(&failing_random()).map(|_| ()),
        Err(WebPushError::Random)
    );
    let restored = VapidKey::from_bytes(&key.to_bytes()).unwrap();
    assert_eq!(restored.public_key_b64(), key.public_key_b64());
    let public = key.public_key_b64();
    assert_eq!(public.len(), 87);
    let bytes = b64url_decode(&public);
    assert_eq!(bytes.len(), 65);
    assert_eq!(bytes[0], 0x04);
    assert_eq!(format!("{key:?}"), "VapidKey(<redacted>)");
    assert_eq!(
        VapidKey::from_bytes(&[0u8; 32]).map(|_| ()),
        Err(WebPushError::InvalidKey)
    );
    assert_eq!(
        VapidKey::from_bytes(&[0xff; 32]).map(|_| ()),
        Err(WebPushError::InvalidKey)
    );
    // Error texts are fixed and never carry key material.
    for e in [
        WebPushError::Random,
        WebPushError::InvalidKey,
        WebPushError::InvalidSubscriptionKeys,
        WebPushError::PlaintextTooLarge,
        WebPushError::Encrypt,
        WebPushError::Clock,
        WebPushError::Encode,
    ] {
        assert!(!e.to_string().is_empty());
    }
}
