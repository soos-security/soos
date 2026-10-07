//! Minimal bounded HTTP/1.1 request-head parsing and response encoding (spec §2.5, §2.8).
//!
//! One request per connection; a body is read only on the four passkey body routes
//! (bounded, spec §4.8); every bound explicit: head size, header
//! count, path length. Responses are computed fully before being written, and every
//! response (error responses included) carries the §2.8 mandatory headers.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
    BODY_READ_TIMEOUT_MS, MAX_AUTH_BODY_BYTES, MAX_BODY_CHUNKS, MAX_CHUNK_SIZE_DIGITS, MAX_HEADERS,
    MAX_PATH_LEN, MAX_REQUEST_HEAD_BYTES, RESPONSE_WRITE_TIMEOUT_MS,
};

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
    /// `413 body_too_large` (body routes only).
    #[error("request body too large")]
    BodyTooLarge,
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
            Self::BodyTooLarge => Some(413),
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

/// Runs `httparse` over `buf` with `MAX_HEADERS` slots and hands the parsed request and the
/// head length to `f`.
fn with_parsed<T>(
    buf: &[u8],
    f: impl FnOnce(&httparse::Request<'_, '_>, usize) -> Result<T, HttpError>,
) -> Result<T, HttpError> {
    let mut slots = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut request = httparse::Request::new(&mut slots);
    match request.parse(buf) {
        Ok(httparse::Status::Complete(head_len)) => f(&request, head_len),
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
    with_parsed(buf, |request, _| parse_no_body(request))
}

/// The method token as a [`Method`].
fn method_of(token: &str) -> Method {
    match token {
        "GET" => Method::Get,
        "HEAD" => Method::Head,
        "POST" => Method::Post,
        _ => Method::Other,
    }
}

/// Version, method and target checks shared by both parsers: HTTP/1.1, an origin-form
/// target, a path (query removed) of at most `MAX_PATH_LEN` bytes.
fn request_line(request: &httparse::Request<'_, '_>) -> Result<(Method, String), HttpError> {
    if request.version != Some(1) {
        return Err(HttpError::Malformed);
    }
    let method = method_of(request.method.ok_or(HttpError::Malformed)?);
    let target = request.path.ok_or(HttpError::Malformed)?;
    if !target.starts_with('/') {
        return Err(HttpError::Malformed);
    }
    let path = target.split('?').next().unwrap_or(target);
    if path.len() > MAX_PATH_LEN {
        return Err(HttpError::PathTooLong);
    }
    Ok((method, path.to_string()))
}

/// The `parse_request_head` rules over a complete parse: any body is refused.
fn parse_no_body(request: &httparse::Request<'_, '_>) -> Result<RequestHead, HttpError> {
    {
        let (method, path) = request_line(request)?;
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
            path,
            headers,
        })
    }
}

/// Pure. The status of an `HttpError::BodyNotAllowed` refusal over the same buffer: `400`
/// when a `Transfer-Encoding` header is present, `413` otherwise (`Content-Length` > 0).
#[must_use]
pub fn body_refusal_status(buf: &[u8]) -> u16 {
    let chunked = with_parsed(buf, |request, _| {
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

// ---------------------------------------------------------------------------------------
// Body framing for the four body routes (architect spec §4.8, S-13).
// ---------------------------------------------------------------------------------------

/// How the body of a body route is framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFraming {
    /// No body.
    None,
    /// `Content-Length: n`, 1..=`MAX_AUTH_BODY_BYTES`.
    Length(usize),
    /// `Transfer-Encoding: chunked`.
    Chunked,
}

/// A parsed request head and its body framing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRequest {
    /// The head.
    pub head: RequestHead,
    /// Bytes of the head (index of the first body byte in the buffer).
    pub head_len: usize,
    /// Never other than `None` when `accepts_body` was false.
    pub framing: BodyFraming,
}

impl ParsedRequest {
    /// True unless `framing == BodyFraming::None`.
    #[must_use]
    pub fn has_body(&self) -> bool {
        self.framing != BodyFraming::None
    }
}

/// `Some((head_len, body_route))` when `buf` starts with a complete head whose method and
/// path could be read; `None` otherwise (the caller falls back to `parse_request_head`).
fn probe(buf: &[u8], accepts_body: fn(Method, &str) -> bool) -> Option<(usize, bool)> {
    with_parsed(buf, |request, head_len| {
        let method = method_of(request.method.ok_or(HttpError::Malformed)?);
        let target = request.path.ok_or(HttpError::Malformed)?;
        let path = target.split('?').next().unwrap_or(target);
        Ok((head_len, accepts_body(method, path)))
    })
    .ok()
}

/// Pure. When `accepts_body(method, path)` is false: exactly [`parse_request_head`] (same
/// head, errors and statuses), framing `None`. When true: `Transfer-Encoding` must be a
/// single header whose OWS-trimmed value is `chunked` (ASCII-case-insensitive) with no
/// `Content-Length` (otherwise [`HttpError::Malformed`], the request-smuggling guard);
/// otherwise every `Content-Length` must be ASCII digits and all equal (else `Malformed`),
/// `0` is no body and a value above `MAX_AUTH_BODY_BYTES` is [`HttpError::BodyTooLarge`].
/// The head itself must still fit in `MAX_REQUEST_HEAD_BYTES`; bytes after `head_len` are
/// the body prefix.
///
/// # Errors
///
/// [`HttpError`].
pub fn parse_request(
    buf: &[u8],
    accepts_body: fn(Method, &str) -> bool,
) -> Result<ParsedRequest, HttpError> {
    match probe(buf, accepts_body) {
        Some((head_len, true)) => {
            if head_len > MAX_REQUEST_HEAD_BYTES {
                return Err(HttpError::HeadTooLarge);
            }
            let head_bytes = buf.get(..head_len).ok_or(HttpError::Malformed)?;
            let (head, framing) = with_parsed(head_bytes, |request, _| parse_with_body(request))?;
            Ok(ParsedRequest {
                head,
                head_len,
                framing,
            })
        }
        probed => {
            let head = parse_request_head(buf)?;
            let head_len = probed.map_or(buf.len(), |(head_len, _)| head_len);
            Ok(ParsedRequest {
                head,
                head_len,
                framing: BodyFraming::None,
            })
        }
    }
}

/// Head rules of a body route (see [`parse_request`]).
fn parse_with_body(
    request: &httparse::Request<'_, '_>,
) -> Result<(RequestHead, BodyFraming), HttpError> {
    let (method, path) = request_line(request)?;
    let mut headers = Vec::with_capacity(request.headers.len());
    let mut lengths: Vec<u64> = Vec::new();
    let mut encodings = 0usize;
    let mut chunked = false;
    for header in request.headers.iter() {
        let name = header.name.to_ascii_lowercase();
        let value = trim_ows(header.value);
        if name == "transfer-encoding" {
            encodings = encodings.saturating_add(1);
            chunked = value.eq_ignore_ascii_case(b"chunked");
        } else if name == "content-length" {
            if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
                return Err(HttpError::Malformed);
            }
            let length = std::str::from_utf8(value)
                .ok()
                .and_then(|text| text.parse::<u64>().ok())
                .ok_or(HttpError::Malformed)?;
            lengths.push(length);
        }
        headers.push((name, value.to_vec()));
    }
    let framing = if encodings > 0 {
        if encodings != 1 || !chunked || !lengths.is_empty() {
            return Err(HttpError::Malformed);
        }
        BodyFraming::Chunked
    } else {
        match lengths.first() {
            None => BodyFraming::None,
            Some(first) if lengths.iter().any(|l| l != first) => {
                return Err(HttpError::Malformed);
            }
            Some(0) => BodyFraming::None,
            Some(first) => {
                let length = usize::try_from(*first).map_err(|_| HttpError::BodyTooLarge)?;
                if length > MAX_AUTH_BODY_BYTES {
                    return Err(HttpError::BodyTooLarge);
                }
                BodyFraming::Length(length)
            }
        }
    };
    Ok((
        RequestHead {
            method,
            path,
            headers,
        },
        framing,
    ))
}

/// Body read failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BodyError {
    /// Connection closed, no response.
    #[error("body read deadline")]
    Timeout,
    /// Connection closed, no response.
    #[error("peer closed")]
    Closed,
    /// `400 bad_request`.
    #[error("body framing invalid")]
    Malformed,
    /// `413 body_too_large`.
    #[error("body too large")]
    TooLarge,
}

/// Largest single read from the socket while reading a body.
const BODY_READ_STEP: usize = 1024;

/// Byte source of a body: the already received `prefix` first, then the stream. Never
/// requests more bytes from the stream than the caller asks for.
struct BodySource<'a> {
    stream: &'a mut tokio::net::UnixStream,
    prefix: &'a [u8],
}

impl BodySource<'_> {
    /// Exactly `out.len()` bytes (prefix first). EOF or a read error is `Closed`.
    async fn fill(&mut self, out: &mut [u8]) -> Result<(), BodyError> {
        let from_prefix = self.prefix.len().min(out.len());
        let (head, mut tail) = out.split_at_mut(from_prefix);
        let (used, rest) = self.prefix.split_at(from_prefix);
        head.copy_from_slice(used);
        self.prefix = rest;
        while !tail.is_empty() {
            let step = tail.len().min(BODY_READ_STEP);
            let (window, after) = tail.split_at_mut(step);
            let mut filled = 0usize;
            while filled < step {
                let Some(dest) = window.get_mut(filled..) else {
                    return Err(BodyError::Closed);
                };
                match self.stream.read(dest).await {
                    Ok(0) | Err(_) => return Err(BodyError::Closed),
                    Ok(n) => filled = filled.saturating_add(n),
                }
            }
            tail = after;
        }
        Ok(())
    }

    /// One byte.
    async fn byte(&mut self) -> Result<u8, BodyError> {
        let mut one = [0u8; 1];
        self.fill(&mut one).await?;
        let [byte] = one;
        Ok(byte)
    }

    /// Exactly CRLF, checked byte by byte (a bare LF or any other byte is `Malformed`).
    async fn crlf(&mut self) -> Result<(), BodyError> {
        if self.byte().await? != b'\r' || self.byte().await? != b'\n' {
            return Err(BodyError::Malformed);
        }
        Ok(())
    }

    /// One chunk-size line: 1..=`MAX_CHUNK_SIZE_DIGITS` hex digits then CRLF, nothing else.
    async fn chunk_size(&mut self) -> Result<u64, BodyError> {
        let mut size: u64 = 0;
        let mut digits = 0usize;
        loop {
            let byte = self.byte().await?;
            if byte == b'\r' {
                if digits == 0 || self.byte().await? != b'\n' {
                    return Err(BodyError::Malformed);
                }
                return Ok(size);
            }
            let value = char::from(byte).to_digit(16).ok_or(BodyError::Malformed)?;
            digits = digits.saturating_add(1);
            if digits > MAX_CHUNK_SIZE_DIGITS {
                return Err(BodyError::Malformed);
            }
            size = size
                .checked_mul(16)
                .and_then(|s| s.checked_add(u64::from(value)))
                .ok_or(BodyError::Malformed)?;
        }
    }

    /// Strict bounded chunked decoding (spec §4.8).
    ///
    /// The buffer reserves `MAX_AUTH_BODY_BYTES` up front and never grows past it, so it is
    /// never reallocated: no earlier copy of a secret-bearing body (the register-options body
    /// carries the enrollment code) is freed without being zeroized.
    async fn chunked(&mut self) -> Result<Vec<u8>, BodyError> {
        let mut body = Vec::with_capacity(MAX_AUTH_BODY_BYTES);
        let mut chunks = 0usize;
        loop {
            let size = self.chunk_size().await?;
            if size == 0 {
                self.crlf().await?;
                return Ok(body);
            }
            chunks = chunks.saturating_add(1);
            if chunks > MAX_BODY_CHUNKS {
                return Err(BodyError::Malformed);
            }
            let size = usize::try_from(size).map_err(|_| BodyError::TooLarge)?;
            let total = body.len().checked_add(size).ok_or(BodyError::TooLarge)?;
            if total > MAX_AUTH_BODY_BYTES {
                return Err(BodyError::TooLarge);
            }
            let start = body.len();
            body.resize(total, 0);
            let Some(dest) = body.get_mut(start..) else {
                return Err(BodyError::Malformed);
            };
            self.fill(dest).await?;
            self.crlf().await?;
        }
    }
}

/// Reads the body under one `BODY_READ_TIMEOUT_MS` deadline, `prefix` (bytes already
/// received after the head) first, then the stream; never reads past the end of the body.
/// `Length(n)` reads exactly `n` bytes; `Chunked` is the strict bounded decoder of spec §4.8
/// (hex sizes of 1..=`MAX_CHUNK_SIZE_DIGITS` digits, no extension, OWS, bare LF or trailer,
/// at most `MAX_BODY_CHUNKS` data chunks, the decoded total checked against
/// `MAX_AUTH_BODY_BYTES` before a chunk's data is read).
///
/// # Errors
///
/// [`BodyError`].
pub async fn read_body(
    stream: &mut tokio::net::UnixStream,
    prefix: &[u8],
    framing: BodyFraming,
) -> Result<zeroize::Zeroizing<Vec<u8>>, BodyError> {
    let deadline = tokio::time::Instant::now()
        .checked_add(Duration::from_millis(BODY_READ_TIMEOUT_MS))
        .unwrap_or_else(tokio::time::Instant::now);
    let mut source = BodySource { stream, prefix };
    let read = async {
        match framing {
            BodyFraming::None => Ok(Vec::new()),
            BodyFraming::Length(length) => {
                if length > MAX_AUTH_BODY_BYTES {
                    return Err(BodyError::TooLarge);
                }
                let mut body = vec![0u8; length];
                source.fill(&mut body).await?;
                Ok(body)
            }
            BodyFraming::Chunked => source.chunked().await,
        }
    };
    match tokio::time::timeout_at(deadline, read).await {
        Ok(Ok(body)) => Ok(zeroize::Zeroizing::new(body)),
        Ok(Err(err)) => Err(err),
        Err(_) => Err(BodyError::Timeout),
    }
}

/// One `event: alerts` SSE event (`event: alerts\ndata: <json>\n\n`; ADR 2026-10-06
/// "Failed-Password Alerts in `soos-remote` From the System Journal").
#[must_use]
pub fn encode_sse_alerts_event(json: &str) -> Vec<u8> {
    format!("event: alerts\ndata: {json}\n\n").into_bytes()
}
