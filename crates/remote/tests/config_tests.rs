//! Contract tests of GitHub #339 for `soos-remote` configuration (spec §2.3, §3, D1, D3).
//!
//! Pure parsing over text plus a bounded file read in a tempdir; nothing touches
//! `$XDG_RUNTIME_DIR`, `$HOME` or `/etc`.

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

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use proptest::prelude::*;

use soos_remote::config::{
    check_not_root, default_config_path, load_config, parse_config, ConfigError, RemoteConfig,
    TailscaleLogin,
};
use soos_remote::{
    ACTION_HEADER, ACTION_LOCK, DBUS_CALL_TIMEOUT_MS, DBUS_CONNECT_TIMEOUT_MS,
    DEFAULT_POLL_INTERVAL_MS, EXIT_CONFIG, EXIT_RUNTIME, IDENTITY_HEADER, LOCK_FLOW_DEADLINE_MS,
    MAX_ALLOWED_HOSTS, MAX_ALLOWED_LOGINS, MAX_CONFIG_BYTES, MAX_CONNECTIONS, MAX_HEADERS,
    MAX_HOST_LEN, MAX_LISTED_SESSIONS, MAX_LOGIND_ERROR_LEN, MAX_LOGIN_LEN, MAX_OWN_SESSIONS,
    MAX_PATH_LEN, MAX_POLL_INTERVAL_MS, MAX_REQUEST_HEAD_BYTES, MAX_SESSION_ID_LEN,
    MAX_SOCKET_PATH_LEN, MAX_SSE_STREAMS, MAX_SSE_STREAM_MS, MIN_LOCK_INTERVAL_MS,
    MIN_POLL_INTERVAL_MS, REQUEST_HEAD_TIMEOUT_MS, RESPONSE_WRITE_TIMEOUT_MS, SNAPSHOT_DEADLINE_MS,
    SOCKET_DIR_NAME, SOCKET_FILE_NAME, SSE_KEEPALIVE_MS, SYSTEM_BUS_ADDRESS, TS_NET_SUFFIX,
};

const RUNTIME_DIR: &str = "/run/user/1000";

fn login(raw: &str) -> TailscaleLogin {
    TailscaleLogin::parse(raw).unwrap_or_else(|| panic!("{raw:?} must be a valid login"))
}

fn parse(text: &str) -> Result<RemoteConfig, ConfigError> {
    parse_config(text, Some(Path::new(RUNTIME_DIR)))
}

// ---------------------------------------------------------------------------------------
// §3 constants (single source) — guard
// ---------------------------------------------------------------------------------------

/// §3: every constant has the specified value.
#[test]
fn test_rmc_constants_match_the_spec() {
    assert_eq!(MAX_CONFIG_BYTES, 16_384);
    assert_eq!(MAX_ALLOWED_LOGINS, 8);
    assert_eq!(MAX_LOGIN_LEN, 254);
    assert_eq!(DEFAULT_POLL_INTERVAL_MS, 1000);
    assert_eq!(MIN_POLL_INTERVAL_MS, 250);
    assert_eq!(MAX_POLL_INTERVAL_MS, 10_000);
    assert_eq!(MAX_SOCKET_PATH_LEN, 107);
    assert_eq!(SOCKET_DIR_NAME, "soos-remote");
    assert_eq!(SOCKET_FILE_NAME, "remote.sock");
    assert_eq!(MAX_CONNECTIONS, 16);
    assert_eq!(MAX_SSE_STREAMS, 4);
    assert_eq!(MAX_REQUEST_HEAD_BYTES, 8192);
    assert_eq!(MAX_HEADERS, 32);
    assert_eq!(MAX_PATH_LEN, 256);
    assert_eq!(REQUEST_HEAD_TIMEOUT_MS, 5000);
    assert_eq!(RESPONSE_WRITE_TIMEOUT_MS, 2000);
    assert_eq!(SSE_KEEPALIVE_MS, 15_000);
    assert_eq!(MAX_SSE_STREAM_MS, 1_800_000);
    assert_eq!(MIN_LOCK_INTERVAL_MS, 2000);
    assert_eq!(DBUS_CALL_TIMEOUT_MS, 500);
    assert_eq!(DBUS_CONNECT_TIMEOUT_MS, 1000);
    assert_eq!(MAX_LISTED_SESSIONS, 256);
    assert_eq!(MAX_OWN_SESSIONS, 16);
    assert_eq!(MAX_SESSION_ID_LEN, 64);
    assert_eq!(MAX_LOGIND_ERROR_LEN, 256);
    assert_eq!(SYSTEM_BUS_ADDRESS, "unix:path=/run/dbus/system_bus_socket");
    assert_eq!(SNAPSHOT_DEADLINE_MS, 1500);
    assert_eq!(LOCK_FLOW_DEADLINE_MS, 2000);
    assert_eq!(MAX_ALLOWED_HOSTS, 4);
    assert_eq!(MAX_HOST_LEN, 253);
    assert_eq!(TS_NET_SUFFIX, ".ts.net");
    assert_eq!(EXIT_CONFIG, 78);
    assert_eq!(EXIT_RUNTIME, 1);
    assert_eq!(IDENTITY_HEADER, "tailscale-user-login");
    assert_eq!(ACTION_HEADER, "x-soos-action");
    assert_eq!(ACTION_LOCK, "lock");
    // The keep-alive must be able to fire several times inside one stream lifetime, and
    // the snapshot deadline must exceed one D-Bus call bound (a snapshot is several calls).
    let (keepalive, stream, snapshot, call, lock_flow) = (
        SSE_KEEPALIVE_MS,
        MAX_SSE_STREAM_MS,
        SNAPSHOT_DEADLINE_MS,
        DBUS_CALL_TIMEOUT_MS,
        LOCK_FLOW_DEADLINE_MS,
    );
    assert!(keepalive * 4 <= stream);
    assert!(snapshot > call);
    assert!(lock_flow >= snapshot);
}

// ---------------------------------------------------------------------------------------
// TailscaleLogin::parse
// ---------------------------------------------------------------------------------------

/// D3: a login is printable ASCII, lowercased, bounded.
#[test]
fn test_rmc_login_parse_accepts_and_lowercases_printable_ascii() {
    let parsed = login("Owner@Example.COM");
    assert_eq!(parsed.as_str(), "owner@example.com");
    assert_eq!(login("a").as_str(), "a");
    let max = "x".repeat(MAX_LOGIN_LEN);
    assert_eq!(login(&max).as_str(), max);
    assert_eq!(login("user+tag@github").as_str(), "user+tag@github");
    assert_eq!(
        login("~!#$%&'()*+-./:<=>?@[]^_`{|}").as_str(),
        "~!#$%&'()*+-./:<=>?@[]^_`{|}"
    );
    assert_eq!(
        login("ABC"),
        login("abc"),
        "equality is on the lowercased form"
    );
}

/// D3: empty, oversize, whitespace, control, non-ASCII and the three separators are refused.
#[test]
fn test_rmc_login_parse_rejects_empty_oversize_and_forbidden_bytes() {
    for raw in [
        "",
        &"x".repeat(MAX_LOGIN_LEN + 1),
        "a b",
        " a",
        "a\t",
        "a\n",
        "a,b",
        "a;b",
        "a\"b",
        "é@example.com",
        "a\u{7f}",
        "a\u{0}",
    ] {
        assert!(
            TailscaleLogin::parse(raw).is_none(),
            "{raw:?} must be refused"
        );
    }
}

proptest! {
    /// The parser never panics; an accepted login is lowercase ASCII within the bound and
    /// re-parses to itself.
    #[test]
    fn prop_rmc_login_parse_never_panics_and_is_idempotent(raw in "\\PC{0,300}") {
        if let Some(parsed) = TailscaleLogin::parse(&raw) {
            let s = parsed.as_str();
            prop_assert!(!s.is_empty() && s.len() <= MAX_LOGIN_LEN);
            prop_assert!(s.bytes().all(|b| (0x21..=0x7e).contains(&b)));
            prop_assert!(!s.bytes().any(|b| b.is_ascii_uppercase()));
            prop_assert!(!s.contains([',', ';', '"']));
            prop_assert_eq!(TailscaleLogin::parse(s), Some(parsed));
        }
    }
}

// ---------------------------------------------------------------------------------------
// parse_config
// ---------------------------------------------------------------------------------------

/// §2.3: a minimal file yields the documented defaults.
#[test]
fn test_rmc_parse_config_minimal_defaults() {
    let config = parse("allowed_logins = [\"owner@example.com\"]\n").unwrap();
    assert_eq!(config.allowed_logins, vec![login("owner@example.com")]);
    assert_eq!(
        config.socket_path,
        PathBuf::from("/run/user/1000/soos-remote/remote.sock")
    );
    assert_eq!(config.poll_interval_ms, DEFAULT_POLL_INTERVAL_MS);
    assert!(config.allowed_hosts.is_empty());
}

/// §2.3: every field; logins deduplicated after lowercasing with the order kept; hosts
/// lowercased.
#[test]
fn test_rmc_parse_config_full_file() {
    let text = "allowed_logins = [\"B@x.io\", \"a@x.io\", \"b@X.IO\", \"c@x.io\"]\n\
                socket_path = \"/tmp/soos-test/remote.sock\"\n\
                poll_interval_ms = 250\n\
                allowed_hosts = [\"MyPC.Tail1234.TS.NET\", \"lan-box.example\"]\n";
    let config = parse(text).unwrap();
    assert_eq!(
        config.allowed_logins,
        vec![login("b@x.io"), login("a@x.io"), login("c@x.io")]
    );
    assert_eq!(
        config.socket_path,
        PathBuf::from("/tmp/soos-test/remote.sock")
    );
    assert_eq!(config.poll_interval_ms, 250);
    assert_eq!(
        config.allowed_hosts,
        vec![
            "mypc.tail1234.ts.net".to_string(),
            "lan-box.example".to_string()
        ]
    );
}

/// D3 / RC-2: no allowlist means no start.
#[test]
fn test_rmc_parse_config_rejects_missing_or_empty_logins() {
    assert_eq!(parse(""), Err(ConfigError::NoAllowedLogins));
    assert_eq!(
        parse("poll_interval_ms = 1000\n"),
        Err(ConfigError::NoAllowedLogins)
    );
    assert_eq!(
        parse("allowed_logins = []\n"),
        Err(ConfigError::NoAllowedLogins)
    );
}

/// §3: 8 logins pass, 9 are refused.
#[test]
fn test_rmc_parse_config_login_count_bound() {
    let eight: Vec<String> = (0..MAX_ALLOWED_LOGINS)
        .map(|i| format!("\"u{i}@x.io\""))
        .collect();
    let ok = parse(&format!("allowed_logins = [{}]\n", eight.join(", "))).unwrap();
    assert_eq!(ok.allowed_logins.len(), MAX_ALLOWED_LOGINS);
    let nine: Vec<String> = (0..=MAX_ALLOWED_LOGINS)
        .map(|i| format!("\"u{i}@x.io\""))
        .collect();
    assert_eq!(
        parse(&format!("allowed_logins = [{}]\n", nine.join(", "))),
        Err(ConfigError::TooManyLogins {
            max: MAX_ALLOWED_LOGINS
        })
    );
}

/// §2.3: an invalid login is reported with its file index.
#[test]
fn test_rmc_parse_config_rejects_invalid_login_with_index() {
    assert_eq!(
        parse("allowed_logins = [\"ok@x.io\", \"bad login\"]\n"),
        Err(ConfigError::InvalidLogin { index: 1 })
    );
    assert_eq!(
        parse("allowed_logins = [\"\"]\n"),
        Err(ConfigError::InvalidLogin { index: 0 })
    );
    assert_eq!(
        parse("allowed_logins = [\"a@x.io\", \"b@x.io\", \"c,d@x.io\"]\n"),
        Err(ConfigError::InvalidLogin { index: 2 })
    );
    // A non-string entry is a TOML shape error.
    assert_eq!(parse("allowed_logins = [1]\n"), Err(ConfigError::Syntax));
}

/// §2.3 sentinels: `poll_interval_ms` is rejected outside the range, never clamped.
#[test]
fn test_rmc_parse_config_poll_interval_bounds() {
    for bad in [
        0,
        MIN_POLL_INTERVAL_MS - 1,
        MAX_POLL_INTERVAL_MS + 1,
        i64::MAX as u64,
    ] {
        assert_eq!(
            parse(&format!(
                "allowed_logins = [\"o@x.io\"]\npoll_interval_ms = {bad}\n"
            )),
            Err(ConfigError::PollIntervalOutOfRange),
            "{bad}"
        );
    }
    for good in [
        MIN_POLL_INTERVAL_MS,
        DEFAULT_POLL_INTERVAL_MS,
        MAX_POLL_INTERVAL_MS,
    ] {
        let config = parse(&format!(
            "allowed_logins = [\"o@x.io\"]\npoll_interval_ms = {good}\n"
        ))
        .unwrap();
        assert_eq!(config.poll_interval_ms, good);
    }
    assert_eq!(
        parse("allowed_logins = [\"o@x.io\"]\npoll_interval_ms = -1\n"),
        Err(ConfigError::Syntax)
    );
    assert_eq!(
        parse("allowed_logins = [\"o@x.io\"]\npoll_interval_ms = \"1000\"\n"),
        Err(ConfigError::Syntax)
    );
}

/// §2.3 `deny_unknown_fields` and TOML syntax.
#[test]
fn test_rmc_parse_config_rejects_unknown_fields_and_bad_toml() {
    assert_eq!(
        parse("allowed_logins = [\"o@x.io\"]\nallowed_login = [\"x\"]\n"),
        Err(ConfigError::Syntax)
    );
    assert_eq!(
        parse("allowed_logins = [\"o@x.io\"]\n[extra]\nkey = 1\n"),
        Err(ConfigError::Syntax)
    );
    assert_eq!(
        parse("allowed_logins = [\"o@x.io\"\n"),
        Err(ConfigError::Syntax)
    );
    assert_eq!(parse("not toml at all ::: {"), Err(ConfigError::Syntax));
}

/// §2.3 / §3: socket path rules and the 107-byte `sun_path` bound.
#[test]
fn test_rmc_parse_config_socket_path_rules() {
    let err = Err(ConfigError::InvalidSocketPath {
        max: MAX_SOCKET_PATH_LEN,
    });
    for bad in [
        "relative/remote.sock",
        "/run/user/1000/soos-remote/",
        "/",
        "",
    ] {
        assert_eq!(
            parse(&format!(
                "allowed_logins = [\"o@x.io\"]\nsocket_path = \"{bad}\"\n"
            )),
            err,
            "{bad:?}"
        );
    }
    let longest = format!("/{}", "a".repeat(MAX_SOCKET_PATH_LEN - 1));
    assert_eq!(longest.len(), MAX_SOCKET_PATH_LEN);
    let config = parse(&format!(
        "allowed_logins = [\"o@x.io\"]\nsocket_path = \"{longest}\"\n"
    ))
    .unwrap();
    assert_eq!(config.socket_path, PathBuf::from(&longest));
    let too_long = format!("/{}", "a".repeat(MAX_SOCKET_PATH_LEN));
    assert_eq!(
        parse(&format!(
            "allowed_logins = [\"o@x.io\"]\nsocket_path = \"{too_long}\"\n"
        )),
        err
    );
}

/// §2.3: without `socket_path`, `XDG_RUNTIME_DIR` must be set and absolute (no `/tmp`
/// fallback); with an explicit `socket_path` the runtime dir is irrelevant.
#[test]
fn test_rmc_parse_config_runtime_dir_rules() {
    let minimal = "allowed_logins = [\"o@x.io\"]\n";
    assert_eq!(parse_config(minimal, None), Err(ConfigError::NoRuntimeDir));
    assert_eq!(
        parse_config(minimal, Some(Path::new(""))),
        Err(ConfigError::NoRuntimeDir)
    );
    assert_eq!(
        parse_config(minimal, Some(Path::new("run/user/1000"))),
        Err(ConfigError::NoRuntimeDir)
    );
    let explicit = "allowed_logins = [\"o@x.io\"]\nsocket_path = \"/tmp/x/remote.sock\"\n";
    let config = parse_config(explicit, None).unwrap();
    assert_eq!(config.socket_path, PathBuf::from("/tmp/x/remote.sock"));
    let config = parse_config(minimal, Some(Path::new("/run/user/4242"))).unwrap();
    assert_eq!(
        config.socket_path,
        Path::new("/run/user/4242")
            .join(SOCKET_DIR_NAME)
            .join(SOCKET_FILE_NAME)
    );
    // A runtime dir of 100 bytes leaves no room for the default file name.
    let deep = format!("/{}", "r".repeat(99));
    assert_eq!(
        parse_config(minimal, Some(Path::new(&deep))),
        Err(ConfigError::InvalidSocketPath {
            max: MAX_SOCKET_PATH_LEN
        })
    );
}

/// D5a / §3: host list bound and DNS-name validation.
#[test]
fn test_rmc_parse_config_host_rules() {
    let five: Vec<String> = (0..=MAX_ALLOWED_HOSTS)
        .map(|i| format!("\"h{i}.ts.net\""))
        .collect();
    assert_eq!(
        parse(&format!(
            "allowed_logins = [\"o@x.io\"]\nallowed_hosts = [{}]\n",
            five.join(", ")
        )),
        Err(ConfigError::TooManyHosts {
            max: MAX_ALLOWED_HOSTS
        })
    );
    let four: Vec<String> = (0..MAX_ALLOWED_HOSTS)
        .map(|i| format!("\"h{i}.ts.net\""))
        .collect();
    let ok = parse(&format!(
        "allowed_logins = [\"o@x.io\"]\nallowed_hosts = [{}]\n",
        four.join(", ")
    ))
    .unwrap();
    assert_eq!(ok.allowed_hosts.len(), MAX_ALLOWED_HOSTS);
    let max_name = format!("{}.ts.net", "a".repeat(MAX_HOST_LEN - ".ts.net".len()));
    assert_eq!(max_name.len(), MAX_HOST_LEN);
    let ok = parse(&format!(
        "allowed_logins = [\"o@x.io\"]\nallowed_hosts = [\"{max_name}\"]\n"
    ))
    .unwrap();
    assert_eq!(ok.allowed_hosts, vec![max_name.clone()]);
    for (index, bad) in [
        (0usize, "bad_host.ts.net"),
        (0, "-pc.ts.net"),
        (0, "pc-.ts.net"),
        (0, "pc.ts.net:443"),
        (0, "a..ts.net"),
        (0, ".ts.net"),
        (0, "pc.ts.net."),
        (0, ""),
        (0, "pc ts.net"),
        (0, "https://pc.ts.net"),
        (0, &format!("x{max_name}")),
    ] {
        assert_eq!(
            parse(&format!(
                "allowed_logins = [\"o@x.io\"]\nallowed_hosts = [\"{bad}\"]\n"
            )),
            Err(ConfigError::InvalidHost { index }),
            "{bad:?}"
        );
    }
    assert_eq!(
        parse("allowed_logins = [\"o@x.io\"]\nallowed_hosts = [\"ok.ts.net\", \"nope!\"]\n"),
        Err(ConfigError::InvalidHost { index: 1 })
    );
    assert_eq!(
        parse("allowed_logins = [\"o@x.io\"]\nallowed_hosts = \"pc.ts.net\"\n"),
        Err(ConfigError::Syntax)
    );
}

// ---------------------------------------------------------------------------------------
// check_not_root
// ---------------------------------------------------------------------------------------

/// D1 / RC-1 / RMC-S10: root (real or effective) is refused.
#[test]
fn test_rmc_check_not_root_table() {
    assert_eq!(check_not_root(1000, 1000), Ok(()));
    assert_eq!(check_not_root(65534, 1), Ok(()));
    assert_eq!(check_not_root(0, 0), Err(ConfigError::RunningAsRoot));
    assert_eq!(check_not_root(0, 1000), Err(ConfigError::RunningAsRoot));
    assert_eq!(check_not_root(1000, 0), Err(ConfigError::RunningAsRoot));
    assert_eq!(
        ConfigError::RunningAsRoot.to_string(),
        "soos-remote must not run as root"
    );
}

// ---------------------------------------------------------------------------------------
// default_config_path
// ---------------------------------------------------------------------------------------

/// §2.3: `$XDG_CONFIG_HOME/soos/remote.toml`, else `$HOME/.config/soos/remote.toml`; only
/// absolute values count.
#[test]
fn test_rmc_default_config_path_prefers_xdg_config_home() {
    let xdg = OsStr::new("/home/u/.cfg");
    let home = OsStr::new("/home/u");
    assert_eq!(
        default_config_path(Some(xdg), Some(home)),
        Ok(PathBuf::from("/home/u/.cfg/soos/remote.toml"))
    );
    assert_eq!(
        default_config_path(None, Some(home)),
        Ok(PathBuf::from("/home/u/.config/soos/remote.toml"))
    );
    assert_eq!(
        default_config_path(Some(OsStr::new("")), Some(home)),
        Ok(PathBuf::from("/home/u/.config/soos/remote.toml")),
        "an empty XDG_CONFIG_HOME is unset"
    );
    assert_eq!(
        default_config_path(Some(OsStr::new("rel/cfg")), Some(home)),
        Ok(PathBuf::from("/home/u/.config/soos/remote.toml")),
        "a relative XDG_CONFIG_HOME is ignored"
    );
    assert_eq!(
        default_config_path(None, None),
        Err(ConfigError::NoConfigPath)
    );
    assert_eq!(
        default_config_path(None, Some(OsStr::new("home/u"))),
        Err(ConfigError::NoConfigPath)
    );
    assert_eq!(
        default_config_path(None, Some(OsStr::new(""))),
        Err(ConfigError::NoConfigPath)
    );
    assert_eq!(
        default_config_path(Some(OsStr::new("rel")), None),
        Err(ConfigError::NoConfigPath)
    );
}

// ---------------------------------------------------------------------------------------
// load_config
// ---------------------------------------------------------------------------------------

/// §2.3: bounded file read; missing → `NotFound`, directory → `Unreadable`, oversize →
/// `TooLarge`, exactly `MAX_CONFIG_BYTES` → parsed.
#[test]
fn test_rmc_load_config_reads_a_bounded_file() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Some(Path::new(RUNTIME_DIR));
    let path = dir.path().join("remote.toml");
    fs::write(&path, "allowed_logins = [\"Owner@Example.com\"]\n").unwrap();
    let config = load_config(&path, runtime).unwrap();
    assert_eq!(config.allowed_logins, vec![login("owner@example.com")]);

    assert_eq!(
        load_config(&dir.path().join("missing.toml"), runtime),
        Err(ConfigError::NotFound)
    );
    assert_eq!(
        load_config(dir.path(), runtime),
        Err(ConfigError::Unreadable)
    );

    let base = "allowed_logins = [\"o@x.io\"]\n";
    let mut exact = String::from(base);
    exact.push('#');
    exact.push_str(&"c".repeat(MAX_CONFIG_BYTES - base.len() - 2));
    exact.push('\n');
    assert_eq!(exact.len(), MAX_CONFIG_BYTES);
    let exact_path = dir.path().join("exact.toml");
    fs::write(&exact_path, &exact).unwrap();
    assert!(load_config(&exact_path, runtime).is_ok());

    let over_path = dir.path().join("over.toml");
    fs::write(&over_path, format!("{exact}#")).unwrap();
    assert_eq!(
        load_config(&over_path, runtime),
        Err(ConfigError::TooLarge {
            max: MAX_CONFIG_BYTES
        })
    );
    // Far larger than the bound: still `TooLarge`, never read in full.
    let huge_path = dir.path().join("huge.toml");
    fs::write(&huge_path, vec![b'#'; MAX_CONFIG_BYTES * 8]).unwrap();
    assert_eq!(
        load_config(&huge_path, runtime),
        Err(ConfigError::TooLarge {
            max: MAX_CONFIG_BYTES
        })
    );
    // Parse errors propagate unchanged.
    let empty_path = dir.path().join("empty.toml");
    fs::write(&empty_path, "allowed_logins = []\n").unwrap();
    assert_eq!(
        load_config(&empty_path, runtime),
        Err(ConfigError::NoAllowedLogins)
    );
}

/// §4: configuration errors carry no secrets and have fixed English messages.
#[test]
fn test_rmc_config_error_messages_are_fixed_english_text() {
    assert_eq!(
        ConfigError::TooLarge { max: 16_384 }.to_string(),
        "configuration file larger than 16384 bytes"
    );
    assert_eq!(
        ConfigError::InvalidLogin { index: 3 }.to_string(),
        "invalid login at index 3"
    );
    assert_eq!(
        ConfigError::NoRuntimeDir.to_string(),
        "XDG_RUNTIME_DIR is not set or not absolute"
    );
    assert_eq!(
        ConfigError::InvalidSocketPath { max: 107 }.to_string(),
        "socket_path must be absolute and at most 107 bytes"
    );
}

// ---------------------------------------------------------------------------------------
// Remote unlock (ADR 2026-10-06 "Remote Unlock in soos-remote", matrix RMC22)
// ---------------------------------------------------------------------------------------

/// RMC22: the unlock constants have the specified values.
#[test]
fn test_rmc_unlock_constants_match_the_adr() {
    assert_eq!(soos_remote::ACTION_UNLOCK, "unlock");
    assert_eq!(soos_remote::MIN_UNLOCK_INTERVAL_MS, 2000);
    assert_eq!(soos_remote::UNLOCK_FLOW_DEADLINE_MS, 2000);
    let (snapshot, unlock_flow) = (SNAPSHOT_DEADLINE_MS, soos_remote::UNLOCK_FLOW_DEADLINE_MS);
    assert!(
        unlock_flow > snapshot,
        "the unlock flow covers one snapshot plus the call"
    );
}

/// RMC22: `allow_unlock` is opt-in: absent means `false`; only a TOML boolean is accepted.
#[test]
fn test_rmc_parse_config_allow_unlock_is_opt_in() {
    let config = parse("allowed_logins = [\"owner@example.com\"]\n").unwrap();
    assert!(
        !config.allow_unlock,
        "unlock is disabled unless explicitly enabled"
    );
    let config = parse("allowed_logins = [\"o@x.io\"]\nallow_unlock = true\n").unwrap();
    assert!(config.allow_unlock);
    let config = parse("allowed_logins = [\"o@x.io\"]\nallow_unlock = false\n").unwrap();
    assert!(!config.allow_unlock);
    for bad in ["\"true\"", "1", "\"yes\"", "[true]"] {
        assert_eq!(
            parse(&format!(
                "allowed_logins = [\"o@x.io\"]\nallow_unlock = {bad}\n"
            )),
            Err(ConfigError::Syntax),
            "{bad}"
        );
    }
    assert_eq!(
        parse("allow_unlock = true\n"),
        Err(ConfigError::NoAllowedLogins),
        "enabling unlock never relaxes the allowlist"
    );
}

// ---------------------------------------------------------------------------------------
// Funnel and passkeys (ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey
// Authentication for `soos-remote`", spec AI/architect_spec_remote_passkey_funnel.md §3,
// tests 1–7, matrix RMC28)
// ---------------------------------------------------------------------------------------

/// Test 1 (RMC28): every §3.1 constant has the specified value and the §3.1 relations hold.
#[test]
#[allow(
    clippy::assertions_on_constants,
    reason = "The §3.1 relations between constants are themselves the contract"
)]
fn test_rmc_passkey_constants_match_the_adr() {
    use soos_remote::*;
    assert_eq!(FUNNEL_HEADER, "tailscale-funnel-request");
    assert_eq!(FUNNEL_HEADER_VALUE, "?1");
    assert_eq!(FORWARDED_FOR_HEADER, "x-forwarded-for");
    assert_eq!(MAX_AUTH_BODY_BYTES, 8192);
    assert_eq!(BODY_READ_TIMEOUT_MS, 5000);
    assert_eq!(MAX_BODY_CHUNKS, 64);
    assert_eq!(MAX_CHUNK_SIZE_DIGITS, 8);
    assert_eq!(MAX_FUNNEL_CONNECTIONS, 8);
    assert_eq!(MAX_ANONYMOUS_FUNNEL_CONNECTIONS, 4);
    assert_eq!(MAX_ANONYMOUS_BODY_READS, 2);
    assert_eq!(MAX_ANONYMOUS_BODY_READS_PER_HINT, 1);
    assert_eq!(MAX_FUNNEL_SSE_STREAMS, 2);
    assert_eq!(MAX_CLIENT_HINTS, 64);
    assert_eq!(MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES, 16);
    assert_eq!(MAX_LOGIN_CHALLENGES_PER_HINT, 2);
    assert_eq!(MAX_CLIENT_DATA_JSON_BYTES, 1024);
    assert_eq!(MAX_ATTESTATION_OBJECT_BYTES, 2048);
    assert_eq!(ASSERTION_AUTH_DATA_LEN, 37);
    assert_eq!(MAX_SIGNATURE_BYTES, 72);
    assert_eq!(MAX_CREDENTIAL_ID_BYTES, 1023);
    assert_eq!(USER_HANDLE_BYTES, 16);
    assert_eq!(CHALLENGE_BYTES, 32);
    assert_eq!(CHALLENGE_TTL_MS, 120_000);
    assert_eq!(WEBAUTHN_TIMEOUT_MS, 120_000);
    assert_eq!(WEBAUTHN_TIMEOUT_MS, CHALLENGE_TTL_MS);
    assert_eq!(MAX_PENDING_CHALLENGES, 4);
    assert_eq!(MAX_PASSKEYS, 4);
    assert_eq!(MAX_CREDENTIAL_STORE_BYTES, 16_384);
    assert_eq!(CREDENTIALS_FILE_NAME, "remote-passkeys.json");
    assert_eq!(MAX_CREDENTIALS_PATH_LEN, 4096);
    assert_eq!(MAX_WEB_SESSIONS, 4);
    assert_eq!(WEB_SESSION_IDLE_MS, 900_000);
    assert_eq!(WEB_SESSION_ABSOLUTE_MS, 28_800_000);
    assert_eq!(SESSION_TOKEN_BYTES, 32);
    assert_eq!(SESSION_COOKIE_NAME, "__Host-soos_session");
    assert_eq!(
        SESSION_COOKIE_ATTRIBUTES,
        "Path=/; Secure; HttpOnly; SameSite=Strict"
    );
    for needle in ["Secure", "HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(SESSION_COOKIE_ATTRIBUTES.contains(needle), "{needle}");
    }
    assert!(!SESSION_COOKIE_ATTRIBUTES
        .to_ascii_lowercase()
        .contains("domain"));
    assert!(SESSION_COOKIE_NAME.starts_with("__Host-"));
    assert_eq!(MAX_AUTH_FAILURES, 5);
    assert_eq!(AUTH_FAILURE_WINDOW_MS, 300_000);
    assert_eq!(MAX_OPTIONS_PER_WINDOW, 10);
    assert_eq!(OPTIONS_WINDOW_MS, 60_000);
    assert_eq!(ENROLL_CODE_LEN, 10);
    assert_eq!(ENROLL_CODE_ALPHABET, b"0123456789ABCDEFGHJKMNPQRSTVWXYZ");
    for forbidden in *b"ILOU" {
        assert!(!ENROLL_CODE_ALPHABET.contains(&forbidden));
    }
    assert_eq!(ENROLL_CODE_TTL_S, 300);
    assert_eq!(MAX_ENROLL_CODE_ATTEMPTS, 3);
    assert_eq!(ENROLL_CODE_FILE_NAME, "enroll-code");
    assert_eq!(MAX_ENROLL_CODE_FILE_BYTES, 256);
    assert_eq!(STORE_LOCK_TIMEOUT_MS, 500);
    assert_eq!(COSE_ALG_ES256, -7);
    assert_eq!(RP_NAME, "soos");
    assert_eq!(ACTION_UNLOCK_OPTIONS, "unlock-options");
    assert_eq!(ACTION_LOGIN_OPTIONS, "login-options");
    assert_eq!(ACTION_LOGIN, "login");
    assert_eq!(ACTION_LOGOUT, "logout");
    assert_eq!(ACTION_REGISTER_OPTIONS, "register-options");
    assert_eq!(ACTION_REGISTER, "register");
    // §3.1 relations.
    assert!(
        MAX_AUTH_BODY_BYTES
            >= 4 * MAX_ATTESTATION_OBJECT_BYTES / 3
                + 4 * MAX_CREDENTIAL_ID_BYTES / 3
                + 4 * MAX_CLIENT_DATA_JSON_BYTES / 3
                + 256
    );
    assert!(MAX_ANONYMOUS_FUNNEL_CONNECTIONS < MAX_FUNNEL_CONNECTIONS);
    assert!(MAX_FUNNEL_CONNECTIONS < MAX_CONNECTIONS);
    assert!(MAX_CONNECTIONS - MAX_FUNNEL_CONNECTIONS >= 8);
    assert!(MAX_ANONYMOUS_BODY_READS < MAX_ANONYMOUS_FUNNEL_CONNECTIONS);
    assert!(MAX_ANONYMOUS_BODY_READS_PER_HINT <= MAX_ANONYMOUS_BODY_READS);
    assert!(MAX_FUNNEL_SSE_STREAMS < MAX_SSE_STREAMS);
    assert!(MAX_LOGIN_CHALLENGES_PER_HINT < MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES);
    assert_eq!(WEB_SESSION_ABSOLUTE_MS / 1000, 28_800, "cookie Max-Age");
}

const LOGINS: &str = "allowed_logins = [\"owner@example.com\"]\n";

fn parse_auth(extra: &str) -> Result<RemoteConfig, ConfigError> {
    parse(&format!("{LOGINS}{extra}"))
}

/// Test 2 (RMC28): absent keys keep every passkey and Funnel feature off.
#[test]
fn test_rmc_parse_config_auth_defaults_are_off() {
    use soos_remote::config::AuthConfig;
    let config = parse(LOGINS).unwrap();
    assert_eq!(config.auth, AuthConfig::default());
    assert_eq!(config.auth.rp_id, None);
    assert!(!config.auth.allow_funnel);
    assert_eq!(config.auth.credentials_path, None);
    let explicit = parse_auth("allow_funnel = false\n").unwrap();
    assert_eq!(explicit.auth, AuthConfig::default());
}

/// Test 3 (RMC28, plan-evaluator G-6): `rp_id` is the full node host: lowercased, a valid
/// DNS name under `.ts.net` with at least two labels before it, no port, no scheme, never
/// empty; with `allowed_hosts` it must also be a member, and the `.ts.net` rule applies in
/// both cases.
#[test]
fn test_rmc_parse_config_rp_id_rules() {
    let config = parse_auth("rp_id = \"PC.Tail1234.TS.NET\"\n").unwrap();
    assert_eq!(config.auth.rp_id.as_deref(), Some("pc.tail1234.ts.net"));
    let config = parse_auth("rp_id = \"my-pc.tail-1234.ts.net\"\n").unwrap();
    assert_eq!(config.auth.rp_id.as_deref(), Some("my-pc.tail-1234.ts.net"));
    for bad in [
        "tail1234.ts.net",
        "ts.net",
        ".ts.net",
        "pc.example.com",
        "pc.tail1234.ts.net:443",
        "pc.tail1234.ts.net:8443",
        "https://pc.tail1234.ts.net",
        "pc.tail1234.ts.net/",
        "pc.tail1234.ts.net.",
        "pc..tail1234.ts.net",
        "-pc.tail1234.ts.net",
        "pc tail.ts.net",
        "",
        " ",
        "100.64.0.1",
        "[fd7a:115c:a1e0::1]",
        "*.tail1234.ts.net",
    ] {
        assert_eq!(
            parse_auth(&format!("rp_id = \"{bad}\"\n")).map(|c| c.auth),
            Err(ConfigError::InvalidRpId),
            "{bad:?}"
        );
    }
    let long_label = format!("{}.tail1234.ts.net", "a".repeat(250));
    assert_eq!(
        parse_auth(&format!("rp_id = \"{long_label}\"\n")).map(|c| c.auth),
        Err(ConfigError::InvalidRpId)
    );
    for bad in ["1", "true", "[\"pc.tail1234.ts.net\"]"] {
        assert_eq!(
            parse_auth(&format!("rp_id = {bad}\n")).map(|c| c.auth),
            Err(ConfigError::Syntax),
            "{bad}"
        );
    }
    // With an allowlist: membership required, and still a full *.ts.net node host (G-6).
    let hosts = "allowed_hosts = [\"pc.tail1234.ts.net\", \"lan-box.example\"]\n";
    let config = parse_auth(&format!("{hosts}rp_id = \"pc.tail1234.ts.net\"\n")).unwrap();
    assert_eq!(config.auth.rp_id.as_deref(), Some("pc.tail1234.ts.net"));
    for bad in ["other.tail1234.ts.net", "lan-box.example"] {
        assert_eq!(
            parse_auth(&format!("{hosts}rp_id = \"{bad}\"\n")).map(|c| c.auth),
            Err(ConfigError::InvalidRpId),
            "{bad}"
        );
    }
}

/// Test 4 (RMC28): `allow_funnel` is a TOML boolean, `false` by default, and requires
/// `rp_id`; `allow_unlock` without `rp_id` stays a valid configuration.
#[test]
fn test_rmc_parse_config_allow_funnel_requires_rp_id() {
    assert_eq!(
        parse_auth("allow_funnel = true\n").map(|c| c.auth),
        Err(ConfigError::FunnelNeedsRpId)
    );
    let config = parse_auth("allow_funnel = true\nrp_id = \"pc.tail1234.ts.net\"\n").unwrap();
    assert!(config.auth.allow_funnel);
    assert_eq!(config.auth.rp_id.as_deref(), Some("pc.tail1234.ts.net"));
    let config = parse_auth("rp_id = \"pc.tail1234.ts.net\"\n").unwrap();
    assert!(!config.auth.allow_funnel, "Funnel is opt-in");
    for bad in ["\"true\"", "1", "\"yes\"", "[true]"] {
        assert_eq!(
            parse_auth(&format!(
                "rp_id = \"pc.tail1234.ts.net\"\nallow_funnel = {bad}\n"
            ))
            .map(|c| c.auth),
            Err(ConfigError::Syntax),
            "{bad}"
        );
    }
    let config = parse_auth("allow_unlock = true\n").unwrap();
    assert!(config.allow_unlock);
    assert_eq!(config.auth.rp_id, None, "allow_unlock never implies rp_id");
    assert_eq!(
        parse("allow_funnel = true\nrp_id = \"pc.tail1234.ts.net\"\n").map(|c| c.auth),
        Err(ConfigError::NoAllowedLogins),
        "enabling Funnel never relaxes the allowlist"
    );
}

/// Test 5 (RMC28): `credentials_path` is absolute, has a parent and a file name, no
/// trailing `/`, at most `MAX_CREDENTIALS_PATH_LEN` bytes.
#[test]
fn test_rmc_parse_config_credentials_path_rules() {
    use soos_remote::MAX_CREDENTIALS_PATH_LEN;
    let config =
        parse_auth("credentials_path = \"/home/me/.config/soos/passkeys.json\"\n").unwrap();
    assert_eq!(
        config.auth.credentials_path,
        Some(PathBuf::from("/home/me/.config/soos/passkeys.json"))
    );
    let at_bound = format!("/{}", "p".repeat(MAX_CREDENTIALS_PATH_LEN - 1));
    assert_eq!(at_bound.len(), MAX_CREDENTIALS_PATH_LEN);
    let config = parse_auth(&format!("credentials_path = \"{at_bound}\"\n")).unwrap();
    assert_eq!(config.auth.credentials_path, Some(PathBuf::from(&at_bound)));
    let over = format!("/{}", "p".repeat(MAX_CREDENTIALS_PATH_LEN));
    for bad in [
        "relative/passkeys.json",
        "passkeys.json",
        "/home/me/.config/soos/",
        "/",
        "",
        over.as_str(),
    ] {
        assert_eq!(
            parse_auth(&format!("credentials_path = \"{bad}\"\n")).map(|c| c.auth),
            Err(ConfigError::InvalidCredentialsPath {
                max: MAX_CREDENTIALS_PATH_LEN
            }),
            "{bad:?}"
        );
    }
    assert_eq!(
        parse_auth("credentials_path = 7\n").map(|c| c.auth),
        Err(ConfigError::Syntax)
    );
}

/// Test 6 (RMC28, S-5): the explicit path wins; otherwise the store is the sibling
/// `remote-passkeys.json` of the configuration file.
#[test]
fn test_rmc_resolve_credentials_path() {
    use soos_remote::config::{resolve_credentials_path, AuthConfig};
    use soos_remote::MAX_CREDENTIALS_PATH_LEN;
    let config_path = Path::new("/home/me/.config/soos/remote.toml");
    assert_eq!(
        resolve_credentials_path(&AuthConfig::default(), config_path),
        Ok(PathBuf::from("/home/me/.config/soos/remote-passkeys.json"))
    );
    let explicit = AuthConfig {
        credentials_path: Some(PathBuf::from("/srv/keys/p.json")),
        ..AuthConfig::default()
    };
    assert_eq!(
        resolve_credentials_path(&explicit, config_path),
        Ok(PathBuf::from("/srv/keys/p.json"))
    );
    assert_eq!(
        resolve_credentials_path(&explicit, Path::new("/")),
        Ok(PathBuf::from("/srv/keys/p.json")),
        "an explicit path never looks at the configuration path"
    );
    let invalid = Err(ConfigError::InvalidCredentialsPath {
        max: MAX_CREDENTIALS_PATH_LEN,
    });
    assert_eq!(
        resolve_credentials_path(&AuthConfig::default(), Path::new("/")),
        invalid,
        "no parent"
    );
    let deep = format!("/{}/remote.toml", "d".repeat(MAX_CREDENTIALS_PATH_LEN - 20));
    assert_eq!(
        resolve_credentials_path(&AuthConfig::default(), Path::new(&deep)),
        invalid,
        "the resolved path is bounded"
    );
}

/// Test 7 (RMC28): the new configuration errors are fixed English texts.
#[test]
fn test_rmc_auth_config_error_messages_are_fixed_english_text() {
    assert_eq!(
        ConfigError::InvalidRpId.to_string(),
        "rp_id must be the full node host name"
    );
    assert_eq!(
        ConfigError::FunnelNeedsRpId.to_string(),
        "allow_funnel requires rp_id"
    );
    assert_eq!(
        ConfigError::InvalidCredentialsPath { max: 4096 }.to_string(),
        "credentials_path must be absolute and at most 4096 bytes"
    );
    // A refused value is never echoed.
    let err = parse_auth("rp_id = \"secret-node.example.com\"\n").unwrap_err();
    assert!(!err.to_string().contains("secret-node"));
}

// ---------------------------------------------------------------------------------------
// Failed-password alerts (ADR 2026-10-06 "Failed-Password Alerts in `soos-remote` From the
// System Journal", architect spec `AI/architect_spec_remote_auth_alerts.md` §3.2, tests 37,
// 38 and 57; matrix RMC45, RMC54). New tests only; nothing above is changed.
// ---------------------------------------------------------------------------------------

mod alerts_contract {
    use std::path::{Path, PathBuf};

    use soos_remote::config::{
        parse_config, resolve_alerts_ack_path, AlertsConfig, ConfigError, RemoteConfig,
        DEFAULT_LOCK_SCREEN_PROGRAMS,
    };
    use soos_remote::{MAX_CREDENTIALS_PATH_LEN, MAX_LOCK_SCREEN_PROGRAMS};

    const BASE: &str = "allowed_logins = [\"owner@example.com\"]\n";

    fn parse(extra: &str) -> Result<RemoteConfig, ConfigError> {
        parse_config(&format!("{BASE}{extra}"), Some(Path::new("/run/user/1000")))
    }

    /// A TOML basic string (control characters as `\\uXXXX`).
    fn toml_string(raw: &str) -> String {
        let mut out = String::from("\"");
        for c in raw.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                    out.push_str(&format!("\\u{:04X}", c as u32));
                }
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    fn programs(list: &[&str]) -> String {
        let items: Vec<String> = list.iter().map(|p| toml_string(p)).collect();
        format!("lock_screen_programs = [{}]\n", items.join(", "))
    }

    /// Test 37 (RMC45, A-2, §3.2): absent keys give `AlertsConfig::default()` (off, the four
    /// built-in lockers); `password_alerts = true`; `lock_screen_programs` holds at most 4
    /// absolute paths, deduplicated in order; `[]` stays empty; bad entries name their index.
    #[test]
    fn test_rmc_alerts_config_keys() {
        assert_eq!(MAX_LOCK_SCREEN_PROGRAMS, 4);
        assert_eq!(
            DEFAULT_LOCK_SCREEN_PROGRAMS,
            [
                "/usr/bin/swaylock",
                "/usr/bin/hyprlock",
                "/usr/bin/gtklock",
                "/usr/bin/waylock"
            ]
        );
        let default = AlertsConfig::default();
        assert!(!default.enabled);
        assert_eq!(
            default.lock_screen_programs,
            DEFAULT_LOCK_SCREEN_PROGRAMS
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(parse("").unwrap().alerts, AlertsConfig::default());
        let config = parse("password_alerts = true\n").unwrap();
        assert!(config.alerts.enabled);
        assert_eq!(
            config.alerts.lock_screen_programs,
            AlertsConfig::default().lock_screen_programs
        );
        assert!(!parse("password_alerts = false\n").unwrap().alerts.enabled);
        assert_eq!(
            parse("password_alerts = \"yes\"\n").map(|c| c.alerts),
            Err(ConfigError::Syntax)
        );
        assert_eq!(
            parse("lock_screen_programs = \"/usr/bin/swaylock\"\n").map(|c| c.alerts),
            Err(ConfigError::Syntax)
        );

        let four = [
            "/home/me/.local/bin/swaylock-plugin",
            "/usr/bin/swaylock",
            "/opt/locker/bin/lock",
            "/usr/local/bin/hyprlock",
        ];
        let config = parse(&programs(&four)).unwrap();
        assert_eq!(config.alerts.lock_screen_programs, four.to_vec());
        assert!(!config.alerts.enabled, "independent of password_alerts");
        let five = ["/a/1", "/a/2", "/a/3", "/a/4", "/a/5"];
        assert_eq!(
            parse(&programs(&five)).map(|c| c.alerts),
            Err(ConfigError::TooManyLockScreenPrograms {
                max: MAX_LOCK_SCREEN_PROGRAMS
            })
        );
        // Duplicates removed, order kept (a list of 5 with one duplicate is 4 entries).
        let config = parse(&programs(&[
            "/b/locker",
            "/a/locker",
            "/b/locker",
            "/c/locker",
        ]))
        .unwrap();
        assert_eq!(
            config.alerts.lock_screen_programs,
            vec!["/b/locker", "/a/locker", "/c/locker"]
        );
        // An empty list stays empty (no owner-UID lock-screen line is ever trusted).
        assert!(parse("lock_screen_programs = []\n")
            .unwrap()
            .alerts
            .lock_screen_programs
            .is_empty());
        // Exactly 4096 bytes is accepted; every invalid entry names its index.
        let at_bound = format!("/{}", "p".repeat(4095));
        assert_eq!(
            parse(&programs(&[&at_bound]))
                .unwrap()
                .alerts
                .lock_screen_programs,
            vec![at_bound.clone()]
        );
        let over = format!("/{}", "p".repeat(4096));
        for bad in [
            "bin/swaylock",
            "swaylock",
            "/usr/bin/",
            "/usr/../bin/swaylock",
            "/usr/bin/..",
            "",
            over.as_str(),
        ] {
            assert_eq!(
                parse(&programs(&["/usr/bin/ok", "/usr/bin/fine", bad])).map(|c| c.alerts),
                Err(ConfigError::InvalidLockScreenProgram { index: 2 }),
                "{bad:?}"
            );
        }
    }

    /// Test 38 (RMC54, §3.2): the ack file is the sibling `remote-alerts.json` of the
    /// credential store; no parent or an over-long result is an error.
    #[test]
    fn test_rmc_alerts_ack_path_resolution() {
        assert_eq!(
            resolve_alerts_ack_path(Path::new("/home/me/.config/soos/remote-passkeys.json")),
            Ok(PathBuf::from("/home/me/.config/soos/remote-alerts.json"))
        );
        assert_eq!(
            resolve_alerts_ack_path(Path::new("/srv/keys/p.json")),
            Ok(PathBuf::from("/srv/keys/remote-alerts.json"))
        );
        let invalid = Err(ConfigError::InvalidCredentialsPath {
            max: MAX_CREDENTIALS_PATH_LEN,
        });
        assert_eq!(
            resolve_alerts_ack_path(Path::new("/")),
            invalid,
            "no parent"
        );
        assert_eq!(
            resolve_alerts_ack_path(Path::new("remote-passkeys.json")),
            invalid,
            "empty parent"
        );
        let deep = format!("/{}/p.json", "d".repeat(MAX_CREDENTIALS_PATH_LEN - 10));
        assert!(deep.len() <= MAX_CREDENTIALS_PATH_LEN);
        assert_eq!(
            resolve_alerts_ack_path(Path::new(&deep)),
            invalid,
            "the resolved path is bounded"
        );
    }

    /// Test 57 (RMC45, F-2 b): a configured path may not end in ` (deleted)` nor contain a
    /// control byte; the error never echoes the path.
    #[test]
    fn test_rmc_alerts_lock_screen_program_validation() {
        for (index, bad) in [
            (0, "/usr/bin/swaylock (deleted)"),
            (1, "/usr/bin/sway\u{1b}lock"),
            (1, "/usr/bin/sway\nlock"),
            (1, "/usr/bin/sway\u{7f}lock"),
            (1, "/usr/bin/sway\tlock"),
        ] {
            let list = if index == 0 {
                vec![bad]
            } else {
                vec!["/usr/bin/ok", bad]
            };
            let err = parse(&programs(&list)).map(|c| c.alerts).unwrap_err();
            assert_eq!(
                err,
                ConfigError::InvalidLockScreenProgram { index },
                "{bad:?}"
            );
            let text = err.to_string();
            assert!(!text.contains("sway"), "the path is never echoed: {text}");
            assert!(!text.contains("deleted"), "{text}");
        }
        // A path merely containing the words is fine.
        assert!(parse(&programs(&["/opt/(deleted) dir/lock"])).is_ok());
        let err = ConfigError::TooManyLockScreenPrograms { max: 4 };
        assert!(err.to_string().contains('4'), "{err}");
    }
}

// ---------------------------------------------------------------------------------------
// Web Push notifications (ADR 2026-10-06 "Web Push Notifications for Failed-Password Alerts
// Through a Separate Sender Unit", architect spec AI/architect_spec_remote_web_push.md §3.2,
// §3.3, tests 37 and 38; matrix RMC60, RMC64)
// ---------------------------------------------------------------------------------------

mod push_contract {
    use std::path::{Path, PathBuf};

    use soos_remote::config::{
        parse_config, resolve_push_store_path, ConfigError, PushConfig, PushPreviews, RemoteConfig,
    };
    use soos_remote::{MAX_CREDENTIALS_PATH_LEN, MAX_SOCKET_PATH_LEN};

    const BASE: &str = "allowed_logins = [\"owner@example.com\"]\n";
    const ENABLED: &str =
        "password_alerts = true\nrp_id = \"pc.tail1234.ts.net\"\npush_notifications = true\n";

    fn parse(extra: &str) -> Result<RemoteConfig, ConfigError> {
        parse_config(&format!("{BASE}{extra}"), Some(Path::new("/run/user/1000")))
    }

    /// A TOML basic string (control characters as `\\uXXXX`).
    fn toml_string(raw: &str) -> String {
        let mut out = String::from("\"");
        for c in raw.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                    out.push_str(&format!("\\u{:04X}", c as u32));
                }
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    fn subject(raw: &str) -> Result<PushConfig, ConfigError> {
        parse(&format!("{ENABLED}vapid_subject = {}\n", toml_string(raw))).map(|c| c.push)
    }

    /// Test 37 (RMC60, RMC64, W-2, W-12, §3.2, §3.3): absent keys give
    /// `PushConfig::default()`; push needs alerts and `rp_id`; `vapid_subject` accepts only
    /// a deliverable `mailto:` or `https:` subject and never echoes a refused value;
    /// `push_socket_path` follows the `socket_path` rules; `push_previews` is `detailed` or
    /// `generic`.
    #[test]
    fn test_rwp_push_config_keys() {
        let default = PushConfig::default();
        assert!(!default.enabled);
        assert_eq!(default.vapid_subject, None);
        assert_eq!(default.socket_path, None);
        assert_eq!(default.previews, PushPreviews::Detailed);
        assert_eq!(parse("").unwrap().push, PushConfig::default());
        assert!(!parse("push_notifications = false\n").unwrap().push.enabled);
        assert_eq!(
            parse("push_notifications = \"yes\"\n").map(|c| c.push),
            Err(ConfigError::Syntax)
        );

        // Requirements.
        assert_eq!(
            parse("rp_id = \"pc.tail1234.ts.net\"\npush_notifications = true\n").map(|c| c.push),
            Err(ConfigError::PushRequiresAlerts)
        );
        assert_eq!(
            parse("password_alerts = false\nrp_id = \"pc.tail1234.ts.net\"\npush_notifications = true\n")
                .map(|c| c.push),
            Err(ConfigError::PushRequiresAlerts)
        );
        assert_eq!(
            parse("password_alerts = true\npush_notifications = true\n").map(|c| c.push),
            Err(ConfigError::PushRequiresRpId)
        );
        let config = parse(ENABLED).unwrap();
        assert!(config.push.enabled);
        assert_eq!(config.push.vapid_subject, None, "default resolved later");
        assert_eq!(config.push.previews, PushPreviews::Detailed);
        let default_socket = PathBuf::from("/run/user/1000/soos-push/push.sock");
        assert!(
            config.push.socket_path.is_none()
                || config.push.socket_path == Some(default_socket.clone()),
            "{:?}",
            config.push.socket_path
        );

        // vapid_subject accepted.
        for good in [
            "mailto:owner@proton.me",
            "https://pc.tail1234.ts.net",
            "https://github.com/Mysticaly622/soos",
        ] {
            assert_eq!(
                subject(good).unwrap().vapid_subject,
                Some(good.to_string()),
                "{good}"
            );
        }
        // vapid_subject refused, never echoed.
        let long = format!("mailto:{}@proton.me", "a".repeat(250));
        assert!(long.len() > 256);
        for bad in [
            "",
            "mailto:",
            "mailto:a@localhost",
            "mailto:a@x.invalid",
            "mailto:a@x.test",
            "mailto:a@box.local",
            "mailto:a@b.example",
            "mailto:a@b.internal",
            "mailto:a@b.home.arpa",
            "mailto:a@x.localhost",
            "mailto:mailto:a@b.co",
            "mailto:a@nodot",
            "mailto:a@@b.co",
            "mailto:a<b@b.co",
            "mailto:@b.co",
            "https://x.test",
            "https://h.local",
            "https://localhost",
            "https://nodot",
            "http://pc.tail1234.ts.net",
            "https://pc.tail1234.ts.net:8443",
            "https://pc.tail1234.ts.net/a?b",
            "https://pc.tail1234.ts.net/a#b",
            "https://user@pc.tail1234.ts.net",
            "pc.tail1234.ts.net",
            "mailto:a @b.co",
            "mailto:a\u{1}@b.co",
            "https://pc.tail1234.ts.net/\u{e9}",
            long.as_str(),
        ] {
            let err = subject(bad).unwrap_err();
            assert_eq!(err, ConfigError::InvalidVapidSubject, "{bad:?}");
            let text = err.to_string();
            for needle in [
                "mailto",
                "proton",
                "localhost",
                "invalid",
                "tail1234",
                "8443",
            ] {
                assert!(!text.contains(needle), "{text} echoes {bad:?}");
            }
        }

        // push_socket_path.
        let invalid = || {
            Err(ConfigError::InvalidPushSocketPath {
                max: MAX_SOCKET_PATH_LEN,
            })
        };
        let custom = parse(&format!(
            "{ENABLED}push_socket_path = \"/run/user/1000/sp/s.sock\"\n"
        ))
        .unwrap();
        assert_eq!(
            custom.push.socket_path,
            Some(PathBuf::from("/run/user/1000/sp/s.sock"))
        );
        // "/" + n + "/s" is n + 3 bytes (contract migration: the fixture used n - 4).
        let at_bound = format!("/{}/s", "d".repeat(MAX_SOCKET_PATH_LEN - 3));
        assert_eq!(at_bound.len(), MAX_SOCKET_PATH_LEN);
        assert_eq!(
            parse(&format!("{ENABLED}push_socket_path = {at_bound:?}\n"))
                .unwrap()
                .push
                .socket_path,
            Some(PathBuf::from(&at_bound))
        );
        let over = format!("/{}/s", "d".repeat(MAX_SOCKET_PATH_LEN - 2));
        for bad in [
            "relative/push.sock",
            "/run/user/1000/soos-push/",
            "/",
            "",
            over.as_str(),
        ] {
            assert_eq!(
                parse(&format!("{ENABLED}push_socket_path = {bad:?}\n")).map(|c| c.push),
                invalid(),
                "{bad:?}"
            );
        }
        // No runtime directory: an error only when push is enabled and no explicit path.
        let explicit_socket = "socket_path = \"/srv/remote/remote.sock\"\n";
        assert_eq!(
            parse_config(&format!("{BASE}{explicit_socket}{ENABLED}"), None).map(|c| c.push),
            Err(ConfigError::NoRuntimeDir)
        );
        assert!(parse_config(
            &format!(
                "{BASE}{explicit_socket}{ENABLED}push_socket_path = \"/srv/push/push.sock\"\n"
            ),
            None
        )
        .is_ok());
        assert!(parse_config(
            &format!("{BASE}{explicit_socket}password_alerts = true\n"),
            None
        )
        .is_ok());

        // push_previews.
        assert_eq!(
            parse(&format!("{ENABLED}push_previews = \"detailed\"\n"))
                .unwrap()
                .push
                .previews,
            PushPreviews::Detailed
        );
        assert_eq!(
            parse(&format!("{ENABLED}push_previews = \"generic\"\n"))
                .unwrap()
                .push
                .previews,
            PushPreviews::Generic
        );
        for bad in ["Generic", "none", "", "detailed "] {
            assert_eq!(
                parse(&format!("{ENABLED}push_previews = {bad:?}\n")).map(|c| c.push),
                Err(ConfigError::InvalidPushPreviews),
                "{bad:?}"
            );
        }
        // Every new error is configuration-class fixed text.
        for err in [
            ConfigError::PushRequiresAlerts,
            ConfigError::PushRequiresRpId,
            ConfigError::InvalidVapidSubject,
            ConfigError::InvalidPushSocketPath {
                max: MAX_SOCKET_PATH_LEN,
            },
            ConfigError::InvalidPushPreviews,
        ] {
            assert!(!err.to_string().is_empty());
        }
    }

    /// Test 38 (RMC65, §3.2): the push store is the sibling `remote-push.json` of the
    /// credential store, with the same errors as the ack file path.
    #[test]
    fn test_rwp_push_store_path_resolution() {
        assert_eq!(
            resolve_push_store_path(Path::new("/home/me/.config/soos/remote-passkeys.json")),
            Ok(PathBuf::from("/home/me/.config/soos/remote-push.json"))
        );
        assert_eq!(
            resolve_push_store_path(Path::new("/srv/keys/p.json")),
            Ok(PathBuf::from("/srv/keys/remote-push.json"))
        );
        let invalid = Err(ConfigError::InvalidCredentialsPath {
            max: MAX_CREDENTIALS_PATH_LEN,
        });
        assert_eq!(resolve_push_store_path(Path::new("/")), invalid);
        assert_eq!(
            resolve_push_store_path(Path::new("remote-passkeys.json")),
            invalid
        );
        let deep = format!("/{}/p.json", "d".repeat(MAX_CREDENTIALS_PATH_LEN - 10));
        assert_eq!(resolve_push_store_path(Path::new(&deep)), invalid);
    }
}
