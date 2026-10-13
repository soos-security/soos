//! End-to-end contract tests of the direct biometric prompt of `soos-remote` (GitHub #354
//! sub-issue 1; ADR 2026-10-12 "Named Credentials on Authenticated Passkey Ceremonies of
//! `soos-remote`"; architect spec `AI/architect_spec_remote_direct_biometric.md` §8.2; matrix
//! RDB1–RDB4, RDB6–RDB8).
//!
//! The unlock and camera options of an authenticated caller (an allowed tailnet identity or
//! a Funnel web session) name every registered credential as `allow_credentials`; the login
//! options never name one; registration stores the bounded, allowlisted transports of the
//! browser. Same deterministic harness as `auth_server_tests.rs` / `camera_server_tests.rs`
//! (frozen paused clock, scripted logind, raw HTTP/1.1 over the Unix socket in a `TempDir`).
//! No network, no logind, no `~/.config`.

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

#[path = "common/push.rs"]
mod push;

#[path = "common/camera.rs"]
mod camera;

use serde_json::{json, Value};

use camera::*;
use harness::*;
use passkey::*;
use soos_remote::config::CameraConfig;
use soos_remote::{AUTH_FAILURE_WINDOW_MS, ENROLL_CODE_TTL_S, MIN_UNLOCK_INTERVAL_MS};

const IP_A: &str = "203.0.113.10";
const IP_B: &str = "203.0.113.20";
const IP_C: &str = "203.0.113.30";
const IP_D: &str = "203.0.113.40";
const CODE: &str = "ABCDE12345";

/// Key set of the unlock and camera options (spec D-1).
const NAMED_KEYS: [&str; 4] = ["allow_credentials", "challenge", "rp_id", "timeout_ms"];
/// Key set of the login options (spec D-2, unchanged).
const LOGIN_KEYS: [&str; 3] = ["challenge", "rp_id", "timeout_ms"];

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

fn body_text(r: &HttpResponse) -> String {
    String::from_utf8_lossy(&r.body).into_owned()
}

/// One expected `allow_credentials` entry.
fn entry(auth: &Authenticator, transports: &[&str]) -> Value {
    json!({"id": auth.credential_id_b64(), "transports": transports})
}

/// `body` (a JSON object) with `"transports": <raw>` appended.
fn with_transports(body: &str, raw: &str) -> String {
    let open = body.strip_suffix('}').expect("a JSON object");
    format!("{open},\"transports\":{raw}}}")
}

/// A fresh enrollment code, register options and a registration body of `auth`, with the
/// raw `transports` value appended when given.
async fn registration_body_of(h: &Harness, auth: &Authenticator, raw: Option<&str>) -> String {
    h.write_code(CODE, h.now_unix_s() + ENROLL_CODE_TTL_S);
    let options = h.register_options(CODE).await;
    let challenge = h.challenge_of(&options);
    let body = h.registration_for(auth, &challenge);
    match raw {
        Some(raw) => with_transports(&body, raw),
        None => body,
    }
}

/// Full tailnet registration of `auth` sending the raw `transports` value (when given).
async fn register_with(h: &Harness, auth: &Authenticator, raw: Option<&str>) -> HttpResponse {
    let body = registration_body_of(h, auth, raw).await;
    h.register_verify(&body).await
}

/// The options response of `route` (`unlock` or `camera`) through `via`.
async fn named_options(h: &Harness, route: &str, via: &Via) -> HttpResponse {
    match route {
        "unlock" => h.unlock_options(via).await,
        "camera" => camera_options(h, via).await,
        other => panic!("unknown route {other}"),
    }
}

/// Asserts the exact named options of RDB1/RDB2: key set, `allow_credentials` in store
/// order, a usable challenge.
fn assert_named(r: &HttpResponse, expected: &Value, what: &str) {
    assert_eq!(r.status, 200, "{what}: {}", r.result_or_body());
    r.assert_mandatory_headers();
    r.assert_json_body();
    assert_eq!(json_keys(r), NAMED_KEYS.to_vec(), "{what}");
    assert_eq!(&r.json()["allow_credentials"], expected, "{what}");
    assert_eq!(r.json()["rp_id"], HOST, "{what}");
    assert_eq!(r.json()["timeout_ms"], 120_000, "{what}");
    let challenge = r.json()["challenge"]
        .as_str()
        .expect("challenge")
        .to_string();
    assert_eq!(challenge.len(), 43, "{what}");
}

fn assert_no_id(r: &HttpResponse, auths: &[&Authenticator], what: &str) {
    let text = body_text(r);
    for auth in auths {
        assert!(
            !text.contains(&auth.credential_id_b64()),
            "{what}: a credential id in {text}"
        );
    }
    assert!(!text.contains("allow"), "{what}: {text}");
    assert!(!text.contains("transports"), "{what}: {text}");
}

fn capture_logs() -> (LogCapture, tracing::subscriber::DefaultGuard) {
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (capture, guard)
}

// ---------------------------------------------------------------------------------------
// RDB1, RDB2 — the authenticated ceremonies name every registered credential
// ---------------------------------------------------------------------------------------

/// RDB1 (spec D-1, D-3, PE-2): tailnet and Funnel-with-session unlock options carry exactly
/// `{allow_credentials, challenge, rp_id, timeout_ms}`; `allow_credentials` lists every
/// registered credential in store order: the legacy owner record with the default
/// `["internal", "hybrid"]`, the registered device with its stored `["internal", "usb"]`.
#[tokio::test(start_paused = true)]
async fn test_rdb_unlock_options_name_every_registered_credential() {
    let h = Harness::start_with(Options::funnel()).await;
    let device = Authenticator::device(0x61, b"rdb-device-0061");
    let r = register_with(&h, &device, Some("[\"internal\",\"usb\"]")).await;
    assert_result(&r, 200, "registered");
    let expected = json!([
        entry(&h.owner, &["internal", "hybrid"]),
        entry(&device, &["internal", "usb"]),
    ]);
    let session = h.session(IP_A).await;
    for (via, what) in [(Via::Tailnet, "tailnet"), (session, "funnel session")] {
        let r = named_options(&h, "unlock", &via).await;
        assert_named(&r, &expected, what);
        // The named options still drive a normal unlock.
        h.source.set_locked();
        let c = h.challenge_of(&r);
        let body = h.valid_assertion(&device, &c);
        assert_result(
            &h.post_unlock(&via, Some(&body)).await,
            202,
            "unlock_requested",
        );
        h.source.set_unlocked();
        h.advance_ms(MIN_UNLOCK_INTERVAL_MS + 1).await;
    }
    assert_eq!(h.source.unlock_ids().len(), 2);
}

/// RDB2 (spec D-1, D-3, PE-2): the camera options name every registered credential the same
/// way, on the tailnet and over Funnel with a session.
#[tokio::test(start_paused = true)]
async fn test_rdb_camera_options_name_every_registered_credential() {
    let spy = SourceSpy::increasing();
    let h = start_camera(Options::funnel(), camera_on_funnel(), &spy, None).await;
    let device = Authenticator::device(0x62, b"rdb-device-0062");
    let r = register_with(&h, &device, Some("[\"usb\",\"internal\"]")).await;
    assert_result(&r, 200, "registered");
    let expected = json!([
        entry(&h.owner, &["internal", "hybrid"]),
        entry(&device, &["internal", "usb"]),
    ]);
    let session = h.session(IP_A).await;
    for (via, what) in [(Via::Tailnet, "tailnet"), (session, "funnel session")] {
        let r = named_options(&h, "camera", &via).await;
        assert_named(&r, &expected, what);
    }
    assert_eq!(spy.built(), 0, "options never start the camera");
}

// ---------------------------------------------------------------------------------------
// RDB3 — sign-in stays discoverable
// ---------------------------------------------------------------------------------------

/// RDB3 (spec D-2, OD-3, PE-3): with two passkeys and stored transports, the login options
/// stay exactly `{challenge, rp_id, timeout_ms}` without any credential id, `allow` or
/// `transports`, for an anonymous Funnel caller **and** a Funnel caller presenting a valid
/// session cookie (the rotation path); the tailnet caller keeps `403 forbidden` (login is
/// Funnel only) and is never named a credential either.
#[tokio::test(start_paused = true)]
async fn test_rdb_login_options_never_name_a_credential() {
    let h = Harness::start_with(Options::funnel()).await;
    let device = Authenticator::device(0x63, b"rdb-device-0063");
    let r = register_with(&h, &device, Some("[\"internal\",\"hybrid\",\"usb\"]")).await;
    assert_result(&r, 200, "registered");
    assert_eq!(
        h.store_json_value()["passkeys"][1]["transports"],
        json!(["internal", "hybrid", "usb"]),
        "stored transports present"
    );
    let owner = h.owner.clone();

    let tailnet = h.login_options(&Via::Tailnet).await;
    assert_result(&tailnet, 403, "forbidden");
    assert_no_id(&tailnet, &[&owner, &device], "tailnet");

    let session = h.session(IP_A).await;
    for (via, what) in [
        (Via::funnel(IP_B), "anonymous funnel"),
        (Via::funnel(IP_C), "another anonymous funnel"),
        (session.clone(), "funnel with a valid session"),
    ] {
        let r = h.login_options(&via).await;
        assert_eq!(r.status, 200, "{what}: {}", r.result_or_body());
        r.assert_json_body();
        assert_eq!(json_keys(&r), LOGIN_KEYS.to_vec(), "{what}");
        assert_no_id(&r, &[&owner, &device], what);
        assert!(r.header("set-cookie").is_none(), "{what}");
    }
    // The session is still valid and its own unlock options are named (positive control).
    let r = h.unlock_options(&session).await;
    assert_eq!(json_keys(&r), NAMED_KEYS.to_vec());
}

// ---------------------------------------------------------------------------------------
// RDB4 — never reachable without a session; refusals never name a credential
// ---------------------------------------------------------------------------------------

/// RDB4 (spec D-7, V-4): over Funnel without a cookie, with an unknown cookie, and with a
/// session revoked by removing its passkey, both named routes answer `403 login_required`
/// before routing, without any credential id, and issue no challenge (more refusals than
/// `MAX_PENDING_CHALLENGES` leave the pool usable); a valid session is named (control).
#[tokio::test(start_paused = true)]
async fn test_rdb_named_options_refused_without_funnel_session() {
    let phone = Authenticator::new(0x64, b"rdb-second-phone", true);
    let owner = Authenticator::owner();
    let spy = SourceSpy::increasing();
    let h = start_camera(
        Options {
            store: StoreSetup::Passkeys(vec![StoredPasskey::of(&owner), StoredPasskey::of(&phone)]),
            ..Options::funnel()
        },
        camera_on_funnel(),
        &spy,
        None,
    )
    .await;
    let owner_session = h.session(IP_A).await;
    let via = Via::funnel(IP_B);
    let r = h.login(&via, &phone).await;
    assert_result(&r, 200, "logged_in");
    let phone_session = via.with_cookie(&cookie_of(&r));
    // Revocation: the phone passkey leaves the store.
    write_store(&h.store_path, &[StoredPasskey::of(&owner)]);

    let unknown = Via::funnel(IP_C).with_cookie(&format!("__Host-soos_session={}", "A".repeat(43)));
    let cases = [
        (Via::funnel(IP_C), "no cookie"),
        (unknown, "unknown cookie"),
        (phone_session, "revoked session"),
    ];
    for (caller, what) in &cases {
        for route in ["unlock", "camera"] {
            let r = named_options(&h, route, caller).await;
            assert_result(&r, 403, "login_required");
            let text = body_text(&r);
            assert!(
                !text.contains(&owner.credential_id_b64())
                    && !text.contains(&phone.credential_id_b64()),
                "{what} {route}: {text}"
            );
            assert!(!text.contains("allow_credentials"), "{what} {route}");
            assert!(!text.contains("challenge"), "{what} {route}");
        }
    }
    // Six refusals issued no challenge: the still-valid session gets named options of both
    // routes (the store now holds the owner only).
    let expected = json!([entry(&owner, &["internal", "hybrid"])]);
    for route in ["unlock", "camera"] {
        let r = named_options(&h, route, &owner_session).await;
        assert_named(&r, &expected, route);
    }
}

/// RDB4 (spec §3.3, §5): `409 no_passkey`, `429 rate_limited`, `403 unlock_disabled` and `403
/// camera_disabled` bodies are the plain result shape, without `allow_credentials` or any
/// credential id; the same routes name the credential once the refusal cause is gone.
#[tokio::test(start_paused = true)]
async fn test_rdb_refusals_never_name_a_credential() {
    let owner = Authenticator::owner();
    let check = |r: &HttpResponse, status: u16, result: &str| {
        assert_result(r, status, result);
        assert_eq!(r.json(), json!({"result": result}), "{status} {result}");
        let text = body_text(r);
        assert!(!text.contains("allow_credentials"), "{result}");
        assert!(!text.contains(&owner.credential_id_b64()), "{result}");
    };
    {
        // No passkey stored.
        let spy = SourceSpy::increasing();
        let empty = start_camera(
            Options {
                store: StoreSetup::Absent,
                ..Options::funnel()
            },
            camera_on(),
            &spy,
            None,
        )
        .await;
        check(
            &empty.unlock_options(&Via::Tailnet).await,
            409,
            "no_passkey",
        );
        check(
            &camera_options(&empty, &Via::Tailnet).await,
            409,
            "no_passkey",
        );
    }
    {
        // Unlock and camera disabled.
        let off = Harness::start_with(Options {
            allow_unlock: false,
            ..Options::funnel()
        })
        .await;
        check(
            &off.unlock_options(&Via::Tailnet).await,
            403,
            "unlock_disabled",
        );
        let spy = SourceSpy::increasing();
        let no_camera = start_camera(Options::funnel(), CameraConfig::default(), &spy, None).await;
        check(
            &camera_options(&no_camera, &Via::Tailnet).await,
            403,
            "camera_disabled",
        );
    }
    {
        // The failure lockout of the tailnet key.
        let spy = SourceSpy::increasing();
        let h = start_camera(Options::funnel(), camera_on(), &spy, None).await;
        // Control: before the lockout both routes are named.
        let expected = json!([entry(&owner, &["internal", "hybrid"])]);
        for route in ["unlock", "camera"] {
            let r = named_options(&h, route, &Via::Tailnet).await;
            assert_named(&r, &expected, route);
        }
        for _ in 0..5 {
            assert_result(
                &h.post_unlock(&Via::Tailnet, Some("{}")).await,
                400,
                "bad_request",
            );
        }
        check(&h.unlock_options(&Via::Tailnet).await, 429, "rate_limited");
        check(
            &camera_options(&h, &Via::Tailnet).await,
            429,
            "rate_limited",
        );
    }
}

// ---------------------------------------------------------------------------------------
// RDB6 — registration transports
// ---------------------------------------------------------------------------------------

/// RDB6 (spec D-5, §3.2, A-2): `["hybrid","internal","usb","future-x","internal"]` is stored
/// as `["internal","hybrid","usb"]` (unknown ignored, duplicates collapsed, canonical order);
/// the legacy owner record keeps no `transports` key; the next unlock options name the
/// device with the same list.
#[tokio::test(start_paused = true)]
async fn test_rdb_register_stores_allowlisted_transports() {
    let h = Harness::start_with(Options::funnel()).await;
    let device = Authenticator::device(0x65, b"rdb-device-0065");
    let r = register_with(
        &h,
        &device,
        Some("[\"hybrid\",\"internal\",\"usb\",\"future-x\",\"internal\"]"),
    )
    .await;
    assert_result(&r, 200, "registered");
    assert!(!h.code_file_exists(), "single use");
    let store = h.store_json_value();
    let records = store["passkeys"].as_array().unwrap();
    assert_eq!(records.len(), 2);
    assert!(
        records[0].get("transports").is_none(),
        "legacy record untouched"
    );
    assert_eq!(records[1]["credential_id"], device.credential_id_b64());
    assert_eq!(
        records[1]["transports"],
        json!(["internal", "hybrid", "usb"])
    );
    let r = h.unlock_options(&Via::Tailnet).await;
    assert_named(
        &r,
        &json!([
            entry(&h.owner, &["internal", "hybrid"]),
            entry(&device, &["internal", "hybrid", "usb"]),
        ]),
        "after registration",
    );
}

/// RDB6 (spec D-5, V-8): the pre-change body (no `transports` key), `null` and `[]` all
/// register; the record has no transports, so the store file has no `transports` key and
/// the unlock options name it with the default `["internal","hybrid"]`.
#[tokio::test(start_paused = true)]
async fn test_rdb_register_without_transports_is_accepted() {
    let h = Harness::start_with(Options::funnel()).await;
    let plain = Authenticator::device(0x66, b"rdb-device-0066");
    let null = Authenticator::device(0x67, b"rdb-device-0067");
    let empty = Authenticator::device(0x68, b"rdb-device-0068");
    assert_result(&register_with(&h, &plain, None).await, 200, "registered");
    assert_result(
        &register_with(&h, &null, Some("null")).await,
        200,
        "registered",
    );
    assert_result(
        &register_with(&h, &empty, Some("[]")).await,
        200,
        "registered",
    );
    let store = h.store_json_value();
    let records = store["passkeys"].as_array().unwrap();
    assert_eq!(records.len(), 4);
    for record in records {
        assert!(record.get("transports").is_none(), "{record}");
    }
    let r = h.unlock_options(&Via::Tailnet).await;
    assert_named(
        &r,
        &json!([
            entry(&h.owner, &["internal", "hybrid"]),
            entry(&plain, &["internal", "hybrid"]),
            entry(&null, &["internal", "hybrid"]),
            entry(&empty, &["internal", "hybrid"]),
        ]),
        "defaults",
    );
}

/// RDB6 (spec §3.2 bounds, PE-4): 9 entries, a 17-byte entry, a number entry, an object value
/// and a string value are each `400 bad_request`, counted on the tailnet key (the next
/// otherwise valid register verify is `429 rate_limited`); nothing is stored and the code
/// file survives (the body is decoded before the code check). At the bound (8 entries of 16
/// bytes, unknown names ignored) the registration is accepted.
#[tokio::test(start_paused = true)]
async fn test_rdb_register_transports_bounds() {
    let h = Harness::start_with(Options::funnel()).await;
    let device = Authenticator::device(0x69, b"rdb-device-0069");
    let base = registration_body_of(&h, &device, None).await;
    let before = std::fs::read(&h.store_path).unwrap();
    let nine = format!("[{}]", ["\"internal\""; 9].join(","));
    let long = format!("[\"{}\"]", "a".repeat(17));
    for raw in [
        nine.as_str(),
        long.as_str(),
        "[\"internal\",1]",
        "{\"internal\":true}",
        "\"internal\"",
    ] {
        let r = h.register_verify(&with_transports(&base, raw)).await;
        assert_result(&r, 400, "bad_request");
        assert!(
            h.code_file_exists(),
            "{raw}: the code survives a malformed body"
        );
    }
    assert_eq!(
        std::fs::read(&h.store_path).unwrap(),
        before,
        "nothing stored"
    );
    // Five counted failures: the otherwise valid body is refused by the lockout.
    assert_result(&h.register_verify(&base).await, 429, "rate_limited");
    assert_eq!(std::fs::read(&h.store_path).unwrap(), before);

    // At the bound: 8 entries, each at most 16 bytes, one known name.
    h.advance_ms(AUTH_FAILURE_WINDOW_MS).await;
    let mut names = vec!["\"internal\"".to_string()];
    for i in 0..7 {
        let name = format!("x-future-name-{i:02}");
        assert_eq!(name.len(), 16);
        names.push(format!("\"{name}\""));
    }
    let at_bound = format!("[{}]", names.join(","));
    let body = registration_body_of(&h, &device, Some(&at_bound)).await;
    assert_result(&h.register_verify(&body).await, 200, "registered");
    let store = h.store_json_value();
    assert_eq!(store["passkeys"][1]["transports"], json!(["internal"]));
}

// ---------------------------------------------------------------------------------------
// RDB7 — counter updates keep the transports
// ---------------------------------------------------------------------------------------

/// RDB7 (spec §2 `persist_counter`): a device-bound passkey registered with transports
/// unlocks with a higher counter; the record stores the new counter and keeps its
/// transports; the next options still name them.
#[tokio::test(start_paused = true)]
async fn test_rdb_counter_update_keeps_transports() {
    let h = Harness::start_with(Options::funnel()).await;
    h.source.set_locked();
    let device = Authenticator::device(0x6a, b"rdb-device-006a");
    let r = register_with(&h, &device, Some("[\"usb\",\"nfc\"]")).await;
    assert_result(&r, 200, "registered");
    let c = h.challenge_of(&h.unlock_options(&Via::Tailnet).await);
    let body = h.assertion(&device, &c, device.flags(), 7, Some(&OWNER_HANDLE));
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some(&body)).await,
        202,
        "unlock_requested",
    );
    let store = h.store_json_value();
    assert_eq!(store["passkeys"][1]["sign_count"], 7, "counter persisted");
    assert_eq!(store["passkeys"][1]["transports"], json!(["usb", "nfc"]));
    assert!(store["passkeys"][0].get("transports").is_none());
    let r = h.unlock_options(&Via::Tailnet).await;
    assert_eq!(
        r.json()["allow_credentials"][1],
        entry(&device, &["usb", "nfc"])
    );
}

// ---------------------------------------------------------------------------------------
// RDB8 — verification unchanged, nothing logged
// ---------------------------------------------------------------------------------------

/// RDB8 (spec D-6, R-3): after named options, an assertion without `userHandle`, with UV
/// clear, or by a credential that is not registered is `403 passkey_rejected` with no
/// logind call; a valid one still unlocks.
#[tokio::test(start_paused = true)]
async fn test_rdb_assertion_rules_unchanged_with_allow_list() {
    let h = Harness::start_with(Options::funnel()).await;
    h.source.set_locked();
    let owner = h.owner.clone();
    let stranger = Authenticator::device(0x6b, b"rdb-never-registered");
    let expected = json!([entry(&owner, &["internal", "hybrid"])]);
    let session = h.session(IP_D).await;
    for via in [Via::Tailnet, session] {
        let named = |r: &HttpResponse| {
            assert_named(r, &expected, "named options");
            h.challenge_of(r)
        };
        let c = named(&h.unlock_options(&via).await);
        let body = h.assertion(&owner, &c, owner.flags(), 0, None);
        assert_result(
            &h.post_unlock(&via, Some(&body)).await,
            403,
            "passkey_rejected",
        );
        let c = named(&h.unlock_options(&via).await);
        let up_only = (owner.flags() & !UV) | UP;
        let body = h.assertion(&owner, &c, up_only, 0, Some(&OWNER_HANDLE));
        assert_result(
            &h.post_unlock(&via, Some(&body)).await,
            403,
            "passkey_rejected",
        );
        let c = named(&h.unlock_options(&via).await);
        let body = h.valid_assertion(&stranger, &c);
        assert_result(
            &h.post_unlock(&via, Some(&body)).await,
            403,
            "passkey_rejected",
        );
        assert!(h.source.unlock_ids().is_empty(), "never unlocks");
    }
    let c = h.challenge_of(&h.unlock_options(&Via::Tailnet).await);
    let body = h.valid_assertion(&owner, &c);
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some(&body)).await,
        202,
        "unlock_requested",
    );
}

/// RDB8 (spec §7 privacy row): the named unlock and camera options, on both paths, write no
/// credential id (base64url or hex) and no transport list to any log line at TRACE.
#[tokio::test(start_paused = true)]
async fn test_rdb_options_never_log_a_credential_id() {
    let (capture, _guard) = capture_logs();
    let spy = SourceSpy::increasing();
    let h = start_camera(Options::funnel(), camera_on_funnel(), &spy, None).await;
    let device = Authenticator::device(0x6c, b"rdb-logged-device");
    let r = register_with(&h, &device, Some("[\"internal\",\"usb\"]")).await;
    assert_result(&r, 200, "registered");
    let session = h.session(IP_A).await;
    let expected = json!([
        entry(&h.owner, &["internal", "hybrid"]),
        entry(&device, &["internal", "usb"]),
    ]);
    for via in [Via::Tailnet, session] {
        for route in ["unlock", "camera"] {
            let r = named_options(&h, route, &via).await;
            assert_named(&r, &expected, route);
        }
    }
    let text = capture.text();
    assert!(!text.is_empty(), "the capture works");
    for needle in [
        h.owner.credential_id_b64(),
        to_hex(&h.owner.credential_id),
        device.credential_id_b64(),
        to_hex(&device.credential_id),
        "allow_credentials".to_string(),
        "\"usb\"".to_string(),
    ] {
        let shown = &needle[..needle.len().min(12)];
        assert!(!text.contains(&needle), "the log leaks {shown}…");
    }
}

// ---------------------------------------------------------------------------------------
// Auditor additions (AI/auditor_constraints_remote_direct_biometric.md)
// ---------------------------------------------------------------------------------------

/// RDB6 (auditor C-4, C-5): the 16-byte entry bound counts UTF-8 bytes, not characters (nine
/// two-byte characters = 18 bytes ⇒ `400 bad_request`, nothing stored, code file kept), and
/// names match exactly: wrongly cased names and a 16-byte multibyte name are unknown values,
/// ignored, so the registration is accepted with no `transports` key stored and the default
/// list named in the next unlock options.
#[tokio::test(start_paused = true)]
async fn test_rdb_register_transports_byte_bound_and_exact_names() {
    let h = Harness::start_with(Options::funnel()).await;
    let device = Authenticator::device(0x6d, b"rdb-device-006d");
    let base = registration_body_of(&h, &device, None).await;
    let before = std::fs::read(&h.store_path).unwrap();
    let wide = "\u{e9}".repeat(9);
    assert_eq!((wide.len(), wide.chars().count()), (18, 9));
    let r = h
        .register_verify(&with_transports(&base, &format!("[\"{wide}\"]")))
        .await;
    assert_result(&r, 400, "bad_request");
    assert!(h.code_file_exists(), "the code survives a malformed body");
    assert_eq!(
        std::fs::read(&h.store_path).unwrap(),
        before,
        "nothing stored"
    );

    let at_bound = "\u{e9}".repeat(8);
    assert_eq!(at_bound.len(), 16);
    let raw = format!("[\"Internal\",\"HYBRID\",\"USB\",\"{at_bound}\"]");
    let r = register_with(&h, &device, Some(&raw)).await;
    assert_result(&r, 200, "registered");
    let store = h.store_json_value();
    assert!(
        store["passkeys"][1].get("transports").is_none(),
        "wrongly cased names are not stored"
    );
    let r = h.unlock_options(&Via::Tailnet).await;
    assert_eq!(
        r.json()["allow_credentials"][1],
        entry(&device, &["internal", "hybrid"])
    );
}
