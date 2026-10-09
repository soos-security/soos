//! End-to-end contract tests of the live camera view of `soos-remote` (ADR 2026-10-07 "Live
//! Camera View in `soos-remote` Through the Daemon Preview Channel", architect spec
//! `AI/architect_spec_remote_live_camera.md` §7, §8, §10, tests 23–35 and 61; matrix
//! RLC1, RLC6–RLC10, RLC12, RLC14, RLC15; GitHub #345).
//!
//! Same deterministic harness as `auth_server_tests.rs` (frozen paused clock, scripted
//! logind, raw HTTP/1.1 over the Unix socket in a `TempDir`, WebAuthn ceremonies), with the
//! camera wired through `ServerState::with_camera` and the tester-owned scripted
//! `PreviewSource` of `tests/common/camera.rs` instead of the daemon. Virtual time only
//! moves when a test advances it; the blocking-pool JPEG encoder is given real time by the
//! reader (never virtual time). No daemon, no video device, no network.

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

use std::time::Duration;

use serde_json::Value;
use tokio::time::Instant;

use camera::*;
use harness::*;
use passkey::*;
use push::*;
use soos_remote::camera_ipc::PreviewError;
use soos_remote::config::{CameraConfig, CameraWidth};
use soos_remote::{
    CAMERA_FIRST_FRAME_TIMEOUT_MS, CAMERA_RATE_LIMITED_BACKOFF_MS,
    CAMERA_REFUSED_AUDIT_MIN_INTERVAL_MS, CAMERA_SESSION_CHECK_MS, CAMERA_STALL_TIMEOUT_MS,
    CAMERA_VIEW_TOKEN_TTL_MS, MAX_AUTH_BODY_BYTES, PUSH_TEST_TOPIC, RESPONSE_WRITE_TIMEOUT_MS,
};

const IP_A: &str = "203.0.113.10";
const IP_B: &str = "203.0.113.20";

fn assert_result(r: &HttpResponse, status: u16, result: &str) {
    assert_eq!(
        (r.status, r.result_or_body()),
        (status, result.to_string()),
        "expected {status} {result}"
    );
    r.assert_mandatory_headers();
}

fn json_keys(v: &Value) -> Vec<String> {
    let mut keys: Vec<String> = v.as_object().expect("object").keys().cloned().collect();
    keys.sort();
    keys
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

fn count_lines(text: &str, needle: &str) -> usize {
    text.lines().filter(|l| l.contains(needle)).count()
}

/// A camera start whose declared body is never sent: the answer must come before any body
/// byte is read.
async fn held_start(h: &Harness, via: &Via, declared: usize) -> HttpResponse {
    let mut held = h
        .hold(
            via,
            "POST",
            CAMERA_START_PATH,
            &[("X-Soos-Action", "camera-view"), ("Origin", ORIGIN)],
            Some(declared),
        )
        .await;
    held.response_now()
        .await
        .expect("answered without reading the body")
}

/// A view that showed pixels: started, head and first part received.
async fn shown_view(h: &Harness, spy: &SourceSpy, via: &Via) -> ViewClient {
    let path = ready_view(h, via).await;
    let view = open_view(h, spy, via, &path, CAMERA_FIRST_FRAME_TIMEOUT_MS + 1000)
        .await
        .view();
    assert!(!view.parts.is_empty(), "the head comes with the first part");
    view
}

/// After a shown view ended: the slot is idle at once (M13: no cooldown, no `cooldown_ms`
/// field), the source was dropped.
async fn assert_ended_shown(h: &Harness, spy: &SourceSpy) {
    settle().await;
    let state = camera_state(h, &Via::Tailnet).await;
    assert_eq!(
        state["state"], "idle",
        "no cooldown after a shown view: {state}"
    );
    assert!(state.get("cooldown_ms").is_none(), "{state}");
    assert_eq!(spy.dropped(), spy.built(), "every source dropped");
}

/// M13: a new start right after a view ended succeeds at once (a fresh assertion, no wait),
/// then the reservation is stopped again.
async fn assert_immediate_restart(h: &Harness) {
    let _path = ready_view(h, &Via::Tailnet).await;
    assert_result(&camera_stop(h, &Via::Tailnet).await, 200, "stopped");
    assert_eq!(camera_state(h, &Via::Tailnet).await["state"], "idle");
}

/// Exactly `expected_new` more `camera view ended` lines since the last check.
fn check_ended_line(capture: &LogCapture, ended: &mut usize, expected_new: usize) {
    let now = count_lines(&capture.text(), "camera view ended");
    assert_eq!(
        now,
        *ended + expected_new,
        "one `ended` line per shown view"
    );
    *ended = now;
}

/// Waits in real time (no clock movement) for the encoder, without reading the socket.
fn real_wait(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

// ---------------------------------------------------------------------------------------
// Test 23 — off by default
// ---------------------------------------------------------------------------------------

async fn assert_camera_disabled(h: &Harness) {
    let v = camera_state(h, &Via::Tailnet).await;
    assert_eq!(
        json_keys(&v),
        vec![
            "enabled",
            "fps",
            "max_view_s",
            "reachable",
            "state",
            "width"
        ]
    );
    assert_eq!(v["enabled"], false, "{v}");
    assert_eq!(v["reachable"], false, "{v}");
    assert_eq!(v["state"], "disabled", "{v}");
    let head = h.request("HEAD", CAMERA_PATH, &[]).await;
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    assert_result(
        &camera_options(h, &Via::Tailnet).await,
        403,
        "camera_disabled",
    );
    assert_result(
        &held_start(h, &Via::Tailnet, 300).await,
        403,
        "camera_disabled",
    );
    assert_result(&camera_stop(h, &Via::Tailnet).await, 403, "camera_disabled");
    let r = h
        .raw(&stream_request(&Via::Tailnet, &unknown_stream_path()))
        .await;
    assert_result(&r, 403, "camera_disabled");
}

/// Test 23 (RLC1): `camera_view` is off by default; the state says `disabled` and every
/// camera route answers `403 camera_disabled`; a wired factory is never called.
#[tokio::test(start_paused = true)]
async fn test_rlc_camera_disabled_by_default() {
    assert_eq!(
        CameraConfig::default(),
        CameraConfig {
            enabled: false,
            funnel: false,
            max_view_s: 120,
            fps: 5,
            width: CameraWidth::Full,
            quality: 70,
        }
    );
    {
        // The shared harness (no camera wiring at all).
        let h = Harness::start_with(Options::passkeys()).await;
        assert_camera_disabled(&h).await;
    }
    // A factory wired, `camera_view` absent.
    let spy = SourceSpy::increasing();
    let h = start_camera(Options::passkeys(), CameraConfig::default(), &spy, None).await;
    assert_camera_disabled(&h).await;
    let v = camera_state(&h, &Via::Tailnet).await;
    assert_eq!(v["max_view_s"], 120);
    assert_eq!(v["fps"], 5);
    assert_eq!(v["width"], 640);
    assert_eq!(spy.built(), 0, "the factory is never called");
    assert_eq!(spy.started(), 0);
}

// ---------------------------------------------------------------------------------------
// Test 24 — fresh UV assertion of purpose CameraView
// ---------------------------------------------------------------------------------------

/// Test 24 (RLC6, RLC-S3): a valid camera assertion answers `200 view_ready`; an unlock
/// challenge on `/api/camera/start` and a camera challenge on `/api/unlock` are rejected; UV
/// cleared and a replay are rejected; the failures share the unlock lockout.
#[tokio::test(start_paused = true)]
async fn test_rlc_start_requires_fresh_uv_camera_assertion() {
    let spy = SourceSpy::increasing();
    let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
    h.source.set_locked();
    let owner = h.owner.clone();

    // No body.
    assert_result(
        &camera_start(&h, &Via::Tailnet, None).await,
        403,
        "passkey_required",
    );
    // A valid camera assertion.
    let options = camera_options(&h, &Via::Tailnet).await;
    assert_eq!(
        json_keys(&options.json()),
        vec!["challenge", "rp_id", "timeout_ms"]
    );
    let c1 = h.challenge_of(&options);
    let first = h.valid_assertion(&owner, &c1);
    let r = camera_start(&h, &Via::Tailnet, Some(&first)).await;
    assert_result(&r, 200, "view_ready");
    let ready = r.json();
    assert_eq!(
        json_keys(&ready),
        vec![
            "fps",
            "max_view_s",
            "result",
            "stream_path",
            "token_ttl_ms",
            "width"
        ]
    );
    assert_eq!(ready["token_ttl_ms"], CAMERA_VIEW_TOKEN_TTL_MS);
    assert_eq!(ready["max_view_s"], 120);
    assert_eq!(ready["fps"], 5);
    assert_eq!(ready["width"], 640);
    let path = ready["stream_path"].as_str().unwrap().to_string();
    let token = path
        .strip_prefix(CAMERA_STREAM_PREFIX)
        .expect("stream path");
    assert_eq!(token.len(), 43);
    assert!(token
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    // Stopping a pending view frees the slot without cooldown.
    assert_result(&camera_stop(&h, &Via::Tailnet).await, 200, "stopped");
    assert_eq!(camera_state(&h, &Via::Tailnet).await["state"], "idle");
    assert_result(&camera_stop(&h, &Via::Tailnet).await, 200, "no_view");

    // Failure 1: an unlock challenge used on /api/camera/start.
    let cu = h.challenge_of(&h.unlock_options(&Via::Tailnet).await);
    let body = h.valid_assertion(&owner, &cu);
    assert_result(
        &camera_start(&h, &Via::Tailnet, Some(&body)).await,
        403,
        "passkey_rejected",
    );
    // Failure 2: a camera challenge used on /api/unlock.
    let cc = h.challenge_of(&camera_options(&h, &Via::Tailnet).await);
    let body = h.valid_assertion(&owner, &cc);
    assert_result(
        &h.post_unlock(&Via::Tailnet, Some(&body)).await,
        403,
        "passkey_rejected",
    );
    assert!(h.source.unlock_ids().is_empty(), "never unlocks");
    // Failure 3: UV cleared.
    let c = h.challenge_of(&camera_options(&h, &Via::Tailnet).await);
    let up_only = (owner.flags() & !UV) | UP;
    let body = h.assertion(&owner, &c, up_only, 0, Some(&OWNER_HANDLE));
    assert_result(
        &camera_start(&h, &Via::Tailnet, Some(&body)).await,
        403,
        "passkey_rejected",
    );
    // Failure 4: the first assertion replayed.
    assert_result(
        &camera_start(&h, &Via::Tailnet, Some(&first)).await,
        403,
        "passkey_rejected",
    );
    // Failure 5: a malformed body (counted).
    assert_result(
        &camera_start(&h, &Via::Tailnet, Some("{}")).await,
        400,
        "bad_request",
    );
    // Sixth attempt: the shared lockout.
    assert_result(
        &camera_options(&h, &Via::Tailnet).await,
        429,
        "rate_limited",
    );
    assert_result(
        &held_start(&h, &Via::Tailnet, 300).await,
        429,
        "rate_limited",
    );
    assert_result(&h.unlock_options(&Via::Tailnet).await, 429, "rate_limited");
    assert_eq!(spy.built(), 0, "no view ever reached the source");

    // The Funnel path also needs a fresh assertion (a session alone is not enough).
    let spy = SourceSpy::increasing();
    let h = start_camera(Options::funnel(), camera_on_funnel(), &spy, None).await;
    let s = h.session(IP_A).await;
    assert_result(&camera_start(&h, &s, None).await, 403, "passkey_required");
    let c = h.challenge_of(&h.login_options(&s).await);
    let login_body = h.valid_assertion(&h.owner.clone(), &c);
    assert_result(
        &camera_start(&h, &s, Some(&login_body)).await,
        403,
        "passkey_rejected",
    );
    let r = camera_start_with(&h, &s, &h.owner.clone()).await;
    assert_result(&r, 200, "view_ready");
}

// ---------------------------------------------------------------------------------------
// Test 25 — gate order
// ---------------------------------------------------------------------------------------

/// Test 25 (RLC6): CSRF (`403 forbidden`) comes before `camera_disabled`; no body byte is
/// read before the lockout and slot checks; a too large body is `413`.
#[tokio::test(start_paused = true)]
async fn test_rlc_start_gate_order() {
    // CSRF before camera_disabled.
    let spy = SourceSpy::increasing();
    let off = start_camera(Options::passkeys(), CameraConfig::default(), &spy, None).await;
    for (path, good) in [
        (CAMERA_OPTIONS_PATH, "camera-options"),
        (CAMERA_START_PATH, "camera-view"),
    ] {
        // Missing action.
        let r = off
            .send(&Via::Tailnet, "POST", path, &[("Origin", ORIGIN)], None)
            .await;
        assert_result(&r, 403, "forbidden");
        // Wrong action.
        for wrong in ["unlock", "camera-stream", "camera-stop", "lock"] {
            let r = off
                .send(
                    &Via::Tailnet,
                    "POST",
                    path,
                    &[("X-Soos-Action", wrong), ("Origin", ORIGIN)],
                    None,
                )
                .await;
            assert_result(&r, 403, "forbidden");
        }
        // Missing Origin (required on the ceremony routes).
        let r = off
            .send(
                &Via::Tailnet,
                "POST",
                path,
                &[("X-Soos-Action", good)],
                None,
            )
            .await;
        assert_result(&r, 403, "forbidden");
        // Foreign Origin.
        let r = off
            .send(
                &Via::Tailnet,
                "POST",
                path,
                &[("X-Soos-Action", good), ("Origin", "https://evil.example")],
                None,
            )
            .await;
        assert_result(&r, 403, "forbidden");
    }
    // Stream and stop: the custom header is required before camera_disabled.
    let r = off
        .send(&Via::Tailnet, "GET", &unknown_stream_path(), &[], None)
        .await;
    assert_result(&r, 403, "forbidden");
    let r = off
        .send(&Via::Tailnet, "POST", CAMERA_STOP_PATH, &[], None)
        .await;
    assert_result(&r, 403, "forbidden");
    assert_result(
        &held_start(&off, &Via::Tailnet, 300).await,
        403,
        "camera_disabled",
    );

    // A stalled body is never awaited when the slot is busy.
    let spy = SourceSpy::increasing();
    let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
    let _path = ready_view(&h, &Via::Tailnet).await;
    assert_result(
        &held_start(&h, &Via::Tailnet, 300).await,
        409,
        "view_in_progress",
    );
    assert_result(&camera_stop(&h, &Via::Tailnet).await, 200, "stopped");
    // A too large body.
    assert_result(
        &held_start(&h, &Via::Tailnet, MAX_AUTH_BODY_BYTES + 1).await,
        413,
        "body_too_large",
    );
    // The stop route takes no body.
    let r = h
        .send(
            &Via::Tailnet,
            "POST",
            CAMERA_STOP_PATH,
            &[("X-Soos-Action", "camera-stop")],
            Some("{}"),
        )
        .await;
    assert_eq!(r.status, 413, "{}", r.result_or_body());
    assert_eq!(spy.built(), 0);
}

// ---------------------------------------------------------------------------------------
// Test 26 — tailnet only by default
// ---------------------------------------------------------------------------------------

/// Test 26 (RLC7, RLC-S2): with `camera_view_funnel = false` a Funnel session gets `403
/// camera_tailnet_only` on options, start and stream and `reachable: false`; with the key the
/// Funnel view works; an anonymous Funnel caller is `403 login_required`.
#[tokio::test(start_paused = true)]
async fn test_rlc_funnel_is_tailnet_only_by_default() {
    let spy = SourceSpy::increasing();
    let h = start_camera(Options::funnel(), camera_on(), &spy, None).await;
    let s = h.session(IP_A).await;
    let v = camera_state(&h, &s).await;
    assert_eq!(v["enabled"], true, "{v}");
    assert_eq!(v["reachable"], false, "{v}");
    let v = camera_state(&h, &Via::Tailnet).await;
    assert_eq!(v["reachable"], true, "{v}");
    assert_eq!(v["state"], "idle", "{v}");
    assert_result(&camera_options(&h, &s).await, 403, "camera_tailnet_only");
    assert_result(&held_start(&h, &s, 300).await, 403, "camera_tailnet_only");
    let r = h.raw(&stream_request(&s, &unknown_stream_path())).await;
    assert_result(&r, 403, "camera_tailnet_only");
    // Anonymous Funnel.
    let anon = Via::funnel(IP_B);
    assert_result(&h.get_via(&anon, CAMERA_PATH).await, 403, "login_required");
    assert_result(&camera_options(&h, &anon).await, 403, "login_required");
    assert_result(&camera_start(&h, &anon, None).await, 403, "login_required");
    assert_result(&camera_stop(&h, &anon).await, 403, "login_required");
    let r = h.raw(&stream_request(&anon, &unknown_stream_path())).await;
    assert_result(&r, 403, "login_required");
    // The tailnet still works.
    let _ = ready_view(&h, &Via::Tailnet).await;
    assert_result(&camera_stop(&h, &Via::Tailnet).await, 200, "stopped");
    assert_eq!(spy.built(), 0);

    // With `camera_view_funnel = true`.
    let spy = SourceSpy::increasing();
    let h = start_camera(Options::funnel(), camera_on_funnel(), &spy, None).await;
    let s = h.session(IP_A).await;
    assert_eq!(camera_state(&h, &s).await["reachable"], true);
    let view = shown_view(&h, &spy, &s).await;
    assert_eq!(view.head.status, 200);
    assert_result(&camera_options(&h, &anon).await, 403, "login_required");
}

// ---------------------------------------------------------------------------------------
// Test 27 — multipart JPEG transport
// ---------------------------------------------------------------------------------------

/// Test 27 (RLC10): `200 application/octet-stream` (owner-approved amendment 2026-10-07: iOS
/// breaks `fetch` on `multipart/x-mixed-replace`) carrying `soosframe` parts, with the unchanged
/// mandatory headers and no `Content-Length`; parts framed by `Content-Length`, each a
/// baseline JPEG; the trailer on `MaxDuration`.
#[tokio::test(start_paused = true)]
async fn test_rlc_stream_is_multipart_jpeg() {
    let spy = SourceSpy::with_default(|i| {
        if i % 2 == 0 {
            Step::Frame(grey_frame(i + 1))
        } else {
            Step::Frame(rgb_frame(i + 1))
        }
    });
    let config = CameraConfig {
        max_view_s: 10,
        ..camera_on()
    };
    let h = start_camera(Options::passkeys(), config, &spy, None).await;
    let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
    let head = &view.head;
    assert_eq!(head.status, 200);
    assert_eq!(
        head.header("content-type"),
        Some("application/octet-stream")
    );
    head.assert_mandatory_headers();
    assert_eq!(head.header("content-security-policy"), Some(CSP));
    assert!(head.header("content-length").is_none());
    assert!(head.header("transfer-encoding").is_none());
    assert_eq!(sof0(&view.parts[0].jpeg), (48, 64, 1), "Grey first part");
    view.run(&h, &spy, 1000, VIEW_STEP_MS).await;
    assert!(view.parts.len() >= 2);
    assert_eq!(sof0(&view.parts[1].jpeg), (48, 64, 3), "RGB24 second part");
    for part in &view.parts {
        assert!(
            !part.jpeg.windows(2).any(|w| w == [0xFF, 0xC2]),
            "never progressive"
        );
    }
    let ended = view.ends_within(&h, &spy, 10_000, 100).await;
    assert!(ended.is_some(), "the view ends at max_view_s");
    assert!(view.trailer, "the closing boundary follows the last part");
    assert!(view.parts.len() <= 5 * 10 + 1, "{}", view.parts.len());
    assert_eq!(spy.dropped(), spy.built());
}

// ---------------------------------------------------------------------------------------
// Test 28 — stream token
// ---------------------------------------------------------------------------------------

/// Test 28 (RLC8, RLC-S4): the stream token is single use, valid 10 s, bound to its path
/// class and Funnel session; the custom header is required; malformed paths are `404`,
/// `HEAD` is `405` with `Allow: GET`.
#[tokio::test(start_paused = true)]
async fn test_rlc_stream_token_single_use_and_owner_bound() {
    let spy = SourceSpy::increasing();
    let h = start_camera(Options::funnel(), camera_on_funnel(), &spy, None).await;
    let path = ready_view(&h, &Via::Tailnet).await;
    // CSRF: missing or wrong action, the token kept.
    let r = h
        .raw(&request_bytes(&Via::Tailnet, "GET", &path, &[], None))
        .await;
    assert_result(&r, 403, "forbidden");
    let r = h
        .raw(&request_bytes(
            &Via::Tailnet,
            "GET",
            &path,
            &[("X-Soos-Action", "camera-stop")],
            None,
        ))
        .await;
    assert_result(&r, 403, "forbidden");
    // HEAD and POST on a well-formed stream path.
    let r = h
        .raw(&request_bytes(
            &Via::Tailnet,
            "HEAD",
            &path,
            &[("X-Soos-Action", "camera-stream")],
            None,
        ))
        .await;
    assert_eq!(r.status, 405);
    assert_eq!(r.header("allow"), Some("GET"));
    let r = h
        .raw(&request_bytes(
            &Via::Tailnet,
            "POST",
            &path,
            &[("X-Soos-Action", "camera-stream")],
            None,
        ))
        .await;
    assert_eq!(r.status, 405);
    assert_eq!(r.header("allow"), Some("GET"));
    // Malformed token paths.
    let token = path.strip_prefix(CAMERA_STREAM_PREFIX).unwrap().to_string();
    for bad in [
        "/api/camera/stream".to_string(),
        CAMERA_STREAM_PREFIX.to_string(),
        format!("{CAMERA_STREAM_PREFIX}{}", &token[..42]),
        format!("{CAMERA_STREAM_PREFIX}{token}A"),
        format!("{path}/x"),
        format!("{CAMERA_STREAM_PREFIX}{}=", &token[..42]),
    ] {
        let r = h.raw(&stream_request(&Via::Tailnet, &bad)).await;
        assert_eq!(r.status, 404, "{bad}");
    }
    // A Funnel session cannot use a tailnet token; the owner still can (Pending kept).
    let s = h.session(IP_A).await;
    let r = open_view(&h, &spy, &s, &path, 1000).await.refused();
    assert_result(&r, 403, "view_token_rejected");
    let view = open_view(
        &h,
        &spy,
        &Via::Tailnet,
        &path,
        CAMERA_FIRST_FRAME_TIMEOUT_MS,
    )
    .await
    .view();
    // Second use.
    let r = open_view(&h, &spy, &Via::Tailnet, &path, 1000)
        .await
        .refused();
    assert_result(&r, 403, "view_token_rejected");
    drop(view);
    h.advance_ms(100).await;
    assert_eq!(
        camera_state(&h, &Via::Tailnet).await["state"],
        "idle",
        "M13: idle right after the view closed, no cooldown"
    );

    // TTL: a token presented at `expires` is rejected and the slot is idle.
    let path = ready_view(&h, &Via::Tailnet).await;
    h.advance_ms(CAMERA_VIEW_TOKEN_TTL_MS).await;
    let r = open_view(&h, &spy, &Via::Tailnet, &path, VIEW_STEP_MS)
        .await
        .refused();
    assert_result(&r, 403, "view_token_rejected");
    let state = camera_state(&h, &Via::Tailnet).await;
    assert_eq!(state["state"], "idle", "{state}");
    assert!(state.get("cooldown_ms").is_none(), "{state}");

    // Funnel session B cannot use session A's token; neither can the tailnet.
    let a = h.session(IP_A).await;
    let b = h.session(IP_B).await;
    let path = ready_view(&h, &a).await;
    for via in [&b, &Via::Tailnet] {
        let r = open_view(&h, &spy, via, &path, 1000).await.refused();
        assert_result(&r, 403, "view_token_rejected");
    }
    let view = open_view(&h, &spy, &a, &path, CAMERA_FIRST_FRAME_TIMEOUT_MS)
        .await
        .view();
    assert_eq!(view.head.status, 200);
    // The token never appears in the state.
    let state = h.get_via(&a, CAMERA_PATH).await;
    let token = path.strip_prefix(CAMERA_STREAM_PREFIX).unwrap();
    assert!(!String::from_utf8_lossy(&state.body).contains(token));
}

// ---------------------------------------------------------------------------------------
// Test 29 — one view, no cooldown
// ---------------------------------------------------------------------------------------

/// Test 29 (RLC9, migration M13): one global view; `409 view_in_progress` during a view;
/// after a shown view is stopped, a new start succeeds at once (no `429 camera_cooldown`,
/// no `retry_after_ms`, state `idle`, still a fresh assertion per view); a view whose first
/// frame failed leaves the slot idle as well.
#[tokio::test(start_paused = true)]
async fn test_rlc_one_view_and_no_cooldown() {
    let spy = SourceSpy::increasing();
    let h = start_camera(Options::funnel(), camera_on_funnel(), &spy, None).await;
    let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
    assert_eq!(camera_state(&h, &Via::Tailnet).await["state"], "streaming");
    // A second start from either path while the view is open.
    assert_result(
        &held_start(&h, &Via::Tailnet, 300).await,
        409,
        "view_in_progress",
    );
    let s = h.session(IP_A).await;
    assert_result(&held_start(&h, &s, 300).await, 409, "view_in_progress");
    assert_result(&camera_stop(&h, &Via::Tailnet).await, 200, "stopped");
    assert!(view.ends_within(&h, &spy, 2000, 100).await.is_some());
    settle().await;
    // No cooldown: the state is idle and an immediate start is accepted.
    let state = camera_state(&h, &Via::Tailnet).await;
    assert_eq!(state["state"], "idle", "{state}");
    assert!(state.get("cooldown_ms").is_none(), "{state}");
    // The immediate restart (fresh assertion, `200 view_ready`, never `429
    // camera_cooldown`) streams pixels again.
    let mut again = shown_view(&h, &spy, &Via::Tailnet).await;
    assert_result(&camera_stop(&h, &Via::Tailnet).await, 200, "stopped");
    assert!(again.ends_within(&h, &spy, 2000, 100).await.is_some());
    settle().await;
    let _path = ready_view(&h, &Via::Tailnet).await;
    assert_result(&camera_stop(&h, &Via::Tailnet).await, 200, "stopped");

    // A failed first frame leaves the slot idle.
    spy.queue(vec![Step::Fail(PreviewError::Refused)]);
    let path = ready_view(&h, &Via::Tailnet).await;
    let r = open_view(
        &h,
        &spy,
        &Via::Tailnet,
        &path,
        CAMERA_FIRST_FRAME_TIMEOUT_MS,
    )
    .await
    .refused();
    assert_result(&r, 403, "camera_refused");
    let state = camera_state(&h, &Via::Tailnet).await;
    assert_eq!(state["state"], "idle", "{state}");
    let _path = ready_view(&h, &Via::Tailnet).await;
}

// ---------------------------------------------------------------------------------------
// Test 30 — end conditions
// ---------------------------------------------------------------------------------------

/// Test 30 (RLC14, RLC-S10): every end condition ends the stream, frees the slot at once
/// (M13: no cooldown; a stop or `max_view_s` end is followed by an immediate restart),
/// emits `camera view ended` once and drops the source: stop route, `max_view_s`, client close, write stall, Funnel session revoked, shutdown, daemon refusal
/// mid-view, no new frame for 5 s. F3: (a) a stop issued while a part write is blocked ends
/// the view right after that write; (b) a stop during the first-frame phase closes without
/// a head and leaves the slot idle; (c) stray read-half bytes never end the view.
#[tokio::test(start_paused = true)]
async fn test_rlc_view_end_conditions() {
    let (capture, _guard) = capture_logs();
    let mut ended = 0;

    // Stop route.
    {
        let spy = SourceSpy::increasing();
        let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
        let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
        view.run(&h, &spy, 600, VIEW_STEP_MS).await;
        assert_result(&camera_stop(&h, &Via::Tailnet).await, 200, "stopped");
        assert!(
            view.ends_within(&h, &spy, RESPONSE_WRITE_TIMEOUT_MS, 100)
                .await
                .is_some(),
            "stopped within one iteration"
        );
        assert_ended_shown(&h, &spy).await;
        check_ended_line(&capture, &mut ended, 1);
        assert_immediate_restart(&h).await;
    }
    // max_view_s.
    {
        let spy = SourceSpy::increasing();
        let config = CameraConfig {
            max_view_s: 10,
            ..camera_on()
        };
        let h = start_camera(Options::passkeys(), config, &spy, None).await;
        let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
        assert!(
            view.ends_within(&h, &spy, 9_000, 100).await.is_none(),
            "still open before max_view_s"
        );
        assert!(view.ends_within(&h, &spy, 2_000, 100).await.is_some());
        assert!(view.trailer);
        assert_ended_shown(&h, &spy).await;
        check_ended_line(&capture, &mut ended, 1);
        assert_immediate_restart(&h).await;
    }
    // Client close.
    {
        let spy = SourceSpy::increasing();
        let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
        let view = shown_view(&h, &spy, &Via::Tailnet).await;
        drop(view);
        h.advance_ms(100).await;
        assert_ended_shown(&h, &spy).await;
        check_ended_line(&capture, &mut ended, 1);
    }
    // Write stall > 2 s (the reader never reads).
    {
        let spy = SourceSpy::with_default(|i| Step::Frame(big_noise_frame(i + 1)));
        let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
        let _view = shown_view(&h, &spy, &Via::Tailnet).await;
        wait_blocked(&h, &spy, 300, 2, 4_000).await;
        h.step_ms(100, RESPONSE_WRITE_TIMEOUT_MS + 100).await;
        assert_ended_shown(&h, &spy).await;
        check_ended_line(&capture, &mut ended, 1);
    }
    // Funnel session revoked.
    {
        let spy = SourceSpy::increasing();
        let h = start_camera(Options::funnel(), camera_on_funnel(), &spy, None).await;
        let s = h.session(IP_A).await;
        let mut view = shown_view(&h, &spy, &s).await;
        assert_result(&h.logout(&s).await, 200, "logged_out");
        assert!(
            view.ends_within(&h, &spy, CAMERA_SESSION_CHECK_MS + 1000, 100)
                .await
                .is_some(),
            "the view ends at the next session check"
        );
        assert_ended_shown(&h, &spy).await;
        check_ended_line(&capture, &mut ended, 1);
    }
    // Server shutdown.
    {
        let spy = SourceSpy::increasing();
        let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
        let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
        assert!(h.shutdown().await.is_ok());
        for _ in 0..POLL_ROUNDS {
            view.poll(&spy).await;
            if view.eof {
                break;
            }
        }
        assert!(view.eof, "the stream ends on shutdown");
        assert_eq!(spy.dropped(), spy.built());
        check_ended_line(&capture, &mut ended, 1);
    }
    // Daemon refusal mid-view.
    {
        let spy = SourceSpy::increasing();
        spy.queue(vec![Step::Frame(grey_frame(1)), Step::Frame(grey_frame(2))]);
        spy.set_default(|_| Step::Fail(PreviewError::Refused));
        let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
        let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
        assert!(view.ends_within(&h, &spy, 2_000, 100).await.is_some());
        assert_ended_shown(&h, &spy).await;
        check_ended_line(&capture, &mut ended, 1);
    }
    // No new frame for CAMERA_STALL_TIMEOUT_MS.
    {
        let spy = SourceSpy::increasing();
        spy.queue(vec![Step::Frame(grey_frame(1))]);
        spy.set_default(|i| Step::Frame(empty_frame(i + 100)));
        let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
        let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
        assert!(
            view.ends_within(&h, &spy, CAMERA_STALL_TIMEOUT_MS - 500, 100)
                .await
                .is_none(),
            "empty frames are not an error before the stall bound"
        );
        assert!(view.ends_within(&h, &spy, 1_500, 100).await.is_some());
        assert_eq!(view.parts.len(), 1);
        assert_ended_shown(&h, &spy).await;
        check_ended_line(&capture, &mut ended, 1);
    }
    // F3 (a): stop while a part write is blocked.
    {
        let spy = SourceSpy::with_default(|i| Step::Frame(big_noise_frame(i + 1)));
        let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
        let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
        // The reader pauses; the server fills the socket buffer and blocks on a part.
        wait_write_blocked(&h, &spy, 300, 2, 4_000).await;
        // Auditor B7: the view is still open and a write is really blocked (at least two
        // frames handed out beyond the parts read: buffered plus the one being written),
        // not a slow encode and not an already ended view.
        assert_eq!(
            camera_state(&h, &Via::Tailnet).await["state"],
            "streaming",
            "the view must still be open before the stop"
        );
        assert!(
            spy.frames_returned() >= view.parts.len() + 2,
            "a part write must be blocked before the stop ({} frames handed out, {} parts read)",
            spy.frames_returned(),
            view.parts.len()
        );
        let frames_at_stop = spy.frames_returned();
        assert_result(&camera_stop(&h, &Via::Tailnet).await, 200, "stopped");
        // The reader resumes: the blocked part completes, no further part follows.
        let end = view.ends_within(&h, &spy, 3_000, 100).await;
        assert!(end.is_some(), "the stream ends after the blocked write");
        assert!(
            view.parts.len() <= frames_at_stop,
            "no part after the stop ({} parts, {} frames handed out before the stop)",
            view.parts.len(),
            frames_at_stop
        );
        assert_ended_shown(&h, &spy).await;
        check_ended_line(&capture, &mut ended, 1);
    }
    // F3 (b): stop during the first-frame phase.
    {
        let spy = SourceSpy::with_default(|_| Step::Pending);
        let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
        let path = ready_view(&h, &Via::Tailnet).await;
        let mut held = Held {
            stream: connect(&h.path).await,
            buf: Vec::new(),
        };
        held.send(&stream_request(&Via::Tailnet, &path)).await;
        settle().await;
        h.advance_ms(VIEW_STEP_MS).await;
        assert!(spy.started() >= 1, "the first frame is being fetched");
        assert_result(&camera_stop(&h, &Via::Tailnet).await, 200, "stopped");
        h.advance_ms(100).await;
        assert!(
            held.closed_silently_now().await,
            "closed without a head: {:?}",
            String::from_utf8_lossy(&held.buf)
        );
        let state = camera_state(&h, &Via::Tailnet).await;
        assert_eq!(
            state["state"], "idle",
            "idle after a stop without pixels: {state}"
        );
        assert_eq!(spy.dropped(), spy.built());
        check_ended_line(&capture, &mut ended, 0);
        assert_eq!(
            count_lines(&capture.text(), "camera view started"),
            ended,
            "a view without pixels is never `started`"
        );
    }
    // F3 (c): stray read-half bytes never end the view.
    {
        let spy = SourceSpy::increasing();
        let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
        let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
        let before = view.parts.len();
        view.send_stray(&[b'x'; 1000]).await;
        view.run(&h, &spy, 2_000, VIEW_STEP_MS).await;
        assert!(!view.eof, "stray bytes are discarded");
        assert!(view.parts.len() > before + 5, "{}", view.parts.len());
        assert_eq!(camera_state(&h, &Via::Tailnet).await["state"], "streaming");
    }
}

// ---------------------------------------------------------------------------------------
// Test 31 — frame rate, no replay, width
// ---------------------------------------------------------------------------------------

/// Test 31 (RLC9, RLC-S9): at most `fps x seconds + 1` parts; a repeated sequence is never
/// re-sent; `camera_width = 320` halves a 640-wide source; F11: a slow exchange is never
/// followed by a burst of catch-up parts.
#[tokio::test(start_paused = true)]
async fn test_rlc_frame_rate_and_no_replay() {
    // Rate.
    {
        let spy = SourceSpy::increasing();
        let budget = RealBudget::start();
        let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
        let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
        let t0 = view.parts[0].at;
        view.run(&h, &spy, 3_000, VIEW_STEP_MS).await;
        budget.check("frozen-clock window");
        let in_window = view
            .parts
            .iter()
            .filter(|p| p.at.duration_since(t0) <= Duration::from_millis(3_000))
            .count();
        assert!(in_window <= 5 * 3 + 1, "{in_window} parts in 3 s at 5 fps");
        assert!(in_window >= 5, "{in_window} parts in 3 s at 5 fps");
    }
    // No stale replay.
    {
        let spy = SourceSpy::with_default(|_| Step::Frame(grey_frame(7)));
        let budget = RealBudget::start();
        let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
        let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
        view.run(&h, &spy, 3_000, VIEW_STEP_MS).await;
        budget.check("frozen-clock window");
        assert!(spy.started() >= 5, "the source is still asked");
        assert_eq!(view.parts.len(), 1, "a repeated sequence is never re-sent");
    }
    // Half width.
    {
        let spy = SourceSpy::with_default(|i| Step::Frame(grey_vga_frame(i + 1)));
        let config = CameraConfig {
            width: CameraWidth::Half,
            ..camera_on()
        };
        let budget = RealBudget::start();
        let h = start_camera(Options::passkeys(), config, &spy, None).await;
        let state = camera_state(&h, &Via::Tailnet).await;
        assert_eq!(state["width"], 320);
        let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
        view.run(&h, &spy, 600, VIEW_STEP_MS).await;
        budget.check("frozen-clock window");
        for part in &view.parts {
            assert_eq!(sof0(&part.jpeg), (240, 320, 1));
        }
    }
    // F11: no burst catch-up after a slow exchange (3 x the frame interval).
    {
        let spy = SourceSpy::increasing();
        spy.queue(vec![
            Step::Frame(grey_frame(1)),
            Step::Frame(grey_frame(2)),
            Step::after(600, Step::Frame(grey_frame(3))),
        ]);
        spy.set_default(|i| Step::Frame(grey_frame(i + 1)));
        let budget = RealBudget::start();
        let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
        let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
        view.run(&h, &spy, 3_000, VIEW_STEP_MS).await;
        budget.check("frozen-clock window");
        let gaps: Vec<u64> = view
            .parts
            .windows(2)
            .map(|w| w[1].at.duration_since(w[0].at).as_millis() as u64)
            .collect();
        assert!(view.parts.len() >= 8, "{gaps:?}");
        let short = gaps.iter().filter(|g| **g < 100).count();
        assert!(
            short <= 1,
            "at most one part right after the slow exchange: {gaps:?}"
        );
        assert!(
            gaps.iter().any(|g| *g >= 600 - VIEW_STEP_MS),
            "the slow exchange is visible: {gaps:?}"
        );
        let in_window = view.parts.len();
        assert!(in_window <= 5 * 3 + 2, "{gaps:?}");
    }
}

// ---------------------------------------------------------------------------------------
// Test 61 — non-terminal arms never cancel a frame
// ---------------------------------------------------------------------------------------

/// Test 61 (RLC14, F2): Funnel session checks and stray read-half bytes firing while a
/// `next_frame` is pending, and while a part write is blocked, never cancel an exchange and
/// never misalign a part.
#[tokio::test(start_paused = true)]
async fn test_rlc_non_terminal_arms_never_cancel_a_frame() {
    // (i) While a scripted `next_frame` is pending (1.5 s each).
    {
        let spy = SourceSpy::with_default(|i| Step::after(1_500, Step::Frame(grey_frame(i + 1))));
        let budget = RealBudget::start();
        let h = start_camera(Options::funnel(), camera_on_funnel(), &spy, None).await;
        let s = h.session(IP_A).await;
        let mut view = shown_view(&h, &spy, &s).await;
        let mut elapsed = 0;
        // Auditor B8: end off the 1 500 ms exchange boundary.
        while elapsed < 2 * CAMERA_SESSION_CHECK_MS + 2_000 + 700 {
            view.send_stray(b"stray bytes from the client\r\n").await;
            view.run(&h, &spy, 300, 100).await;
            budget.check("frozen-clock window");
            elapsed += 300;
            assert!(!view.eof, "the view is still open at {elapsed} ms");
        }
        assert!(view.parts.len() >= 6, "{}", view.parts.len());
        assert_eq!(spy.cancelled(), 0, "no exchange was cancelled");
        assert!(
            spy.completed() + 1 >= spy.started() && spy.completed() <= spy.started(),
            "every started exchange but at most the current one completed ({} of {})",
            spy.completed(),
            spy.started()
        );
    }
    // (ii) While a part write is blocked (reader paused in 1.4 s windows).
    {
        let spy = SourceSpy::with_default(|i| Step::Frame(big_noise_frame(i + 1)));
        let budget = RealBudget::start();
        let h = start_camera(Options::funnel(), camera_on_funnel(), &spy, None).await;
        let s = h.session(IP_A).await;
        let mut view = shown_view(&h, &spy, &s).await;
        let mut elapsed = 0;
        let mut seen = spy.frames_returned();
        while elapsed < 2 * CAMERA_SESSION_CHECK_MS + 2_000 {
            // Paused window: no read, stray bytes, the clock moves (session checks fire).
            view.send_stray(b"x").await;
            for _ in 0..14 {
                h.advance_ms(100).await;
                elapsed += 100;
                let now = spy.frames_returned();
                if now > seen {
                    seen = now;
                    real_wait(150);
                    settle().await;
                }
            }
            // Resume: drain without moving the clock.
            for _ in 0..20 {
                view.poll(&spy).await;
                real_wait(2);
            }
            assert!(!view.eof, "the view is still open at {elapsed} ms");
            budget.check("test 61(ii) blocked-write windows");
        }
        assert!(view.parts.len() >= 3, "{}", view.parts.len());
        assert_eq!(spy.cancelled(), 0, "no exchange was cancelled");
    }
}

// ---------------------------------------------------------------------------------------
// Test 32 — first-frame failures
// ---------------------------------------------------------------------------------------

/// Test 32 (RLC12, S-2): before the head, a daemon refusal is `403 camera_refused`, no
/// frame within 5 s is `503 camera_unavailable`, an NV12 frame is `503
/// camera_format_unsupported`; empty frames then a real one within 5 s are served.
#[tokio::test(start_paused = true)]
async fn test_rlc_first_frame_failures_are_json_errors() {
    let spy = SourceSpy::increasing();
    let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
    let bound = CAMERA_FIRST_FRAME_TIMEOUT_MS + 2_000;

    spy.queue(vec![Step::Fail(PreviewError::Refused)]);
    let path = ready_view(&h, &Via::Tailnet).await;
    let r = open_view(&h, &spy, &Via::Tailnet, &path, bound)
        .await
        .refused();
    assert_result(&r, 403, "camera_refused");
    r.assert_json_body();
    assert_eq!(spy.dropped(), spy.built());

    spy.set_default(|_| Step::Fail(PreviewError::Io));
    let path = ready_view(&h, &Via::Tailnet).await;
    let started = Instant::now();
    let r = open_view(&h, &spy, &Via::Tailnet, &path, bound)
        .await
        .refused();
    assert_result(&r, 503, "camera_unavailable");
    let took = started.elapsed().as_millis() as u64;
    assert!(
        took + 2 * VIEW_STEP_MS >= CAMERA_FIRST_FRAME_TIMEOUT_MS,
        "repeated Io until the first-frame bound ({took} ms)"
    );
    assert!(spy.started() >= 3, "retried with back-off");
    assert_eq!(spy.dropped(), spy.built());

    spy.set_default(|i| Step::Frame(grey_frame(i + 1)));
    spy.queue(vec![Step::Frame(nv12_frame(500))]);
    let path = ready_view(&h, &Via::Tailnet).await;
    let r = open_view(&h, &spy, &Via::Tailnet, &path, bound)
        .await
        .refused();
    assert_result(&r, 503, "camera_format_unsupported");

    spy.queue(vec![Step::Frame(mjpeg_frame(501))]);
    let path = ready_view(&h, &Via::Tailnet).await;
    let r = open_view(&h, &spy, &Via::Tailnet, &path, bound)
        .await
        .refused();
    assert_result(&r, 503, "camera_format_unsupported");

    spy.queue(vec![
        Step::Frame(empty_frame(600)),
        Step::Frame(empty_frame(601)),
        Step::Frame(empty_frame(602)),
    ]);
    let path = ready_view(&h, &Via::Tailnet).await;
    let view = open_view(&h, &spy, &Via::Tailnet, &path, bound)
        .await
        .view();
    assert_eq!(view.head.status, 200);
    // Each failed view left the slot idle (checked by the successive starts).
}

// ---------------------------------------------------------------------------------------
// Test 33 — RateLimited
// ---------------------------------------------------------------------------------------

/// Test 33 (RLC12): `RateLimited` replies never end the view; the next request is at least
/// `CAMERA_RATE_LIMITED_BACKOFF_MS` later.
#[tokio::test(start_paused = true)]
async fn test_rlc_rate_limited_backs_off_without_ending() {
    let spy = SourceSpy::increasing();
    spy.queue(vec![
        Step::Frame(grey_frame(1)),
        Step::Fail(PreviewError::RateLimited),
        Step::Frame(grey_frame(2)),
        Step::Fail(PreviewError::RateLimited),
        Step::Fail(PreviewError::RateLimited),
        Step::Frame(grey_frame(3)),
    ]);
    spy.set_default(|i| Step::Frame(grey_frame(i + 100)));
    let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
    let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
    view.run(&h, &spy, 3_000, VIEW_STEP_MS).await;
    assert!(!view.eof, "RateLimited never ends the view");
    assert!(view.parts.len() >= 5, "{}", view.parts.len());
    let calls = spy.calls();
    let mut checked = 0;
    for pair in calls.windows(2) {
        if pair[0].error == Some(PreviewError::RateLimited) {
            let gap = pair[1]
                .started_at
                .duration_since(pair[0].ended_at.unwrap())
                .as_millis() as u64;
            assert!(
                gap >= CAMERA_RATE_LIMITED_BACKOFF_MS,
                "back-off after RateLimited: {gap} ms"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 3);
    assert_eq!(camera_state(&h, &Via::Tailnet).await["state"], "streaming");
}

// ---------------------------------------------------------------------------------------
// Test 34 — audit lines
// ---------------------------------------------------------------------------------------

/// Test 34 (RLC15, RLC-S11): exactly `camera view started` and `camera view ended` (INFO)
/// for one view, `camera view refused` (WARN) for a rejected token, at most one refused
/// line per `CAMERA_REFUSED_AUDIT_MIN_INTERVAL_MS`; no field, no token in any line.
#[tokio::test(start_paused = true)]
async fn test_rlc_camera_audit_lines() {
    let (capture, _guard) = capture_logs();
    let spy = SourceSpy::increasing();
    let h = start_camera(Options::passkeys(), camera_on(), &spy, None).await;
    let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
    view.run(&h, &spy, 1_000, VIEW_STEP_MS).await;
    assert_result(&camera_stop(&h, &Via::Tailnet).await, 200, "stopped");
    assert!(view.ends_within(&h, &spy, 2_000, 100).await.is_some());
    settle().await;
    let text = capture.text();
    assert_eq!(count_lines(&text, "camera view started"), 1, "{text}");
    assert_eq!(count_lines(&text, "camera view ended"), 1, "{text}");
    assert_eq!(count_lines(&text, "camera view refused"), 0, "{text}");

    // Two rejected tokens within the interval: one refused line.
    for _ in 0..2 {
        let r = open_view(
            &h,
            &spy,
            &Via::Tailnet,
            &unknown_stream_path(),
            VIEW_STEP_MS,
        )
        .await
        .refused();
        assert_result(&r, 403, "view_token_rejected");
    }
    assert_eq!(count_lines(&capture.text(), "camera view refused"), 1);
    h.advance_ms(CAMERA_REFUSED_AUDIT_MIN_INTERVAL_MS).await;
    let r = open_view(
        &h,
        &spy,
        &Via::Tailnet,
        &unknown_stream_path(),
        VIEW_STEP_MS,
    )
    .await
    .refused();
    assert_result(&r, 403, "view_token_rejected");
    let text = capture.text();
    assert_eq!(count_lines(&text, "camera view refused"), 2, "{text}");

    for (message, level) in [
        ("camera view started", "INFO"),
        ("camera view ended", "INFO"),
        ("camera view refused", "WARN"),
    ] {
        for line in text.lines().filter(|l| l.contains(message)) {
            assert!(line.contains(level), "{line}");
            assert!(
                line.trim_end().ends_with(message),
                "an audit line carries no field: {line}"
            );
        }
    }
    for secret in h.secrets.lock().unwrap().iter() {
        assert!(
            !text.contains(secret.as_str()),
            "a secret leaked into the logs"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Test 35 — no push for a camera view (M14)
// ---------------------------------------------------------------------------------------

/// Subscribes one Apple endpoint with fixture keys `ua` (push must be active).
async fn subscribe_one(h: &Harness, ua: &UaFixture) {
    let view = h.get("/api/push").await;
    assert_eq!(view.json()["state"], "active", "push active");
    let r = h
        .send(
            &Via::Tailnet,
            "POST",
            "/api/push/subscribe",
            &[("X-Soos-Action", "push-subscribe"), ("Origin", ORIGIN)],
            Some(&subscribe_body(&apple_endpoint(1), ua)),
        )
        .await;
    assert_result(&r, 200, "subscribed");
}

/// Test 35 (RLC15, RLC-S11; M14, owner request 2026-10-07): with push active and one
/// subscription, starting, running and ending a camera view sends no Web Push at all (zero
/// calls on the transport spy, whatever the topic), and neither does a view whose first
/// frame failed; the same spy still receives the test notification, so the silence is not
/// an unwired transport.
#[tokio::test(start_paused = true)]
async fn test_rlc_no_push_on_camera_view() {
    // A shown view: started, running, stopped, ended.
    {
        let t = FakeTransport::new();
        let spy = SourceSpy::increasing();
        let h = start_camera(Options::passkeys(), camera_on(), &spy, Some(&t)).await;
        let ua = UaFixture::new(1);
        subscribe_one(&h, &ua).await;
        let mut view = shown_view(&h, &spy, &Via::Tailnet).await;
        view.run(&h, &spy, 2_000, VIEW_STEP_MS).await;
        assert!(!view.eof);
        assert!(view.parts.len() >= 8, "frames flow: {}", view.parts.len());
        assert_eq!(t.count(), 0, "no Web Push while a view runs");
        assert_result(&camera_stop(&h, &Via::Tailnet).await, 200, "stopped");
        assert!(
            view.ends_within(&h, &spy, RESPONSE_WRITE_TIMEOUT_MS, 100)
                .await
                .is_some(),
            "stopped within one iteration"
        );
        assert_ended_shown(&h, &spy).await;
        h.step_ms(500, 10_000).await;
        assert_eq!(t.count(), 0, "no Web Push when a view ends");

        // Positive control: the same transport spy receives the test notification.
        let r = h
            .send(
                &Via::Tailnet,
                "POST",
                "/api/push/test",
                &[("X-Soos-Action", "push-test"), ("Origin", ORIGIN)],
                None,
            )
            .await;
        assert_result(&r, 202, "test_queued");
        h.step_ms(500, 10_000).await;
        assert_eq!(t.count(), 1, "only the test notification");
        assert_eq!(t.calls_with_topic(PUSH_TEST_TOPIC).len(), 1);
    }
    // A view whose first frame failed.
    {
        let t = FakeTransport::new();
        let spy = SourceSpy::increasing();
        spy.queue(vec![Step::Fail(PreviewError::Refused)]);
        let h = start_camera(Options::passkeys(), camera_on(), &spy, Some(&t)).await;
        let ua = UaFixture::new(3);
        subscribe_one(&h, &ua).await;
        let path = ready_view(&h, &Via::Tailnet).await;
        let r = open_view(
            &h,
            &spy,
            &Via::Tailnet,
            &path,
            CAMERA_FIRST_FRAME_TIMEOUT_MS,
        )
        .await
        .refused();
        assert_result(&r, 403, "camera_refused");
        h.step_ms(500, 10_000).await;
        assert_eq!(t.count(), 0, "no Web Push for a refused view");
    }
}
