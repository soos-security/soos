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

// ---------------------------------------------------------------------------------------
// Funnel classification and client hint (ADR 2026-10-06 "Tailscale Funnel Access and
// In-House Passkey Authentication for `soos-remote`", spec §2.1, §2.4, tests 8–10 and 58,
// matrix RMC26 / RMC37)
// ---------------------------------------------------------------------------------------

mod funnel_classification {
    use super::*;
    use soos_remote::identity::{classify_request, client_hint, Caller, ClientHint, PathClass};
    use soos_remote::{FORWARDED_FOR_HEADER, FUNNEL_HEADER, FUNNEL_HEADER_VALUE};

    const LOGIN_HEADER: &str = "Tailscale-User-Login";
    const MARKER: &str = "Tailscale-Funnel-Request";

    fn classify(headers: &[(&str, &[u8])], allow_funnel: bool) -> Result<Caller, AuthError> {
        classify_request(headers, &allowlist(), allow_funnel)
    }

    /// Test 8 (RMC26): the six rules of spec §2.1, in order.
    #[test]
    fn test_rmc_classify_request_table() {
        assert_eq!(FUNNEL_HEADER, "tailscale-funnel-request");
        assert_eq!(FUNNEL_HEADER_VALUE, "?1");
        let owner: &[u8] = b"owner@example.com";
        for allow_funnel in [false, true] {
            // Rule 2: no marker ⇒ the unchanged identity check.
            assert_eq!(
                classify(&[(LOGIN_HEADER, owner)], allow_funnel),
                Ok(Caller::Tailnet(login("owner@example.com")))
            );
            assert_eq!(
                classify(&[("host", b"pc.tail1234.ts.net")], allow_funnel),
                Err(AuthError::Missing)
            );
            assert_eq!(
                classify(
                    &[(LOGIN_HEADER, owner), (LOGIN_HEADER, owner)],
                    allow_funnel
                ),
                Err(AuthError::Repeated)
            );
            assert_eq!(
                classify(&[(LOGIN_HEADER, b"evil@example.com")], allow_funnel),
                Err(AuthError::NotAllowed)
            );
            assert_eq!(
                classify(&[(LOGIN_HEADER, b"a,b")], allow_funnel),
                Err(AuthError::Malformed)
            );
            // Rule 1: both markers ⇒ ambiguous, whatever the values and the setting.
            for marker in [&b"?1"[..], b"?0", b""] {
                assert_eq!(
                    classify(&[(LOGIN_HEADER, owner), (MARKER, marker)], allow_funnel),
                    Err(AuthError::Ambiguous),
                    "{marker:?}"
                );
                assert_eq!(
                    classify(
                        &[(MARKER, marker), (LOGIN_HEADER, b"evil@x.io")],
                        allow_funnel
                    ),
                    Err(AuthError::Ambiguous)
                );
            }
            // Rule 3: a repeated marker, before the value and the setting.
            assert_eq!(
                classify(&[(MARKER, b"?1"), (MARKER, b"?1")], allow_funnel),
                Err(AuthError::Repeated)
            );
            // Rule 4: another value, before the setting.
            for bad in [&b"?0"[..], b"?1?1", b"1", b"", b"?1,?1", b"?T", b"\xff"] {
                assert_eq!(
                    classify(&[(MARKER, bad)], allow_funnel),
                    Err(AuthError::Malformed),
                    "{bad:?}"
                );
            }
        }
        // Rules 5 and 6.
        for value in [&b"?1"[..], b" ?1", b"?1\t", b" \t?1 "] {
            assert_eq!(
                classify(&[(MARKER, value)], false),
                Err(AuthError::FunnelDisabled),
                "{value:?}"
            );
            assert_eq!(classify(&[(MARKER, value)], true), Ok(Caller::Funnel));
        }
        assert_eq!(
            classify(&[("TAILSCALE-FUNNEL-REQUEST", b"?1")], true),
            Ok(Caller::Funnel),
            "names are ASCII-case-insensitive"
        );
        assert_eq!(Caller::Funnel.class(), PathClass::Funnel);
        assert_eq!(
            Caller::Tailnet(login("owner@example.com")).class(),
            PathClass::Tailnet
        );
        assert_eq!(
            AuthError::Ambiguous.to_string(),
            "identity and funnel markers both present"
        );
        assert_eq!(
            AuthError::FunnelDisabled.to_string(),
            "funnel access disabled"
        );
    }

    /// Test 9 (RMC26, spec §2.2 item 4): spelling variants pass through `tailscaled` but are
    /// never matched by the service.
    #[test]
    fn test_rmc_classify_request_ignores_spelling_variants() {
        let owner: &[u8] = b"owner@example.com";
        for name in [
            "Tailscale_User_Login",
            "Tailscale-User-Name",
            "Tailscale-User-Login-Name",
            "X-Tailscale-User-Login",
            "TailscaleUserLogin",
        ] {
            for allow_funnel in [false, true] {
                assert_eq!(
                    classify(&[(name, owner)], allow_funnel),
                    Err(AuthError::Missing),
                    "{name} never authorizes"
                );
            }
        }
        for name in [
            "Tailscale_Funnel_Request",
            "Tailscale-Funnel-Requests",
            "X-Tailscale-Funnel-Request",
        ] {
            assert_eq!(
                classify(&[(name, b"?1")], true),
                Err(AuthError::Missing),
                "{name} never classifies as Funnel"
            );
            assert_eq!(
                classify(&[(LOGIN_HEADER, owner), (name, b"?1")], true),
                Ok(Caller::Tailnet(login("owner@example.com"))),
                "{name} is not the marker, so the request is not ambiguous"
            );
        }
        assert_eq!(
            classify(&[(MARKER, b"?1"), ("Tailscale_User_Login", owner)], true),
            Ok(Caller::Funnel),
            "an underscore login next to the marker is an anonymous Funnel caller"
        );
    }

    /// Test 10 (RMC26, spec §2.2 item 8): `X-Forwarded-For`, `X-Forwarded-Host` and `Host`
    /// never change the classification.
    #[test]
    fn test_rmc_classify_request_never_reads_forwarding_headers() {
        let owner: &[u8] = b"owner@example.com";
        let bases: Vec<Vec<(&str, &[u8])>> = vec![
            vec![],
            vec![(LOGIN_HEADER, owner)],
            vec![(LOGIN_HEADER, b"evil@example.com")],
            vec![(MARKER, b"?1")],
            vec![(MARKER, b"?0")],
            vec![(MARKER, b"?1"), (LOGIN_HEADER, owner)],
        ];
        let extras: Vec<Vec<(&str, &[u8])>> = vec![
            vec![("x-forwarded-for", b"203.0.113.7")],
            vec![
                ("x-forwarded-for", b"127.0.0.1"),
                ("x-forwarded-for", b"::1"),
            ],
            vec![("x-forwarded-host", b"pc.tail1234.ts.net")],
            vec![("x-forwarded-host", b"evil.example.com:8443")],
            vec![("host", b"localhost")],
            vec![
                ("host", b"pc.tail1234.ts.net"),
                ("x-forwarded-proto", b"http"),
            ],
            vec![("forwarded", b"for=203.0.113.7;host=pc.tail1234.ts.net")],
        ];
        for base in &bases {
            for allow_funnel in [false, true] {
                let expected = classify(base, allow_funnel);
                for extra in &extras {
                    let mut headers = base.clone();
                    headers.extend_from_slice(extra);
                    assert_eq!(
                        classify(&headers, allow_funnel),
                        expected,
                        "{base:?} + {extra:?}"
                    );
                }
            }
        }
    }

    fn hint(value: &[u8]) -> ClientHint {
        client_hint(&[(FORWARDED_FOR_HEADER, value)])
    }

    /// Test 58 (RMC37, plan-evaluator G-4): exactly one `X-Forwarded-For` holding one IP
    /// address; IPv4 and IPv4-mapped IPv6 share a hint, distinct IPv4 addresses never do,
    /// IPv6 is masked to its /64; anything else is the shared `UNKNOWN` bucket; `Debug` is
    /// redacted.
    #[test]
    fn test_rmc_client_hint_parsing() {
        assert_eq!(FORWARDED_FOR_HEADER, "x-forwarded-for");
        let v4 = hint(b"203.0.113.7");
        assert_ne!(v4, ClientHint::UNKNOWN);
        assert_eq!(
            v4,
            hint(b"::ffff:203.0.113.7"),
            "IPv4-mapped IPv6 shares the hint"
        );
        assert_eq!(v4, hint(b" 203.0.113.7\t"), "OWS trimmed");
        assert_eq!(
            v4,
            client_hint(&[("X-Forwarded-For", b"203.0.113.7")]),
            "name is ASCII-case-insensitive"
        );
        // G-4: IPv4 is never collapsed by the /64 mask.
        assert_ne!(v4, hint(b"203.0.113.8"));
        assert_ne!(v4, hint(b"203.0.114.7"));
        assert_ne!(hint(b"10.0.0.1"), hint(b"10.0.0.2"));
        assert_ne!(hint(b"198.51.100.1"), hint(b"2001:db8::1"));
        assert_ne!(hint(b"::ffff:203.0.113.7"), hint(b"::ffff:203.0.113.9"));
        // IPv6 /64.
        let a = hint(b"2001:db8:1:2:aaaa::1");
        assert_ne!(a, ClientHint::UNKNOWN);
        assert_eq!(a, hint(b"2001:db8:1:2:bbbb::2"));
        assert_eq!(a, hint(b"2001:DB8:1:2:ffff:ffff:ffff:ffff"));
        assert_ne!(a, hint(b"2001:db8:1:3::1"));
        assert_ne!(a, hint(b"2001:db9:1:2::1"));
        // UNKNOWN.
        assert_eq!(client_hint(&[]), ClientHint::UNKNOWN);
        assert_eq!(
            client_hint(&[("host", b"pc.tail1234.ts.net")]),
            ClientHint::UNKNOWN
        );
        assert_eq!(
            client_hint(&[
                (FORWARDED_FOR_HEADER, b"203.0.113.7"),
                (FORWARDED_FOR_HEADER, b"203.0.113.7"),
            ]),
            ClientHint::UNKNOWN,
            "a repeated header"
        );
        let forty_six = "1".repeat(46);
        for bad in [
            &b"203.0.113.7, 198.51.100.1"[..],
            b"203.0.113.7,",
            b"203.0.113.7:443",
            b"[2001:db8::1]",
            b"[2001:db8::1]:443",
            b"fe80::1%eth0",
            forty_six.as_bytes(),
            b"\xff\xfe",
            b"",
            b" ",
            b"unknown",
            b"203.0.113",
            b"203.0.113.256",
            b"0x7f.0.0.1",
        ] {
            assert_eq!(
                hint(bad),
                ClientHint::UNKNOWN,
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        // Debug never shows the address.
        for h in [v4, a, ClientHint::UNKNOWN] {
            let shown = format!("{h:?}");
            assert!(shown.contains("<redacted>"), "{shown}");
            assert!(!shown.bytes().any(|b| b.is_ascii_digit()), "{shown}");
        }
    }
}
