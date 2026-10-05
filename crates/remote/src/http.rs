//! Minimal bounded HTTP/1.1 request-head parsing and response encoding (spec §2.5, §2.8).
//!
//! One request per connection, no body ever read, every bound explicit: head size, header
//! count, path length. Responses are computed fully before being written, and every
//! response (error responses included) carries the §2.8 mandatory headers.

use std::time::Duration;

use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::{MAX_HEADERS, MAX_PATH_LEN, MAX_REQUEST_HEAD_BYTES, RESPONSE_WRITE_TIMEOUT_MS};

/// Request method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `GET`.
    Get,
    /// `HEAD`.
    Head,
    /// `POST`.
    Post,
    /// Any other token.
    Other,
}

/// Owned, bounded request head (no body is ever read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHead {
    /// Method.
    pub method: Method,
    /// Path without query string; ≤ `MAX_PATH_LEN` bytes.
    pub path: String,
    /// ≤ `MAX_HEADERS` entries; names lowercased; values trimmed of optional whitespace.
    pub headers: Vec<(String, Vec<u8>)>,
}

/// Request-head failure and its HTTP mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HttpError {
    /// Need more bytes (not a response).
    #[error("request head incomplete")]
    Incomplete,
    /// `431`.
    #[error("request head too large")]
    HeadTooLarge,
    /// `431`.
    #[error("too many headers")]
    TooManyHeaders,
    /// `400` (also HTTP/1.0 and the HTTP/2 preface).
    #[error("malformed request")]
    Malformed,
    /// `413` (`Content-Length` > 0) or `400` (`Transfer-Encoding`).
    #[error("request body not allowed")]
    BodyNotAllowed,
    /// `414`.
    #[error("path too long")]
    PathTooLong,
}

impl HttpError {
    /// The status the server answers with; `Incomplete` has none (the connection is kept
    /// or closed) and `BodyNotAllowed` depends on the request (see [`body_refusal_status`]).
    #[must_use]
    pub fn status(self, buf: &[u8]) -> Option<u16> {
        match self {
            Self::Incomplete => None,
            Self::HeadTooLarge | Self::TooManyHeaders => Some(431),
            Self::Malformed => Some(400),
            Self::BodyNotAllowed => Some(body_refusal_status(buf)),
            Self::PathTooLong => Some(414),
        }
    }
}

/// Optional whitespace around a header value (SP / HTAB).
fn trim_ows(value: &[u8]) -> &[u8] {
    let is_ows = |b: &u8| *b == b' ' || *b == b'\t';
    let start = value.iter().position(|b| !is_ows(b)).unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|b| !is_ows(b))
        .map_or(start, |p| p.saturating_add(1));
    value.get(start..end.max(start)).unwrap_or_default()
}

/// Runs `httparse` over `buf` with `MAX_HEADERS` slots and hands the parsed request to `f`.
fn with_parsed<T>(
    buf: &[u8],
    f: impl FnOnce(&httparse::Request<'_, '_>) -> Result<T, HttpError>,
) -> Result<T, HttpError> {
    let mut slots = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut request = httparse::Request::new(&mut slots);
    match request.parse(buf) {
        Ok(httparse::Status::Complete(_)) => f(&request),
        Ok(httparse::Status::Partial) => Err(if buf.len() >= MAX_REQUEST_HEAD_BYTES {
            HttpError::HeadTooLarge
        } else {
            HttpError::Incomplete
        }),
        Err(httparse::Error::TooManyHeaders) => Err(HttpError::TooManyHeaders),
        Err(_) => Err(HttpError::Malformed),
    }
}

/// Pure. `buf` holds the bytes read so far. Requires HTTP/1.1 and an origin-form target
/// (`/...`); the path is kept verbatim without its query; names are lowercased and values
/// trimmed of optional whitespace. Any `Transfer-Encoding`, or any `Content-Length` above
/// zero, is a refused body; an unparsable or conflicting `Content-Length` is malformed.
///
/// # Errors
///
/// [`HttpError`].
pub fn parse_request_head(buf: &[u8]) -> Result<RequestHead, HttpError> {
    if buf.len() > MAX_REQUEST_HEAD_BYTES {
        return Err(HttpError::HeadTooLarge);
    }
    with_parsed(buf, |request| {
        if request.version != Some(1) {
            return Err(HttpError::Malformed);
        }
        let method = match request.method {
            Some("GET") => Method::Get,
            Some("HEAD") => Method::Head,
            Some("POST") => Method::Post,
            Some(_) => Method::Other,
            None => return Err(HttpError::Malformed),
        };
        let target = request.path.ok_or(HttpError::Malformed)?;
        if !target.starts_with('/') {
            return Err(HttpError::Malformed);
        }
        let path = target.split('?').next().unwrap_or(target);
        if path.len() > MAX_PATH_LEN {
            return Err(HttpError::PathTooLong);
        }
        let mut headers = Vec::with_capacity(request.headers.len());
        let mut lengths: Vec<u64> = Vec::new();
        let mut transfer_encoding = false;
        for header in request.headers.iter() {
            let name = header.name.to_ascii_lowercase();
            let value = trim_ows(header.value);
            if name == "transfer-encoding" {
                transfer_encoding = true;
            } else if name == "content-length" {
                let length = std::str::from_utf8(value)
                    .ok()
                    .and_then(|text| text.parse::<u64>().ok())
                    .ok_or(HttpError::Malformed)?;
                lengths.push(length);
            }
            headers.push((name, value.to_vec()));
        }
        if transfer_encoding {
            return Err(HttpError::BodyNotAllowed);
        }
        if let (Some(min), Some(max)) = (lengths.iter().min(), lengths.iter().max()) {
            if *max > 0 {
                // A zero next to a positive length is still a declared body; two different
                // positive lengths are a malformed request.
                if *min > 0 && min != max {
                    return Err(HttpError::Malformed);
                }
                return Err(HttpError::BodyNotAllowed);
            }
        }
        Ok(RequestHead {
            method,
            path: path.to_string(),
            headers,
        })
    })
}

/// Pure. The status of an `HttpError::BodyNotAllowed` refusal over the same buffer: `400`
/// when a `Transfer-Encoding` header is present, `413` otherwise (`Content-Length` > 0).
#[must_use]
pub fn body_refusal_status(buf: &[u8]) -> u16 {
    let chunked = with_parsed(buf, |request| {
        Ok(request
            .headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("transfer-encoding")))
    });
    match chunked {
        Ok(true) => 400,
        _ => 413,
    }
}

/// A fully computed non-streaming response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// Status code.
    pub status: u16,
    /// `Content-Type` value.
    pub content_type: &'static str,
    /// Body bytes.
    pub body: Vec<u8>,
    /// Additional headers (for example `Allow`).
    pub extra_headers: Vec<(&'static str, String)>,
}

impl Response {
    /// A JSON response `{"result":"<result>"}`.
    #[must_use]
    pub fn json(status: u16, result: &str) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: format!("{{\"result\":\"{result}\"}}").into_bytes(),
            extra_headers: Vec::new(),
        }
    }
}

/// Reason phrase of the statuses the server emits.
fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Content Too Large",
        414 => "URI Too Long",
        421 => "Misdirected Request",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

/// The §2.8 headers carried by every response, streams included.
const MANDATORY_HEADERS: &str = "Cache-Control: no-store\r\n\
    Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; \
    img-src 'self'; connect-src 'self'; manifest-src 'self'; base-uri 'none'; \
    form-action 'none'; frame-ancestors 'none'\r\n\
    X-Content-Type-Options: nosniff\r\n\
    Referrer-Policy: no-referrer\r\n\
    X-Frame-Options: DENY\r\n\
    Connection: close\r\n";

/// Status line + `Content-Type` + `Content-Length` + mandatory headers + extras + blank
/// line, without the body (what a `HEAD` request receives).
#[must_use]
pub fn encode_response_head(response: &Response) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{MANDATORY_HEADERS}",
        response.status,
        reason_phrase(response.status),
        response.content_type,
        response.body.len()
    );
    for (name, value) in &response.extra_headers {
        out.push_str(name);
        out.push_str(": ");
        out.push_str(value);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    out.into_bytes()
}

/// Serializes status line + mandatory headers (spec §2.8) + `Content-Length` + body.
#[must_use]
pub fn encode_response(response: &Response) -> Vec<u8> {
    let mut out = encode_response_head(response);
    out.extend_from_slice(&response.body);
    out
}

/// SSE response head (no `Content-Length`; `Content-Type: text/event-stream`).
#[must_use]
pub fn encode_sse_head() -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n{MANDATORY_HEADERS}\r\n")
        .into_bytes()
}

/// One SSE event: `event: status\ndata: <json>\n\n` (`json` carries no newline).
#[must_use]
pub fn encode_sse_event(json: &str) -> Vec<u8> {
    format!("event: status\ndata: {json}\n\n").into_bytes()
}

/// Why a bounded write failed (the caller drops the connection either way).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// `RESPONSE_WRITE_TIMEOUT_MS` elapsed before the bytes were written.
    #[error("response write timed out")]
    Timeout,
    /// The peer closed or the socket failed.
    #[error("response write failed")]
    Io,
}

/// Writes `bytes` fully under `RESPONSE_WRITE_TIMEOUT_MS`; a slow or broken peer fails.
///
/// # Errors
///
/// [`WriteError`].
pub async fn write_bounded<W: AsyncWrite + Unpin>(
    writer: &mut W,
    bytes: &[u8],
) -> Result<(), WriteError> {
    match tokio::time::timeout(
        Duration::from_millis(RESPONSE_WRITE_TIMEOUT_MS),
        writer.write_all(bytes),
    )
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(WriteError::Io),
        Err(_) => Err(WriteError::Timeout),
    }
}
