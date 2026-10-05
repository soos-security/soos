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
