//! Contract tests of the per-class capacity reservation and the anonymous Funnel limits of
//! `soos-remote` (ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey
//! Authentication for `soos-remote`", architect spec S-11, S-12, §3.1, §5.2, §6, tests
//! 54–57 and 59, plus test 54a required by plan-evaluator finding G-1; matrix RMC37 / RMC41).
//!
//! Same deterministic harness as `server_tests.rs` (frozen paused clock; `tests/common`).
//! "At once" means: answered with scheduler rounds only, without moving the clock.

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

use harness::*;
use soos_remote::auth::{Capacity, CapacityError};
use soos_remote::identity::{client_hint, ClientHint};
use soos_remote::{
    BODY_READ_TIMEOUT_MS, MAX_ANONYMOUS_BODY_READS, MAX_ANONYMOUS_FUNNEL_CONNECTIONS,
    MAX_CLIENT_HINTS, MAX_CONNECTIONS, MAX_FUNNEL_CONNECTIONS, MAX_FUNNEL_SSE_STREAMS,
    MIN_UNLOCK_INTERVAL_MS,
};

const IP_A: &str = "203.0.113.10";
const IP_B: &str = "203.0.113.20";
const OWNER_IP: &str = "203.0.113.30";

/// Plan-evaluator G-1: the longest virtual time a refused or capped Funnel connection may
/// keep a global connection permit after its answer. The bound is the crate constant
/// `soos_remote::FUNNEL_REFUSAL_LINGER_MS` (spec, 100 ms); this test copy stays an
/// independent literal so a change of the crate constant is caught here.
const FUNNEL_REFUSAL_LINGER_BOUND_MS: u64 = 100;

fn assert_result(r: &HttpResponse, status: u16, result: &str) {
    assert_eq!(
        (r.status, r.result_or_body()),
        (status, result.to_string()),
        "expected {status} {result}"
    );
    r.assert_mandatory_headers();
}

async fn login_verify_held(h: &Harness, via: &Via, declared: usize) -> Held {
    h.hold(
        via,
        "POST",
        "/api/auth/login/verify",
        &[("X-Soos-Action", "login"), ("Origin", ORIGIN)],
        Some(declared),
    )
    .await
}

fn hint_ip(i: usize) -> String {
    format!("198.18.{}.{}", i / 200, i % 200 + 1)
}

/// Test 54 (RMC41, S-11): with every Funnel slot held (2 session streams, 2 anonymous
/// login bodies, 4 session unlock bodies), the 9th Funnel request is `503 busy` at once
/// while the tailnet still gets status and lock; the body deadline releases the slots;
/// Funnel streams are capped at `MAX_FUNNEL_SSE_STREAMS` while the tailnet keeps its own.
#[tokio::test(start_paused = true)]
async fn test_rmc_tailnet_is_served_while_funnel_slots_are_saturated() {
    assert_eq!(MAX_FUNNEL_CONNECTIONS, 8);
    let h = Harness::start_with(Options::funnel()).await;
    let session = h.session(OWNER_IP).await;
    let mut streams = Vec::new();
    for _ in 0..MAX_FUNNEL_SSE_STREAMS {
        let mut s = h.open_stream_via(&session).await.expect("Funnel stream");
        s.expect_event_now().await;
        streams.push(s);
    }
    let mut held = vec![
        login_verify_held(&h, &Via::funnel(IP_A), 4096).await,
        login_verify_held(&h, &Via::funnel(IP_B), 4096).await,
    ];
    for _ in 0..4 {
        held.push(
            h.hold(
                &session,
                "POST",
                "/api/unlock",
                &[("X-Soos-Action", "unlock")],
                Some(4096),
            )
            .await,
        );
    }
    for request in &mut held {
        assert!(
            request.response_now().await.is_none(),
            "waiting for its body"
        );
    }

    let r = h.get_via(&Via::funnel("203.0.113.99"), "/").await;
    assert_result(&r, 503, "busy");
    let r = h.get_via(&session, "/api/status").await;
    assert_result(&r, 503, "busy");
    assert_eq!(h.get_via(&Via::Tailnet, "/api/status").await.status, 200);
    let lock = h
        .send(
            &Via::Tailnet,
            "POST",
            "/api/lock",
            &[("X-Soos-Action", "lock")],
            None,
        )
        .await;
    assert_result(&lock, 202, "lock_requested");

    h.advance_ms(BODY_READ_TIMEOUT_MS).await;
    for mut request in held {
        assert!(
            request.closed_silently_now().await,
            "a body that never comes closes the connection without a response"
        );
    }
    assert_eq!(
        h.get_via(&Via::funnel(IP_A), "/").await.status,
        200,
        "slots released"
    );

    // Funnel streams are capped; the tailnet keeps its share of MAX_SSE_STREAMS.
    match h.open_stream_via(&session).await {
        Err(Outcome::Response(r)) => assert_eq!(r.status, 503),
        Ok(_) => panic!("a third Funnel stream must be refused"),
        Err(other) => panic!("{other:?}"),
    }
    let mut tailnet = h
        .open_stream_via(&Via::Tailnet)
        .await
        .expect("tailnet stream");
    tailnet.expect_event_now().await;
    drop(streams);
}

/// Test 54a (RMC41, plan-evaluator G-1): Funnel refusals and capped Funnel connections can
/// never keep the global connection permits long enough to starve the tailnet: with 8
/// Funnel connections held by served requests and 8 more refused (`503 busy`, or a declared
/// body on a non-body route) but kept open by the client, a tailnet status request is
/// served within `FUNNEL_REFUSAL_LINGER_BOUND_MS` of virtual time.
#[tokio::test(start_paused = true)]
async fn test_rmc_funnel_refusals_never_starve_the_tailnet_of_connections() {
    assert_eq!(MAX_CONNECTIONS, 16);
    let h = Harness::start_with(Options::funnel()).await;
    let session = h.session(OWNER_IP).await;
    let mut served = Vec::new();
    for _ in 0..MAX_FUNNEL_CONNECTIONS {
        let mut c = h.hold(&session, "GET", "/", &[], None).await;
        let r = c.response_now().await.expect("served at once");
        assert_eq!(r.status, 200);
        served.push(c);
    }
    let mut refused = Vec::new();
    for i in 0..MAX_CONNECTIONS - MAX_FUNNEL_CONNECTIONS {
        let mut c = h
            .hold(
                &Via::funnel(&format!("203.0.113.{}", 100 + i)),
                "GET",
                "/",
                &[],
                None,
            )
            .await;
        let r = c.response_now().await.expect("refused at once");
        assert_result(&r, 503, "busy");
        refused.push(c);
    }
    h.advance_ms(FUNNEL_REFUSAL_LINGER_BOUND_MS).await;
    let r = h.get_via(&Via::Tailnet, "/api/status").await;
    assert_eq!(
        r.status, 200,
        "the tailnet is served while Funnel clients hold sockets"
    );

    // Pre-classification refusals (a declared body on a non-body route).
    let mut early = Vec::new();
    for i in 0..MAX_CONNECTIONS - MAX_FUNNEL_CONNECTIONS {
        let ip = format!("203.0.113.{}", 150 + i);
        let mut c = h
            .hold(
                &Via::funnel(&ip),
                "GET",
                "/",
                &[("Content-Length", "4096")],
                None,
            )
            .await;
        let r = c.response_now().await.expect("refused at once");
        assert!(r.status == 413 || r.status == 503, "{}", r.status);
        early.push(c);
    }
    h.advance_ms(FUNNEL_REFUSAL_LINGER_BOUND_MS).await;
    let r = h.get_via(&Via::Tailnet, "/api/status").await;
    assert_eq!(r.status, 200);
    drop((served, refused, early));
}

/// Test 55 (RMC41, S-11): anonymous body reads are capped globally and per client hint,
/// refused with `503 busy` before any body byte is read, never counted as failures, while
/// anonymous assets and session holders are still served.
#[tokio::test(start_paused = true)]
async fn test_rmc_anonymous_funnel_capacity_and_body_reads() {
    assert_eq!(MAX_ANONYMOUS_BODY_READS, 2);
    let h = Harness::start_with(Options::funnel()).await;
    let session = h.session(OWNER_IP).await;
    let a = Via::funnel(IP_A);
    let mut held_a = login_verify_held(&h, &a, 4096).await;
    let mut held_b = login_verify_held(&h, &Via::funnel(IP_B), 4096).await;
    assert!(held_a.response_now().await.is_none());
    assert!(held_b.response_now().await.is_none());

    let mut second_a = login_verify_held(&h, &a, 4096).await;
    let r = second_a
        .response_now()
        .await
        .expect("refused at once, body unread");
    assert_result(&r, 503, "busy");
    let mut third = login_verify_held(&h, &Via::funnel("203.0.113.31"), 4096).await;
    let r = third
        .response_now()
        .await
        .expect("refused at once, body unread");
    assert_result(&r, 503, "busy");
    drop((second_a, third));
    settle().await;

    assert_eq!(
        h.get_via(&Via::funnel("203.0.113.32"), "/").await.status,
        200
    );
    assert_eq!(h.get_via(&session, "/api/status").await.status, 200);

    h.advance_ms(BODY_READ_TIMEOUT_MS).await;
    assert!(held_a.closed_silently_now().await);
    assert!(held_b.closed_silently_now().await);

    // Hint A still needs five counted failures: busy refusals and timeouts never count.
    for _ in 0..4 {
        assert_result(&h.post_login(&a, "{}").await, 400, "bad_request");
    }
    assert_eq!(h.login_options(&a).await.status, 200);
    assert_result(&h.post_login(&a, "{}").await, 400, "bad_request");
    assert_result(&h.login_options(&a).await, 429, "rate_limited");
}

/// Test 55, unit part (RMC41): `Capacity` hands out exactly the specified permits and
/// releases them on drop.
#[test]
#[allow(
    clippy::drop_non_drop,
    reason = "BodyReadGuard releases its reservation on drop (spec §4.6)"
)]
fn test_rmc_capacity_permits_are_bounded() {
    let capacity = Capacity::new();
    let mut anonymous = Vec::new();
    for _ in 0..MAX_ANONYMOUS_FUNNEL_CONNECTIONS {
        anonymous.push(capacity.enter_anonymous().expect("anonymous permit"));
    }
    assert!(matches!(
        capacity.enter_anonymous(),
        Err(CapacityError::Busy)
    ));
    anonymous.pop();
    assert!(
        capacity.enter_anonymous().is_ok(),
        "a dropped permit frees a slot"
    );

    let mut funnel = Vec::new();
    for _ in 0..MAX_FUNNEL_CONNECTIONS {
        funnel.push(capacity.enter_funnel().expect("Funnel permit"));
    }
    assert!(matches!(capacity.enter_funnel(), Err(CapacityError::Busy)));
    drop(funnel);
    assert!(capacity.enter_funnel().is_ok());

    let hint = |ip: &str| client_hint(&[("x-forwarded-for", ip.as_bytes())]);
    let (ha, hb, hc) = (hint(IP_A), hint(IP_B), hint("203.0.113.40"));
    assert_ne!(ha, ClientHint::UNKNOWN);
    let ga = capacity.reserve_anonymous_body_read(ha).expect("A");
    assert!(
        matches!(
            capacity.reserve_anonymous_body_read(ha),
            Err(CapacityError::Busy)
        ),
        "one per hint"
    );
    let gb = capacity.reserve_anonymous_body_read(hb).expect("B");
    assert!(
        matches!(
            capacity.reserve_anonymous_body_read(hc),
            Err(CapacityError::Busy)
        ),
        "MAX_ANONYMOUS_BODY_READS in all"
    );
    let mut in_flight = capacity.hints_in_flight();
    in_flight.sort_by_key(|h| format!("{h:?}"));
    assert_eq!(in_flight.len(), 2);
    assert!(in_flight.contains(&ha) && in_flight.contains(&hb));
    drop(ga);
    assert!(!capacity.hints_in_flight().contains(&ha));
    let gc = capacity
        .reserve_anonymous_body_read(hc)
        .expect("C after A released");
    assert!(
        capacity.reserve_anonymous_body_read(ha).is_err(),
        "total is still 2"
    );
    drop((gb, gc));
    assert!(capacity.hints_in_flight().is_empty());
    assert!(capacity.reserve_anonymous_body_read(ha).is_ok());
    assert_eq!(CapacityError::Busy.to_string(), "capacity reached");
}

/// Test 56 (RMC37, F-2): one hint filling its options window and its login challenges never
/// prevents the owner (another hint) from logging in; the distributed residual: enough
/// fresh hints evict the owner's pending challenge (verify refused, no session).
#[tokio::test(start_paused = true)]
async fn test_rmc_owner_login_survives_a_single_hint_filling_the_login_pool() {
    let h = Harness::start_with(Options::funnel()).await;
    let attacker = Via::funnel(IP_A);
    for _ in 0..10 {
        assert_eq!(h.login_options(&attacker).await.status, 200);
    }
    let owner = h.owner.clone();
    let r = h.login(&Via::funnel(OWNER_IP), &owner).await;
    assert_result(&r, 200, "logged_in");
    assert_result(&h.login_options(&attacker).await, 429, "rate_limited");
    assert_eq!(h.login_options(&Via::funnel(OWNER_IP)).await.status, 200);

    // Residual (i): 16 fresh hints issuing after the owner's options evict it.
    let phone = Via::funnel("203.0.113.70");
    let c = h.challenge_of(&h.login_options(&phone).await);
    for i in 0..16 {
        assert_eq!(h.login_options(&Via::funnel(&hint_ip(i))).await.status, 200);
    }
    let r = h.post_login(&phone, &h.valid_assertion(&owner, &c)).await;
    assert_result(&r, 403, "passkey_rejected");
    assert!(r.header("set-cookie").is_none());
}

/// Test 57 (RMC37, F-2): anonymous garbage locks only its own hint: never another hint, a
/// session holder or the tailnet; a missing or malformed `X-Forwarded-For` shares the
/// `UNKNOWN` bucket.
#[tokio::test(start_paused = true)]
async fn test_rmc_anonymous_failures_never_lock_session_holders_or_other_hints() {
    let h = Harness::start_with(Options::funnel()).await;
    h.source.set_locked();
    let owner = h.owner.clone();
    let session = h.session(OWNER_IP).await;
    let a = Via::funnel(IP_A);
    for _ in 0..5 {
        assert_result(&h.post_login(&a, "{}").await, 400, "bad_request");
    }
    assert_result(&h.login_options(&a).await, 429, "rate_limited");
    assert_eq!(h.login_options(&Via::funnel(IP_B)).await.status, 200);
    assert_result(
        &h.unlock_with(&session, &owner).await,
        202,
        "unlock_requested",
    );
    h.advance_ms(MIN_UNLOCK_INTERVAL_MS + 1).await;
    assert_result(
        &h.unlock_with(&Via::Tailnet, &owner).await,
        202,
        "unlock_requested",
    );

    // UNKNOWN bucket: no header and a malformed header share it.
    let no_header = Via::funnel("");
    for _ in 0..5 {
        assert_result(&h.post_login(&no_header, "{}").await, 400, "bad_request");
    }
    assert_result(&h.login_options(&no_header).await, 429, "rate_limited");
    let listed = Via::funnel("203.0.113.1, 198.51.100.1");
    assert_result(&h.login_options(&listed).await, 429, "rate_limited");
    assert_eq!(h.login_options(&Via::funnel(IP_B)).await.status, 200);
}

/// Test 59 (RMC37, F-2, plan-evaluator G-5): at most `MAX_CLIENT_HINTS` anonymous buckets;
/// the bucket with the oldest window is evicted first (a locked hint forgotten that way is
/// the documented residual), but never a bucket with a body read in flight.
#[tokio::test(start_paused = true)]
async fn test_rmc_client_hint_table_is_bounded() {
    assert_eq!(MAX_CLIENT_HINTS, 64);
    let h = Harness::start_with(Options::funnel()).await;
    let locked = Via::funnel("203.0.113.80");
    for _ in 0..5 {
        assert_result(&h.post_login(&locked, "{}").await, 400, "bad_request");
    }
    assert_result(&h.login_options(&locked).await, 429, "rate_limited");
    for i in 0..MAX_CLIENT_HINTS {
        h.advance_ms(1).await;
        assert_eq!(
            h.login_options(&Via::funnel(&hint_ip(i))).await.status,
            200,
            "{i}"
        );
    }
    assert_eq!(
        h.login_options(&locked).await.status,
        200,
        "the oldest bucket was evicted (residual G-5)"
    );

    let h = Harness::start_with(Options::funnel()).await;
    let busy = Via::funnel("203.0.113.81");
    for _ in 0..4 {
        assert_result(&h.post_login(&busy, "{}").await, 400, "bad_request");
    }
    let mut in_flight = login_verify_held(&h, &busy, 2).await;
    assert!(in_flight.response_now().await.is_none());
    for i in 0..MAX_CLIENT_HINTS {
        h.advance_ms(1).await;
        assert_eq!(
            h.login_options(&Via::funnel(&hint_ip(i))).await.status,
            200,
            "{i}"
        );
    }
    in_flight.send(b"{}").await;
    let r = in_flight.response_now().await.expect("answered");
    assert_result(&r, 400, "bad_request");
    // Contract Migration (candid review 2026-10-06, MAJOR): the explanation was passed as
    // the expected `result` and could never match; the intended result is `rate_limited`.
    // The 429 status check is unchanged and the result check is now exact.
    let r = h.login_options(&busy).await;
    assert_eq!(
        (r.status, r.result_or_body()),
        (429, "rate_limited".to_string()),
        "a bucket with a body read in flight is never evicted"
    );
    r.assert_mandatory_headers();
}

/// A-T3 (C7b, spec §6): anonymous Funnel requests refused with `503 busy` whose sockets the
/// client keeps open hold no Funnel slot: a session holder is still served at once.
#[tokio::test(start_paused = true)]
async fn test_rmc_refused_anonymous_funnel_requests_never_hold_funnel_slots() {
    let h = Harness::start_with(Options::funnel()).await;
    let session = h.session(OWNER_IP).await;
    let mut held = Vec::new();
    for i in 0..MAX_ANONYMOUS_FUNNEL_CONNECTIONS {
        let mut c =
            login_verify_held(&h, &Via::funnel(&format!("203.0.113.{}", 40 + i)), 4096).await;
        // Body reads are capped at MAX_ANONYMOUS_BODY_READS: the others are refused at once
        // but kept open by the client.
        let _ = c.response_now().await;
        held.push(c);
    }
    let mut refused = Vec::new();
    for i in 0..4 {
        let mut c = h
            .hold(
                &Via::funnel(&format!("203.0.113.{}", 60 + i)),
                "GET",
                "/",
                &[],
                None,
            )
            .await;
        if let Some(r) = c.response_now().await {
            assert!(r.status == 503 || r.status == 200, "{}", r.status);
        }
        refused.push(c);
    }
    let r = h.get_via(&session, "/api/status").await;
    assert_eq!(r.status, 200, "a session holder keeps its Funnel slots");
    drop((held, refused));
}
