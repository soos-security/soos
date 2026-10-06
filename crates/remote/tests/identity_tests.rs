//! Contract tests of GitHub #339 for the pure Host and identity checks (spec §2.4, D3, D5a,
//! D5a′ effective host from `X-Forwarded-Host` (§13.2, Revision 4), RC-2).

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

use soos_remote::config::TailscaleLogin;
use soos_remote::identity::{authorize, check_host, AuthError, HostError};
use soos_remote::{
    FORWARDED_HOST_HEADER, FORWARDED_PROTO_HEADER, FORWARDED_PROTO_HTTPS, MAX_HOST_LEN,
    MAX_LOGIN_LEN,
};

fn login(raw: &str) -> TailscaleLogin {
    TailscaleLogin::parse(raw).unwrap()
}

fn allowlist() -> Vec<TailscaleLogin> {
    vec![login("owner@example.com"), login("second@example.org")]
}

fn host_check(headers: &[(&str, &[u8])]) -> Result<String, HostError> {
    check_host(headers, &[])
}

// ---------------------------------------------------------------------------------------
// authorize
// ---------------------------------------------------------------------------------------

/// D3: exactly one allowlisted login, case-insensitive name and value, OWS trimmed.
#[test]
fn test_rmc_authorize_accepts_exactly_one_allowlisted_login_case_insensitively() {
    let allowed = allowlist();
    let ok = authorize(
        &[
            ("host", b"pc.tail1234.ts.net"),
            ("Tailscale-User-Login", b" Owner@Example.COM \t"),
            ("tailscale-user-name", b"Owner"),
        ],
        &allowed,
    )
    .unwrap();
    assert_eq!(ok, login("owner@example.com"));
    let ok = authorize(&[("TAILSCALE-USER-LOGIN", b"second@example.org")], &allowed).unwrap();
    assert_eq!(ok.as_str(), "second@example.org");
}

/// RC-2: a missing identity is refused even when other headers look right.
#[test]
fn test_rmc_authorize_rejects_missing_header() {
    let allowed = allowlist();
    assert_eq!(authorize(&[], &allowed), Err(AuthError::Missing));
    assert_eq!(
        authorize(
            &[
                ("host", b"pc.tail1234.ts.net"),
                ("tailscale-user-name", b"owner@example.com"),
                ("x-tailscale-user-login", b"owner@example.com"),
                ("tailscale-user-login-x", b"owner@example.com"),
            ],
            &allowed
        ),
        Err(AuthError::Missing)
    );
}

/// D3: two identity headers are refused, even when both are allowed and identical.
#[test]
fn test_rmc_authorize_rejects_repeated_header() {
    let allowed = allowlist();
    assert_eq!(
        authorize(
            &[
                ("tailscale-user-login", b"owner@example.com"),
                ("Tailscale-User-Login", b"owner@example.com"),
            ],
            &allowed
        ),
        Err(AuthError::Repeated)
    );
    assert_eq!(
        authorize(
            &[
                ("tailscale-user-login", b"owner@example.com"),
                ("tailscale-user-login", b"nobody@example.com"),
            ],
            &allowed
        ),
        Err(AuthError::Repeated)
    );
}

/// D3: invalid UTF-8, empty, oversize or separator-bearing values are malformed.
#[test]
fn test_rmc_authorize_rejects_malformed_values() {
    let allowed = allowlist();
    let oversize = "x".repeat(MAX_LOGIN_LEN + 1);
    let cases: [&[u8]; 7] = [
        b"\xff\xfe",
        b"",
        b"   ",
        oversize.as_bytes(),
        b"owner@example.com,second@example.org",
        b"owner@example.com;x",
        b"owner@exam ple.com",
    ];
    for value in cases {
        assert_eq!(
            authorize(&[("tailscale-user-login", value)], &allowed),
            Err(AuthError::Malformed),
            "{value:?}"
        );
    }
}

/// RC-2: an unknown login, or any login against an empty allowlist, is refused; prefixes
/// and suffixes never match.
#[test]
fn test_rmc_authorize_rejects_unknown_login_and_never_matches_substrings() {
    let allowed = allowlist();
    for value in [
        &b"nobody@example.com"[..],
        b"owner@example.com.evil",
        b"xowner@example.com",
        b"owner@example.co",
        b"owner@example.com/",
    ] {
        assert_eq!(
            authorize(&[("tailscale-user-login", value)], &allowed),
            Err(AuthError::NotAllowed),
            "{value:?}"
        );
    }
    assert_eq!(
        authorize(&[("tailscale-user-login", b"owner@example.com")], &[]),
        Err(AuthError::NotAllowed)
    );
}

// ---------------------------------------------------------------------------------------
// check_host
// ---------------------------------------------------------------------------------------

/// D5a: exactly one Host header.
#[test]
fn test_rmc_check_host_requires_exactly_one_host_header() {
    assert_eq!(host_check(&[]), Err(HostError::Missing));
    assert_eq!(
        host_check(&[("tailscale-user-login", b"owner@example.com")]),
        Err(HostError::Missing)
    );
    assert_eq!(
        host_check(&[
            ("host", b"pc.tail1234.ts.net"),
            ("Host", b"pc.tail1234.ts.net")
        ]),
        Err(HostError::Repeated)
    );
}

/// D5a: without `allowed_hosts`, any syntactically valid `*.ts.net` name is accepted and
/// normalized (lowercase, `:443` stripped); everything else is `421`.
#[test]
fn test_rmc_check_host_table_without_allowlist() {
    assert_eq!(
        host_check(&[("host", b"pc.tail1234.ts.net")]),
        Ok("pc.tail1234.ts.net".to_string())
    );
    assert_eq!(
        host_check(&[("HOST", b"PC.Tail1234.TS.NET:443")]),
        Ok("pc.tail1234.ts.net".to_string())
    );
    assert_eq!(
        host_check(&[("host", b" pc.tail1234.ts.net ")]),
        Ok("pc.tail1234.ts.net".to_string()),
        "optional whitespace is trimmed"
    );
    assert_eq!(
        host_check(&[("host", b"a.ts.net")]),
        Ok("a.ts.net".to_string())
    );
    let longest = format!("{}.ts.net", "a".repeat(MAX_HOST_LEN - ".ts.net".len()));
    assert_eq!(longest.len(), MAX_HOST_LEN);
    assert_eq!(
        host_check(&[("host", longest.as_bytes())]),
        Ok(longest.clone())
    );
    let too_long = format!("a{longest}");
    let cases: Vec<Vec<u8>> = vec![
        b"pc.tail1234.ts.net:8443".to_vec(),
        b"pc.tail1234.ts.net:".to_vec(),
        b"pc.tail1234.ts.net:443:443".to_vec(),
        b"ts.net".to_vec(),
        b".ts.net".to_vec(),
        b"evil.com".to_vec(),
        b"pc.ts.net.evil.com".to_vec(),
        b"pc.tail1234.ts.net.".to_vec(),
        b"100.64.0.1".to_vec(),
        b"[fd7a:115c::1]".to_vec(),
        b"[fd7a:115c::1]:443".to_vec(),
        b"-pc.ts.net".to_vec(),
        b"pc-.ts.net".to_vec(),
        b"p_c.ts.net".to_vec(),
        b"a..ts.net".to_vec(),
        b"pc.ts.net/".to_vec(),
        b"https://pc.ts.net".to_vec(),
        b"".to_vec(),
        b"\xff.ts.net".to_vec(),
        too_long.into_bytes(),
    ];
    for value in cases {
        assert_eq!(
            host_check(&[("host", &value)]),
            Err(HostError::NotAllowed),
            "{value:?}"
        );
    }
}

/// D5a: with `allowed_hosts`, only those names pass (other `*.ts.net` names are refused),
/// still normalized.
#[test]
fn test_rmc_check_host_table_with_allowlist() {
    let allowed = vec![
        "mypc.tail1234.ts.net".to_string(),
        "lan-box.example".to_string(),
    ];
    assert_eq!(
        check_host(&[("host", b"mypc.tail1234.ts.net")], &allowed),
        Ok("mypc.tail1234.ts.net".to_string())
    );
    assert_eq!(
        check_host(&[("host", b"MYPC.tail1234.ts.net:443")], &allowed),
        Ok("mypc.tail1234.ts.net".to_string())
    );
    assert_eq!(
        check_host(&[("host", b"lan-box.example")], &allowed),
        Ok("lan-box.example".to_string())
    );
    for value in [
        &b"other.tail1234.ts.net"[..],
        b"mypc.tail1234.ts.net:8443",
        b"xmypc.tail1234.ts.net",
        b"mypc.tail1234.ts.net.evil.com",
        b"lan-box.example:80",
    ] {
        assert_eq!(
            check_host(&[("host", value)], &allowed),
            Err(HostError::NotAllowed),
            "{value:?}"
        );
    }
}

/// §4: refusal messages are fixed text (no header value echoed).
#[test]
fn test_rmc_identity_errors_never_echo_values() {
    assert_eq!(AuthError::Missing.to_string(), "identity header missing");
    assert_eq!(AuthError::Repeated.to_string(), "identity header repeated");
    assert_eq!(
        AuthError::Malformed.to_string(),
        "identity header malformed"
    );
    assert_eq!(AuthError::NotAllowed.to_string(), "identity not allowed");
    assert_eq!(HostError::Missing.to_string(), "host header missing");
    assert_eq!(HostError::Repeated.to_string(), "host header repeated");
    assert_eq!(HostError::NotAllowed.to_string(), "host not allowed");
}

// ---------------------------------------------------------------------------------------
// check_host — D5a′ effective host (spec §13.2, Revision 4)
// ---------------------------------------------------------------------------------------

const XFH: &str = "X-Forwarded-Host";
const XFP: &str = "X-Forwarded-Proto";
const OK_HOST: &str = "pc.tail1234.ts.net";

fn ok_host() -> Result<String, HostError> {
    Ok(OK_HOST.to_string())
}

/// §3 / §13.3: the forwarded header names and the accepted scheme are single-source
/// constants (lowercased names, as `IDENTITY_HEADER`).
#[test]
fn test_rmc_forwarded_header_constants() {
    assert_eq!(FORWARDED_HOST_HEADER, "x-forwarded-host");
    assert_eq!(FORWARDED_PROTO_HEADER, "x-forwarded-proto");
    assert_eq!(FORWARDED_PROTO_HTTPS, "https");
}

/// D5a′ "Effective host": when `X-Forwarded-Host` is present exactly once it decides the
/// effective host and `Host` is not inspected (absent, single or repeated give the same
/// result); `Missing` means neither header is present; a repeated `X-Forwarded-Host` is
/// `Repeated` before the proto rule is evaluated.
#[test]
fn test_rmc_check_host_effective_host_comes_from_x_forwarded_host() {
    // The captured Serve head (§13.1): `Host: localhost`, real name in XFH.
    assert_eq!(
        host_check(&[
            ("Host", b"localhost"),
            ("Tailscale-User-Login", b"owner@example.com"),
            ("X-Forwarded-For", b"100.64.0.1"),
            (XFH, OK_HOST.as_bytes()),
            (XFP, b"https"),
        ]),
        ok_host(),
        "Host: localhost is not inspected when X-Forwarded-Host is present"
    );
    assert_eq!(
        host_check(&[(XFH, OK_HOST.as_bytes()), (XFP, b"https")]),
        ok_host(),
        "Host absent"
    );
    assert_eq!(
        host_check(&[
            ("Host", b"localhost"),
            ("host", b"evil.com"),
            (XFH, OK_HOST.as_bytes()),
            (XFP, b"https"),
        ]),
        ok_host(),
        "a repeated Host is not inspected either"
    );
    assert_eq!(
        host_check(&[
            ("Host", b"evil.com"),
            (XFH, OK_HOST.as_bytes()),
            (XFP, b"https")
        ]),
        ok_host(),
        "a single Host that would be refused on its own is not inspected"
    );
    assert_eq!(
        host_check(&[(XFP, b"https")]),
        Err(HostError::Missing),
        "neither X-Forwarded-Host nor Host"
    );
    assert_eq!(
        host_check(&[("X-Forwarded-For", b"100.64.0.1"), (XFP, b"https")]),
        Err(HostError::Missing),
        "X-Forwarded-For never stands in for the host"
    );
    assert_eq!(
        host_check(&[
            ("Host", b"localhost"),
            (XFH, OK_HOST.as_bytes()),
            ("x-forwarded-host", OK_HOST.as_bytes()),
            (XFP, b"https"),
        ]),
        Err(HostError::Repeated),
        "X-Forwarded-Host twice (identical values, case-insensitive name)"
    );
    assert_eq!(
        host_check(&[
            (XFH, OK_HOST.as_bytes()),
            (XFH, OK_HOST.as_bytes()),
            (XFP, b"http"),
        ]),
        Err(HostError::Repeated),
        "the repeated check precedes the proto check"
    );
    assert_eq!(
        host_check(&[("Host", OK_HOST.as_bytes()), ("Host", OK_HOST.as_bytes())]),
        Err(HostError::Repeated),
        "without X-Forwarded-Host the D5a Host rules still apply"
    );
}

/// D5a′ "Normalisation": the forwarded host goes through the same pipeline as `Host`
/// (OWS, lowercase, one `:443`, `is_valid_host_name`, `*.ts.net` rule).
#[test]
fn test_rmc_check_host_normalises_the_forwarded_host() {
    assert_eq!(
        host_check(&[
            ("Host", b"localhost"),
            (XFH, b" PC.Tail1234.TS.NET:443 "),
            (XFP, b"https"),
        ]),
        ok_host()
    );
    assert_eq!(
        host_check(&[("Host", b"localhost"), (XFH, b"a.ts.net"), (XFP, b"https")]),
        Ok("a.ts.net".to_string())
    );
    let longest = format!("{}.ts.net", "a".repeat(MAX_HOST_LEN - ".ts.net".len()));
    assert_eq!(
        host_check(&[
            ("Host", b"localhost"),
            (XFH, longest.as_bytes()),
            (XFP, b"https"),
        ]),
        Ok(longest.clone())
    );
    let too_long = format!("a{longest}");
    let cases: Vec<Vec<u8>> = vec![
        b"pc.tail1234.ts.net:8443".to_vec(),
        b"pc.tail1234.ts.net:443:443".to_vec(),
        b"100.64.0.1".to_vec(),
        b"[fd7a:115c::1]:443".to_vec(),
        b"evil.com".to_vec(),
        b"localhost".to_vec(),
        b"a.ts.net, b.ts.net".to_vec(),
        b"a.ts.net,b.ts.net".to_vec(),
        b"".to_vec(),
        b"   ".to_vec(),
        b"pc.tail1234.ts.net.".to_vec(),
        b"ts.net".to_vec(),
        b"pc.ts.net.evil.com".to_vec(),
        b"p_c.ts.net".to_vec(),
        b"https://pc.tail1234.ts.net".to_vec(),
        b"\xff.ts.net".to_vec(),
        too_long.into_bytes(),
    ];
    for value in cases {
        assert_eq!(
            host_check(&[("Host", b"localhost"), (XFH, &value), (XFP, b"https")]),
            Err(HostError::NotAllowed),
            "{value:?}"
        );
        assert_eq!(
            host_check(&[("Host", OK_HOST.as_bytes()), (XFH, &value), (XFP, b"https")]),
            Err(HostError::NotAllowed),
            "a valid Host never rescues a refused X-Forwarded-Host: {value:?}"
        );
    }
}

/// D5a′ "Transport": with `X-Forwarded-Host`, exactly one `X-Forwarded-Proto` equal to
/// `https` (ASCII-case-insensitive, OWS-trimmed) is mandatory; without it the proto header
/// is optional but, when present, must still be exactly one `https`.
#[test]
fn test_rmc_check_host_requires_https_forwarded_proto() {
    let xfh: (&str, &[u8]) = (XFH, OK_HOST.as_bytes());
    let host: (&str, &[u8]) = ("Host", OK_HOST.as_bytes());
    let localhost: (&str, &[u8]) = ("Host", b"localhost");

    // With X-Forwarded-Host: mandatory.
    assert_eq!(
        host_check(&[localhost, xfh]),
        Err(HostError::NotAllowed),
        "proto absent with X-Forwarded-Host"
    );
    assert_eq!(
        host_check(&[localhost, xfh, (XFP, b"http")]),
        Err(HostError::NotAllowed),
        "tailscale serve --http"
    );
    assert_eq!(
        host_check(&[
            localhost,
            xfh,
            (XFP, b"https"),
            ("x-forwarded-proto", b"https")
        ]),
        Err(HostError::NotAllowed),
        "two identical https values are still repeated"
    );
    assert_eq!(
        host_check(&[localhost, xfh, (XFP, b"HTTPS")]),
        ok_host(),
        "scheme compared ASCII-case-insensitively"
    );
    assert_eq!(
        host_check(&[localhost, xfh, (XFP, b" https ")]),
        ok_host(),
        "OWS around the scheme is trimmed"
    );
    assert_eq!(
        host_check(&[localhost, xfh, (XFP, b"\thttps\t")]),
        ok_host(),
        "HTAB is OWS"
    );
    for bad in [
        &b"https, https"[..],
        b"https,http",
        b"http, https",
        b"https:",
        b"https://",
        b"wss",
        b"",
        b"   ",
        b"\xff",
        b"httpsx",
        b"xhttps",
    ] {
        assert_eq!(
            host_check(&[localhost, xfh, (XFP, bad)]),
            Err(HostError::NotAllowed),
            "{bad:?}"
        );
    }

    // Without X-Forwarded-Host: optional, but https when present.
    assert_eq!(host_check(&[host]), ok_host(), "direct local client");
    assert_eq!(host_check(&[host, (XFP, b"https")]), ok_host());
    assert_eq!(host_check(&[host, (XFP, b"HTTPS")]), ok_host());
    assert_eq!(
        host_check(&[host, (XFP, b"http")]),
        Err(HostError::NotAllowed),
        "Host only + proto http"
    );
    assert_eq!(
        host_check(&[host, (XFP, b"https"), (XFP, b"https")]),
        Err(HostError::NotAllowed),
        "Host only + proto https twice"
    );
    assert_eq!(
        host_check(&[host, (XFP, b"")]),
        Err(HostError::NotAllowed),
        "Host only + empty proto"
    );
}

/// D5a′ + `allowed_hosts`: the allowlist applies to the effective host, so a listed
/// `X-Forwarded-Host` passes whatever `Host` says, and an unlisted one is refused even when
/// `Host` is listed.
#[test]
fn test_rmc_check_host_allowlist_applies_to_the_forwarded_host() {
    let allowed = vec!["mypc.tail1234.ts.net".to_string()];
    assert_eq!(
        check_host(
            &[
                ("Host", b"other.tail1234.ts.net"),
                (XFH, b"mypc.tail1234.ts.net"),
                (XFP, b"https"),
            ],
            &allowed
        ),
        Ok("mypc.tail1234.ts.net".to_string())
    );
    assert_eq!(
        check_host(
            &[
                ("Host", b"localhost"),
                (XFH, b"MYPC.tail1234.ts.net:443"),
                (XFP, b"https"),
            ],
            &allowed
        ),
        Ok("mypc.tail1234.ts.net".to_string())
    );
    assert_eq!(
        check_host(
            &[
                ("Host", b"mypc.tail1234.ts.net"),
                (XFH, b"other.tail1234.ts.net"),
                (XFP, b"https"),
            ],
            &allowed
        ),
        Err(HostError::NotAllowed),
        "a listed Host never rescues an unlisted X-Forwarded-Host"
    );
    assert_eq!(
        check_host(
            &[
                ("Host", b"localhost"),
                (XFH, b"mypc.tail1234.ts.net"),
                (XFP, b"http"),
            ],
            &allowed
        ),
        Err(HostError::NotAllowed),
        "the proto rule holds with an allowlist"
    );
}
