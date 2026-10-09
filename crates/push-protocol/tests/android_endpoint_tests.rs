//! Android contract tests of the shared push wire crate `soos-push-protocol` (GitHub #349,
//! ADR 2026-10-09 "Android Support for the `soos-remote` Phone Companion and Web Push",
//! architect spec `AI/architect_spec_remote_android.md` §2.1 and §11.2; matrix RAN8, RAN9).
//!
//! Pure: no file, no socket, no network. Endpoints are real-shaped FCM (`/fcm/send/`, `/wp/`)
//! and Mozilla autopush (`/wpush/v1/`, `/wpush/v2/`) URLs as Chrome, Samsung Internet and
//! Firefox for Android return them from `PushManager.subscribe()`.

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

use soos_push_protocol::{
    EndpointError, PushEndpoint, PushHost, MAX_PUSH_ENDPOINT_BYTES, PUSH_HOSTS,
};

/// Chrome / Samsung Internet legacy FCM path: `/fcm/send/<instance id>:<token>` (`:`, `-`, `_`).
const FCM_SEND: &str = "https://fcm.googleapis.com/fcm/send/eT4wQx9-R_k:APA91bF0q8mD2nK7-vL3_pZ9sXyWcE5tH1uJ6oR4iA8gB2dN0mQ7kS3fV9xC1zY5wT6rE2_uI8oP4aL0jH7gD3sK9nM5bV1cX6zQ2wE8rT4yU0iO7pA3sD";
/// Chrome current FCM Web Push path: `/wp/<token with ':'>`.
const FCM_WP: &str = "https://fcm.googleapis.com/wp/fXy7Kd2pQ1s:APA91bE8rT4yU0iO7pA3sD9fG2hJ5kL1zX6cV8bN3mQ0wE4rT7yU2iO5pA9sD1fG6hJ3kL8zX0cV4bN7mQ2wE5rT";
/// Firefox autopush v1 path: `/wpush/v1/<base64url>`.
const MOZILLA_V1: &str = "https://updates.push.services.mozilla.com/wpush/v1/gAAAAABl9x2Kq7Lm3Np8Rs1Tu4Vw6Xy0Za5Bc9De2Fg7Hi1Jk4Lm8No3Pq6Rs0Tu5Vw9Xy2Za7Bc1De4Fg8Hi3Jk6Lm0No5Pq9Rs2Tu7Vw";
/// Firefox autopush v2 path: `/wpush/v2/<base64url ending '='>`.
const MOZILLA_V2: &str = "https://updates.push.services.mozilla.com/wpush/v2/gAAAAABm1a2Bb3Cc4Dd5Ee6Ff7Gg8Hh9Ii0Jj1Kk2Ll3Mm4Nn5Oo6Pp7Qq8Rr9Ss0Tt1Uu2Vv3Ww4Xx5Yy6Zz7_-aB3cD4eF5gH6iJ7kL8mN9o=";

/// The four Android shapes with their expected host and enum.
fn android_shapes() -> [(&'static str, &'static str, PushHost); 4] {
    [
        (FCM_SEND, "fcm.googleapis.com", PushHost::Google),
        (FCM_WP, "fcm.googleapis.com", PushHost::Google),
        (
            MOZILLA_V1,
            "updates.push.services.mozilla.com",
            PushHost::Mozilla,
        ),
        (
            MOZILLA_V2,
            "updates.push.services.mozilla.com",
            PushHost::Mozilla,
        ),
    ]
}

/// `endpoint` with its last path segment padded (a valid path byte) to exactly `len` bytes.
fn padded_to(endpoint: &str, len: usize) -> String {
    assert!(endpoint.len() <= len, "{} > {len}", endpoint.len());
    let mut out = endpoint.to_string();
    // Keep a trailing '=' last (Mozilla v2 padding) by inserting before it.
    let filler = "A".repeat(len - endpoint.len());
    if out.ends_with('=') {
        out.insert_str(out.len() - 1, &filler);
    } else {
        out.push_str(&filler);
    }
    assert_eq!(out.len(), len);
    out
}

/// RAN9 (spec §11.2): every real-shaped Android endpoint is accepted with the right `host()`,
/// `origin()` (the VAPID `aud`) and `push_host()`; each is accepted at exactly
/// `MAX_PUSH_ENDPOINT_BYTES` and refused (`TooLong`) one byte above.
#[test]
fn test_ran_endpoint_shapes_are_accepted() {
    for (raw, host, push_host) in android_shapes() {
        let endpoint = PushEndpoint::parse(raw).unwrap_or_else(|e| panic!("{raw}: {e:?}"));
        assert_eq!(endpoint.as_str(), raw);
        assert_eq!(endpoint.host(), host, "{raw}");
        assert_eq!(endpoint.origin(), format!("https://{host}"), "{raw}");
        assert_eq!(endpoint.push_host(), push_host, "{raw}");
        assert_eq!(
            format!("{endpoint:?}"),
            "PushEndpoint(<redacted>)",
            "Debug stays redacted"
        );

        let at_bound = padded_to(raw, MAX_PUSH_ENDPOINT_BYTES);
        let parsed = PushEndpoint::parse(&at_bound)
            .unwrap_or_else(|e| panic!("{host} at {MAX_PUSH_ENDPOINT_BYTES} bytes: {e:?}"));
        assert_eq!(parsed.push_host(), push_host);
        assert_eq!(parsed.host(), host);
        let over = padded_to(raw, MAX_PUSH_ENDPOINT_BYTES + 1);
        assert_eq!(
            PushEndpoint::parse(&over),
            Err(EndpointError::TooLong),
            "{host} at {} bytes",
            MAX_PUSH_ENDPOINT_BYTES + 1
        );
    }
}

/// RAN8 (spec §2.1, D-1): `PushHost::ALL` mapped by `name` is exactly `PUSH_HOSTS`;
/// `push_host().name() == host()` for every allowlisted shape; look-alike hosts are still
/// `HostNotAllowed`.
#[test]
fn test_ran_push_host_is_the_single_source_of_the_allowlist() {
    assert_eq!(PushHost::ALL.map(PushHost::name), PUSH_HOSTS);
    assert_eq!(
        PushHost::ALL,
        [PushHost::Apple, PushHost::Google, PushHost::Mozilla]
    );
    assert_eq!(PushHost::Apple.name(), "web.push.apple.com");
    assert_eq!(PushHost::Google.name(), "fcm.googleapis.com");
    assert_eq!(
        PushHost::Mozilla.name(),
        "updates.push.services.mozilla.com"
    );

    let apple = "https://web.push.apple.com/QGuQyavXutnMH8l2ce1fmbpFBjgLv9SQK3-ASsYlvuDH";
    let mut all: Vec<&str> = android_shapes().iter().map(|(raw, _, _)| *raw).collect();
    all.push(apple);
    for raw in all {
        let endpoint = PushEndpoint::parse(raw).unwrap();
        assert_eq!(endpoint.push_host().name(), endpoint.host(), "{raw}");
        assert!(PUSH_HOSTS.contains(&endpoint.host()));
    }
    assert_eq!(
        PushEndpoint::parse(apple).unwrap().push_host(),
        PushHost::Apple
    );

    for look_alike in [
        "https://fcm.googleapis.com.evil.example/fcm/send/abc:def",
        "https://xfcm.googleapis.com/fcm/send/abc:def",
        "https://FCM.googleapis.com/fcm/send/abc:def",
        "https://updates.push.services.mozilla.com.evil.example/wpush/v2/gAAAA",
        "https://push.services.mozilla.com/wpush/v2/gAAAA",
        "https://googleapis.com/wp/abc",
    ] {
        assert_eq!(
            PushEndpoint::parse(look_alike),
            Err(EndpointError::HostNotAllowed),
            "{look_alike}"
        );
    }
}
