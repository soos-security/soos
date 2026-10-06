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
