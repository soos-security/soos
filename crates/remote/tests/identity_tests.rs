//! Contract tests of GitHub #339 for the pure Host and identity checks (spec §2.4, D3, D5a,
//! RC-2).

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
use soos_remote::{MAX_HOST_LEN, MAX_LOGIN_LEN};

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
