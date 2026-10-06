//! End-to-end contract tests of Tailscale Funnel access and in-house passkey authentication
//! in `soos-remote::server::serve` (ADR 2026-10-06 "Tailscale Funnel Access and In-House
//! Passkey Authentication for `soos-remote`", architect spec §5, §5.1, §6, tests 32–45,
//! 39a, 41a, 41b; matrix RMC26, RMC27, RMC32–RMC38).
//!
//! Same deterministic harness as `server_tests.rs` (frozen paused clock, scripted logind,
//! raw HTTP/1.1 over the Unix socket in a `TempDir`), extended with the passkey store
//! fixture and the WebAuthn ceremonies (`tests/common/harness.rs`). No network, no logind,
//! no `~/.config`.
//!
//! Plan-evaluator round-2 findings encoded here (the approved spec had not folded them):
//! G-2a register routes on Funnel answer `403 forbidden` before `login_required`; G-2b
//! `POST /api/auth/logout` is reachable on Funnel without a session; G-3 asset requests
//! never refresh the idle time.

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

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use harness::*;
use passkey::*;
use soos_remote::auth::{RandomError, RandomSource};
use soos_remote::{
    AUTH_FAILURE_WINDOW_MS, CHALLENGE_BYTES, ENROLL_CODE_TTL_S, MAX_PASSKEYS, MAX_WEB_SESSIONS,
    MIN_UNLOCK_INTERVAL_MS, OPTIONS_WINDOW_MS, SNAPSHOT_DEADLINE_MS, SSE_KEEPALIVE_MS,
    STORE_LOCK_TIMEOUT_MS, WEB_SESSION_ABSOLUTE_MS, WEB_SESSION_IDLE_MS,
};

const IP_A: &str = "203.0.113.10";
const IP_B: &str = "203.0.113.20";
const IP_C: &str = "203.0.113.30";
const CODE: &str = "ABCDE12345";

fn assert_result(r: &HttpResponse, status: u16, result: &str) {
    assert_eq!(
        (r.status, r.result_or_body()),
        (status, result.to_string()),
        "expected {status} {result}"
    );
    r.assert_mandatory_headers();
}

fn json_keys(r: &HttpResponse) -> Vec<String> {
    let mut keys: Vec<String> = r
        .json()
        .as_object()
        .expect("object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

fn assert_no_logind(h: &Harness) {
    assert_eq!(h.source.reads(), 0, "no logind read");
    assert!(h.source.lock_ids().is_empty(), "no lock call");
    assert!(h.source.unlock_ids().is_empty(), "no unlock call");
}

/// A CSPRNG that fails while the flag is set.
fn switchable_random(failing: Arc<AtomicBool>) -> RandomSource {
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(1));
    Arc::new(move |buf: &mut [u8]| -> Result<(), RandomError> {
        if failing.load(Ordering::SeqCst) {
            return Err(RandomError::Failed);
        }
        let n = counter.fetch_add(1, Ordering::SeqCst);
        let seed = sha256(&n.to_be_bytes());
        for (i, b) in buf.iter_mut().enumerate() {
            *b = seed[i % 32] ^ (i / 32) as u8;
        }
        Ok(())
    })
}

/// Holds the credential store lock (`<path>.lock`) until dropped.
fn hold_store_lock(store: &Path) -> nix::fcntl::Flock<fs::File> {
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(format!("{}.lock", store.display()))
        .unwrap();
    nix::fcntl::Flock::lock(lock, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap()
}

// ---------------------------------------------------------------------------------------
// Test 32–35 — Funnel gate
// ---------------------------------------------------------------------------------------

/// Test 32 (RMC26, S-2): with the default `allow_funnel = false`, a Funnel request is `403
/// forbidden` on every route, exactly as before, and never reaches logind.
#[tokio::test(start_paused = true)]
async fn test_rmc_funnel_disabled_by_default_is_forbidden() {
    for options in [Options::passkeys(), Options::default()] {
        let h = Harness::start_with(options).await;
        h.source.set_locked();
        let via = Via::funnel(IP_A);
        for (method, target, action) in [
            ("GET", "/", ""),
            ("GET", "/app.js", ""),
            ("HEAD", "/", ""),
            ("GET", "/api/status", ""),
            ("GET", "/api/events", ""),
            ("GET", "/api/auth/state", ""),
            ("GET", "/nope", ""),
            ("POST", "/api/lock", "lock"),
            ("POST", "/api/unlock", "unlock"),
            ("POST", "/api/auth/login/options", "login-options"),
            ("POST", "/api/auth/login/verify", "login"),
            ("POST", "/api/auth/logout", "logout"),
            ("POST", "/api/auth/unlock/options", "unlock-options"),
            ("POST", "/api/auth/register/options", "register-options"),
            ("POST", "/api/auth/register/verify", "register"),
        ] {
            let r = h
                .send(
                    &via,
                    method,
                    target,
                    &[("X-Soos-Action", action), ("Origin", ORIGIN)],
                    None,
                )
                .await;
            assert_eq!(r.status, 403, "{method} {target}");
            r.assert_mandatory_headers();
            if method != "HEAD" {
                assert_eq!(r.result(), "forbidden", "{method} {target}");
            }
            assert!(r.header("set-cookie").is_none());
        }
        assert_no_logind(&h);
    }
}

/// Test 33 (RMC27, D-D, G-2): without a session a Funnel caller reaches only the assets,
/// `/api/auth/state` (minimal shape), the login ceremony and logout; status, events, lock,
/// unlock and unlock options are `403 login_required`; registration is `403 forbidden`; a
/// gated body route answers after its head (a declared 4 KiB body is never awaited); no
/// logind read.
#[tokio::test(start_paused = true)]
async fn test_rmc_funnel_public_routes_and_login_required() {
    let h = Harness::start_with(Options::funnel()).await;
    h.source.set_locked();
    let via = Via::funnel(IP_A);

    let r = h.get_via(&via, "/").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("text/html; charset=utf-8"));
    r.assert_mandatory_headers();
    assert_eq!(h.get_via(&via, "/app.js").await.status, 200);
    assert_eq!(h.get_via(&via, "/style.css").await.status, 200);
    assert_eq!(h.send(&via, "HEAD", "/", &[], None).await.status, 200);
    assert_result(&h.get_via(&via, "/nope").await, 404, "not_found");
    let r = h.send(&via, "POST", "/", &[], None).await;
    assert_eq!(r.status, 405);

    let r = h.auth_state(&via).await;
    assert_eq!(r.status, 200);
    r.assert_json_body();
    assert_eq!(
        r.json(),
        serde_json::json!({"mode": "funnel", "authenticated": false}),
        "an anonymous caller learns nothing else"
    );

    for (method, target, action) in [
        ("GET", "/api/status", ""),
        ("GET", "/api/events", ""),
        ("POST", "/api/lock", "lock"),
        ("POST", "/api/unlock", "unlock"),
        ("POST", "/api/auth/unlock/options", "unlock-options"),
    ] {
        let r = h
            .send(
                &via,
                method,
                target,
                &[("X-Soos-Action", action), ("Origin", ORIGIN)],
                None,
            )
            .await;
        assert_result(&r, 403, "login_required");
    }
    for (target, action) in [
        ("/api/auth/register/options", "register-options"),
        ("/api/auth/register/verify", "register"),
    ] {
        let r = h
            .ceremony(&via, target, action, Some("{\"code\":\"x\"}"))
            .await;
        assert_result(&r, 403, "forbidden");
    }

    // Logout without a session is answered and clears the cookie (G-2b).
    let r = h.logout(&via).await;
    assert_result(&r, 200, "logged_out");
    assert_eq!(
        r.header("set-cookie"),
        Some("__Host-soos_session=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0")
    );

    // A gated body route answers after its head: the declared body is never awaited.
    for (target, action, expected) in [
        ("/api/unlock", "unlock", "login_required"),
        ("/api/auth/register/verify", "register", "forbidden"),
        (
            "/api/auth/register/options",
            "register-options",
            "forbidden",
        ),
    ] {
        let mut held = h
            .hold(
                &via,
                "POST",
                target,
                &[("X-Soos-Action", action), ("Origin", ORIGIN)],
                Some(4096),
            )
            .await;
        let r = held
            .response_now()
            .await
            .unwrap_or_else(|| panic!("{target}: answered without reading the body"));
        assert_result(&r, 403, expected);
    }
    assert_no_logind(&h);
}

/// Test 34 (RMC27, S-1, §2.3): a Funnel caller must present exactly `rp_id` as effective
/// host (`421` otherwise, also on port 8443); auth routes require it on every path.
#[tokio::test(start_paused = true)]
async fn test_rmc_funnel_host_must_equal_rp_id() {
    let h = Harness::start_with(Options::funnel()).await;
    let with_host = |host: &str| {
        vec![
            ("Host".to_string(), "localhost".to_string()),
            ("X-Forwarded-Host".to_string(), host.to_string()),
            ("X-Forwarded-Proto".to_string(), "https".to_string()),
            ("Tailscale-Funnel-Request".to_string(), "?1".to_string()),
            ("X-Forwarded-For".to_string(), IP_A.to_string()),
        ]
    };
    for host in [
        "other.tail1234.ts.net",
        "pc.tail1234.ts.net:8443",
        "pc.tail1234.ts.net:10000",
        "pc.other.ts.net",
    ] {
        for target in ["/", "/api/auth/state"] {
            let headers = with_host(host);
            let borrowed: Vec<(&str, &str)> = headers
                .iter()
                .map(|(n, v)| (n.as_str(), v.as_str()))
                .collect();
            let r = h.raw(&raw_request("GET", target, &borrowed)).await;
            assert_result(&r, 421, "misdirected_request");
        }
    }
    for host in ["pc.tail1234.ts.net", "PC.tail1234.ts.net:443"] {
        let headers = with_host(host);
        let borrowed: Vec<(&str, &str)> = headers
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            h.raw(&raw_request("GET", "/", &borrowed)).await.status,
            200,
            "{host}"
        );
    }
    // Tailnet: assets keep the existing host rule, auth routes need rp_id.
    let tailnet_other = [
        ("Host", "other.tail1234.ts.net"),
        ("Tailscale-User-Login", LOGIN),
        ("X-Soos-Action", "unlock-options"),
        ("Origin", ORIGIN),
    ];
    assert_eq!(
        h.raw(&raw_request("GET", "/", &tailnet_other[..2]))
            .await
            .status,
        200,
        "unchanged tailnet host rule"
    );
    let r = h
        .raw(&raw_request(
            "POST",
            "/api/auth/unlock/options",
            &tailnet_other,
        ))
        .await;
    assert_result(&r, 421, "misdirected_request");
    assert_no_logind(&h);
}

/// Test 35 (RMC26, §2.2): an identity next to the Funnel marker is refused; an underscore
/// spelling is not an identity (anonymous Funnel caller).
#[tokio::test(start_paused = true)]
async fn test_rmc_forged_identity_on_funnel_is_refused() {
    let h = Harness::start_with(Options::funnel()).await;
    let via = Via::funnel(IP_A);
    for target in ["/", "/api/status", "/api/auth/state"] {
        let r = h
            .send(
                &via,
                "GET",
                target,
                &[("Tailscale-User-Login", LOGIN)],
                None,
            )
            .await;
        assert_result(&r, 403, "forbidden");
    }
    let r = h
        .send(
            &via,
            "GET",
            "/api/status",
            &[("Tailscale_User_Login", LOGIN)],
            None,
        )
        .await;
    assert_result(&r, 403, "login_required");
    let r = h
        .send(&via, "GET", "/", &[("Tailscale_User_Login", LOGIN)], None)
        .await;
    assert_eq!(r.status, 200);
    // A tailnet caller forging the marker (tailscaled would strip it) is ambiguous.
    let r = h
        .send(
            &Via::Tailnet,
            "GET",
            "/",
            &[("Tailscale-Funnel-Request", "?1")],
            None,
        )
        .await;
    assert_result(&r, 403, "forbidden");
    assert_no_logind(&h);
}

// ---------------------------------------------------------------------------------------
// Tests 36–38 — login and web sessions
// ---------------------------------------------------------------------------------------

/// Test 36 (RMC33, S-3): the full Funnel login: options carry only challenge, rp_id and
/// timeout (no credential id); verify sets the `__Host-` cookie with the exact attributes;
/// the cookie then opens status, events and lock; the token never comes back.
#[tokio::test(start_paused = true)]
async fn test_rmc_funnel_login_issues_a_host_cookie_session() {
    let h = Harness::start_with(Options::funnel()).await;
    let via = Via::funnel(IP_A);
    let options = h.login_options(&via).await;
    assert_eq!(options.status, 200);
    options.assert_mandatory_headers();
    options.assert_json_body();
    assert_eq!(
        json_keys(&options),
        vec!["challenge", "rp_id", "timeout_ms"]
    );
    assert_eq!(options.json()["rp_id"], HOST);
    assert_eq!(options.json()["timeout_ms"], 120_000);
    let challenge = h.challenge_of(&options);
    assert_eq!(challenge.len(), 43);
    assert_eq!(b64url_decode(&challenge).len(), CHALLENGE_BYTES);
    let text = String::from_utf8_lossy(&options.body).into_owned();
    assert!(
        !text.contains(&h.owner.credential_id_b64()),
        "no credential id"
    );
    assert!(!text.contains("allowCredentials") && !text.contains("allow_credentials"));
    assert!(options.header("set-cookie").is_none());

    let body = h.valid_assertion(&h.owner, &challenge);
    let r = h.post_login(&via, &body).await;
    assert_result(&r, 200, "logged_in");
    let set = r.header("set-cookie").expect("Set-Cookie").to_string();
    let value = set
        .strip_prefix("__Host-soos_session=")
        .and_then(|rest| rest.split(';').next())
        .expect("cookie value")
        .to_string();
    assert_eq!(value.len(), 43);
    assert_eq!(b64url_decode(&value).len(), 32);
    assert_eq!(
        set,
        format!(
            "__Host-soos_session={value}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=28800"
        )
    );
    assert!(!set.to_ascii_lowercase().contains("domain"));
    let session = via.with_cookie(&format!("__Host-soos_session={value}"));

    let status = h.get_via(&session, "/api/status").await;
    assert_eq!(status.status, 200);
    assert_eq!(status.json()["state"], "unlocked");
    let state = h.auth_state(&session).await;
    assert_eq!(state.status, 200);
    assert_eq!(
        json_keys(&state),
        vec![
            "authenticated",
            "enrollment",
            "mode",
            "passkeys",
            "unlock_enabled"
        ]
    );
    assert_eq!(state.json()["mode"], "funnel");
    assert_eq!(state.json()["authenticated"], true);
    assert_eq!(state.json()["passkeys"], true);
    assert_eq!(state.json()["unlock_enabled"], true);
    assert_eq!(state.json()["enrollment"], false, "never from Funnel");
    let mut stream = match h.open_stream_via(&session).await {
        Ok(stream) => stream,
        Err(other) => panic!("stream refused: {other:?}"),
    };
    assert_sse_head(&stream.head);
    stream.expect_state_now("unlocked").await;
    let lock = h
        .send(
            &session,
            "POST",
            "/api/lock",
            &[("X-Soos-Action", "lock")],
            None,
        )
        .await;
    assert_result(&lock, 202, "lock_requested");
    assert_eq!(h.source.lock_ids().len(), 1);
    for r in [&status, &state, &lock] {
        let all = format!("{:?}{}", r.headers, String::from_utf8_lossy(&r.body));
        assert!(!all.contains(&value), "the token never comes back");
        assert!(r.header("set-cookie").is_none());
    }

    // The tailnet path ignores cookies and keeps its one-tap status.
    let tailnet = h.auth_state(&Via::Tailnet).await;
    assert_eq!(tailnet.json()["mode"], "tailnet");
    assert_eq!(tailnet.json()["enrollment"], true);
    assert!(tailnet.json()["authenticated"].is_boolean());
    assert_eq!(h.get_via(&Via::Tailnet, "/api/status").await.status, 200);
}

/// Test 37 (RMC32): every refused verification consumes its challenge: a replay, a login
/// assertion sent to unlock (purpose), a tailnet challenge used on Funnel (class), an
/// unknown credential.
#[tokio::test(start_paused = true)]
async fn test_rmc_funnel_login_rejections_consume_the_challenge() {
    let h = Harness::start_with(Options::funnel()).await;
    h.source.set_locked();
    let via = Via::funnel(IP_A);

    // Replay of a successful login.
    let c = h.challenge_of(&h.login_options(&via).await);
    let body = h.valid_assertion(&h.owner, &c);
    let ok = h.post_login(&via, &body).await;
    assert_result(&ok, 200, "logged_in");
    let session = via.with_cookie(&cookie_of(&ok));
    assert_result(&h.post_login(&via, &body).await, 403, "passkey_rejected");

    // A login assertion sent to unlock is refused, and its challenge is gone.
    let c = h.challenge_of(&h.login_options(&via).await);
    let login_body = h.valid_assertion(&h.owner, &c);
    assert_result(
        &h.post_unlock(&session, Some(&login_body)).await,
        403,
        "passkey_rejected",
    );
    assert_result(
        &h.post_login(&via, &login_body).await,
        403,
        "passkey_rejected",
    );

    // A tailnet unlock challenge is useless on Funnel (class).
    let c = h.challenge_of(&h.unlock_options(&Via::Tailnet).await);
    let tailnet_body = h.valid_assertion(&h.owner, &c);
    assert_result(
        &h.post_unlock(&session, Some(&tailnet_body)).await,
        403,
        "passkey_rejected",
    );
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some(&tailnet_body)).await,
        403,
        "passkey_rejected",
    );

    // An unknown credential.
    let c = h.challenge_of(&h.login_options(&Via::funnel(IP_B)).await);
    let stranger = Authenticator::device(0x70, b"never-registered");
    let body = h.valid_assertion(&stranger, &c);
    assert_result(
        &h.post_login(&Via::funnel(IP_B), &body).await,
        403,
        "passkey_rejected",
    );
    let body = h.valid_assertion(&h.owner, &c);
    assert_result(
        &h.post_login(&Via::funnel(IP_B), &body).await,
        403,
        "passkey_rejected",
    );
    assert!(h.source.unlock_ids().is_empty());
}

/// Test 38 (RMC33, F-4, F-6, G-3): idle expiry at 15 min, absolute at 8 h, only
/// request-level validations refresh (an asset or an SSE keep-alive never does), logout,
/// streams end within one keep-alive, the session cap, and rotation on login.
#[tokio::test(start_paused = true)]
async fn test_rmc_funnel_sessions_expire_and_logout() {
    let keepalive_bound = SSE_KEEPALIVE_MS + 1000 + SNAPSHOT_DEADLINE_MS;
    {
        let h = Harness::start_with(Options::funnel()).await;
        // Idle: exactly WEB_SESSION_IDLE_MS.
        let s1 = h.session(IP_A).await;
        h.advance_ms(WEB_SESSION_IDLE_MS - 1).await;
        let s2 = h.session(IP_B).await;
        assert_eq!(
            h.get_via(&s1, "/api/status").await.status,
            200,
            "15 min - 1 ms"
        );
        h.advance_ms(WEB_SESSION_IDLE_MS).await;
        assert_result(&h.get_via(&s1, "/api/status").await, 403, "login_required");
        assert_result(&h.get_via(&s2, "/api/status").await, 403, "login_required");

        // A request-level validation refreshes; an asset does not (G-3).
        let s = h.session(IP_A).await;
        h.advance_ms(14 * 60_000).await;
        assert_eq!(h.get_via(&s, "/api/status").await.status, 200);
        h.advance_ms(6 * 60_000).await;
        assert_eq!(h.get_via(&s, "/api/status").await.status, 200, "at 20 min");
        let t = h.session(IP_B).await;
        h.advance_ms(14 * 60_000).await;
        assert_eq!(h.get_via(&t, "/").await.status, 200);
        assert_eq!(h.get_via(&t, "/nope").await.status, 404);
        h.advance_ms(60_000).await;
        assert_result(&h.get_via(&t, "/api/status").await, 403, "login_required");

        // Absolute lifetime, however often touched.
        let a = h.session(IP_C).await;
        let mut elapsed = 0;
        while elapsed + 14 * 60_000 < WEB_SESSION_ABSOLUTE_MS {
            h.advance_ms(14 * 60_000).await;
            elapsed += 14 * 60_000;
            assert_eq!(h.get_via(&a, "/api/status").await.status, 200, "{elapsed}");
        }
        h.advance_ms(WEB_SESSION_ABSOLUTE_MS - elapsed - 1).await;
        assert_eq!(h.get_via(&a, "/api/status").await.status, 200);
        h.advance_ms(1).await;
        assert_result(&h.get_via(&a, "/api/status").await, 403, "login_required");
    }
    {
        // Logout clears cookie and record; an open stream ends within one keep-alive.
        let h = Harness::start_with(Options::funnel()).await;
        let s = h.session(IP_A).await;
        let mut stream = h.open_stream_via(&s).await.expect("stream");
        stream.expect_event_now().await;
        let r = h.logout(&s).await;
        assert_result(&r, 200, "logged_out");
        assert_eq!(
            r.header("set-cookie"),
            Some("__Host-soos_session=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0")
        );
        assert_result(&h.get_via(&s, "/api/status").await, 403, "login_required");
        assert!(
            stream.ends_within(&h, keepalive_bound).await.is_some(),
            "the stream ends after logout"
        );
        // Idle expiry of a stream left alone: keep-alives never refresh (F-4).
        let s = h.session(IP_B).await;
        let mut stream = h.open_stream_via(&s).await.expect("stream");
        stream.expect_event_now().await;
        assert!(
            stream
                .ends_within(&h, WEB_SESSION_IDLE_MS - SSE_KEEPALIVE_MS)
                .await
                .is_none(),
            "still open well before the idle expiry"
        );
        let ended = stream
            .ends_within(&h, SSE_KEEPALIVE_MS + keepalive_bound)
            .await;
        assert!(
            ended.is_some(),
            "an untouched stream ends after 15 min idle"
        );
        assert_result(&h.get_via(&s, "/api/status").await, 403, "login_required");
    }
    {
        // Cap: MAX_WEB_SESSIONS from distinct cookie-less clients, then 429.
        let h = Harness::start_with(Options::funnel()).await;
        for i in 0..MAX_WEB_SESSIONS {
            h.session(&format!("198.51.100.{}", i + 1)).await;
        }
        let r = h
            .login(&Via::funnel("198.51.100.99"), &h.owner.clone())
            .await;
        assert_result(&r, 429, "too_many_sessions");
        assert!(r.header("set-cookie").is_none());
    }
    {
        // Rotation (F-6): six logins from one browser presenting the previous cookie leave
        // exactly one record.
        let h = Harness::start_with(Options::funnel()).await;
        let mut browser = h.session(IP_A).await;
        let mut previous = Vec::new();
        for _ in 0..5 {
            previous.push(browser.clone());
            let r = h.login(&browser, &h.owner.clone()).await;
            assert_result(&r, 200, "logged_in");
            browser = Via::funnel(IP_A).with_cookie(&cookie_of(&r));
        }
        for old in &previous {
            assert_result(&h.get_via(old, "/api/status").await, 403, "login_required");
        }
        assert_eq!(h.get_via(&browser, "/api/status").await.status, 200);
        for i in 0..MAX_WEB_SESSIONS - 1 {
            h.session(&format!("198.51.100.{}", i + 1)).await;
        }
        let r = h
            .login(&Via::funnel("198.51.100.99"), &h.owner.clone())
            .await;
        assert_result(&r, 429, "too_many_sessions");
    }
}

// ---------------------------------------------------------------------------------------
// Tests 39–40 — unlock
// ---------------------------------------------------------------------------------------

/// Test 39 (RMC36, D-C): a fresh unlock-purpose assertion is required on every path; the
/// options shape; the Funnel challenge is bound to its session; `rp_id` absent; empty store.
#[tokio::test(start_paused = true)]
async fn test_rmc_unlock_requires_a_fresh_passkey_on_every_path() {
    let h = Harness::start_with(Options::funnel()).await;
    h.source.set_locked();

    assert_result(
        &h.post_unlock(&Via::Tailnet, None).await,
        403,
        "passkey_required",
    );
    assert_no_logind(&h);
    let options = h.unlock_options(&Via::Tailnet).await;
    assert_eq!(
        json_keys(&options),
        vec!["challenge", "rp_id", "timeout_ms"]
    );
    assert!(!String::from_utf8_lossy(&options.body).contains(&h.owner.credential_id_b64()));
    let c = h.challenge_of(&options);
    let body = h.valid_assertion(&h.owner, &c);
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some(&body)).await,
        202,
        "unlock_requested",
    );
    assert_eq!(h.source.unlock_ids().len(), 1);
    h.advance_ms(MIN_UNLOCK_INTERVAL_MS + 1).await;
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some(&body)).await,
        403,
        "passkey_rejected",
    );
    assert_eq!(h.source.unlock_ids().len(), 1, "a replay never unlocks");

    let s1 = h.session(IP_A).await;
    assert_result(
        &h.unlock_with(&s1, &h.owner.clone()).await,
        202,
        "unlock_requested",
    );
    assert_eq!(h.source.unlock_ids().len(), 2);
    h.advance_ms(MIN_UNLOCK_INTERVAL_MS + 1).await;
    assert_result(&h.post_unlock(&s1, None).await, 403, "passkey_required");
    // Bound to the session that asked for it.
    let s2 = h.session(IP_B).await;
    let c = h.challenge_of(&h.unlock_options(&s1).await);
    let body = h.valid_assertion(&h.owner, &c);
    assert_result(
        &h.post_unlock(&s2, Some(&body)).await,
        403,
        "passkey_rejected",
    );
    assert_result(
        &h.post_unlock(&s1, Some(&body)).await,
        403,
        "passkey_rejected",
    );
    assert_eq!(h.source.unlock_ids().len(), 2);

    // allow_unlock = false: options refused too.
    let off = Harness::start_with(Options {
        allow_unlock: false,
        ..Options::funnel()
    })
    .await;
    assert_result(
        &off.unlock_options(&Via::Tailnet).await,
        403,
        "unlock_disabled",
    );
    // rp_id absent.
    let none = Harness::start_with(Options {
        allow_unlock: true,
        ..Options::default()
    })
    .await;
    none.source.set_locked();
    assert_result(
        &none.post_unlock(&Via::Tailnet, None).await,
        403,
        "passkeys_not_configured",
    );
    assert_result(
        &none.unlock_options(&Via::Tailnet).await,
        403,
        "passkeys_not_configured",
    );
    assert_no_logind(&none);
    // No passkey stored.
    let empty = Harness::start_with(Options {
        store: StoreSetup::Absent,
        ..Options::funnel()
    })
    .await;
    assert_result(
        &empty.unlock_options(&Via::Tailnet).await,
        409,
        "no_passkey",
    );
    assert_result(
        &empty.login_options(&Via::funnel(IP_A)).await,
        409,
        "no_passkey",
    );
}

/// Test 39a (RMC36, F-5): a validly signed assertion without UV never unlocks, on either
/// path: `403 passkey_rejected`, zero logind calls, the challenge consumed, exactly one
/// failure counted; a login-purpose assertion is refused; a session alone is not enough.
#[tokio::test(start_paused = true)]
async fn test_rmc_unlock_refuses_assertions_without_user_verification_end_to_end() {
    let h = Harness::start_with(Options::funnel()).await;
    h.source.set_locked();
    let owner = h.owner.clone();
    let up_only = (owner.flags() & !UV) | UP;

    // Tailnet.
    let c = h.challenge_of(&h.unlock_options(&Via::Tailnet).await);
    let body = h.assertion(&owner, &c, up_only, 0, Some(&OWNER_HANDLE));
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some(&body)).await,
        403,
        "passkey_rejected",
    );
    assert_no_logind(&h);
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some(&body)).await,
        403,
        "passkey_rejected",
    );
    assert_no_logind(&h);
    // Failures so far: 2 (UV clear, consumed retry). Two malformed bodies make 4: still
    // open; a valid unlock works; a fifth failure locks the tailnet key.
    for _ in 0..2 {
        assert_result(
            &h.post_unlock(&Via::Tailnet, Some("{}")).await,
            400,
            "bad_request",
        );
    }
    assert_result(
        &h.unlock_with(&Via::Tailnet, &owner).await,
        202,
        "unlock_requested",
    );
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some("{}")).await,
        400,
        "bad_request",
    );
    assert_result(&h.unlock_options(&Via::Tailnet).await, 429, "rate_limited");
    assert_eq!(h.source.unlock_ids().len(), 1);

    // Funnel with a valid session.
    let h = Harness::start_with(Options::funnel()).await;
    h.source.set_locked();
    let s = h.session(IP_A).await;
    let c = h.challenge_of(&h.unlock_options(&s).await);
    let body = h.assertion(&owner, &c, up_only, 0, Some(&OWNER_HANDLE));
    assert_result(
        &h.post_unlock(&s, Some(&body)).await,
        403,
        "passkey_rejected",
    );
    assert_result(
        &h.post_unlock(&s, Some(&body)).await,
        403,
        "passkey_rejected",
    );
    // A UV login-purpose assertion with a valid session.
    let c = h.challenge_of(&h.login_options(&s).await);
    let login_body = h.valid_assertion(&owner, &c);
    assert_result(
        &h.post_unlock(&s, Some(&login_body)).await,
        403,
        "passkey_rejected",
    );
    assert_result(&h.post_unlock(&s, None).await, 403, "passkey_required");
    assert_no_logind(&h);
}

/// Test 40 (RMC36): `allow_unlock = false` answers `unlock_disabled` before reading the body
/// or calling logind, on both paths.
#[tokio::test(start_paused = true)]
async fn test_rmc_unlock_order_disabled_before_body_and_logind() {
    let h = Harness::start_with(Options {
        allow_unlock: false,
        ..Options::funnel()
    })
    .await;
    h.source.set_locked();
    let mut held = h
        .hold(
            &Via::Tailnet,
            "POST",
            "/api/unlock",
            &[("X-Soos-Action", "unlock")],
            Some(4096),
        )
        .await;
    let r = held
        .response_now()
        .await
        .expect("answered without the body");
    assert_result(&r, 403, "unlock_disabled");
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some("{\"id\":\"x\"}")).await,
        403,
        "unlock_disabled",
    );
    let s = h.session(IP_A).await;
    assert_result(&h.post_unlock(&s, Some("{}")).await, 403, "unlock_disabled");
    assert_no_logind(&h);
}

// ---------------------------------------------------------------------------------------
// Tests 41, 41a, 41b — registration
// ---------------------------------------------------------------------------------------

/// Test 41 (RMC34, D-E): registration only from the tailnet with a local one-time code:
/// Funnel is `403` before any body read; no, wrong (3 attempts delete it), expired or
/// future-dated code is `enroll_code_rejected`; a valid code registers (store `0600`, code
/// removed); duplicate and limit are `409`.
#[tokio::test(start_paused = true)]
async fn test_rmc_registration_is_tailnet_only_with_a_local_code() {
    let h = Harness::start_with(Options::funnel()).await;
    let now = h.now_unix_s();
    h.write_code(CODE, now + ENROLL_CODE_TTL_S);
    let s = h.session(IP_A).await;
    for via in [Via::funnel(IP_B), s.clone()] {
        let body = format!("{{\"code\":\"{CODE}\"}}");
        let r = h
            .ceremony(
                &via,
                "/api/auth/register/options",
                "register-options",
                Some(&body),
            )
            .await;
        assert_result(&r, 403, "forbidden");
    }
    assert!(
        h.code_file_exists(),
        "a Funnel attempt never touches the code"
    );
    fs::remove_file(h.dir.join("enroll-code")).unwrap();

    // No code file.
    assert_result(&h.register_options(CODE).await, 403, "enroll_code_rejected");
    // Three wrong codes delete the file; the right one is then useless.
    h.write_code(CODE, now + ENROLL_CODE_TTL_S);
    for wrong in ["ZZZZZ99999", "ABCDE12346", "00000-00000"] {
        assert_result(
            &h.register_options(wrong).await,
            403,
            "enroll_code_rejected",
        );
    }
    assert!(
        !h.code_file_exists(),
        "deleted after MAX_ENROLL_CODE_ATTEMPTS"
    );
    assert_result(&h.register_options(CODE).await, 403, "enroll_code_rejected");
    h.advance_ms(AUTH_FAILURE_WINDOW_MS).await;

    // Expired (expires <= now) and future-dated codes are rejected and removed.
    let now = h.now_unix_s();
    h.write_code(CODE, now);
    assert_result(&h.register_options(CODE).await, 403, "enroll_code_rejected");
    assert!(!h.code_file_exists());
    h.write_code(CODE, now + ENROLL_CODE_TTL_S + 1);
    assert_result(&h.register_options(CODE).await, 403, "enroll_code_rejected");
    assert!(!h.code_file_exists());
    h.advance_ms(AUTH_FAILURE_WINDOW_MS).await;

    // A valid code.
    let device = Authenticator::device(0x71, b"device-0071");
    let now = h.now_unix_s();
    h.write_code(CODE, now + ENROLL_CODE_TTL_S);
    let options = h.register_options("abcde-12345").await;
    assert_eq!(options.status, 200, "{}", options.result_or_body());
    assert_eq!(
        json_keys(&options),
        vec![
            "challenge",
            "exclude_credentials",
            "rp_id",
            "timeout_ms",
            "user_id"
        ]
    );
    assert_eq!(options.json()["user_id"], b64url(&OWNER_HANDLE));
    assert_eq!(
        options.json()["exclude_credentials"],
        serde_json::json!([h.owner.credential_id_b64()])
    );
    let c = h.challenge_of(&options);
    let r = h.register_verify(&h.registration_for(&device, &c)).await;
    assert_result(&r, 200, "registered");
    assert!(!h.code_file_exists(), "single use");
    assert_eq!(
        fs::metadata(&h.store_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let store = h.store_json_value();
    assert_eq!(
        store["user_handle"],
        b64url(&OWNER_HANDLE),
        "handle never changes"
    );
    assert_eq!(store["passkeys"].as_array().unwrap().len(), 2);
    assert_eq!(
        store["passkeys"][1]["credential_id"],
        device.credential_id_b64()
    );
    assert_eq!(
        store["passkeys"][1]["public_key"],
        b64url(&device.public_key())
    );

    // The same credential again.
    h.write_code(CODE, h.now_unix_s() + ENROLL_CODE_TTL_S);
    let (_, r) = h.register(&device, CODE).await;
    assert_result(&r, 409, "already_registered");
    // Up to MAX_PASSKEYS, then passkey_limit at options time.
    for i in 0..(MAX_PASSKEYS - 2) as u8 {
        h.write_code(CODE, h.now_unix_s() + ENROLL_CODE_TTL_S);
        let extra = Authenticator::device(0x72 + i, &[0x72 + i; 20]);
        let (_, r) = h.register(&extra, CODE).await;
        assert_result(&r, 200, "registered");
    }
    assert_eq!(
        h.store_json_value()["passkeys"].as_array().unwrap().len(),
        MAX_PASSKEYS
    );
    h.write_code(CODE, h.now_unix_s() + ENROLL_CODE_TTL_S);
    assert_result(&h.register_options(CODE).await, 409, "passkey_limit");
    assert_no_logind(&h);
}

/// Test 41a (RMC34, F-1): on an empty store the first registration writes exactly the user
/// handle sent in its options, and that passkey then logs in and unlocks on both paths.
#[tokio::test(start_paused = true)]
async fn test_rmc_first_registration_then_login_and_unlock_use_the_same_user_handle() {
    let h = Harness::start_with(Options {
        store: StoreSetup::Absent,
        ..Options::funnel()
    })
    .await;
    h.source.set_locked();
    assert!(!h.store_path.exists());
    let phone = Authenticator::new(0x73, b"first-phone-passkey", true);
    h.write_code(CODE, h.now_unix_s() + ENROLL_CODE_TTL_S);
    let options = h.register_options(CODE).await;
    assert_eq!(options.status, 200, "{}", options.result_or_body());
    assert_eq!(options.json()["exclude_credentials"], serde_json::json!([]));
    let user_id = options.json()["user_id"].as_str().unwrap().to_string();
    let handle = b64url_decode(&user_id);
    assert_eq!(handle.len(), 16);
    let c = h.challenge_of(&options);
    assert_result(
        &h.register_verify(&h.registration_for(&phone, &c)).await,
        200,
        "registered",
    );
    assert!(!h.store_path.exists() || h.store_json_value()["user_handle"] == user_id);
    assert_eq!(h.store_json_value()["user_handle"], user_id);
    assert_eq!(
        fs::metadata(&h.store_path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let via = Via::funnel(IP_A);
    let c = h.challenge_of(&h.login_options(&via).await);
    let body = h.assertion(&phone, &c, phone.flags(), 0, Some(&handle));
    let r = h.post_login(&via, &body).await;
    assert_result(&r, 200, "logged_in");
    let session = via.with_cookie(&cookie_of(&r));
    let c = h.challenge_of(&h.unlock_options(&session).await);
    let body = h.assertion(&phone, &c, phone.flags(), 0, Some(&handle));
    assert_result(
        &h.post_unlock(&session, Some(&body)).await,
        202,
        "unlock_requested",
    );
    h.advance_ms(MIN_UNLOCK_INTERVAL_MS + 1).await;
    let c = h.challenge_of(&h.unlock_options(&Via::Tailnet).await);
    let body = h.assertion(&phone, &c, phone.flags(), 0, Some(&handle));
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some(&body)).await,
        202,
        "unlock_requested",
    );
    assert_eq!(h.source.unlock_ids().len(), 2);
    // The owner fixture handle is not this store's handle.
    let c = h.challenge_of(&h.unlock_options(&Via::Tailnet).await);
    let body = h.assertion(&phone, &c, phone.flags(), 0, Some(&OWNER_HANDLE));
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some(&body)).await,
        403,
        "passkey_rejected",
    );
}

/// Test 41b (RMC34, F-1): interleaved registrations on an empty store: the second verified
/// ceremony wins and consumes the code; the first is `enroll_code_rejected` without writing;
/// a store created meanwhile with another handle gives `409 registration_conflict`, writes
/// nothing, keeps the code and counts no failure.
#[tokio::test(start_paused = true)]
async fn test_rmc_interleaved_registrations_on_an_empty_store() {
    let h = Harness::start_with(Options {
        store: StoreSetup::Absent,
        ..Options::funnel()
    })
    .await;
    h.write_code(CODE, h.now_unix_s() + ENROLL_CODE_TTL_S);
    let first = h.register_options(CODE).await;
    let second = h.register_options(CODE).await;
    assert_eq!((first.status, second.status), (200, 200));
    let x = first.json()["user_id"].as_str().unwrap().to_string();
    let y = second.json()["user_id"].as_str().unwrap().to_string();
    assert_ne!(
        x, y,
        "a fresh handle per ceremony while the store is absent"
    );
    let phone_x = Authenticator::new(0x74, b"phone-x", true);
    let phone_y = Authenticator::new(0x75, b"phone-y", true);
    let cx = h.challenge_of(&first);
    let cy = h.challenge_of(&second);
    assert_result(
        &h.register_verify(&h.registration_for(&phone_y, &cy)).await,
        200,
        "registered",
    );
    assert_eq!(h.store_json_value()["user_handle"], y);
    assert!(!h.code_file_exists());
    let bytes = fs::read(&h.store_path).unwrap();
    assert_result(
        &h.register_verify(&h.registration_for(&phone_x, &cx)).await,
        403,
        "enroll_code_rejected",
    );
    assert_eq!(fs::read(&h.store_path).unwrap(), bytes, "nothing written");
    let via = Via::funnel(IP_A);
    let c = h.challenge_of(&h.login_options(&via).await);
    let body = h.assertion(&phone_y, &c, phone_y.flags(), 0, Some(&b64url_decode(&y)));
    assert_result(&h.post_login(&via, &body).await, 200, "logged_in");

    // A conflicting store appears between options and verify.
    let h = Harness::start_with(Options {
        store: StoreSetup::Absent,
        ..Options::funnel()
    })
    .await;
    h.write_code(CODE, h.now_unix_s() + ENROLL_CODE_TTL_S);
    let options = h.register_options(CODE).await;
    assert_eq!(options.status, 200);
    let c = h.challenge_of(&options);
    write_store_with_handle(&h.store_path, &[0x5e; 16], &[]);
    let bytes = fs::read(&h.store_path).unwrap();
    let phone = Authenticator::new(0x76, b"phone-z", true);
    assert_result(
        &h.register_verify(&h.registration_for(&phone, &c)).await,
        409,
        "registration_conflict",
    );
    assert_eq!(fs::read(&h.store_path).unwrap(), bytes, "nothing written");
    assert!(h.code_file_exists(), "the code is kept");
    // Not counted: four malformed bodies leave the tailnet key open, the fifth locks it.
    for _ in 0..4 {
        assert_result(&h.register_verify("{}").await, 400, "bad_request");
    }
    assert_ne!(h.unlock_options(&Via::Tailnet).await.status, 429);
    assert_result(&h.register_verify("{}").await, 400, "bad_request");
    assert_result(&h.unlock_options(&Via::Tailnet).await, 429, "rate_limited");
}

// ---------------------------------------------------------------------------------------
// Tests 42–45 — limiters, fail-closed, revocation, logs
// ---------------------------------------------------------------------------------------

/// Test 42 (RMC37, F-2): 10 options per minute and 5 failures per 5 minutes, per limiter
/// key; a locked anonymous hint never affects another hint, a session holder or the
/// tailnet, and a locked session key never affects anonymous login or the tailnet.
#[tokio::test(start_paused = true)]
async fn test_rmc_auth_limiters_are_per_limit_key() {
    let h = Harness::start_with(Options::funnel()).await;
    let owner = h.owner.clone();
    h.source.set_unlocked();
    // Options limiter, tailnet key: 10 issuances, each consumed by a valid 409 unlock.
    for i in 0..10 {
        let r = h.unlock_with(&Via::Tailnet, &owner).await;
        assert_result(&r, 409, "already_unlocked");
        let _ = i;
    }
    assert_result(&h.unlock_options(&Via::Tailnet).await, 429, "rate_limited");
    h.advance_ms(OPTIONS_WINDOW_MS).await;
    assert_eq!(h.unlock_options(&Via::Tailnet).await.status, 200);

    // Failure lockout, tailnet key.
    let h = Harness::start_with(Options::funnel()).await;
    let session = h.session(IP_C).await;
    for _ in 0..5 {
        assert_result(
            &h.post_unlock(&Via::Tailnet, Some("{}")).await,
            400,
            "bad_request",
        );
    }
    assert_result(&h.unlock_options(&Via::Tailnet).await, 429, "rate_limited");
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some("{}")).await,
        429,
        "rate_limited",
    );
    h.write_code(CODE, h.now_unix_s() + ENROLL_CODE_TTL_S);
    assert_result(&h.register_options(CODE).await, 429, "rate_limited");
    assert_eq!(
        h.unlock_options(&session).await.status,
        200,
        "session key unaffected"
    );
    assert_eq!(h.login_options(&Via::funnel(IP_A)).await.status, 200);
    h.advance_ms(AUTH_FAILURE_WINDOW_MS - 1).await;
    assert_result(&h.unlock_options(&Via::Tailnet).await, 429, "rate_limited");
    h.advance_ms(1).await;
    assert_eq!(
        h.unlock_options(&Via::Tailnet).await.status,
        200,
        "window over"
    );

    // Anonymous hint A locked.
    let h = Harness::start_with(Options::funnel()).await;
    let session = h.session(IP_C).await;
    let a = Via::funnel(IP_A);
    for _ in 0..5 {
        assert_result(&h.post_login(&a, "{}").await, 400, "bad_request");
    }
    assert_result(&h.login_options(&a).await, 429, "rate_limited");
    assert_result(&h.post_login(&a, "{}").await, 429, "rate_limited");
    assert_eq!(h.login_options(&Via::funnel(IP_B)).await.status, 200);
    assert_eq!(h.unlock_options(&session).await.status, 200);
    assert_eq!(h.unlock_options(&Via::Tailnet).await.status, 200);

    // Session key locked.
    for _ in 0..5 {
        assert_result(
            &h.post_unlock(&session, Some("{}")).await,
            400,
            "bad_request",
        );
    }
    assert_result(&h.unlock_options(&session).await, 429, "rate_limited");
    assert_eq!(
        h.login_options(&Via::funnel("203.0.113.40")).await.status,
        200
    );
    assert_eq!(h.unlock_options(&Via::Tailnet).await.status, 200);

    // Anonymous options limiter per hint.
    let d = Via::funnel("203.0.113.50");
    for _ in 0..10 {
        assert_eq!(h.login_options(&d).await.status, 200);
    }
    assert_result(&h.login_options(&d).await, 429, "rate_limited");
    assert_eq!(
        h.login_options(&Via::funnel("203.0.113.60")).await.status,
        200
    );
    assert_no_logind(&h);
}

/// Test 43 (RMC38, F-7): fail-closed store and RNG: an insecure store drops every session
/// and answers `503 store_unavailable`; an RNG failure answers `503 unavailable` and
/// creates nothing; a store write failing after a valid assertion (lock held) is `503
/// store_unavailable` after `STORE_LOCK_TIMEOUT_MS`, never an unlock, not a counted
/// failure, keeps the sessions (`Busy`) and consumes the challenge; a `0/0` synced
/// assertion never takes the store lock.
#[tokio::test(start_paused = true)]
async fn test_rmc_auth_fails_closed_on_store_and_random_errors() {
    // Insecure store.
    let h = Harness::start_with(Options::funnel()).await;
    let s = h.session(IP_A).await;
    fs::set_permissions(&h.store_path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_result(&h.get_via(&s, "/api/status").await, 403, "login_required");
    assert_result(
        &h.login_options(&Via::funnel(IP_B)).await,
        503,
        "store_unavailable",
    );
    assert_result(
        &h.unlock_options(&Via::Tailnet).await,
        503,
        "store_unavailable",
    );
    fs::set_permissions(&h.store_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_result(&h.get_via(&s, "/api/status").await, 403, "login_required");
    assert_eq!(
        h.login_options(&Via::funnel(IP_B)).await.status,
        200,
        "recovers"
    );

    // RNG failure.
    let failing = Arc::new(AtomicBool::new(false));
    let h = Harness::start_with(Options {
        random: Some(switchable_random(Arc::clone(&failing))),
        ..Options::funnel()
    })
    .await;
    let via = Via::funnel(IP_A);
    let c = h.challenge_of(&h.login_options(&via).await);
    let body = h.valid_assertion(&h.owner, &c);
    failing.store(true, Ordering::SeqCst);
    assert_result(&h.login_options(&via).await, 503, "unavailable");
    assert_result(&h.unlock_options(&Via::Tailnet).await, 503, "unavailable");
    h.write_code(CODE, h.now_unix_s() + ENROLL_CODE_TTL_S);
    assert_result(&h.register_options(CODE).await, 503, "unavailable");
    let r = h.post_login(&via, &body).await;
    assert_result(&r, 503, "unavailable");
    assert!(
        r.header("set-cookie").is_none(),
        "no session without a token"
    );
    failing.store(false, Ordering::SeqCst);
    for i in 0..MAX_WEB_SESSIONS {
        h.session(&format!("198.51.100.{}", i + 1)).await;
    }

    // Store write after a valid assertion while another holder keeps the lock.
    let device = Authenticator::device(0x77, b"counted-device");
    let h = Harness::start_with(Options {
        store: StoreSetup::Passkeys(vec![
            StoredPasskey::of(&Authenticator::owner()),
            StoredPasskey {
                sign_count: 6,
                ..StoredPasskey::of(&device)
            },
        ]),
        ..Options::funnel()
    })
    .await;
    h.source.set_locked();
    let s = h.session(IP_A).await;
    let c = h.challenge_of(&h.unlock_options(&Via::Tailnet).await);
    let body = h.assertion(&device, &c, UP | UV, 7, Some(&OWNER_HANDLE));
    let lock = hold_store_lock(&h.store_path);
    let pending = spawn_exchange(
        &h.path,
        request_bytes(
            &Via::Tailnet,
            "POST",
            "/api/unlock",
            &[("X-Soos-Action", "unlock")],
            Some(&body),
        ),
    );
    let mut waited = 0;
    while !finished_soon(&pending).await && waited <= STORE_LOCK_TIMEOUT_MS + 1000 {
        h.advance_ms(25).await;
        waited += 25;
    }
    match pending.await.unwrap() {
        Outcome::Response(r) => assert_result(&r, 503, "store_unavailable"),
        other => panic!("expected 503 store_unavailable, got {other:?}"),
    }
    assert!(h.source.unlock_ids().is_empty(), "never an unlock");
    // A synced 0/0 assertion never takes the lock.
    assert_result(
        &h.unlock_with(&Via::Tailnet, &h.owner.clone()).await,
        202,
        "unlock_requested",
    );
    drop(lock);
    assert_eq!(
        h.get_via(&s, "/api/status").await.status,
        200,
        "Busy keeps sessions"
    );
    assert_eq!(
        h.store_json_value()["passkeys"][1]["sign_count"],
        6,
        "nothing written"
    );
    // Not counted: four malformed bodies, then the consumed challenge is the fifth failure.
    for _ in 0..4 {
        assert_result(
            &h.post_unlock(&Via::Tailnet, Some("{}")).await,
            400,
            "bad_request",
        );
    }
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some(&body)).await,
        403,
        "passkey_rejected",
    );
    assert_result(&h.unlock_options(&Via::Tailnet).await, 429, "rate_limited");
    assert_eq!(h.source.unlock_ids().len(), 1);
}

/// Test 44 (RMC33, RMC35, F-6): removing a passkey from the store revokes its sessions at
/// the next request (and its stream at the next keep-alive); other sessions stay.
#[tokio::test(start_paused = true)]
async fn test_rmc_passkey_removal_revokes_sessions() {
    let phone = Authenticator::new(0x78, b"second-phone", true);
    let h = Harness::start_with(Options {
        store: StoreSetup::Passkeys(vec![
            StoredPasskey::of(&Authenticator::owner()),
            StoredPasskey::of(&phone),
        ]),
        ..Options::funnel()
    })
    .await;
    let owner_session = h.session(IP_A).await;
    let via = Via::funnel(IP_B);
    let r = h.login(&via, &phone).await;
    assert_result(&r, 200, "logged_in");
    let phone_session = via.with_cookie(&cookie_of(&r));
    assert_eq!(h.get_via(&phone_session, "/api/status").await.status, 200);
    let mut stream = h.open_stream_via(&phone_session).await.expect("stream");
    stream.expect_event_now().await;

    write_store(&h.store_path, &[StoredPasskey::of(&Authenticator::owner())]);
    assert_result(
        &h.get_via(&phone_session, "/api/status").await,
        403,
        "login_required",
    );
    assert!(
        stream
            .ends_within(&h, SSE_KEEPALIVE_MS + 1000 + SNAPSHOT_DEADLINE_MS)
            .await
            .is_some(),
        "the revoked session's stream ends"
    );
    assert_eq!(h.get_via(&owner_session, "/api/status").await.status, 200);
}

/// Test 45 (RMC38, D-H): no challenge, token, cookie, credential id, public key, user
/// handle, enrollment code, login, host, client address or `Set-Cookie` in any log line at
/// TRACE; exactly one `info` line per accepted login and per registration.
#[tokio::test(start_paused = true)]
async fn test_rmc_auth_never_logs_secrets() {
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let h = Harness::start_with(Options::funnel()).await;
    h.source.set_locked();
    let owner = h.owner.clone();
    let via = Via::funnel(IP_A);
    // Failures of every kind.
    let c = h.challenge_of(&h.login_options(&via).await);
    let bad = h.assertion(&owner, &c, UP, 0, Some(&OWNER_HANDLE));
    assert_eq!(h.post_login(&via, &bad).await.status, 403);
    assert_eq!(h.post_login(&via, "{}").await.status, 400);
    h.write_code(CODE, h.now_unix_s() + ENROLL_CODE_TTL_S);
    assert_eq!(h.register_options("ZZZZZ99999").await.status, 403);
    // One success of each.
    let s = h.session(IP_B).await;
    assert_eq!(h.unlock_with(&s, &owner).await.status, 202);
    let device = Authenticator::device(0x79, b"logged-device-id");
    let (_, r) = h.register(&device, CODE).await;
    assert_result(&r, 200, "registered");
    assert_eq!(h.logout(&s).await.status, 200);

    let text = capture.text();
    let count = |needle: &str| text.lines().filter(|l| l.contains(needle)).count();
    assert_eq!(count("remote login accepted"), 1, "{text}");
    assert_eq!(count("passkey registered"), 1, "{text}");
    for line in text
        .lines()
        .filter(|l| l.contains("remote login accepted") || l.contains("passkey registered"))
    {
        assert!(line.contains("INFO"), "{line}");
    }
    let mut forbidden: Vec<String> = h.secrets.lock().unwrap().clone();
    forbidden.extend([
        owner.credential_id_b64(),
        to_hex(&owner.credential_id),
        b64url(&owner.public_key()),
        device.credential_id_b64(),
        to_hex(&device.credential_id),
        b64url(&device.public_key()),
        b64url(&OWNER_HANDLE),
        to_hex(&OWNER_HANDLE),
        "ABCDE-12345".to_string(),
        "ZZZZZ99999".to_string(),
        to_hex(&code_hash(CODE)),
        LOGIN.to_string(),
        HOST.to_string(),
        IP_A.to_string(),
        IP_B.to_string(),
        "203.0.113".to_string(),
        "__Host-soos_session".to_string(),
        "set-cookie".to_string(),
        "Set-Cookie".to_string(),
        "x-forwarded-for".to_string(),
    ]);
    for needle in forbidden {
        assert!(!needle.is_empty());
        let shown = if needle.len() > 12 {
            &needle[..12]
        } else {
            &needle[..]
        };
        assert!(!text.contains(&needle), "the log leaks {shown}…");
    }
}
