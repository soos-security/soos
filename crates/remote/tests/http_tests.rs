//! Contract tests of GitHub #339 for the bounded HTTP/1.1 head parser and the response
//! encoders (spec §2.5, §2.8, §4, RC-4).

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

use proptest::prelude::*;

use soos_remote::http::{
    encode_response, encode_sse_event, encode_sse_head, parse_request_head, HttpError, Method,
    RequestHead, Response,
};
use soos_remote::{MAX_HEADERS, MAX_PATH_LEN, MAX_REQUEST_HEAD_BYTES};

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; \
                   connect-src 'self'; manifest-src 'self'; base-uri 'none'; \
                   form-action 'none'; frame-ancestors 'none'";

fn head(method: &str, target: &str, headers: &[(&str, &str)]) -> Vec<u8> {
    let mut out = format!("{method} {target} HTTP/1.1\r\n");
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    out.into_bytes()
}

fn get(target: &str) -> Vec<u8> {
    head("GET", target, &[("Host", "pc.tail1234.ts.net")])
}

/// `(lowercased name, value)` pairs of an encoded response head, plus the body.
fn split_response(bytes: &[u8]) -> (String, Vec<(String, String)>, Vec<u8>) {
    let end = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response head terminator");
    let head = std::str::from_utf8(&bytes[..end]).expect("ASCII head");
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap().to_string();
    let headers = lines
        .map(|l| {
            let (name, value) = l.split_once(':').unwrap_or_else(|| panic!("header {l:?}"));
            (name.trim().to_ascii_lowercase(), value.trim().to_string())
        })
        .collect();
    (status_line, headers, bytes[end + 4..].to_vec())
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Vec<&'a str> {
    headers
        .iter()
        .filter(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
        .collect()
}

fn assert_mandatory_headers(headers: &[(String, String)]) {
    assert_eq!(header(headers, "cache-control"), vec!["no-store"]);
    assert_eq!(header(headers, "content-security-policy"), vec![CSP]);
    assert_eq!(header(headers, "x-content-type-options"), vec!["nosniff"]);
    assert_eq!(header(headers, "referrer-policy"), vec!["no-referrer"]);
    assert_eq!(header(headers, "x-frame-options"), vec!["DENY"]);
    assert_eq!(header(headers, "connection"), vec!["close"]);
}

// ---------------------------------------------------------------------------------------
// parse_request_head
// ---------------------------------------------------------------------------------------

/// §2.5: method, path without query, lowercased header names, raw values.
#[test]
fn test_rmc_parse_request_head_nominal() {
    let parsed = parse_request_head(&head(
        "GET",
        "/api/status?x=1&y=%20",
        &[
            ("Host", "pc.tail1234.ts.net"),
            ("Tailscale-User-Login", "Owner@Example.com"),
            ("X-Soos-Action", "lock"),
        ],
    ))
    .unwrap();
    assert_eq!(
        parsed,
        RequestHead {
            method: Method::Get,
            path: "/api/status".to_string(),
            headers: vec![
                ("host".to_string(), b"pc.tail1234.ts.net".to_vec()),
                (
                    "tailscale-user-login".to_string(),
                    b"Owner@Example.com".to_vec()
                ),
                ("x-soos-action".to_string(), b"lock".to_vec()),
            ],
        }
    );
    // Trailing bytes after the terminator are ignored (no body is read).
    let mut with_trailer = get("/");
    with_trailer.extend_from_slice(b"GET /next HTTP/1.1\r\n\r\n");
    let parsed = parse_request_head(&with_trailer).unwrap();
    assert_eq!(parsed.path, "/");
    assert_eq!(parsed.headers.len(), 1);
}

/// §2.5: HEAD and POST are recognised; any other token (case-sensitive) is `Other`.
#[test]
fn test_rmc_parse_request_head_methods() {
    for (token, method) in [
        ("GET", Method::Get),
        ("HEAD", Method::Head),
        ("POST", Method::Post),
        ("DELETE", Method::Other),
        ("OPTIONS", Method::Other),
        ("PUT", Method::Other),
        ("get", Method::Other),
        ("Post", Method::Other),
    ] {
        let parsed = parse_request_head(&head(token, "/", &[("Host", "a.ts.net")])).unwrap();
        assert_eq!(parsed.method, method, "{token}");
    }
}

/// §2.5: a partial head is `Incomplete` (not a response), including the empty buffer.
#[test]
fn test_rmc_parse_request_head_incomplete() {
    let full = get("/api/status");
    assert_eq!(parse_request_head(&[]), Err(HttpError::Incomplete));
    for cut in [1, 3, 10, full.len() - 4, full.len() - 1] {
        assert_eq!(
            parse_request_head(&full[..cut]),
            Err(HttpError::Incomplete),
            "cut at {cut}"
        );
    }
    assert_eq!(
        parse_request_head(b"GET / HTTP/1.1\r\nHost: a.ts.net\r\n\r"),
        Err(HttpError::Incomplete)
    );
}

/// §2.5 / §4: HTTP/1.0, the HTTP/2 preface, garbage and non-origin-form targets are `400`.
#[test]
fn test_rmc_parse_request_head_rejects_http10_h2_preface_and_garbage() {
    let cases: Vec<Vec<u8>> = vec![
        b"GET / HTTP/1.0\r\nHost: a.ts.net\r\n\r\n".to_vec(),
        b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec(),
        b"GET / HTTP/2.0\r\n\r\n".to_vec(),
        b"GET / HTTP/1.2\r\n\r\n".to_vec(),
        b"\x00\x01\x02\x03\r\n\r\n".to_vec(),
        b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03\r\n\r\n".to_vec(),
        b"GET  HTTP/1.1\r\n\r\n".to_vec(),
        b"GET\r\n\r\n".to_vec(),
        b"GET * HTTP/1.1\r\n\r\n".to_vec(),
        b"GET http://a.ts.net/ HTTP/1.1\r\n\r\n".to_vec(),
        b"GET api/status HTTP/1.1\r\n\r\n".to_vec(),
        b"GET / HTTP/1.1\r\nBad Header\r\n\r\n".to_vec(),
        b"GET / HTTP/1.1\r\n: novalue\r\n\r\n".to_vec(),
        b"G\xffT / HTTP/1.1\r\n\r\n".to_vec(),
    ];
    for raw in cases {
        assert_eq!(
            parse_request_head(&raw),
            Err(HttpError::Malformed),
            "{:?}",
            String::from_utf8_lossy(&raw)
        );
    }
}

/// §3 / §4: a head over `MAX_REQUEST_HEAD_BYTES` is `431`, complete or not; a complete
/// head of exactly the bound parses.
#[test]
fn test_rmc_parse_request_head_too_large() {
    let base = head("GET", "/", &[("Host", "a.ts.net")]);
    // Pad a single header value so the complete head is exactly the bound.
    let pad_len = MAX_REQUEST_HEAD_BYTES - base.len() - "X-Pad: \r\n".len();
    let exact = head(
        "GET",
        "/",
        &[("Host", "a.ts.net"), ("X-Pad", &"p".repeat(pad_len))],
    );
    assert_eq!(exact.len(), MAX_REQUEST_HEAD_BYTES);
    assert!(parse_request_head(&exact).is_ok());

    let over = head(
        "GET",
        "/",
        &[("Host", "a.ts.net"), ("X-Pad", &"p".repeat(pad_len + 1))],
    );
    assert_eq!(over.len(), MAX_REQUEST_HEAD_BYTES + 1);
    assert_eq!(parse_request_head(&over), Err(HttpError::HeadTooLarge));

    // The bound reached without a terminator: still incomplete, so too large (never
    // `Incomplete`, which would wait for more bytes).
    let mut full_no_end = b"GET / HTTP/1.1\r\nX-Pad: ".to_vec();
    full_no_end.resize(MAX_REQUEST_HEAD_BYTES, b'p');
    assert_eq!(
        parse_request_head(&full_no_end),
        Err(HttpError::HeadTooLarge)
    );
    let mut huge = b"GET / HTTP/1.1\r\nX-Pad: ".to_vec();
    huge.resize(MAX_REQUEST_HEAD_BYTES * 4, b'p');
    assert_eq!(parse_request_head(&huge), Err(HttpError::HeadTooLarge));
}

/// §3 / §4: 32 headers parse, 33 are `431`.
#[test]
fn test_rmc_parse_request_head_too_many_headers() {
    let names: Vec<String> = (0..=MAX_HEADERS).map(|i| format!("X-H{i}")).collect();
    let thirty_two: Vec<(&str, &str)> = names[..MAX_HEADERS]
        .iter()
        .map(|n| (n.as_str(), "v"))
        .collect();
    let parsed = parse_request_head(&head("GET", "/", &thirty_two)).unwrap();
    assert_eq!(parsed.headers.len(), MAX_HEADERS);
    let thirty_three: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "v")).collect();
    assert_eq!(
        parse_request_head(&head("GET", "/", &thirty_three)),
        Err(HttpError::TooManyHeaders)
    );
}

/// D5 / §4: no request body is ever accepted.
#[test]
fn test_rmc_parse_request_head_rejects_bodies() {
    assert_eq!(
        parse_request_head(&head("POST", "/api/lock", &[("Content-Length", "1")])),
        Err(HttpError::BodyNotAllowed)
    );
    assert_eq!(
        parse_request_head(&head("GET", "/", &[("content-length", "4096")])),
        Err(HttpError::BodyNotAllowed)
    );
    assert_eq!(
        parse_request_head(&head(
            "POST",
            "/api/lock",
            &[("Transfer-Encoding", "chunked")]
        )),
        Err(HttpError::BodyNotAllowed)
    );
    assert_eq!(
        parse_request_head(&head(
            "POST",
            "/api/lock",
            &[("transfer-encoding", "gzip, chunked")]
        )),
        Err(HttpError::BodyNotAllowed)
    );
    assert_eq!(
        parse_request_head(&head("POST", "/api/lock", &[("Content-Length", "abc")])),
        Err(HttpError::Malformed)
    );
    assert_eq!(
        parse_request_head(&head("POST", "/api/lock", &[("Content-Length", "-1")])),
        Err(HttpError::Malformed)
    );
    let parsed =
        parse_request_head(&head("POST", "/api/lock", &[("Content-Length", "0")])).unwrap();
    assert_eq!(parsed.method, Method::Post);
}

/// §3 / §4: the path without query is bounded by `MAX_PATH_LEN`; the query does not count.
#[test]
fn test_rmc_parse_request_head_path_length_bound() {
    let exact = format!("/{}", "p".repeat(MAX_PATH_LEN - 1));
    assert_eq!(exact.len(), MAX_PATH_LEN);
    let parsed = parse_request_head(&get(&exact)).unwrap();
    assert_eq!(parsed.path, exact);
    let with_query = format!("{exact}?{}", "q".repeat(2000));
    let parsed = parse_request_head(&get(&with_query)).unwrap();
    assert_eq!(parsed.path, exact);
    let over = format!("/{}", "p".repeat(MAX_PATH_LEN));
    assert_eq!(parse_request_head(&get(&over)), Err(HttpError::PathTooLong));
}

/// §2.5: the query is stripped and the path is kept verbatim (no percent-decoding, no
/// normalisation: those forms simply do not route).
#[test]
fn test_rmc_parse_request_head_strips_query_and_keeps_path_verbatim() {
    assert_eq!(parse_request_head(&get("/?")).unwrap().path, "/");
    assert_eq!(
        parse_request_head(&get("/api/status?")).unwrap().path,
        "/api/status"
    );
    assert_eq!(
        parse_request_head(&get("/%2e%2e/x")).unwrap().path,
        "/%2e%2e/x"
    );
    assert_eq!(
        parse_request_head(&get("/API/Status")).unwrap().path,
        "/API/Status"
    );
    assert_eq!(parse_request_head(&get("/a/./b")).unwrap().path, "/a/./b");
    assert_eq!(parse_request_head(&get("/x#frag")).unwrap().path, "/x#frag");
}

proptest! {
    /// RC-4: the parser never panics on arbitrary bytes, and any buffer over the bound is
    /// `HeadTooLarge`.
    #[test]
    fn prop_rmc_parse_request_head_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..9000)) {
        let result = parse_request_head(&bytes);
        if bytes.len() > MAX_REQUEST_HEAD_BYTES {
            prop_assert_eq!(result.clone(), Err(HttpError::HeadTooLarge));
        }
        if let Ok(parsed) = result {
            prop_assert!(parsed.path.len() <= MAX_PATH_LEN);
            prop_assert!(parsed.path.starts_with('/'));
            prop_assert!(parsed.headers.len() <= MAX_HEADERS);
            prop_assert!(parsed.headers.iter().all(|(n, _)| !n.bytes().any(|b| b.is_ascii_uppercase())));
        }
    }

    /// RC-4: a valid head with random header values always parses and keeps the values.
    #[test]
    fn prop_rmc_parse_request_head_round_trips_header_values(
        value in "[\\x21-\\x7e][\\x20-\\x7e]{0,200}",
        path in "/[a-zA-Z0-9/._-]{0,100}",
    ) {
        let raw = head("GET", &path, &[("Host", "a.ts.net"), ("X-Value", &value)]);
        let parsed = parse_request_head(&raw).unwrap();
        prop_assert_eq!(parsed.method, Method::Get);
        prop_assert_eq!(parsed.path, path);
        prop_assert_eq!(parsed.headers[1].0.as_str(), "x-value");
        prop_assert_eq!(parsed.headers[1].1.as_slice(), value.trim_end().as_bytes());
    }

    /// `Content-Length` is bounded: every strictly positive value is a body.
    #[test]
    fn prop_rmc_parse_request_head_any_positive_content_length_is_a_body(len in 1u64..) {
        let raw = head("POST", "/api/lock", &[("Content-Length", &len.to_string())]);
        prop_assert_eq!(parse_request_head(&raw), Err(HttpError::BodyNotAllowed));
    }
}

// ---------------------------------------------------------------------------------------
// encode_response / encode_sse_*
// ---------------------------------------------------------------------------------------

/// §2.8: status line, every mandatory header, `Content-Type`, `Content-Length`, extras,
/// then the body.
#[test]
fn test_rmc_encode_response_has_mandatory_headers_and_content_length() {
    let response = Response {
        status: 405,
        content_type: "application/json",
        body: b"{\"result\":\"method_not_allowed\"}".to_vec(),
        extra_headers: vec![("Allow", "GET, HEAD".to_string())],
    };
    let bytes = encode_response(&response);
    let (status_line, headers, body) = split_response(&bytes);
    assert_eq!(status_line, "HTTP/1.1 405 Method Not Allowed");
    assert_mandatory_headers(&headers);
    assert_eq!(header(&headers, "content-type"), vec!["application/json"]);
    assert_eq!(
        header(&headers, "content-length"),
        vec![response.body.len().to_string().as_str()]
    );
    assert_eq!(header(&headers, "allow"), vec!["GET, HEAD"]);
    assert_eq!(body, response.body);
    assert!(
        header(&headers, "server").is_empty() && header(&headers, "date").is_empty(),
        "no fingerprinting headers"
    );
}

/// §2.5 route table: the status phrases of every status the server emits.
#[test]
fn test_rmc_encode_response_status_lines() {
    for (status, line) in [
        (200, "HTTP/1.1 200 OK"),
        (202, "HTTP/1.1 202 Accepted"),
        (400, "HTTP/1.1 400 Bad Request"),
        (403, "HTTP/1.1 403 Forbidden"),
        (404, "HTTP/1.1 404 Not Found"),
        (405, "HTTP/1.1 405 Method Not Allowed"),
        (409, "HTTP/1.1 409 Conflict"),
        (414, "HTTP/1.1 414 URI Too Long"),
        (421, "HTTP/1.1 421 Misdirected Request"),
        (429, "HTTP/1.1 429 Too Many Requests"),
        (431, "HTTP/1.1 431 Request Header Fields Too Large"),
        (503, "HTTP/1.1 503 Service Unavailable"),
    ] {
        let bytes = encode_response(&Response {
            status,
            content_type: "text/plain; charset=utf-8",
            body: Vec::new(),
            extra_headers: Vec::new(),
        });
        let (status_line, headers, body) = split_response(&bytes);
        assert_eq!(status_line, line);
        assert_eq!(header(&headers, "content-length"), vec!["0"]);
        assert!(body.is_empty());
        assert_mandatory_headers(&headers);
    }
    let bytes = encode_response(&Response {
        status: 413,
        content_type: "application/json",
        body: Vec::new(),
        extra_headers: Vec::new(),
    });
    let (status_line, _, _) = split_response(&bytes);
    assert!(status_line.starts_with("HTTP/1.1 413 "), "{status_line}");
}

/// §2.5: the SSE head carries the mandatory headers, `text/event-stream` and no
/// `Content-Length`; an event is exactly `event: status\ndata: <json>\n\n`.
#[test]
fn test_rmc_encode_sse_head_and_event() {
    let bytes = encode_sse_head();
    let (status_line, headers, body) = split_response(&bytes);
    assert_eq!(status_line, "HTTP/1.1 200 OK");
    assert_mandatory_headers(&headers);
    assert_eq!(header(&headers, "content-type"), vec!["text/event-stream"]);
    assert!(header(&headers, "content-length").is_empty());
    assert!(header(&headers, "transfer-encoding").is_empty());
    assert!(body.is_empty(), "the head ends at the blank line");
    assert_eq!(
        encode_sse_event("{\"state\":\"locked\"}"),
        b"event: status\ndata: {\"state\":\"locked\"}\n\n".to_vec()
    );
}

proptest! {
    /// `Content-Length` always equals the body length and the body is the suffix.
    #[test]
    fn prop_rmc_encode_response_length_matches_body(body in proptest::collection::vec(any::<u8>(), 0..5000), status in 200u16..600) {
        let bytes = encode_response(&Response { status, content_type: "application/octet-stream", body: body.clone(), extra_headers: Vec::new() });
        let (status_line, headers, got) = split_response(&bytes);
        let expected_prefix = format!("HTTP/1.1 {status} ");
        prop_assert!(status_line.starts_with(&expected_prefix));
        let expected_length = body.len().to_string();
        prop_assert_eq!(header(&headers, "content-length"), vec![expected_length.as_str()]);
        prop_assert_eq!(got, body);
    }
}

// ---------------------------------------------------------------------------------------
// Body framing on the four body routes (ADR 2026-10-06 "Tailscale Funnel Access and
// In-House Passkey Authentication for `soos-remote`", spec §4.8, tests 11–13, matrix RMC29)
// ---------------------------------------------------------------------------------------

mod body_framing {
    use super::*;
    use soos_remote::http::{parse_request, read_body, BodyError, BodyFraming, ParsedRequest};
    use soos_remote::{BODY_READ_TIMEOUT_MS, MAX_AUTH_BODY_BYTES, MAX_BODY_CHUNKS};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    fn never(_: Method, _: &str) -> bool {
        false
    }

    /// Local stand-in for `routes::accepts_body` so the framing contract is tested alone.
    fn unlock_only(method: Method, path: &str) -> bool {
        method == Method::Post && path == "/api/unlock"
    }

    fn post_unlock(headers: &[(&str, &str)]) -> Vec<u8> {
        head("POST", "/api/unlock", headers)
    }

    fn framing(headers: &[(&str, &str)]) -> Result<BodyFraming, HttpError> {
        parse_request(&post_unlock(headers), unlock_only).map(|p| p.framing)
    }

    fn same_as_head_parser(buf: &[u8]) -> Result<(), String> {
        let expected = parse_request_head(buf);
        let got = parse_request(buf, never);
        match (expected, got) {
            (Ok(head), Ok(parsed)) => {
                let end = buf
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .ok_or("no terminator")?
                    + 4;
                if parsed.head != head {
                    return Err("head differs".into());
                }
                if parsed.framing != BodyFraming::None || parsed.has_body() {
                    return Err("non-body route got a body framing".into());
                }
                if parsed.head_len != end {
                    return Err(format!("head_len {} != {end}", parsed.head_len));
                }
                Ok(())
            }
            (Err(a), Err(b)) if a == b => {
                if a.status(buf) == b.status(buf) {
                    Ok(())
                } else {
                    Err("status differs".into())
                }
            }
            (a, b) => Err(format!("{a:?} vs {b:?}")),
        }
    }

    proptest! {
        /// Test 11 (RMC29): when `accepts_body` is false, `parse_request` is exactly
        /// `parse_request_head` (same head, same error, same status), framing `None`.
        #[test]
        fn test_rmc_parse_request_matches_parse_request_head_for_non_body_routes(
            method in prop::sample::select(vec!["GET", "HEAD", "POST", "PUT", "OPTIONS"]),
            path in prop::sample::select(vec![
                "/", "/api/status", "/api/lock", "/api/unlock", "/api/auth/login/verify",
                "/api/auth/register/verify", "/api/auth/state?x=1",
            ]),
            extra in proptest::collection::vec(prop::sample::select(vec![
                ("Content-Length", "0"), ("Content-Length", "5"), ("Content-Length", "abc"),
                ("Content-Length", "8193"), ("Transfer-Encoding", "chunked"),
                ("Transfer-Encoding", "gzip"), ("Host", "pc.tail1234.ts.net"),
                ("Tailscale-Funnel-Request", "?1"), ("X-Soos-Action", "unlock"),
            ]), 0..4),
            trailing in proptest::collection::vec(any::<u8>(), 0..16),
        ) {
            let mut buf = head(method, path, &extra);
            buf.extend_from_slice(&trailing);
            prop_assert_eq!(same_as_head_parser(&buf), Ok(()));
        }

        /// Test 11 (RMC29, RC-4): on arbitrary bytes `parse_request` never panics and, for a
        /// non-body route, never differs from `parse_request_head`.
        #[test]
        fn prop_rmc_parse_request_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..9000)) {
            prop_assert_eq!(same_as_head_parser(&bytes), Ok(()));
            let _ = parse_request(&bytes, |_, _| true);
        }
    }

    /// Test 12 (RMC29, S-13): framing of a body route; every refusal and its status; the
    /// non-body routes keep their existing refusals.
    #[test]
    fn test_rmc_parse_request_body_framing() {
        assert_eq!(framing(&[]), Ok(BodyFraming::None));
        assert_eq!(framing(&[("Content-Length", "0")]), Ok(BodyFraming::None));
        assert_eq!(
            framing(&[("Content-Length", "1")]),
            Ok(BodyFraming::Length(1))
        );
        assert_eq!(
            framing(&[("Content-Length", "8192")]),
            Ok(BodyFraming::Length(MAX_AUTH_BODY_BYTES))
        );
        assert_eq!(
            framing(&[("Content-Length", "8192"), ("Content-Length", "8192")]),
            Ok(BodyFraming::Length(8192)),
            "equal repeated lengths"
        );
        let over = post_unlock(&[("Content-Length", "8193")]);
        assert_eq!(
            parse_request(&over, unlock_only).map(|p| p.framing),
            Err(HttpError::BodyTooLarge)
        );
        assert_eq!(HttpError::BodyTooLarge.status(&over), Some(413));
        assert_eq!(
            framing(&[("Content-Length", "18446744073709551615")]),
            Err(HttpError::BodyTooLarge)
        );
        for value in ["chunked", "Chunked", "CHUNKED", " chunked ", "chunked\t"] {
            assert_eq!(
                framing(&[("Transfer-Encoding", value)]),
                Ok(BodyFraming::Chunked),
                "{value:?}"
            );
        }
        for bad in [
            vec![("Transfer-Encoding", "gzip")],
            vec![("Transfer-Encoding", "gzip, chunked")],
            vec![("Transfer-Encoding", "chunked, chunked")],
            vec![("Transfer-Encoding", "")],
            vec![
                ("Transfer-Encoding", "chunked"),
                ("Transfer-Encoding", "chunked"),
            ],
            vec![("Transfer-Encoding", "chunked"), ("Content-Length", "5")],
            vec![("Content-Length", "5"), ("Transfer-Encoding", "chunked")],
            vec![("Transfer-Encoding", "chunked"), ("Content-Length", "0")],
            vec![("Content-Length", "5"), ("Content-Length", "6")],
            vec![("Content-Length", "0"), ("Content-Length", "5")],
            vec![("Content-Length", "abc")],
            vec![("Content-Length", "-1")],
            vec![("Content-Length", "+5")],
        ] {
            let buf = post_unlock(&bad);
            let err = parse_request(&buf, unlock_only).map(|p| p.framing);
            assert_eq!(err, Err(HttpError::Malformed), "{bad:?}");
            assert_eq!(HttpError::Malformed.status(&buf), Some(400));
        }
        // head_len points at the first body byte, even with the body already buffered.
        let mut buf = post_unlock(&[("Content-Length", "4")]);
        let head_len = buf.len();
        buf.extend_from_slice(b"{}{}");
        let parsed: ParsedRequest = parse_request(&buf, unlock_only).unwrap();
        assert_eq!(parsed.head_len, head_len);
        assert_eq!(parsed.framing, BodyFraming::Length(4));
        assert!(parsed.has_body());
        assert_eq!(parsed.head.path, "/api/unlock");
        let chunked = parse_request(
            &post_unlock(&[("Transfer-Encoding", "chunked")]),
            unlock_only,
        )
        .unwrap();
        assert!(chunked.has_body());
        let none = parse_request(&post_unlock(&[]), unlock_only).unwrap();
        assert!(!none.has_body());

        // A non-body route keeps the existing refusals and statuses.
        let lock = head("POST", "/api/lock", &[("Content-Length", "5")]);
        assert_eq!(
            parse_request(&lock, unlock_only).map(|p| p.framing),
            Err(HttpError::BodyNotAllowed)
        );
        assert_eq!(HttpError::BodyNotAllowed.status(&lock), Some(413));
        let lock_te = head("POST", "/api/lock", &[("Transfer-Encoding", "chunked")]);
        assert_eq!(
            parse_request(&lock_te, unlock_only).map(|p| p.framing),
            Err(HttpError::BodyNotAllowed)
        );
        assert_eq!(HttpError::BodyNotAllowed.status(&lock_te), Some(400));
        let get_unlock = head("GET", "/api/unlock", &[("Content-Length", "5")]);
        assert_eq!(
            parse_request(&get_unlock, unlock_only).map(|p| p.framing),
            Err(HttpError::BodyNotAllowed),
            "the method is part of the body-route decision"
        );
        assert_eq!(
            HttpError::BodyTooLarge.to_string(),
            "request body too large"
        );
    }

    async fn pair_with(sent: &[u8]) -> (UnixStream, UnixStream) {
        let (server, mut client) = UnixStream::pair().unwrap();
        client.write_all(sent).await.unwrap();
        (server, client)
    }

    async fn body(prefix: &[u8], sent: &[u8], framing: BodyFraming) -> Result<Vec<u8>, BodyError> {
        let (mut server, _client) = pair_with(sent).await;
        read_body(&mut server, prefix, framing)
            .await
            .map(|b| b.to_vec())
    }

    async fn chunked(prefix: &[u8], sent: &[u8]) -> Result<Vec<u8>, BodyError> {
        body(prefix, sent, BodyFraming::Chunked).await
    }

    /// The bytes still unread on `server` (the client is closed first).
    async fn rest(server: &mut UnixStream, client: UnixStream) -> Vec<u8> {
        drop(client);
        let mut out = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), server.read_to_end(&mut out))
            .await
            .unwrap()
            .unwrap();
        out
    }

    /// Test 13 (RMC29, S-13): `Length` reads exactly the body (prefix first, pipelined bytes
    /// ignored, never past the end), times out at exactly `BODY_READ_TIMEOUT_MS`, EOF is
    /// `Closed`; the strict bounded chunked decoder accepts what Go's reverse proxy writes and
    /// nothing looser.
    #[tokio::test(start_paused = true)]
    async fn test_rmc_read_body_is_bounded() {
        // Length.
        assert_eq!(
            body(b"hello", b"", BodyFraming::Length(5)).await,
            Ok(b"hello".to_vec())
        );
        assert_eq!(
            body(b"hel", b"lo", BodyFraming::Length(5)).await,
            Ok(b"hello".to_vec())
        );
        assert_eq!(
            body(b"hello world", b"", BodyFraming::Length(5)).await,
            Ok(b"hello".to_vec()),
            "pipelined bytes in the prefix are ignored"
        );
        let (mut server, client) = pair_with(b"llo EXTRA").await;
        assert_eq!(
            read_body(&mut server, b"he", BodyFraming::Length(5))
                .await
                .map(|b| b.to_vec()),
            Ok(b"hello".to_vec())
        );
        assert_eq!(
            rest(&mut server, client).await,
            b" EXTRA".to_vec(),
            "never reads past the body"
        );
        assert_eq!(body(b"", b"", BodyFraming::None).await, Ok(Vec::new()));
        // EOF before the end.
        let (mut server, client) = pair_with(b"l").await;
        drop(client);
        assert_eq!(
            read_body(&mut server, b"he", BodyFraming::Length(5))
                .await
                .map(|b| b.to_vec()),
            Err(BodyError::Closed)
        );
        // Deadline: exactly BODY_READ_TIMEOUT_MS, the peer staying open and silent.
        let (mut server, _client) = pair_with(b"").await;
        let start = tokio::time::Instant::now();
        let result = read_body(&mut server, b"he", BodyFraming::Length(5))
            .await
            .map(|b| b.to_vec());
        assert_eq!(result, Err(BodyError::Timeout));
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(BODY_READ_TIMEOUT_MS)
                && elapsed <= Duration::from_millis(BODY_READ_TIMEOUT_MS + 1),
            "{elapsed:?}"
        );

        // Chunked: Go-style body split across the prefix and the stream.
        let data = b"abcdefghijklmnopqrstuvwxyz";
        let mut go = b"1a\r\n".to_vec();
        go.extend_from_slice(data);
        go.extend_from_slice(b"\r\n0\r\n\r\n");
        for split in [0, 1, 4, 10, go.len() - 3, go.len()] {
            assert_eq!(
                chunked(&go[..split], &go[split..]).await,
                Ok(data.to_vec()),
                "split at {split}"
            );
        }
        assert_eq!(
            chunked(b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n", b"").await,
            Ok(b"hello world".to_vec())
        );
        assert_eq!(
            chunked(b"A\r\n0123456789\r\n0\r\n\r\n", b"").await,
            Ok(b"0123456789".to_vec()),
            "uppercase hex"
        );
        assert_eq!(
            chunked(b"00000005\r\nhello\r\n0\r\n\r\n", b"").await,
            Ok(b"hello".to_vec()),
            "8 hex digits"
        );
        assert_eq!(chunked(b"0\r\n\r\n", b"").await, Ok(Vec::new()), "empty");
        let mut sixty_four = b"1\r\na\r\n".repeat(MAX_BODY_CHUNKS);
        sixty_four.extend_from_slice(b"0\r\n\r\n");
        assert_eq!(
            chunked(b"", &sixty_four).await,
            Ok(vec![b'a'; MAX_BODY_CHUNKS])
        );
        // Bytes after the last CRLF are never read.
        let (mut server, mut client) = pair_with(b"").await;
        client.write_all(b"\r\nNEXT").await.unwrap();
        assert_eq!(
            read_body(&mut server, b"5\r\nhello\r\n0\r\n", BodyFraming::Chunked)
                .await
                .map(|b| b.to_vec()),
            Ok(b"hello".to_vec())
        );
        assert_eq!(rest(&mut server, client).await, b"NEXT".to_vec());

        // Chunked refusals.
        let mut sixty_five = b"1\r\na\r\n".repeat(MAX_BODY_CHUNKS + 1);
        sixty_five.extend_from_slice(b"0\r\n\r\n");
        for bad in [
            b"5;x=1\r\nhello\r\n0\r\n\r\n".to_vec(),
            b"5 \r\nhello\r\n0\r\n\r\n".to_vec(),
            b" 5\r\nhello\r\n0\r\n\r\n".to_vec(),
            b"5\nhello\r\n0\r\n\r\n".to_vec(),
            b"5\r\nhello\n0\r\n\r\n".to_vec(),
            b"5\r\nhelloXY0\r\n\r\n".to_vec(),
            b"5\r\nhello\r\n0\r\nX-Trailer: a\r\n\r\n".to_vec(),
            b"5\r\nhello\r\n0\r\n\n".to_vec(),
            b"000000005\r\nhello\r\n0\r\n\r\n".to_vec(),
            b"\r\nhello\r\n0\r\n\r\n".to_vec(),
            b"g\r\nhello\r\n0\r\n\r\n".to_vec(),
            b"-5\r\nhello\r\n0\r\n\r\n".to_vec(),
            b"0x5\r\nhello\r\n0\r\n\r\n".to_vec(),
            sixty_five,
        ] {
            assert_eq!(
                chunked(b"", &bad).await,
                Err(BodyError::Malformed),
                "{:?}",
                String::from_utf8_lossy(&bad[..bad.len().min(40)])
            );
        }

        // Decoded size bound, checked before a chunk's data is read.
        let mut full = b"2000\r\n".to_vec();
        full.extend(vec![b'a'; MAX_AUTH_BODY_BYTES]);
        full.extend_from_slice(b"\r\n0\r\n\r\n");
        assert_eq!(
            chunked(b"", &full).await.map(|b| b.len()),
            Ok(MAX_AUTH_BODY_BYTES)
        );
        assert_eq!(
            chunked(b"2001\r\n", b"").await,
            Err(BodyError::TooLarge),
            "refused at the size line, without waiting for the data"
        );
        let mut split = b"1000\r\n".to_vec();
        split.extend(vec![b'a'; 4096]);
        split.extend_from_slice(b"\r\n1001\r\n");
        assert_eq!(chunked(b"", &split).await, Err(BodyError::TooLarge));
        assert_eq!(
            chunked(b"ffffffff\r\n", b"").await,
            Err(BodyError::TooLarge)
        );
        // EOF inside a chunked body.
        let (mut server, client) = pair_with(b"5\r\nhel").await;
        drop(client);
        assert_eq!(
            read_body(&mut server, b"", BodyFraming::Chunked)
                .await
                .map(|b| b.to_vec()),
            Err(BodyError::Closed)
        );
        // A chunked body that never ends times out.
        let (mut server, _client) = pair_with(b"5\r\nhello\r\n").await;
        assert_eq!(
            read_body(&mut server, b"", BodyFraming::Chunked)
                .await
                .map(|b| b.to_vec()),
            Err(BodyError::Timeout)
        );

        assert_eq!(BodyError::Timeout.to_string(), "body read deadline");
        assert_eq!(BodyError::Closed.to_string(), "peer closed");
        assert_eq!(BodyError::Malformed.to_string(), "body framing invalid");
        assert_eq!(BodyError::TooLarge.to_string(), "body too large");
    }
}

// ---------------------------------------------------------------------------------------
// Failed-password alerts (ADR 2026-10-06 "Failed-Password Alerts in `soos-remote` From the
// System Journal", architect spec `AI/architect_spec_remote_auth_alerts.md` §6, test 41;
// matrix RMC55). New tests only; nothing above is changed.
// ---------------------------------------------------------------------------------------

mod alerts_contract {
    use soos_remote::http::{encode_sse_alerts_event, encode_sse_event};

    /// Test 41 (RMC55): `event: alerts\ndata: <json>\n\n`; the status encoding is unchanged.
    #[test]
    fn test_rmc_alerts_sse_alerts_event_encoding() {
        assert_eq!(
            encode_sse_alerts_event("{\"state\":\"active\",\"through\":3}"),
            b"event: alerts\ndata: {\"state\":\"active\",\"through\":3}\n\n".to_vec()
        );
        assert_eq!(
            encode_sse_alerts_event(""),
            b"event: alerts\ndata: \n\n".to_vec()
        );
        assert_eq!(
            encode_sse_event("{\"state\":\"locked\"}"),
            b"event: status\ndata: {\"state\":\"locked\"}\n\n".to_vec()
        );
    }
}
