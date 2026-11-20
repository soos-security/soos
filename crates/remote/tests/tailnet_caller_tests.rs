//! Contract tests of the per-login tailnet limiter keys and challenge pools of `soos-remote`
//! (security review 2026-11-10 batch 2, GitHub #479; finding PTSN-03, GitHub #584; ADR
//! "[2026-11-20] Per-Login Tailnet Limiters, Sign-Out Push Clean-Up and the Tailnet Local
//! Listener Residual"; spec `AI/architect_spec_remote_tailnet_identity.md`; matrix RTI1-RTI3).
//!
//! Before the change every tailnet caller shared one limiter key and one challenge pool, so a
//! second allowed login could lock the owner's tailnet unlock, camera view and enrollment out.
//! The store-level tests are pure (`tokio::time::Instant` built from one origin); the server
//! tests use the deterministic harness of `auth_server_tests.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

#[path = "common/passkey.rs"]
mod passkey;

#[path = "common/harness.rs"]
mod harness;

use tokio::time::Instant;

use harness::*;
use soos_remote::auth::{AuthState, LimitKey};
use soos_remote::challenge::{ChallengeBinding, ChallengeError, ChallengePurpose, ChallengeStore};
use soos_remote::config::TailscaleLogin;
use soos_remote::identity::{PathClass, TailnetCaller};
use soos_remote::{
    AUTH_FAILURE_WINDOW_MS, CHALLENGE_BYTES, ENROLL_CODE_TTL_S, MAX_ALLOWED_LOGINS,
    MAX_AUTH_FAILURES, MAX_PENDING_CHALLENGES, MAX_PENDING_TAILNET_CHALLENGES,
};

use ChallengePurpose::{CameraView, Unlock};
use PathClass::Tailnet;

/// The second allowed login of the server tests (synthetic).
const OTHER: &str = "second@example.com";
const CODE: &str = "ABCDE12345";

fn login(text: &str) -> TailscaleLogin {
    TailscaleLogin::parse(text).unwrap()
}

/// `n` distinct synthetic logins `user<i>@example.com`.
fn logins(n: usize) -> Vec<TailscaleLogin> {
    (0..n)
        .map(|i| login(&format!("user{i}@example.com")))
        .collect()
}

fn bytes(seed: u32) -> [u8; CHALLENGE_BYTES] {
    let mut out = [0u8; CHALLENGE_BYTES];
    for (i, b) in out.iter_mut().enumerate() {
        *b = (seed as u8).wrapping_mul(31).wrapping_add(i as u8);
    }
    out[0] = (seed >> 8) as u8;
    out[1] = seed as u8;
    out
}

fn assert_result(r: &HttpResponse, status: u16, result: &str) {
    assert_eq!(
        (r.status, r.result_or_body()),
        (status, result.to_string()),
        "expected {status} {result}"
    );
    r.assert_mandatory_headers();
}

// ---------------------------------------------------------------------------------------
// RTI1 — the caller identity and the per-caller challenge pools
// ---------------------------------------------------------------------------------------

/// RTI1 (PTSN-03): `TailnetCaller::of` names each allowed login apart (case-insensitive like
/// the allowlist), refuses a login outside the list, and never prints the login.
#[test]
fn test_rti1_tailnet_caller_identifies_each_allowed_login() {
    let allowed = logins(MAX_ALLOWED_LOGINS);
    let callers: Vec<TailnetCaller> = allowed
        .iter()
        .map(|l| TailnetCaller::of(l, &allowed).expect("allowed login"))
        .collect();
    for (i, a) in callers.iter().enumerate() {
        for (j, b) in callers.iter().enumerate() {
            assert_eq!(a == b, i == j, "callers {i} and {j}");
        }
    }
    assert_eq!(
        TailnetCaller::of(&login("USER3@Example.com"), &allowed),
        Some(callers[3]),
        "the login is compared in its lowercased form"
    );
    assert_eq!(
        TailnetCaller::of(&login("stranger@example.com"), &allowed),
        None
    );
    assert_eq!(TailnetCaller::of(&allowed[0], &[]), None);
    let shown = format!("{:?} {:?}", callers[5], LimitKey::Tailnet(callers[5]));
    assert!(!shown.contains("user5"), "redacted: {shown}");
    assert!(shown.contains("<redacted>"), "{shown}");
    let binding = format!("{:?}", ChallengeBinding::Tailnet(callers[5]));
    assert!(!binding.contains("user5") && binding.contains("<redacted>"));
}

/// RTI1 (PTSN-03): the tailnet unlock and camera pools hold `MAX_PENDING_CHALLENGES` entries
/// per caller (the next one of that caller is `PoolFull`, never an eviction) while another
/// caller still gets its own; a challenge is taken only by the caller it was issued to (another
/// caller's take is `Mismatch` and removes it).
#[test]
fn test_rti1_tailnet_challenge_pools_are_per_caller() {
    let allowed = logins(2);
    let a = ChallengeBinding::Tailnet(TailnetCaller::of(&allowed[0], &allowed).unwrap());
    let b = ChallengeBinding::Tailnet(TailnetCaller::of(&allowed[1], &allowed).unwrap());
    let t0 = Instant::now();
    for purpose in [Unlock, CameraView] {
        let mut store = ChallengeStore::new();
        for i in 0..MAX_PENDING_CHALLENGES as u32 {
            assert_eq!(
                store.issue(t0, Tailnet, purpose, a.clone(), None, bytes(i)),
                Ok(0),
                "{purpose:?} caller a, entry {i}"
            );
        }
        assert_eq!(
            store.issue(t0, Tailnet, purpose, a.clone(), None, bytes(100)),
            Err(ChallengeError::PoolFull),
            "{purpose:?}: caller a is full"
        );
        for i in 0..MAX_PENDING_CHALLENGES as u32 {
            assert_eq!(
                store.issue(t0, Tailnet, purpose, b.clone(), None, bytes(200 + i)),
                Ok(0),
                "{purpose:?}: caller b keeps its own pool ({i})"
            );
        }
        assert_eq!(
            store.issue(t0, Tailnet, purpose, b.clone(), None, bytes(300)),
            Err(ChallengeError::PoolFull)
        );
        assert_eq!(store.pending(Tailnet, purpose), 2 * MAX_PENDING_CHALLENGES);
        // Nothing was evicted: caller a's first entry is still valid for caller a only.
        assert_eq!(
            store.take(t0, Tailnet, purpose, &b, &bytes(0)),
            Err(ChallengeError::Mismatch),
            "{purpose:?}: another caller cannot take it"
        );
        assert_eq!(
            store.take(t0, Tailnet, purpose, &a, &bytes(0)),
            Err(ChallengeError::Unknown),
            "the mismatched take removed it"
        );
        assert!(store.take(t0, Tailnet, purpose, &a, &bytes(1)).is_ok());
        assert_eq!(
            store.take(t0, Tailnet, purpose, &ChallengeBinding::None, &bytes(2)),
            Err(ChallengeError::Mismatch),
            "a caller-bound challenge is not an unbound one"
        );
        // Two slots of caller a were freed.
        assert_eq!(
            store.issue(t0, Tailnet, purpose, a.clone(), None, bytes(400)),
            Ok(0)
        );
    }
}

/// RTI1 (PTSN-03): the tailnet pool as a whole is bounded by `MAX_PENDING_TAILNET_CHALLENGES`
/// (every allowed login at its own bound), and the caller binding is refused on the Funnel
/// path and for registration.
#[test]
fn test_rti1_tailnet_pool_total_is_bounded() {
    assert_eq!(
        MAX_PENDING_TAILNET_CHALLENGES,
        MAX_PENDING_CHALLENGES * MAX_ALLOWED_LOGINS
    );
    let allowed = logins(MAX_ALLOWED_LOGINS);
    let t0 = Instant::now();
    let mut store = ChallengeStore::new();
    let mut seed = 0u32;
    for l in &allowed {
        let binding = ChallengeBinding::Tailnet(TailnetCaller::of(l, &allowed).unwrap());
        for _ in 0..MAX_PENDING_CHALLENGES {
            assert_eq!(
                store.issue(t0, Tailnet, Unlock, binding.clone(), None, bytes(seed)),
                Ok(0)
            );
            seed += 1;
        }
    }
    assert_eq!(
        store.pending(Tailnet, Unlock),
        MAX_PENDING_TAILNET_CHALLENGES
    );
    let first = ChallengeBinding::Tailnet(TailnetCaller::of(&allowed[0], &allowed).unwrap());
    assert_eq!(
        store.issue(t0, Tailnet, Unlock, first.clone(), None, bytes(9999)),
        Err(ChallengeError::PoolFull)
    );
    for purpose in [Unlock, CameraView] {
        assert_eq!(
            store.issue(
                t0,
                PathClass::Funnel,
                purpose,
                first.clone(),
                None,
                bytes(9000)
            ),
            Err(ChallengeError::Mismatch),
            "{purpose:?}: no caller binding on the Funnel path"
        );
    }
    assert_eq!(
        store.issue(
            t0,
            Tailnet,
            ChallengePurpose::Register,
            first,
            Some(soos_remote::challenge::PendingRegistration {
                user_handle: [1; soos_remote::USER_HANDLE_BYTES],
            }),
            bytes(9001)
        ),
        Err(ChallengeError::Mismatch),
        "registration stays bound to the enrollment code"
    );
}

// ---------------------------------------------------------------------------------------
// RTI2 — per-caller limiter keys
// ---------------------------------------------------------------------------------------

/// RTI2 (PTSN-03): failures and issuances of one tailnet caller never lock another caller,
/// the Funnel session key or an anonymous hint.
#[test]
fn test_rti2_auth_limiter_keys_are_per_tailnet_caller() {
    let allowed = logins(2);
    let a = LimitKey::Tailnet(TailnetCaller::of(&allowed[0], &allowed).unwrap());
    let b = LimitKey::Tailnet(TailnetCaller::of(&allowed[1], &allowed).unwrap());
    assert_ne!(a, b);
    let t0 = Instant::now();
    let mut auth = AuthState::new(None);
    for _ in 0..MAX_AUTH_FAILURES {
        auth.record_failure(a, t0, &[]);
    }
    assert!(auth.locked_out(a, t0));
    assert!(!auth.locked_out(b, t0), "caller b is not locked out");
    assert!(!auth.locked_out(LimitKey::FunnelSession, t0));
    for _ in 0..soos_remote::MAX_OPTIONS_PER_WINDOW {
        auth.record_issuance(b, t0, &[]);
    }
    assert!(auth.options_exhausted(b, t0));
    assert!(!auth.options_exhausted(a, t0), "caller a keeps its options");
}

/// RTI2 (PTSN-03, the review's regression test): with two allowed logins, five counted
/// failures of login A lock login A only; the owner's login B still gets unlock options,
/// register options and an unlock; login A's lockout ends with its window.
#[tokio::test(start_paused = true)]
async fn test_rti2_one_login_cannot_lock_another_out() {
    let h = Harness::start_with(Options {
        extra_logins: vec![OTHER.to_string()],
        ..Options::passkeys()
    })
    .await;
    let other = Via::TailnetAs(OTHER.to_string());
    for _ in 0..5 {
        assert_result(&h.post_unlock(&other, Some("{}")).await, 400, "bad_request");
    }
    assert_result(&h.unlock_options(&other).await, 429, "rate_limited");
    assert_result(
        &h.post_unlock(&other, Some("{}")).await,
        429,
        "rate_limited",
    );

    // The owner's login is unaffected: unlock options, register options and an unlock.
    assert_eq!(
        h.unlock_options(&Via::Tailnet).await.status,
        200,
        "the owner's tailnet key is not locked"
    );
    h.write_code(CODE, h.now_unix_s() + ENROLL_CODE_TTL_S);
    assert_eq!(
        h.register_options(CODE).await.status,
        200,
        "the owner's enrollment is not locked"
    );
    h.source.set_locked();
    let owner = h.owner.clone();
    assert_result(
        &h.unlock_with(&Via::Tailnet, &owner).await,
        202,
        "unlock_requested",
    );

    // The owner's failures do not lift or extend login A's lockout; it ends with its window.
    h.advance_ms(AUTH_FAILURE_WINDOW_MS).await;
    assert_eq!(h.unlock_options(&other).await.status, 200, "window over");
}

/// RTI2 (PTSN-03): the register routes count on the caller's own key: malformed register
/// bodies of the owner lock the owner (unchanged behaviour), never a second login.
#[tokio::test(start_paused = true)]
async fn test_rti2_register_failures_count_on_the_callers_key() {
    let h = Harness::start_with(Options {
        extra_logins: vec![OTHER.to_string()],
        ..Options::passkeys()
    })
    .await;
    h.write_code(CODE, h.now_unix_s() + ENROLL_CODE_TTL_S);
    for _ in 0..5 {
        assert_result(&h.register_verify("{}").await, 400, "bad_request");
    }
    assert_result(&h.unlock_options(&Via::Tailnet).await, 429, "rate_limited");
    let other = Via::TailnetAs(OTHER.to_string());
    assert_eq!(
        h.unlock_options(&other).await.status,
        200,
        "the other login keeps its key"
    );
}

// ---------------------------------------------------------------------------------------
// RTI3 — per-caller challenge pools through the server
// ---------------------------------------------------------------------------------------

/// RTI3 (PTSN-03): login A filling its unlock challenges never makes the owner's unlock
/// options answer `too_many_challenges`, and a challenge issued to login A is rejected when
/// the owner's login presents it.
#[tokio::test(start_paused = true)]
async fn test_rti3_challenge_pool_of_one_login_is_not_the_owners() {
    let h = Harness::start_with(Options {
        extra_logins: vec![OTHER.to_string()],
        ..Options::passkeys()
    })
    .await;
    let other = Via::TailnetAs(OTHER.to_string());
    let mut issued = Vec::new();
    for _ in 0..MAX_PENDING_CHALLENGES {
        let r = h.unlock_options(&other).await;
        issued.push(h.challenge_of(&r));
    }
    assert_result(&h.unlock_options(&other).await, 429, "too_many_challenges");
    assert_eq!(
        h.unlock_options(&Via::Tailnet).await.status,
        200,
        "the owner's pool is not full"
    );

    // Login A's challenge, signed by the owner's passkey, presented by the owner's login.
    h.source.set_locked();
    let owner = h.owner.clone();
    let body = h.valid_assertion(&owner, &issued[0]);
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some(&body)).await,
        403,
        "passkey_rejected",
    );
    assert!(h.source.unlock_ids().is_empty(), "no unlock");
}
