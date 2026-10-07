//! Contract tests of the shared push wire crate `soos-push-protocol` (ADR 2026-10-06 "Web
//! Push Notifications for Failed-Password Alerts Through a Separate Sender Unit", architect
//! spec `AI/architect_spec_remote_web_push.md` §2, tests 1–6; matrix RMC61, RMC62, RMC68,
//! RMC71).
//!
//! Pure: no file, no socket, no network. Every value is a literal of the spec.

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

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use proptest::prelude::*;
use zeroize::Zeroizing;

use soos_push_protocol::{
    classify_status, decode_reply, decode_request, encode_reply, encode_request, frame_len,
    is_public_address, parse_retry_after, DeliveryReply, DeliveryRequest, EndpointError,
    FrameError, Outcome, PushEndpoint, Urgency, EXIT_CONFIG, EXIT_RUNTIME, MAX_AUTHORIZATION_BYTES,
    MAX_PUSH_BODY_BYTES, MAX_PUSH_ENDPOINT_BYTES, MAX_PUSH_FRAME_BYTES, MAX_PUSH_REPLY_BYTES,
    MAX_PUSH_RESPONSE_BODY_BYTES, MAX_PUSH_RESPONSE_HEADER_BYTES, MAX_PUSH_TTL_S,
    MAX_RESOLVED_ADDRESSES, MAX_RETRY_AFTER_S, MAX_TOPIC_LEN, PUSH_CONNECT_TIMEOUT_MS,
    PUSH_FRAME_IO_TIMEOUT_MS, PUSH_FRAME_VERSION, PUSH_HOSTS, PUSH_SEND_TIMEOUT_MS,
    PUSH_SOCKET_DIR_NAME, PUSH_SOCKET_FILE_NAME,
};

/// A real-shaped Apple endpoint (token path).
const APPLE: &str = "https://web.push.apple.com/QGuQyavXutnMH8l2ce1fmbpFBjgLv9SQK3-ASsYlvuDH_q9n1n5qG9WiCH8n2ehfKdROhvyQrifVAJ9ruRvbSixWmnvbQKbLZRRV4AqYB1JC1R3aC3sfxL6LkW7c6E7aKFmOGFsiQKYEkG8vFI";
/// A real-shaped FCM endpoint (`/fcm/send/<token:with-colon>`).
const FCM: &str = "https://fcm.googleapis.com/fcm/send/dpH5lCsTSSM:APA91bHqjZxM0VImWWqDRN7U0a3AycjUf4O-byuxb_wJsKRaKvV_iKw56s16ekq6FUqoCF7k8nk4IzgBtTxkJwH2ZJV8aXn1fSPhZBvwPCnTXKw6OIxjzLtJ-CEGo6ZcdfGBUpoHzQ1M";
/// A real-shaped Mozilla endpoint (`/wpush/v2/<b64url>`).
const MOZILLA: &str = "https://updates.push.services.mozilla.com/wpush/v2/gAAAAABl3n0yK1b2Qz8sP-xY_u7fT4r3Wq9e6dC5vB0nM2lK1jH8gF7dS6aA5zX4cV3bN2mQ1w=";

/// Constant values of spec §2.1 (the only definition of each lives in the crate).
#[test]
fn test_rwp_protocol_constants_match_the_spec() {
    assert_eq!(
        PUSH_HOSTS,
        [
            "web.push.apple.com",
            "fcm.googleapis.com",
            "updates.push.services.mozilla.com"
        ]
    );
    assert_eq!(MAX_PUSH_ENDPOINT_BYTES, 1024);
    assert_eq!(MAX_PUSH_BODY_BYTES, 4096);
    assert_eq!(MAX_AUTHORIZATION_BYTES, 1024);
    assert_eq!(MAX_PUSH_TTL_S, 2_419_200);
    assert_eq!(MAX_TOPIC_LEN, 32);
    assert_eq!(MAX_PUSH_FRAME_BYTES, 12_288);
    assert_eq!(MAX_PUSH_REPLY_BYTES, 256);
    assert_eq!(PUSH_FRAME_VERSION, 1);
    assert_eq!(PUSH_FRAME_IO_TIMEOUT_MS, 2000);
    assert_eq!(PUSH_SEND_TIMEOUT_MS, 10_000);
    assert_eq!(PUSH_CONNECT_TIMEOUT_MS, 5000);
    assert_eq!(MAX_RETRY_AFTER_S, 300);
    assert_eq!(MAX_RESOLVED_ADDRESSES, 16);
    assert_eq!(MAX_PUSH_RESPONSE_HEADER_BYTES, 16_384);
    assert_eq!(MAX_PUSH_RESPONSE_BODY_BYTES, 1024);
    assert_eq!(PUSH_SOCKET_DIR_NAME, "soos-push");
    assert_eq!(PUSH_SOCKET_FILE_NAME, "push.sock");
    assert_eq!(EXIT_CONFIG, 78);
    assert_eq!(EXIT_RUNTIME, 1);
    // §2.1 compile-time relations, restated.
    const {
        assert!(
            MAX_PUSH_FRAME_BYTES
                >= 4 * MAX_PUSH_BODY_BYTES / 3
                    + MAX_PUSH_ENDPOINT_BYTES
                    + MAX_AUTHORIZATION_BYTES
                    + 512
        );
    };
    const { assert!(PUSH_CONNECT_TIMEOUT_MS < PUSH_SEND_TIMEOUT_MS) };
}

// ---------------------------------------------------------------------------------------
// Test 1 — allowlisted endpoints
// ---------------------------------------------------------------------------------------

/// Test 1 (RMC61, W-5 a): the three push services are accepted with their real path shapes;
/// `host()` is the allowlist element and `origin()` the VAPID `aud` (no path, no slash).
#[test]
fn test_rwp_endpoint_allowlist_accepts_known_push_services() {
    for (raw, host) in [
        (APPLE, "web.push.apple.com"),
        (FCM, "fcm.googleapis.com"),
        (MOZILLA, "updates.push.services.mozilla.com"),
    ] {
        let endpoint = PushEndpoint::parse(raw).unwrap_or_else(|e| panic!("{host}: {e:?}"));
        assert_eq!(endpoint.as_str(), raw);
        assert_eq!(endpoint.host(), host);
        assert!(PUSH_HOSTS.contains(&endpoint.host()));
        assert_eq!(endpoint.origin(), format!("https://{host}"));
        assert_eq!(endpoint.clone(), endpoint, "Clone + Eq");
    }
    // Every path byte of §2.2 is accepted.
    let all = "https://web.push.apple.com/AZaz09-._~/:=+%";
    assert!(PushEndpoint::parse(all).is_ok(), "{all}");
    // Exactly MAX_PUSH_ENDPOINT_BYTES is accepted.
    let prefix = "https://web.push.apple.com/";
    let at_bound = format!(
        "{prefix}{}",
        "a".repeat(MAX_PUSH_ENDPOINT_BYTES - prefix.len())
    );
    assert_eq!(at_bound.len(), 1024);
    assert!(PushEndpoint::parse(&at_bound).is_ok());
}

// ---------------------------------------------------------------------------------------
// Test 2 — SSRF shapes
// ---------------------------------------------------------------------------------------

/// Test 2 (RMC61, W-5 a, §2.2): every refusal with its exact error, in the order of the
/// rules; neither `Display` nor `Debug` of an error, nor `Debug` of a valid endpoint,
/// contains the input.
#[test]
fn test_rwp_endpoint_refuses_ssrf_shapes() {
    let prefix = "https://web.push.apple.com/";
    let over = format!(
        "{prefix}{}",
        "a".repeat(MAX_PUSH_ENDPOINT_BYTES + 1 - prefix.len())
    );
    assert_eq!(over.len(), 1025);
    let cases: Vec<(String, EndpointError)> = vec![
        (String::new(), EndpointError::TooLong),
        (over, EndpointError::TooLong),
        // Bytes outside 0x21..=0x7E anywhere.
        (
            "https://web.push.apple.com/a b".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com/a\u{1}b".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com/a\u{7f}b".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com/caf\u{e9}".into(),
            EndpointError::BadPath,
        ),
        // Scheme.
        (
            "http://web.push.apple.com/abc".into(),
            EndpointError::NotHttps,
        ),
        (
            "HTTPS://web.push.apple.com/abc".into(),
            EndpointError::NotHttps,
        ),
        (
            "Https://web.push.apple.com/abc".into(),
            EndpointError::NotHttps,
        ),
        ("web.push.apple.com/abc".into(), EndpointError::NotHttps),
        (
            "wss://web.push.apple.com/abc".into(),
            EndpointError::NotHttps,
        ),
        // No path at all.
        ("https://web.push.apple.com".into(), EndpointError::BadPath),
        // Authority shapes.
        (
            "https://user@web.push.apple.com/abc".into(),
            EndpointError::UserInfo,
        ),
        (
            "https://user:pw@web.push.apple.com/abc".into(),
            EndpointError::UserInfo,
        ),
        (
            "https://web.push.apple.com:443/abc".into(),
            EndpointError::Port,
        ),
        (
            "https://web.push.apple.com:8443/abc".into(),
            EndpointError::Port,
        ),
        ("https://[::1]/abc".into(), EndpointError::Port),
        (
            "https://17.188.143.78/abc".into(),
            EndpointError::HostNotAllowed,
        ),
        (
            "https://127.0.0.1/abc".into(),
            EndpointError::HostNotAllowed,
        ),
        (
            "https://web.push.apple.com./abc".into(),
            EndpointError::HostNotAllowed,
        ),
        (
            "https://web.push.apple.com.evil.example/abc".into(),
            EndpointError::HostNotAllowed,
        ),
        (
            "https://evil.example/web.push.apple.com".into(),
            EndpointError::HostNotAllowed,
        ),
        (
            "https://WEB.PUSH.APPLE.COM/abc".into(),
            EndpointError::HostNotAllowed,
        ),
        (
            "https://web%2Epush.apple.com/abc".into(),
            EndpointError::HostNotAllowed,
        ),
        (
            "https://push.apple.com/abc".into(),
            EndpointError::HostNotAllowed,
        ),
        (
            "https://api.push.apple.com/abc".into(),
            EndpointError::HostNotAllowed,
        ),
        (
            "https://wns2-by3p.notify.windows.com/w/?token=x".into(),
            EndpointError::HostNotAllowed,
        ),
        // Path shapes.
        (
            "https://web.push.apple.com/abc?q".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com/abc#f".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com/a/../b".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com/a/..".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com/./b".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com/a/./b".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com//b".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com/a//b".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com/a\\b".into(),
            EndpointError::BadPath,
        ),
        ("https://web.push.apple.com/".into(), EndpointError::BadPath),
        (
            "https://web.push.apple.com/a\"b".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com/a<b".into(),
            EndpointError::BadPath,
        ),
        (
            "https://web.push.apple.com/a@b".into(),
            EndpointError::BadPath,
        ),
    ];
    for (raw, expected) in &cases {
        let err = PushEndpoint::parse(raw).map(|e| e.as_str().to_string());
        assert_eq!(err, Err(*expected), "{raw:?}");
        let display = expected.to_string();
        let debug = format!("{expected:?}");
        for needle in ["evil", "17.188", "pw@", "8443", "::1", "abc", "%2E"] {
            assert!(!display.contains(needle), "{display}");
            assert!(!debug.contains(needle), "{debug}");
        }
    }
    // Every variant has a non-empty fixed text.
    for e in [
        EndpointError::TooLong,
        EndpointError::NotHttps,
        EndpointError::UserInfo,
        EndpointError::Port,
        EndpointError::HostNotAllowed,
        EndpointError::BadPath,
    ] {
        assert!(!e.to_string().is_empty());
    }
    // A valid endpoint is a capability URL: its Debug is redacted.
    let debug = format!("{:?}", PushEndpoint::parse(APPLE).unwrap());
    assert_eq!(debug, "PushEndpoint(<redacted>)");
}

// ---------------------------------------------------------------------------------------
// Test 3 — public addresses
// ---------------------------------------------------------------------------------------

fn v4(text: &str) -> IpAddr {
    IpAddr::V4(text.parse::<Ipv4Addr>().unwrap())
}

fn v6(text: &str) -> IpAddr {
    IpAddr::V6(text.parse::<Ipv6Addr>().unwrap())
}

/// First and last address of an IPv4 prefix.
fn v4_range(base: &str, len: u32) -> (IpAddr, IpAddr) {
    let first = u32::from(base.parse::<Ipv4Addr>().unwrap());
    let mask = if len == 0 { 0 } else { u32::MAX << (32 - len) };
    assert_eq!(first & mask, first, "{base}/{len} is a network address");
    let last = first | !mask;
    (
        IpAddr::V4(Ipv4Addr::from(first)),
        IpAddr::V4(Ipv4Addr::from(last)),
    )
}

/// First and last address of an IPv6 prefix.
fn v6_range(base: &str, len: u32) -> (IpAddr, IpAddr) {
    let first = u128::from(base.parse::<Ipv6Addr>().unwrap());
    let mask = if len == 0 {
        0
    } else {
        u128::MAX << (128 - len)
    };
    assert_eq!(first & mask, first, "{base}/{len} is a network address");
    let last = first | !mask;
    (
        IpAddr::V6(Ipv6Addr::from(first)),
        IpAddr::V6(Ipv6Addr::from(last)),
    )
}

/// Test 3 (RMC62, W-5 b, §2.3): the first and last address of every refused range are
/// refused; the push services' real addresses and the neighbours of the ranges are public.
#[test]
fn test_rwp_public_address_predicate() {
    let v4_refused = [
        ("0.0.0.0", 8),
        ("10.0.0.0", 8),
        ("100.64.0.0", 10),
        ("127.0.0.0", 8),
        ("169.254.0.0", 16),
        ("172.16.0.0", 12),
        ("192.0.0.0", 24),
        ("192.0.2.0", 24),
        ("192.88.99.0", 24),
        ("192.168.0.0", 16),
        ("198.18.0.0", 15),
        ("198.51.100.0", 24),
        ("203.0.113.0", 24),
        ("224.0.0.0", 4),
        ("240.0.0.0", 4),
    ];
    for (base, len) in v4_refused {
        let (first, last) = v4_range(base, len);
        assert!(!is_public_address(first), "{first} ({base}/{len})");
        assert!(!is_public_address(last), "{last} ({base}/{len})");
    }
    let v6_refused = [
        ("::", 128),
        ("::1", 128),
        ("::ffff:0:0", 96),
        ("64:ff9b::", 96),
        ("64:ff9b:1::", 48),
        ("100::", 64),
        ("2001::", 23),
        ("2001:db8::", 32),
        ("2002::", 16),
        ("fc00::", 7),
        ("fe80::", 10),
        ("fec0::", 10),
        ("ff00::", 8),
        // Outside 2000::/3.
        ("::", 3),
        ("4000::", 2),
        ("8000::", 1),
    ];
    for (base, len) in v6_refused {
        let (first, last) = v6_range(base, len);
        assert!(!is_public_address(first), "{first} ({base}/{len})");
        assert!(!is_public_address(last), "{last} ({base}/{len})");
    }
    for refused in [
        v4("100.100.100.100"),
        v4("255.255.255.255"),
        v6("fd7a:115c:a1e0::53"),
        v6("::ffff:17.188.143.78"),
        v6("64:ff9b::1101:1"),
        v6("2002:1111:1111::1"),
        v6("2001:0:1::1"),
        v6("1fff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
        v6("::2"),
        v6("::127.0.0.1"),
    ] {
        assert!(!is_public_address(refused), "{refused}");
    }
    for public in [
        v4("17.188.143.78"),
        v4("216.239.38.55"),
        v4("199.232.169.91"),
        v4("1.1.1.1"),
        v4("9.255.255.255"),
        v4("11.0.0.0"),
        v4("100.63.255.255"),
        v4("100.128.0.0"),
        v4("172.15.255.255"),
        v4("172.32.0.0"),
        v4("192.0.1.0"),
        v4("192.167.255.255"),
        v4("198.17.255.255"),
        v4("198.20.0.0"),
        v4("223.255.255.255"),
        v6("2a04:4e42:6a::347"),
        v6("2001:200::1"),
        v6("2003::1"),
        v6("3fff:ffff:ffff:ffff:ffff:ffff:ffff:fffe"),
        v6("2620:149:a44::1"),
    ] {
        assert!(is_public_address(public), "{public}");
    }
}

// ---------------------------------------------------------------------------------------
// Test 4 — frames
// ---------------------------------------------------------------------------------------

const AUTHORIZATION: &str = "vapid t=eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.e30.c2ln, k=BAAA";

fn request() -> DeliveryRequest {
    DeliveryRequest {
        endpoint: PushEndpoint::parse("https://web.push.apple.com/token-1").unwrap(),
        authorization: Zeroizing::new(AUTHORIZATION.to_string()),
        ttl_s: 43_200,
        urgency: Urgency::High,
        topic: Some("soos-alerts".to_string()),
        body: Zeroizing::new(vec![0xfb, 0xff, 0x00, 0x01]),
    }
}

/// The exact JSON payload of [`request`].
const REQUEST_JSON: &str = "{\"v\":1,\"endpoint\":\"https://web.push.apple.com/token-1\",\"authorization\":\"vapid t=eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.e30.c2ln, k=BAAA\",\"ttl\":43200,\"urgency\":\"high\",\"topic\":\"soos-alerts\",\"body\":\"-_8AAQ\"}";

/// A request payload with one key replaced (the value is raw JSON).
fn request_json_with(key: &str, value: &str) -> String {
    let mut fields: Vec<(String, String)> = vec![
        ("v".into(), "1".into()),
        (
            "endpoint".into(),
            "\"https://web.push.apple.com/token-1\"".into(),
        ),
        ("authorization".into(), format!("\"{AUTHORIZATION}\"")),
        ("ttl".into(), "43200".into()),
        ("urgency".into(), "\"high\"".into()),
        ("topic".into(), "\"soos-alerts\"".into()),
        ("body".into(), "\"-_8AAQ\"".into()),
    ];
    for field in &mut fields {
        if field.0 == key {
            field.1 = value.to_string();
        }
    }
    let inner: Vec<String> = fields.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect();
    format!("{{{}}}", inner.join(","))
}

fn request_json_without(key: &str) -> String {
    let full = request_json_with("", "");
    let value: serde_json_lite::Map = serde_json_lite::parse(&full);
    serde_json_lite::render_without(&value, key)
}

/// Tiny local helpers to drop one key from the flat request object (keeps this test free of
/// a JSON dependency on the crate's own types).
mod serde_json_lite {
    pub type Map = Vec<(String, String)>;

    pub fn parse(text: &str) -> Map {
        let inner = &text[1..text.len() - 1];
        let mut out = Vec::new();
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escaped = false;
        let mut start = 0;
        let bytes = inner.as_bytes();
        let mut parts = Vec::new();
        for (i, &b) in bytes.iter().enumerate() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    in_string = false;
                }
                continue;
            }
            match b {
                b'"' => in_string = true,
                b'{' | b'[' => depth += 1,
                b'}' | b']' => depth -= 1,
                b',' if depth == 0 => {
                    parts.push(&inner[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        parts.push(&inner[start..]);
        for part in parts {
            let colon = part.find("\":").unwrap();
            out.push((part[1..colon].to_string(), part[colon + 2..].to_string()));
        }
        out
    }

    pub fn render_without(map: &Map, key: &str) -> String {
        let inner: Vec<String> = map
            .iter()
            .filter(|(k, _)| k != key)
            .map(|(k, v)| format!("\"{k}\":{v}"))
            .collect();
        format!("{{{}}}", inner.join(","))
    }
}

/// Test 4 (RMC71, §2.4): exact request and reply JSON, big-endian prefix, round trips; the
/// frame bound and every field rule of the decoders (each re-validates what the encoder
/// validated) with its exact `FrameError`.
#[test]
fn test_rwp_frame_round_trip_and_bounds() {
    // Request: prefix + exact JSON, round trip.
    let frame = encode_request(&request()).unwrap();
    assert_eq!(
        &frame[..4],
        &(REQUEST_JSON.len() as u32).to_be_bytes(),
        "4-byte big-endian length prefix"
    );
    assert_eq!(std::str::from_utf8(&frame[4..]).unwrap(), REQUEST_JSON);
    let decoded = decode_request(&frame[4..]).unwrap();
    assert!(decoded == request(), "request round trip");
    let without_topic = DeliveryRequest {
        topic: None,
        urgency: Urgency::VeryLow,
        ..request()
    };
    let frame = encode_request(&without_topic).unwrap();
    let text = std::str::from_utf8(&frame[4..]).unwrap();
    assert!(text.contains("\"topic\":null"), "{text}");
    assert!(text.contains("\"urgency\":\"very-low\""), "{text}");
    assert!(decode_request(&frame[4..]).unwrap() == without_topic);
    for (urgency, text) in [
        (Urgency::Low, "low"),
        (Urgency::Normal, "normal"),
        (Urgency::High, "high"),
    ] {
        let frame = encode_request(&DeliveryRequest {
            urgency,
            ..request()
        })
        .unwrap();
        assert!(std::str::from_utf8(&frame[4..])
            .unwrap()
            .contains(&format!("\"urgency\":\"{text}\"")));
    }

    // Reply: exact JSON, round trip.
    let reply = DeliveryReply {
        outcome: Outcome::Delivered,
        status: Some(201),
        retry_after_s: None,
    };
    let frame = encode_reply(&reply).unwrap();
    let json = "{\"v\":1,\"outcome\":\"delivered\",\"status\":201,\"retry_after_s\":null}";
    assert_eq!(&frame[..4], &(json.len() as u32).to_be_bytes());
    assert_eq!(std::str::from_utf8(&frame[4..]).unwrap(), json);
    assert_eq!(decode_reply(&frame[4..]).unwrap(), reply);
    for reply in [
        DeliveryReply {
            outcome: Outcome::Retry,
            status: Some(429),
            retry_after_s: Some(120),
        },
        DeliveryReply {
            outcome: Outcome::Refused,
            status: None,
            retry_after_s: None,
        },
        DeliveryReply {
            outcome: Outcome::Gone,
            status: Some(410),
            retry_after_s: None,
        },
        DeliveryReply {
            outcome: Outcome::Rejected,
            status: Some(301),
            retry_after_s: None,
        },
        DeliveryReply {
            outcome: Outcome::Retry,
            status: None,
            retry_after_s: Some(MAX_RETRY_AFTER_S),
        },
    ] {
        let frame = encode_reply(&reply).unwrap();
        assert!(frame.len() - 4 <= MAX_PUSH_REPLY_BYTES);
        assert_eq!(decode_reply(&frame[4..]).unwrap(), reply);
    }
    let frame = encode_reply(&DeliveryReply {
        outcome: Outcome::Refused,
        status: None,
        retry_after_s: None,
    })
    .unwrap();
    assert!(std::str::from_utf8(&frame[4..])
        .unwrap()
        .contains("\"outcome\":\"refused\""));

    // Frame length bound, before any allocation.
    let max = MAX_PUSH_FRAME_BYTES as u32;
    assert_eq!(
        frame_len(max.to_be_bytes(), MAX_PUSH_FRAME_BYTES),
        Ok(MAX_PUSH_FRAME_BYTES)
    );
    assert_eq!(
        frame_len((max + 1).to_be_bytes(), MAX_PUSH_FRAME_BYTES),
        Err(FrameError::TooLarge)
    );
    assert_eq!(
        frame_len(u32::MAX.to_be_bytes(), MAX_PUSH_FRAME_BYTES),
        Err(FrameError::TooLarge)
    );
    assert_eq!(frame_len([0, 0, 1, 0], MAX_PUSH_REPLY_BYTES), Ok(256));
    assert_eq!(
        frame_len([0, 0, 1, 1], MAX_PUSH_REPLY_BYTES),
        Err(FrameError::TooLarge)
    );
    // The largest valid request fits in one frame (§2.1 relation).
    let prefix = "https://web.push.apple.com/";
    let largest = DeliveryRequest {
        endpoint: PushEndpoint::parse(&format!(
            "{prefix}{}",
            "a".repeat(MAX_PUSH_ENDPOINT_BYTES - prefix.len())
        ))
        .unwrap(),
        authorization: Zeroizing::new(format!(
            "vapid t={}",
            "a".repeat(MAX_AUTHORIZATION_BYTES - "vapid t=".len())
        )),
        ttl_s: MAX_PUSH_TTL_S,
        urgency: Urgency::VeryLow,
        topic: Some("a".repeat(MAX_TOPIC_LEN)),
        body: Zeroizing::new(vec![0xff; MAX_PUSH_BODY_BYTES]),
    };
    let frame = encode_request(&largest).unwrap();
    assert!(frame.len() - 4 <= MAX_PUSH_FRAME_BYTES, "{}", frame.len());
    assert_eq!(
        frame_len(frame[..4].try_into().unwrap(), MAX_PUSH_FRAME_BYTES),
        Ok(frame.len() - 4)
    );
    assert!(decode_request(&frame[4..]).unwrap() == largest);

    // Encoder refusals (the encoder validates too).
    for (bad, expected) in [
        (
            DeliveryRequest {
                ttl_s: 0,
                ..request()
            },
            FrameError::Ttl,
        ),
        (
            DeliveryRequest {
                ttl_s: MAX_PUSH_TTL_S + 1,
                ..request()
            },
            FrameError::Ttl,
        ),
        (
            DeliveryRequest {
                body: Zeroizing::new(Vec::new()),
                ..request()
            },
            FrameError::Body,
        ),
        (
            DeliveryRequest {
                body: Zeroizing::new(vec![1; MAX_PUSH_BODY_BYTES + 1]),
                ..request()
            },
            FrameError::Body,
        ),
        (
            DeliveryRequest {
                topic: Some("a".repeat(MAX_TOPIC_LEN + 1)),
                ..request()
            },
            FrameError::Topic,
        ),
        (
            DeliveryRequest {
                topic: Some(String::new()),
                ..request()
            },
            FrameError::Topic,
        ),
        (
            DeliveryRequest {
                authorization: Zeroizing::new("Bearer x".to_string()),
                ..request()
            },
            FrameError::Authorization,
        ),
    ] {
        assert_eq!(encode_request(&bad).map(|_| ()), Err(expected));
    }

    // Decoder refusals.
    let long_auth = format!(
        "\"vapid t={}\"",
        "a".repeat(MAX_AUTHORIZATION_BYTES + 1 - "vapid t=".len())
    );
    let long_body = format!("\"{}\"", "A".repeat((MAX_PUSH_BODY_BYTES + 1) * 4 / 3 + 1));
    let topic33 = format!("\"{}\"", "a".repeat(MAX_TOPIC_LEN + 1));
    let ttl_over = (MAX_PUSH_TTL_S + 1).to_string();
    let decode_cases: Vec<(String, FrameError)> = vec![
        (
            request_json_with("v", "1").replacen('{', "{\"extra\":1,", 1),
            FrameError::Json,
        ),
        (request_json_without("topic"), FrameError::Json),
        (request_json_without("body"), FrameError::Json),
        (request_json_without("v"), FrameError::Json),
        (request_json_with("v", "2"), FrameError::Version),
        (request_json_with("v", "0"), FrameError::Version),
        (request_json_with("body", "\"-_8AAQ==\""), FrameError::Body),
        (request_json_with("body", "\"+/8AAQ\""), FrameError::Body),
        (request_json_with("body", "\"\""), FrameError::Body),
        (request_json_with("body", &long_body), FrameError::Body),
        (
            request_json_with("authorization", &long_auth),
            FrameError::Authorization,
        ),
        (
            request_json_with("authorization", "\"Bearer abc\""),
            FrameError::Authorization,
        ),
        (
            request_json_with("authorization", "\"vapid t=a\\u0001b, k=c\""),
            FrameError::Authorization,
        ),
        (
            request_json_with("authorization", "\"\""),
            FrameError::Authorization,
        ),
        (request_json_with("ttl", "0"), FrameError::Ttl),
        (request_json_with("ttl", &ttl_over), FrameError::Ttl),
        (request_json_with("topic", &topic33), FrameError::Topic),
        (
            request_json_with("topic", "\"soos+alerts\""),
            FrameError::Topic,
        ),
        (request_json_with("topic", "\"\""), FrameError::Topic),
        (
            request_json_with("endpoint", "\"https://evil.example/x\""),
            FrameError::Endpoint(EndpointError::HostNotAllowed),
        ),
        (
            request_json_with("endpoint", "\"http://web.push.apple.com/x\""),
            FrameError::Endpoint(EndpointError::NotHttps),
        ),
        (request_json_with("urgency", "\"urgent\""), FrameError::Json),
        ("not json".to_string(), FrameError::Json),
        (String::new(), FrameError::Json),
    ];
    for (payload, expected) in &decode_cases {
        assert_eq!(
            decode_request(payload.as_bytes()).map(|_| ()),
            Err(*expected),
            "{payload}"
        );
    }
    // The exact long body value is over the bound, a body of exactly the bound passes.
    let at_bound = encode_request(&DeliveryRequest {
        body: Zeroizing::new(vec![7; MAX_PUSH_BODY_BYTES]),
        ..request()
    })
    .unwrap();
    assert!(decode_request(&at_bound[4..]).is_ok());

    let reply_cases = [
        (
            "{\"v\":1,\"outcome\":\"delivered\",\"status\":99,\"retry_after_s\":null}",
            FrameError::Reply,
        ),
        (
            "{\"v\":1,\"outcome\":\"delivered\",\"status\":600,\"retry_after_s\":null}",
            FrameError::Reply,
        ),
        (
            "{\"v\":1,\"outcome\":\"delivered\",\"status\":201,\"retry_after_s\":5}",
            FrameError::Reply,
        ),
        (
            "{\"v\":1,\"outcome\":\"retry\",\"status\":429,\"retry_after_s\":0}",
            FrameError::Reply,
        ),
        (
            "{\"v\":1,\"outcome\":\"retry\",\"status\":429,\"retry_after_s\":301}",
            FrameError::Reply,
        ),
        (
            "{\"v\":2,\"outcome\":\"delivered\",\"status\":201,\"retry_after_s\":null}",
            FrameError::Version,
        ),
        (
            "{\"v\":1,\"outcome\":\"delivered\",\"status\":201}",
            FrameError::Json,
        ),
        (
            "{\"v\":1,\"outcome\":\"delivered\",\"status\":201,\"retry_after_s\":null,\"x\":1}",
            FrameError::Json,
        ),
        (
            "{\"v\":1,\"outcome\":\"ok\",\"status\":201,\"retry_after_s\":null}",
            FrameError::Json,
        ),
    ];
    for (payload, expected) in reply_cases {
        assert_eq!(decode_reply(payload.as_bytes()), Err(expected), "{payload}");
    }
    assert_eq!(
        decode_reply(b"{\"v\":1,\"outcome\":\"retry\",\"status\":503,\"retry_after_s\":300}"),
        Ok(DeliveryReply {
            outcome: Outcome::Retry,
            status: Some(503),
            retry_after_s: Some(300)
        })
    );
    // Encoding an invalid reply is refused as well.
    assert_eq!(
        encode_reply(&DeliveryReply {
            outcome: Outcome::Delivered,
            status: Some(201),
            retry_after_s: Some(5),
        }),
        Err(FrameError::Reply)
    );
    // Error texts are fixed.
    for e in [
        FrameError::Truncated,
        FrameError::TooLarge,
        FrameError::Json,
        FrameError::Version,
        FrameError::Endpoint(EndpointError::Port),
        FrameError::Authorization,
        FrameError::Ttl,
        FrameError::Topic,
        FrameError::Body,
        FrameError::Reply,
    ] {
        let text = e.to_string();
        assert!(!text.is_empty());
        assert!(!text.contains("vapid"), "{text}");
    }
}

// ---------------------------------------------------------------------------------------
// Test 5 — decoders never panic
// ---------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Test 5 (RMC71, §2.4): arbitrary bytes never panic any decoder or the endpoint parser.
    #[test]
    fn test_rwp_frame_decoding_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..16_384)) {
        let _ = decode_request(&bytes);
        let _ = decode_reply(&bytes);
        let mut prefix = [0u8; 4];
        for (i, b) in bytes.iter().take(4).enumerate() {
            prefix[i] = *b;
        }
        let _ = frame_len(prefix, MAX_PUSH_FRAME_BYTES);
        let _ = frame_len(prefix, MAX_PUSH_REPLY_BYTES);
        let text = String::from_utf8_lossy(&bytes);
        let _ = PushEndpoint::parse(&text);
    }

    /// Companion of test 5: JSON-shaped strings around the request keys never panic.
    #[test]
    fn test_rwp_frame_decoding_never_panics_on_json_shapes(
        endpoint in "[ -~]{0,64}",
        auth in "[ -~]{0,64}",
        ttl in any::<i64>(),
        topic in "[ -~]{0,40}",
        body in "[ -~]{0,64}",
    ) {
        let payload = format!(
            "{{\"v\":1,\"endpoint\":{endpoint:?},\"authorization\":{auth:?},\"ttl\":{ttl},\"urgency\":\"high\",\"topic\":{topic:?},\"body\":{body:?}}}"
        );
        let _ = decode_request(payload.as_bytes());
        let _ = PushEndpoint::parse(&format!("https://web.push.apple.com/{endpoint}"));
    }
}

// ---------------------------------------------------------------------------------------
// Test 6 — status classification
// ---------------------------------------------------------------------------------------

/// Test 6 (RMC68, §2.5): the classification table with its edges; `Retry-After` is
/// delta-seconds only, 1..=10 digits, 0 ignored, capped at 300.
#[test]
fn test_rwp_status_classification() {
    let outcome = |status: u16| classify_status(status, None).outcome;
    for status in [200, 201, 202, 204, 299] {
        assert_eq!(outcome(status), Outcome::Delivered, "{status}");
    }
    for status in [404, 410] {
        assert_eq!(outcome(status), Outcome::Gone, "{status}");
    }
    for status in [429, 500, 502, 503, 599] {
        assert_eq!(outcome(status), Outcome::Retry, "{status}");
    }
    for status in [
        100, 101, 199, 300, 301, 302, 307, 308, 399, 400, 401, 403, 405, 409, 411, 413, 415, 499,
    ] {
        assert_eq!(outcome(status), Outcome::Rejected, "{status}");
    }
    // The status is carried; Retry-After only with Retry.
    assert_eq!(
        classify_status(201, Some(b"120")),
        DeliveryReply {
            outcome: Outcome::Delivered,
            status: Some(201),
            retry_after_s: None
        }
    );
    assert_eq!(
        classify_status(410, Some(b"120")),
        DeliveryReply {
            outcome: Outcome::Gone,
            status: Some(410),
            retry_after_s: None
        }
    );
    assert_eq!(
        classify_status(429, Some(b"120")),
        DeliveryReply {
            outcome: Outcome::Retry,
            status: Some(429),
            retry_after_s: Some(120)
        }
    );
    assert_eq!(
        classify_status(503, Some(b"9999")),
        DeliveryReply {
            outcome: Outcome::Retry,
            status: Some(503),
            retry_after_s: Some(MAX_RETRY_AFTER_S)
        }
    );
    assert_eq!(
        classify_status(503, None),
        DeliveryReply {
            outcome: Outcome::Retry,
            status: Some(503),
            retry_after_s: None
        }
    );
    assert_eq!(
        classify_status(429, Some(b"Wed, 21 Oct 2015 07:28:00 GMT")),
        DeliveryReply {
            outcome: Outcome::Retry,
            status: Some(429),
            retry_after_s: None
        }
    );
    assert_eq!(
        classify_status(302, Some(b"5")),
        DeliveryReply {
            outcome: Outcome::Rejected,
            status: Some(302),
            retry_after_s: None
        }
    );

    assert_eq!(parse_retry_after(b"120"), Some(120));
    assert_eq!(parse_retry_after(b"1"), Some(1));
    assert_eq!(parse_retry_after(b"300"), Some(300));
    assert_eq!(parse_retry_after(b"0"), None);
    assert_eq!(parse_retry_after(b"000"), None);
    assert_eq!(parse_retry_after(b"301"), Some(300));
    assert_eq!(parse_retry_after(b"9999999999"), Some(300));
    assert_eq!(parse_retry_after(b"99999999999"), None, "11 digits");
    for bad in [
        &b"-1"[..],
        b"+5",
        b" 5",
        b"5 ",
        b"5s",
        b"1.5",
        b"Wed, 21 Oct 2015 07:28:00 GMT",
        b"",
        b"\xff",
        b"0x10",
    ] {
        assert_eq!(
            parse_retry_after(bad),
            None,
            "{:?}",
            String::from_utf8_lossy(bad)
        );
    }
}
