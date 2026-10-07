//! Contract tests of the in-memory challenge and web-session stores of `soos-remote` (ADR
//! 2026-10-06 "Tailscale Funnel Access and In-House Passkey Authentication for
//! `soos-remote`", architect spec §4.2, §4.3, tests 23–26, matrix RMC32 / RMC33).
//!
//! Pure: every time is a `tokio::time::Instant` built by adding durations to one origin, so
//! the bounds are exact without any clock.

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

use std::time::Duration;

use tokio::time::Instant;

use passkey::{b64url, sha256};
use soos_remote::challenge::{
    ChallengeBinding, ChallengeError, ChallengePurpose, ChallengeStore, PendingRegistration, Taken,
};
use soos_remote::identity::{client_hint, ClientHint, PathClass};
use soos_remote::websession::{
    clear_cookie_header, session_token_hash, set_cookie_header, Touch, WebSessionError,
    WebSessionStore,
};
use soos_remote::{
    CHALLENGE_BYTES, CHALLENGE_TTL_MS, MAX_LOGIN_CHALLENGES_PER_HINT,
    MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES, MAX_PENDING_CHALLENGES, MAX_WEB_SESSIONS,
    SESSION_TOKEN_BYTES, USER_HANDLE_BYTES, WEB_SESSION_ABSOLUTE_MS, WEB_SESSION_IDLE_MS,
};

use ChallengePurpose::{Login, Register, Unlock};
use PathClass::{Funnel, Tailnet};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn bytes(n: u32) -> [u8; CHALLENGE_BYTES] {
    let mut out = [0u8; CHALLENGE_BYTES];
    out[..4].copy_from_slice(&n.to_be_bytes());
    out[31] = 0xa5;
    out
}

fn hint(i: u32) -> ClientHint {
    let ip = format!("198.51.{}.{}", i / 250, i % 250 + 1);
    let h = client_hint(&[("x-forwarded-for", ip.as_bytes())]);
    assert_ne!(h, ClientHint::UNKNOWN, "{ip} is a valid hint");
    h
}

fn is_plain(r: Result<Taken, ChallengeError>) -> bool {
    matches!(r, Ok(Taken::Plain))
}

fn err(r: Result<Taken, ChallengeError>) -> Option<ChallengeError> {
    r.err()
}

// ---------------------------------------------------------------------------------------
// Challenges
// ---------------------------------------------------------------------------------------

/// Test 23 (RMC32, S-10): single use (removed on the first attempt whatever the outcome),
/// scoped by class, purpose and binding (the login hint is not a binding), expiry at exactly
/// `CHALLENGE_TTL_MS`; a register challenge carries its pending user handle.
#[test]
fn test_rmc_challenge_store_is_single_use_and_scoped() {
    let t0 = Instant::now();
    let mut store = ChallengeStore::new();
    let none = ChallengeBinding::None;

    // Single use.
    assert_eq!(
        store.issue(t0, Tailnet, Unlock, none.clone(), None, bytes(1)),
        Ok(0)
    );
    assert_eq!(store.pending(Tailnet, Unlock), 1);
    assert!(is_plain(store.take(t0, Tailnet, Unlock, &none, &bytes(1))));
    assert_eq!(store.pending(Tailnet, Unlock), 0);
    assert_eq!(
        err(store.take(t0, Tailnet, Unlock, &none, &bytes(1))),
        Some(ChallengeError::Unknown)
    );

    // Unknown bytes leave the other entries alone.
    store
        .issue(t0, Tailnet, Unlock, none.clone(), None, bytes(2))
        .unwrap();
    assert_eq!(
        err(store.take(t0, Tailnet, Unlock, &none, &bytes(99))),
        Some(ChallengeError::Unknown)
    );
    assert!(is_plain(store.take(t0, Tailnet, Unlock, &none, &bytes(2))));

    // Wrong purpose, class or binding: Mismatch, and the entry is gone.
    let wrong: Vec<(PathClass, ChallengePurpose, ChallengeBinding)> = vec![
        (Tailnet, Login, none.clone()),
        (Tailnet, Register, none.clone()),
        (Funnel, Unlock, none.clone()),
        (Tailnet, Unlock, ChallengeBinding::WebSession([1; 32])),
        (Tailnet, Unlock, ChallengeBinding::EnrollCode([1; 32])),
    ];
    for (i, (class, purpose, binding)) in wrong.into_iter().enumerate() {
        let b = bytes(10 + i as u32);
        store
            .issue(t0, Tailnet, Unlock, none.clone(), None, b)
            .unwrap();
        assert_eq!(
            err(store.take(t0, class, purpose, &binding, &b)),
            Some(ChallengeError::Mismatch),
            "case {i}"
        );
        assert_eq!(
            err(store.take(t0, Tailnet, Unlock, &none, &b)),
            Some(ChallengeError::Unknown),
            "case {i}: removed on the first attempt"
        );
    }
    let session_a = ChallengeBinding::WebSession(sha256(b"token a"));
    let session_b = ChallengeBinding::WebSession(sha256(b"token b"));
    store
        .issue(t0, Funnel, Unlock, session_a.clone(), None, bytes(20))
        .unwrap();
    assert_eq!(
        err(store.take(t0, Funnel, Unlock, &session_b, &bytes(20))),
        Some(ChallengeError::Mismatch),
        "a Funnel unlock challenge is bound to its web session"
    );
    store
        .issue(t0, Funnel, Unlock, session_a.clone(), None, bytes(21))
        .unwrap();
    assert!(is_plain(store.take(
        t0,
        Funnel,
        Unlock,
        &session_a,
        &bytes(21)
    )));

    // Expiry at exactly CHALLENGE_TTL_MS.
    store
        .issue(t0, Tailnet, Unlock, none.clone(), None, bytes(30))
        .unwrap();
    store
        .issue(t0, Tailnet, Unlock, none.clone(), None, bytes(31))
        .unwrap();
    assert!(is_plain(store.take(
        t0 + ms(CHALLENGE_TTL_MS - 1),
        Tailnet,
        Unlock,
        &none,
        &bytes(30)
    )));
    assert_eq!(
        err(store.take(
            t0 + ms(CHALLENGE_TTL_MS),
            Tailnet,
            Unlock,
            &none,
            &bytes(31)
        )),
        Some(ChallengeError::Expired)
    );
    assert_eq!(
        err(store.take(
            t0 + ms(CHALLENGE_TTL_MS),
            Tailnet,
            Unlock,
            &none,
            &bytes(31)
        )),
        Some(ChallengeError::Unknown),
        "an expired entry is removed"
    );

    // Register challenges carry the pending user handle (F-1).
    let code = ChallengeBinding::EnrollCode(sha256(b"ABCDE12345"));
    let handle = [0x77; USER_HANDLE_BYTES];
    store
        .issue(
            t0,
            Tailnet,
            Register,
            code.clone(),
            Some(PendingRegistration {
                user_handle: handle,
            }),
            bytes(40),
        )
        .unwrap();
    match store.take(t0, Tailnet, Register, &code, &bytes(40)) {
        Ok(Taken::Register(pending)) => assert_eq!(pending.user_handle, handle),
        Ok(Taken::Plain) => panic!("a register challenge returns its pending registration"),
        Err(e) => panic!("{e:?}"),
    }
    store
        .issue(
            t0,
            Tailnet,
            Register,
            code.clone(),
            Some(PendingRegistration {
                user_handle: handle,
            }),
            bytes(41),
        )
        .unwrap();
    assert_eq!(
        err(store.take(
            t0,
            Tailnet,
            Register,
            &ChallengeBinding::EnrollCode(sha256(b"OTHERCODE1")),
            &bytes(41)
        )),
        Some(ChallengeError::Mismatch),
        "bound to the enrollment code"
    );
    // `pending` is Some exactly for Register.
    assert_eq!(
        store.issue(t0, Tailnet, Register, code.clone(), None, bytes(42)),
        Err(ChallengeError::Mismatch)
    );
    let pending = Some(PendingRegistration {
        user_handle: handle,
    });
    assert_eq!(
        store.issue(
            t0,
            Funnel,
            Login,
            ChallengeBinding::Login(hint(1)),
            pending,
            bytes(43)
        ),
        Err(ChallengeError::Mismatch)
    );
    assert_eq!(
        store.issue(t0, Tailnet, Unlock, none.clone(), pending, bytes(44)),
        Err(ChallengeError::Mismatch)
    );
    assert_eq!(store.pending(Tailnet, Register), 0, "nothing stored");
    assert_eq!(store.pending(Tailnet, Unlock), 0);
    assert_eq!(store.pending(Funnel, Login), 0);

    // The login hint is a pool key, not a binding.
    store
        .issue(
            t0,
            Funnel,
            Login,
            ChallengeBinding::Login(hint(1)),
            None,
            bytes(50),
        )
        .unwrap();
    assert!(is_plain(store.take(
        t0,
        Funnel,
        Login,
        &ChallengeBinding::Login(hint(2)),
        &bytes(50)
    )));
}

/// Test 24 (RMC32): every authenticated pool holds at most `MAX_PENDING_CHALLENGES`, refuses
/// the next one (never evicts), is independent of the other pools, and expiry frees a slot.
#[test]
fn test_rmc_challenge_pools_are_bounded_without_eviction() {
    let t0 = Instant::now();
    let session = ChallengeBinding::WebSession(sha256(b"s"));
    let code = ChallengeBinding::EnrollCode(sha256(b"c"));
    let pending = Some(PendingRegistration {
        user_handle: [1; USER_HANDLE_BYTES],
    });
    let pools: Vec<(
        PathClass,
        ChallengePurpose,
        ChallengeBinding,
        Option<PendingRegistration>,
    )> = vec![
        (Tailnet, Unlock, ChallengeBinding::None, None),
        (Tailnet, Register, code, pending),
        (Funnel, Unlock, session, None),
    ];
    let mut store = ChallengeStore::new();
    for (p, (class, purpose, binding, pending)) in pools.iter().enumerate() {
        for i in 0..MAX_PENDING_CHALLENGES {
            let b = bytes((p * 100 + i) as u32);
            assert_eq!(
                store.issue(t0, *class, *purpose, binding.clone(), *pending, b),
                Ok(0)
            );
        }
        let extra = bytes((p * 100 + 50) as u32);
        assert_eq!(
            store.issue(t0, *class, *purpose, binding.clone(), *pending, extra),
            Err(ChallengeError::PoolFull),
            "pool {p}"
        );
        assert_eq!(store.pending(*class, *purpose), MAX_PENDING_CHALLENGES);
    }
    // Nothing was evicted: every first entry is still valid.
    for (p, (class, purpose, binding, _)) in pools.iter().enumerate() {
        assert!(store
            .take(t0, *class, *purpose, binding, &bytes((p * 100) as u32))
            .is_ok());
    }
    // One slot free again in each pool; full again; then expiry frees everything.
    let (class, purpose, binding, pending) = &pools[0];
    store
        .issue(t0, *class, *purpose, binding.clone(), *pending, bytes(60))
        .unwrap();
    assert_eq!(
        store.issue(t0, *class, *purpose, binding.clone(), *pending, bytes(61)),
        Err(ChallengeError::PoolFull)
    );
    assert_eq!(
        store.issue(
            t0 + ms(CHALLENGE_TTL_MS - 1),
            *class,
            *purpose,
            binding.clone(),
            *pending,
            bytes(62)
        ),
        Err(ChallengeError::PoolFull),
        "nothing expired yet"
    );
    assert_eq!(
        store.issue(
            t0 + ms(CHALLENGE_TTL_MS),
            *class,
            *purpose,
            binding.clone(),
            *pending,
            bytes(63)
        ),
        Ok(0),
        "expired entries are purged first and never counted as evictions"
    );
    assert_eq!(store.pending(*class, *purpose), 1);
}

/// Test 24a (RMC32, F-2): the anonymous login pool never refuses: a hint at
/// `MAX_LOGIN_CHALLENGES_PER_HINT` loses its own oldest entry, a full pool loses its oldest
/// entry; an evicted challenge is `Unknown`.
#[test]
fn test_rmc_anonymous_login_pool_evicts_oldest() {
    let t0 = Instant::now();
    let mut store = ChallengeStore::new();
    let login = |h: u32| ChallengeBinding::Login(hint(h));
    let at = |i: u64| t0 + ms(i);

    // Per hint.
    assert_eq!(
        store.issue(at(0), Funnel, Login, login(1), None, bytes(1)),
        Ok(0)
    );
    assert_eq!(
        store.issue(at(1), Funnel, Login, login(2), None, bytes(2)),
        Ok(0)
    );
    assert_eq!(
        store.issue(at(2), Funnel, Login, login(1), None, bytes(3)),
        Ok(0)
    );
    assert_eq!(
        store.issue(at(3), Funnel, Login, login(1), None, bytes(4)),
        Ok(1),
        "the 3rd from one hint evicts that hint's oldest"
    );
    assert_eq!(
        store.pending_for_hint(hint(1)),
        MAX_LOGIN_CHALLENGES_PER_HINT
    );
    assert_eq!(store.pending_for_hint(hint(2)), 1, "other hints untouched");
    assert_eq!(
        err(store.take(at(4), Funnel, Login, &login(1), &bytes(1))),
        Some(ChallengeError::Unknown)
    );
    for b in [2, 3, 4] {
        assert!(is_plain(store.take(
            at(4),
            Funnel,
            Login,
            &login(1),
            &bytes(b)
        )));
    }
    assert_eq!(store.pending(Funnel, Login), 0);

    // Pool: 16 entries from 8 hints, then a 9th hint evicts the pool's oldest.
    let mut n = 100u32;
    for h in 10..18u32 {
        for _ in 0..2 {
            assert_eq!(
                store.issue(at(u64::from(n)), Funnel, Login, login(h), None, bytes(n)),
                Ok(0)
            );
            n += 1;
        }
    }
    assert_eq!(
        store.pending(Funnel, Login),
        MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES
    );
    assert_eq!(
        store.issue(at(500), Funnel, Login, login(30), None, bytes(500)),
        Ok(1)
    );
    assert_eq!(
        store.pending(Funnel, Login),
        MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES
    );
    assert_eq!(
        err(store.take(at(501), Funnel, Login, &login(10), &bytes(100))),
        Some(ChallengeError::Unknown),
        "the pool's oldest was evicted"
    );
    assert!(is_plain(store.take(
        at(501),
        Funnel,
        Login,
        &login(10),
        &bytes(101)
    )));
    assert!(is_plain(store.take(
        at(501),
        Funnel,
        Login,
        &login(30),
        &bytes(500)
    )));

    // A hint at its cap in a full pool: only its own oldest goes.
    let mut store = ChallengeStore::new();
    let mut n = 1000u32;
    for h in 40..48u32 {
        for _ in 0..2 {
            store
                .issue(at(u64::from(n)), Funnel, Login, login(h), None, bytes(n))
                .unwrap();
            n += 1;
        }
    }
    assert_eq!(
        store.issue(at(2000), Funnel, Login, login(47), None, bytes(2000)),
        Ok(1)
    );
    assert!(
        is_plain(store.take(at(2001), Funnel, Login, &login(40), &bytes(1000))),
        "another hint's oldest entry survives"
    );

    // Never PoolFull, bounds always hold.
    for i in 0..200u32 {
        let result = store.issue(
            at(3000 + u64::from(i)),
            Funnel,
            Login,
            login(i % 13),
            None,
            bytes(5000 + i),
        );
        assert!(matches!(result, Ok(0..=2)), "{result:?}");
        assert!(store.pending(Funnel, Login) <= MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES);
        for h in 0..13 {
            assert!(store.pending_for_hint(hint(h)) <= MAX_LOGIN_CHALLENGES_PER_HINT);
        }
    }

    // Expired entries are purged, not evicted.
    let later = at(3300) + ms(CHALLENGE_TTL_MS);
    assert_eq!(
        store.issue(later, Funnel, Login, login(1), None, bytes(9000)),
        Ok(0)
    );
    assert_eq!(store.pending(Funnel, Login), 1);
}

// ---------------------------------------------------------------------------------------
// Web sessions
// ---------------------------------------------------------------------------------------

fn token(n: u8) -> [u8; SESSION_TOKEN_BYTES] {
    [n; SESSION_TOKEN_BYTES]
}

/// Test 25 (RMC33, F-4): idle expiry at exactly 15 min, absolute at exactly 8 h even when
/// touched, at most `MAX_WEB_SESSIONS` (never evicts), idempotent removal, revocation by
/// credential; `Touch::Keep` never moves the idle time, `Touch::Refresh` does.
#[test]
fn test_rmc_web_sessions_ttl_cap_and_logout() {
    let t0 = Instant::now();
    let cred_a = sha256(b"credential a");
    let cred_b = sha256(b"credential b");
    let mut store = WebSessionStore::new();
    let issued = store.create(t0, token(1), cred_a).unwrap();
    assert_eq!(issued.set_cookie_value(), b64url(&token(1)));
    assert_eq!(issued.set_cookie_value().len(), 43);
    let h1 = sha256(&token(1));
    assert_eq!(store.len(), 1);
    assert_eq!(store.validate(t0, &h1, Touch::Keep), Some(h1));
    assert_eq!(store.validate(t0, &sha256(&token(9)), Touch::Refresh), None);

    // Idle: exactly WEB_SESSION_IDLE_MS after the last refresh.
    assert_eq!(
        store.validate(t0 + ms(WEB_SESSION_IDLE_MS - 1), &h1, Touch::Keep),
        Some(h1)
    );
    assert_eq!(
        store.validate(t0 + ms(WEB_SESSION_IDLE_MS), &h1, Touch::Keep),
        None
    );
    assert_eq!(store.len(), 0, "an expired record is removed");

    // Keep every 15 s never extends the idle time.
    store.create(t0, token(2), cred_a).unwrap();
    let h2 = sha256(&token(2));
    let mut t = 0;
    while t + 15_000 < WEB_SESSION_IDLE_MS {
        t += 15_000;
        assert_eq!(
            store.validate(t0 + ms(t), &h2, Touch::Keep),
            Some(h2),
            "{t}"
        );
    }
    assert_eq!(
        store.validate(t0 + ms(WEB_SESSION_IDLE_MS), &h2, Touch::Keep),
        None,
        "keep-alive validations never refresh"
    );

    // Refresh does extend it; the absolute bound still holds.
    let start = t0 + ms(WEB_SESSION_IDLE_MS);
    store.create(start, token(3), cred_a).unwrap();
    let h3 = sha256(&token(3));
    let step = WEB_SESSION_IDLE_MS - 60_000;
    let mut t = 0;
    while t + step < WEB_SESSION_ABSOLUTE_MS {
        t += step;
        assert_eq!(
            store.validate(start + ms(t), &h3, Touch::Refresh),
            Some(h3),
            "{t}"
        );
    }
    assert_eq!(
        store.validate(start + ms(WEB_SESSION_ABSOLUTE_MS - 1), &h3, Touch::Refresh),
        Some(h3)
    );
    assert_eq!(
        store.validate(start + ms(WEB_SESSION_ABSOLUTE_MS), &h3, Touch::Refresh),
        None,
        "absolute lifetime, however often touched"
    );

    // Cap: never evicts; expiry or removal frees a slot.
    let t1 = start + ms(WEB_SESSION_ABSOLUTE_MS);
    let mut store = WebSessionStore::new();
    for n in 10..10 + MAX_WEB_SESSIONS as u8 {
        store.create(t1, token(n), cred_a).unwrap();
    }
    assert!(matches!(
        store.create(t1, token(50), cred_a),
        Err(WebSessionError::TooMany)
    ));
    assert_eq!(store.len(), MAX_WEB_SESSIONS);
    for n in 10..10 + MAX_WEB_SESSIONS as u8 {
        assert!(store
            .validate(t1, &sha256(&token(n)), Touch::Keep)
            .is_some());
    }
    store.remove(&sha256(&token(10)));
    store.remove(&sha256(&token(10)));
    store.remove(&sha256(&token(99)));
    assert_eq!(store.len(), MAX_WEB_SESSIONS - 1);
    assert_eq!(
        store.validate(t1, &sha256(&token(10)), Touch::Refresh),
        None
    );
    store.create(t1, token(51), cred_b).unwrap();
    assert!(matches!(
        store.create(t1, token(52), cred_b),
        Err(WebSessionError::TooMany)
    ));
    assert!(
        store
            .create(t1 + ms(WEB_SESSION_IDLE_MS), token(53), cred_b)
            .is_ok(),
        "expired records are purged before the cap check"
    );

    // Revocation by credential.
    let mut store = WebSessionStore::new();
    store.create(t1, token(60), cred_a).unwrap();
    store.create(t1, token(61), cred_b).unwrap();
    store.retain_credentials(Some(&[cred_b]));
    assert_eq!(store.validate(t1, &sha256(&token(60)), Touch::Keep), None);
    assert!(store
        .validate(t1, &sha256(&token(61)), Touch::Keep)
        .is_some());
    store.retain_credentials(Some(&[cred_a, cred_b]));
    assert_eq!(store.len(), 1);
    store.retain_credentials(None);
    assert_eq!(store.len(), 0, "an unreadable store drops every session");
    assert_eq!(
        WebSessionError::TooMany.to_string(),
        "too many web sessions"
    );
}

/// Test 26 (RMC33): cookie parsing over every `cookie` header (exactly one pair with the
/// exact name, a 43-char base64url value of 32 bytes) and the exact `Set-Cookie` values.
#[test]
fn test_rmc_session_cookie_parsing() {
    let t = token(0x42);
    let value = b64url(&t);
    assert_eq!(value.len(), 43);
    let expected = Some(sha256(&t));
    let pair = format!("__Host-soos_session={value}");
    assert_eq!(session_token_hash(&[("cookie", pair.as_bytes())]), expected);
    let mixed = format!("a=b; {pair}; c=d");
    assert_eq!(
        session_token_hash(&[("cookie", mixed.as_bytes())]),
        expected
    );
    let ows = format!("a=b;   {pair}  ;c=d");
    assert_eq!(session_token_hash(&[("cookie", ows.as_bytes())]), expected);
    assert_eq!(
        session_token_hash(&[
            ("cookie", b"a=b"),
            ("host", b"pc.tail1234.ts.net"),
            ("cookie", pair.as_bytes())
        ]),
        expected,
        "HTTP/2 cookie crumbs re-joined as several headers"
    );
    assert_eq!(
        session_token_hash(&[("Cookie", pair.as_bytes())]),
        expected,
        "header name is case-insensitive"
    );

    let other = format!("__Host-soos_session={}", b64url(&token(0x43)));
    let refused: Vec<Vec<(&str, Vec<u8>)>> = vec![
        vec![],
        vec![("cookie", b"a=b".to_vec())],
        vec![("cookie", format!("{pair}; {pair}").into_bytes())],
        vec![
            ("cookie", pair.clone().into_bytes()),
            ("cookie", other.clone().into_bytes()),
        ],
        vec![("cookie", format!("{pair}; {other}").into_bytes())],
        vec![(
            "cookie",
            format!("__Host-soos_session={}", &value[..42]).into_bytes(),
        )],
        vec![(
            "cookie",
            format!("__Host-soos_session={value}A").into_bytes(),
        )],
        vec![(
            "cookie",
            format!("__Host-soos_session={value}=").into_bytes(),
        )],
        vec![(
            "cookie",
            format!("__Host-soos_session={}+", &value[..42]).into_bytes(),
        )],
        vec![(
            "cookie",
            format!("__Host-soos_session={}/", &value[..42]).into_bytes(),
        )],
        vec![(
            "cookie",
            format!("__host-soos_session={value}").into_bytes(),
        )],
        vec![("cookie", format!("soos_session={value}").into_bytes())],
        vec![(
            "cookie",
            format!("__Secure-soos_session={value}").into_bytes(),
        )],
        vec![("cookie", b"__Host-soos_session=".to_vec())],
        vec![("set-cookie", pair.clone().into_bytes())],
        vec![("cookie", vec![0xff, 0xfe])],
    ];
    for headers in refused {
        let borrowed: Vec<(&str, &[u8])> =
            headers.iter().map(|(n, v)| (*n, v.as_slice())).collect();
        assert_eq!(session_token_hash(&borrowed), None, "{headers:?}");
    }

    let mut store = WebSessionStore::new();
    let issued = store.create(Instant::now(), t, sha256(b"c")).unwrap();
    assert_eq!(
        set_cookie_header(&issued),
        format!(
            "__Host-soos_session={value}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=28800"
        )
    );
    assert_eq!(
        clear_cookie_header(),
        "__Host-soos_session=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"
    );
}

// ---------------------------------------------------------------------------------------
// Auditor addition A-T1 (constraint C14, D-H)
// ---------------------------------------------------------------------------------------

/// A-T1 (C14, D-H): every secret-bearing type with a `Debug` implementation prints
/// `<redacted>` and none of its bytes (decimal list, hex or base64url).
#[test]
fn test_rmc_secret_types_debug_is_redacted() {
    use soos_remote::auth::{AuthState, LimitKey, RegisterOptionsBody};
    use soos_remote::credentials::{CredentialStore, PasskeyFile, PasskeyRecord};
    use soos_remote::enroll::{CodeFile, EnrollCode};

    let secret = [0x5c_u8; 32];
    let decimal = format!("{:?}", &secret[..4])
        .trim_end_matches(']')
        .to_string();
    let needles = [
        decimal.clone(),
        "92, 92".to_string(),
        passkey::to_hex(&secret[..4]),
        b64url(&secret[..6]),
    ];
    let check = |shown: String| {
        assert!(shown.contains("<redacted>"), "{shown}");
        for needle in &needles {
            assert!(!shown.contains(needle.as_str()), "{shown} leaks {needle}");
        }
    };

    check(format!("{:?}", ChallengeBinding::WebSession(secret)));
    check(format!("{:?}", ChallengeBinding::EnrollCode(secret)));
    let pending = PendingRegistration {
        user_handle: [0x5c; USER_HANDLE_BYTES],
    };
    check(format!("{pending:?}"));
    check(format!("{:?}", Taken::Register(pending)));
    let mut store = ChallengeStore::new();
    store
        .issue(
            Instant::now(),
            Tailnet,
            Unlock,
            ChallengeBinding::None,
            None,
            secret,
        )
        .unwrap();
    check(format!("{store:?}"));

    let mut sessions = WebSessionStore::new();
    let issued = sessions
        .create(Instant::now(), [0x5c; SESSION_TOKEN_BYTES], secret)
        .unwrap();
    check(format!("{issued:?}"));
    check(format!("{sessions:?}"));

    let record = PasskeyRecord {
        credential_id: secret.to_vec(),
        public_key: [0x5c; 65],
        sign_count: 0,
        backup_eligible: true,
        backup_state: true,
        created_unix_s: 1,
    };
    check(format!("{record:?}"));
    let file = PasskeyFile {
        user_handle: [0x5c; USER_HANDLE_BYTES],
        passkeys: vec![record],
    };
    check(format!("{file:?}"));
    check(format!(
        "{:?}",
        CredentialStore::new(std::path::PathBuf::from("/nonexistent/p.json"), 1)
    ));

    let filler: soos_remote::auth::RandomSource = std::sync::Arc::new(
        |buf: &mut [u8]| -> Result<(), soos_remote::auth::RandomError> {
            buf.fill(0x5c);
            Ok(())
        },
    );
    let code = EnrollCode::generate(&filler).unwrap();
    let shown_code = code.display();
    let shown = format!("{code:?}");
    assert!(shown.contains("<redacted>") && !shown.contains(&shown_code[..5]));
    check(format!(
        "{:?}",
        CodeFile {
            code_hash: secret,
            expires_unix_s: 1
        }
    ));

    let hint = client_hint(&[("x-forwarded-for", b"203.0.113.92")]);
    let key = format!("{:?}", LimitKey::FunnelAnonymous(hint));
    assert!(key.contains("<redacted>") && !key.contains("203"), "{key}");
    check(format!("{:?}", AuthState::new(None)));
    let body: RegisterOptionsBody = serde_json::from_str("{\"code\":\"ABCDE-12345\"}").unwrap();
    let shown = format!("{body:?}");
    assert!(
        shown.contains("<redacted>") && !shown.contains("ABCDE"),
        "{shown}"
    );
}
