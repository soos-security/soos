//! Contract tests of the opt-in native Android app binding of `soos-remote` (GitHub #382, PR 1
//! of #357, part 2 of #354; ADR "[2026-10-13] Native Android App for `soos-remote` (Kotlin and
//! Jetpack Compose)"; architect spec `AI/architect_spec_android_native_app.md`; matrix
//! ANA1–ANA6).
//!
//! - `[android]` configuration: strict, fail-closed validation, redacted `Debug`;
//! - `GET|HEAD /.well-known/assetlinks.json`: `404` byte-identical to an unknown path without the
//!   table, the exact Digital Asset Links statement with it, on the tailnet and on the Funnel path;
//! - `clientDataJSON.origin = android:apk-key-hash:<digest>` accepted for the **login** ceremony
//!   only, and only for a configured signing-certificate digest.
//!
//! Same deterministic harness as `auth_server_tests.rs` (frozen paused clock, scripted logind,
//! raw HTTP/1.1 over the Unix socket in a `TempDir`). No network, no logind, no `~/.config`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

#[path = "common/passkey.rs"]
mod passkey;

#[path = "common/harness.rs"]
mod harness;

use std::path::Path;
use std::sync::{Arc, Mutex};

use proptest::prelude::*;
use tokio::net::UnixListener;
use tokio::sync::oneshot;

use harness::*;
use passkey::*;
use soos_remote::config::{
    load_config_with_android, parse_config, parse_config_with_android, AndroidAppConfig,
    AuthConfig, ConfigError, RemoteConfig, TailscaleLogin,
};
use soos_remote::http::Method;
use soos_remote::routes::{
    accepts_body, allow_header, is_funnel_public, route, Route, ASSET_LINKS_PATH,
};
use soos_remote::server::{serve, ServerState};
use soos_remote::webauthn::{
    parse_client_data, parse_client_data_with_policy, CeremonyType, ClientOrigin, OriginPolicy,
    RelyingParty, WebAuthnError,
};
use soos_remote::{
    ANDROID_CERT_FINGERPRINT_LEN, ANDROID_ORIGIN_PREFIX, CREDENTIALS_FILE_NAME,
    DEFAULT_POLL_INTERVAL_MS, MAX_ANDROID_CERT_FINGERPRINTS, MAX_ANDROID_PACKAGE_LEN,
    MAX_ASSET_LINKS_BYTES,
};

/// The application id of the soos app (assumption A-1 of the spec).
const PACKAGE: &str = "org.soos.remote";

/// First signing-certificate digest of the fixtures.
fn fp1() -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(7).wrapping_add(0xA3);
    }
    out
}

/// Second digest (rotation).
fn fp2() -> [u8; 32] {
    [0x5C; 32]
}

/// `AA:BB:..` upper-case form of a digest.
fn colon_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// The native app origin of `digest`.
fn android_origin(digest: &[u8; 32]) -> String {
    format!("android:apk-key-hash:{}", b64url(digest))
}

const BASE: &str = "allowed_logins = [\"owner@example.com\"]\n\
socket_path = \"/run/user/1000/soos-remote/remote.sock\"\n\
rp_id = \"pc.tail1234.ts.net\"\n\
allow_funnel = true\n";

fn with_android(table: &str) -> String {
    format!("{BASE}\n[android]\n{table}")
}

fn valid_table() -> String {
    format!(
        "package = \"{PACKAGE}\"\nsha256_cert_fingerprints = [\"{}\"]\n",
        colon_hex(&fp1())
    )
}

fn android_err(table: &str) -> ConfigError {
    parse_config_with_android(&with_android(table), None).unwrap_err()
}

fn app_with(fingerprints: &[[u8; 32]]) -> AndroidAppConfig {
    let list = fingerprints
        .iter()
        .map(|f| format!("\"{}\"", colon_hex(f)))
        .collect::<Vec<_>>()
        .join(", ");
    let text = with_android(&format!(
        "package = \"{PACKAGE}\"\nsha256_cert_fingerprints = [{list}]\n"
    ));
    parse_config_with_android(&text, None)
        .unwrap()
        .android
        .expect("android table")
}

// ---------------------------------------------------------------------------------------
// ANA1: configuration
// ---------------------------------------------------------------------------------------

#[test]
fn test_ana_config_constants() {
    assert_eq!(MAX_ANDROID_PACKAGE_LEN, 128);
    assert_eq!(MAX_ANDROID_CERT_FINGERPRINTS, 2);
    assert_eq!(ANDROID_CERT_FINGERPRINT_LEN, 95);
    assert_eq!(ANDROID_ORIGIN_PREFIX, "android:apk-key-hash:");
    assert_eq!(MAX_ASSET_LINKS_BYTES, 1024);
    assert_eq!(
        soos_remote::ASSET_LINKS_RELATION,
        "delegate_permission/common.get_login_creds"
    );
    assert_eq!(ASSET_LINKS_PATH, "/.well-known/assetlinks.json");
}

#[test]
fn test_ana_config_without_android_table_is_unchanged() {
    let loaded = parse_config_with_android(BASE, None).unwrap();
    assert!(loaded.android.is_none());
    assert_eq!(loaded.remote, parse_config(BASE, None).unwrap());
}

#[test]
fn test_ana_config_valid_android_table() {
    let text = with_android(&format!(
        "package = \"{PACKAGE}\"\nsha256_cert_fingerprints = [\"{}\", \"{}\"]\n",
        colon_hex(&fp1()),
        colon_hex(&fp2())
    ));
    let loaded = parse_config_with_android(&text, None).unwrap();
    assert_eq!(loaded.remote, parse_config(&text, None).unwrap());
    let app = loaded.android.expect("android table");
    assert_eq!(app.package(), PACKAGE);
    assert_eq!(app.cert_fingerprints(), &[fp1(), fp2()]);
    assert_eq!(
        app.origins(),
        vec![android_origin(&fp1()), android_origin(&fp2())]
    );
    for origin in app.origins() {
        assert!(origin.starts_with(ANDROID_ORIGIN_PREFIX));
        assert!(!origin.contains('='), "unpadded base64url: {origin}");
        assert!(!origin.contains('+') && !origin.contains('/'), "{origin}");
        assert_eq!(origin.len(), ANDROID_ORIGIN_PREFIX.len() + 43);
    }
}

#[test]
fn test_ana_config_android_requires_rp_id() {
    let text = format!(
        "allowed_logins = [\"owner@example.com\"]\n\
         socket_path = \"/run/user/1000/soos-remote/remote.sock\"\n\n[android]\n{}",
        valid_table()
    );
    assert_eq!(
        parse_config_with_android(&text, None).unwrap_err(),
        ConfigError::AndroidRequiresRpId
    );
    assert_eq!(
        parse_config(&text, None).unwrap_err(),
        ConfigError::AndroidRequiresRpId,
        "parse_config validates the table too"
    );
}

#[test]
fn test_ana_config_android_requires_funnel() {
    let text = format!(
        "allowed_logins = [\"owner@example.com\"]\n\
         socket_path = \"/run/user/1000/soos-remote/remote.sock\"\n\
         rp_id = \"pc.tail1234.ts.net\"\n\n[android]\n{}",
        valid_table()
    );
    assert_eq!(
        parse_config_with_android(&text, None).unwrap_err(),
        ConfigError::AndroidRequiresFunnel
    );
    let explicit_off = text.replace("\n\n[android]", "\nallow_funnel = false\n\n[android]");
    assert_eq!(
        parse_config(&explicit_off, None).unwrap_err(),
        ConfigError::AndroidRequiresFunnel
    );
}

#[test]
fn test_ana_config_accepts_valid_packages() {
    let longest = format!("a.{}", "b".repeat(MAX_ANDROID_PACKAGE_LEN - 2));
    assert_eq!(longest.len(), MAX_ANDROID_PACKAGE_LEN);
    for package in [
        "a.b",
        "org.soos.remote",
        "Org.Soos_X.r2",
        "com.example.app_1.x9",
        longest.as_str(),
    ] {
        let text = with_android(&format!(
            "package = \"{package}\"\nsha256_cert_fingerprints = [\"{}\"]\n",
            colon_hex(&fp1())
        ));
        let app = parse_config_with_android(&text, None)
            .unwrap_or_else(|e| panic!("{package}: {e}"))
            .android
            .unwrap();
        assert_eq!(app.package(), package, "kept verbatim (case-sensitive)");
    }
}

#[test]
fn test_ana_config_rejects_invalid_packages() {
    let too_long = format!("a.{}", "b".repeat(MAX_ANDROID_PACKAGE_LEN - 1));
    for package in [
        "",
        "soos",
        ".org.soos",
        "org.soos.",
        "org..soos",
        "1org.soos",
        "org.1soos",
        "_org.soos",
        "org.soos-remote",
        "org.soos remote",
        "org.sööos.remote",
        "org.soos.remote\n",
        "org/soos",
        too_long.as_str(),
    ] {
        let table = format!(
            "package = \"{}\"\nsha256_cert_fingerprints = [\"{}\"]\n",
            package.replace('\n', "\\n"),
            colon_hex(&fp1())
        );
        assert_eq!(
            android_err(&table),
            ConfigError::InvalidAndroidPackage,
            "{package:?}"
        );
    }
    let missing = format!("sha256_cert_fingerprints = [\"{}\"]\n", colon_hex(&fp1()));
    assert_eq!(android_err(&missing), ConfigError::InvalidAndroidPackage);
}

#[test]
fn test_ana_config_fingerprint_count() {
    let max = MAX_ANDROID_CERT_FINGERPRINTS;
    for table in [
        format!("package = \"{PACKAGE}\"\n"),
        format!("package = \"{PACKAGE}\"\nsha256_cert_fingerprints = []\n"),
        format!(
            "package = \"{PACKAGE}\"\nsha256_cert_fingerprints = [\"{}\", \"{}\", \"{}\"]\n",
            colon_hex(&fp1()),
            colon_hex(&fp2()),
            colon_hex(&[0x01; 32])
        ),
    ] {
        assert_eq!(
            android_err(&table),
            ConfigError::AndroidFingerprintCount { max },
            "{table}"
        );
    }
}

#[test]
fn test_ana_config_rejects_invalid_fingerprints() {
    let good = colon_hex(&fp1());
    let cases = [
        good.to_ascii_lowercase(),
        good.replace(':', ""),
        good[..good.len() - 3].to_string(),
        format!("{good}:AB"),
        format!("{good}:"),
        format!(":{good}"),
        good.replacen("A", "G", 1),
        good.replacen(':', "-", 1),
        format!(" {good}"),
        String::new(),
    ];
    for bad in &cases {
        let table = format!(
            "package = \"{PACKAGE}\"\nsha256_cert_fingerprints = [\"{}\", \"{bad}\"]\n",
            colon_hex(&fp2())
        );
        assert_eq!(
            android_err(&table),
            ConfigError::InvalidAndroidFingerprint { index: 1 },
            "{bad:?}"
        );
    }
    let duplicate =
        format!("package = \"{PACKAGE}\"\nsha256_cert_fingerprints = [\"{good}\", \"{good}\"]\n");
    assert_eq!(
        android_err(&duplicate),
        ConfigError::InvalidAndroidFingerprint { index: 1 }
    );
    assert_eq!(good.len(), ANDROID_CERT_FINGERPRINT_LEN);
}

#[test]
fn test_ana_config_unknown_or_malformed_android_table_is_syntax() {
    let unknown = format!("{}relation = \"x\"\n", valid_table());
    assert_eq!(android_err(&unknown), ConfigError::Syntax);
    let not_a_table = format!("{BASE}android = 1\n");
    assert_eq!(
        parse_config_with_android(&not_a_table, None).unwrap_err(),
        ConfigError::Syntax
    );
    let wrong_type = format!("package = \"{PACKAGE}\"\nsha256_cert_fingerprints = \"AB\"\n");
    assert_eq!(android_err(&wrong_type), ConfigError::Syntax);
}

#[test]
fn test_ana_config_checks_run_after_existing_ones() {
    // An existing error keeps its precedence over an invalid [android] table.
    let text =
        "allowed_logins = []\nsocket_path = \"/run/x/remote.sock\"\n\n[android]\npackage = \"x\"\n";
    assert_eq!(
        parse_config_with_android(text, None).unwrap_err(),
        ConfigError::NoAllowedLogins
    );
}

#[test]
fn test_ana_config_values_never_reach_debug_or_errors() {
    let loaded = parse_config_with_android(&with_android(&valid_table()), None).unwrap();
    let fingerprint = colon_hex(&fp1());
    let origin = android_origin(&fp1());
    for text in [
        format!("{:?}", loaded.android.as_ref().unwrap()),
        format!("{loaded:?}"),
    ] {
        assert!(!text.contains(PACKAGE), "{text}");
        assert!(!text.contains(&fingerprint), "{text}");
        assert!(!text.contains(&origin), "{text}");
        assert!(!text.contains(&b64url(&fp1())), "{text}");
    }
    assert_eq!(
        format!("{:?}", loaded.android.unwrap()),
        "AndroidAppConfig(<redacted>)"
    );
    let bad_package =
        format!("package = \"secret.pkg-name\"\nsha256_cert_fingerprints = [\"{fingerprint}\"]\n");
    let bad_fingerprint = format!(
        "package = \"{PACKAGE}\"\nsha256_cert_fingerprints = [\"{}\"]\n",
        fingerprint.to_ascii_lowercase()
    );
    for (table, needle) in [
        (bad_package, "secret".to_string()),
        (bad_fingerprint, fingerprint.to_ascii_lowercase()),
    ] {
        let message = android_err(&table).to_string();
        assert!(!message.contains(&needle), "{message}");
        assert!(message.contains("[android]"), "{message}");
    }
}

#[test]
fn test_ana_load_config_with_android_reads_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote.toml");
    std::fs::write(&path, with_android(&valid_table())).unwrap();
    let loaded = load_config_with_android(&path, None).unwrap();
    assert_eq!(loaded.android.unwrap().package(), PACKAGE);
    assert_eq!(
        loaded.remote,
        soos_remote::config::load_config(&path, None).unwrap()
    );
    assert_eq!(
        load_config_with_android(&dir.path().join("absent.toml"), None).unwrap_err(),
        ConfigError::NotFound
    );
}

#[test]
fn test_ana_asset_links_json_is_exact_and_bounded() {
    let app = app_with(&[fp1(), fp2()]);
    let expected = format!(
        "[{{\"relation\":[\"delegate_permission/common.get_login_creds\"],\
         \"target\":{{\"namespace\":\"android_app\",\"package_name\":\"{PACKAGE}\",\"sha256_cert_fingerprints\":\
         [\"{}\",\"{}\"]}}}}]",
        colon_hex(&fp1()),
        colon_hex(&fp2())
    );
    let body = app.asset_links_json();
    assert_eq!(String::from_utf8(body.clone()).unwrap(), expected);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed.as_array().unwrap().len(), 1, "exactly one statement");

    // The largest accepted table stays under the bound.
    let longest = format!("a.{}", "b".repeat(MAX_ANDROID_PACKAGE_LEN - 2));
    let text = with_android(&format!(
        "package = \"{longest}\"\nsha256_cert_fingerprints = [\"{}\", \"{}\"]\n",
        colon_hex(&[0xFF; 32]),
        colon_hex(&[0xEE; 32])
    ));
    let big = parse_config_with_android(&text, None)
        .unwrap()
        .android
        .unwrap();
    assert!(big.asset_links_json().len() <= MAX_ASSET_LINKS_BYTES);
}

// ---------------------------------------------------------------------------------------
// ANA2: origin policy of the WebAuthn verifier
// ---------------------------------------------------------------------------------------

const CHALLENGE: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

fn rp_with_app() -> RelyingParty {
    RelyingParty::new(HOST).with_android_app(PACKAGE, &[fp1(), fp2()])
}

fn get_data(origin: &str, extra: &str) -> Vec<u8> {
    client_data("webauthn.get", CHALLENGE, origin, extra)
}

#[test]
fn test_ana_android_origin_accepted_only_with_policy_for_get() {
    let rp = rp_with_app();
    for digest in [fp1(), fp2()] {
        let raw = get_data(&android_origin(&digest), "");
        let parsed = parse_client_data_with_policy(
            &rp,
            &raw,
            CeremonyType::Get,
            OriginPolicy::WebOrAndroidApp,
        )
        .expect("configured app origin");
        assert_eq!(parsed.origin, ClientOrigin::AndroidApp);
        assert_eq!(b64url(&parsed.challenge), CHALLENGE);
        assert_eq!(
            parse_client_data(&rp, &raw, CeremonyType::Get).err(),
            Some(WebAuthnError::OriginMismatch),
            "parse_client_data stays web-only"
        );
        assert_eq!(
            parse_client_data_with_policy(&rp, &raw, CeremonyType::Get, OriginPolicy::WebOnly)
                .err(),
            Some(WebAuthnError::OriginMismatch)
        );
        let create = client_data("webauthn.create", CHALLENGE, &android_origin(&digest), "");
        assert_eq!(
            parse_client_data_with_policy(
                &rp,
                &create,
                CeremonyType::Create,
                OriginPolicy::WebOrAndroidApp
            )
            .err(),
            Some(WebAuthnError::OriginMismatch),
            "never for a registration"
        );
    }
    let web = get_data(ORIGIN, "");
    for policy in [OriginPolicy::WebOnly, OriginPolicy::WebOrAndroidApp] {
        let parsed = parse_client_data_with_policy(&rp, &web, CeremonyType::Get, policy).unwrap();
        assert_eq!(parsed.origin, ClientOrigin::Web);
    }
    assert_eq!(
        parse_client_data(&rp, &web, CeremonyType::Get)
            .unwrap()
            .origin,
        ClientOrigin::Web
    );
}

#[test]
fn test_ana_android_origin_refused_without_app() {
    let rp = RelyingParty::new(HOST);
    let raw = get_data(&android_origin(&fp1()), "");
    assert_eq!(
        parse_client_data_with_policy(&rp, &raw, CeremonyType::Get, OriginPolicy::WebOrAndroidApp)
            .err(),
        Some(WebAuthnError::OriginMismatch)
    );
}

#[test]
fn test_ana_near_android_origins_are_refused() {
    let rp = rp_with_app();
    let good = android_origin(&fp1());
    let padded = format!("{good}=");
    let standard = format!(
        "android:apk-key-hash:{}",
        b64url(&fp1()).replace('-', "+").replace('_', "/")
    );
    let other = android_origin(&[0x01; 32]);
    let colon_form = format!("android:apk-key-hash:{}", colon_hex(&fp1()));
    let upper = good.replace("android:apk-key-hash:", "ANDROID:apk-key-hash:");
    let slash = format!("{good}/");
    let truncated = good[..good.len() - 1].to_string();
    for origin in [padded, standard, other, colon_form, upper, slash, truncated] {
        if origin == good {
            continue;
        }
        let raw = get_data(&origin, "");
        assert_eq!(
            parse_client_data_with_policy(
                &rp,
                &raw,
                CeremonyType::Get,
                OriginPolicy::WebOrAndroidApp
            )
            .err(),
            Some(WebAuthnError::OriginMismatch),
            "{origin}"
        );
    }
}

#[test]
fn test_ana_android_package_name_member_must_match_when_present() {
    let rp = rp_with_app();
    let origin = android_origin(&fp1());
    let same = get_data(&origin, &format!(",\"androidPackageName\":\"{PACKAGE}\""));
    assert!(parse_client_data_with_policy(
        &rp,
        &same,
        CeremonyType::Get,
        OriginPolicy::WebOrAndroidApp
    )
    .is_ok());
    for other in [
        "org.evil.app",
        "org.soos.remote.debug",
        "ORG.SOOS.REMOTE",
        "",
    ] {
        let raw = get_data(&origin, &format!(",\"androidPackageName\":\"{other}\""));
        assert_eq!(
            parse_client_data_with_policy(
                &rp,
                &raw,
                CeremonyType::Get,
                OriginPolicy::WebOrAndroidApp
            )
            .err(),
            Some(WebAuthnError::OriginMismatch),
            "{other}"
        );
    }
    let not_string = get_data(&origin, ",\"androidPackageName\":7");
    assert!(parse_client_data_with_policy(
        &rp,
        &not_string,
        CeremonyType::Get,
        OriginPolicy::WebOrAndroidApp
    )
    .is_err());
}

#[test]
fn test_ana_android_origin_keeps_cross_origin_rules() {
    let rp = rp_with_app();
    let origin = android_origin(&fp1());
    let cross = format!(
        "{{\"type\":\"webauthn.get\",\"challenge\":\"{CHALLENGE}\",\"origin\":\"{origin}\",\"crossOrigin\":true}}"
    );
    let top = get_data(&origin, ",\"topOrigin\":\"https://evil.example\"");
    for raw in [cross.into_bytes(), top] {
        assert_eq!(
            parse_client_data_with_policy(
                &rp,
                &raw,
                CeremonyType::Get,
                OriginPolicy::WebOrAndroidApp
            )
            .err(),
            Some(WebAuthnError::CrossOrigin)
        );
    }
}

proptest! {
    #[test]
    fn test_ana_arbitrary_origins_never_pass_as_android(origin in "[ -~]{0,120}") {
        let rp = rp_with_app();
        let accepted = [android_origin(&fp1()), android_origin(&fp2()), ORIGIN.to_string()];
        let escaped = origin.replace('\\', "\\\\").replace('"', "\\\"");
        let raw = get_data(&escaped, "");
        let result = parse_client_data_with_policy(
            &rp, &raw, CeremonyType::Get, OriginPolicy::WebOrAndroidApp,
        );
        if accepted.contains(&origin) {
            prop_assert!(result.is_ok());
        } else {
            prop_assert!(result.is_err());
        }
    }
}

// ---------------------------------------------------------------------------------------
// ANA3: route table
// ---------------------------------------------------------------------------------------

#[test]
fn test_ana_route_asset_links() {
    for method in [Method::Get, Method::Head, Method::Post, Method::Other] {
        assert_eq!(
            route(method, ASSET_LINKS_PATH),
            Route::AssetLinks,
            "{method:?}"
        );
        assert_eq!(
            route(method, &format!("{ASSET_LINKS_PATH}?x=1")),
            Route::AssetLinks
        );
        assert!(!accepts_body(method, ASSET_LINKS_PATH));
    }
    assert!(is_funnel_public(Route::AssetLinks));
    assert_eq!(allow_header(ASSET_LINKS_PATH), "GET, HEAD");
    for near in [
        "/.well-known/assetlinks.json/",
        "/.well-known/AssetLinks.json",
        "/.well-known/",
        "/.well-known",
        "/.well-known/apple-app-site-association",
        "/assetlinks.json",
        "/.well-known//assetlinks.json",
    ] {
        for method in [Method::Get, Method::Post] {
            assert_eq!(route(method, near), Route::NotFound, "{method:?} {near}");
        }
    }
}

// ---------------------------------------------------------------------------------------
// ANA4–ANA6: server
// ---------------------------------------------------------------------------------------

/// [`Harness::start_with`] with an optional `[android]` binding.
async fn start(options: Options, android: Option<AndroidAppConfig>) -> Harness {
    let frozen = FrozenClock::hold();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote.sock");
    let store_path = dir.path().join(CREDENTIALS_FILE_NAME);
    let owner = Authenticator::owner();
    write_store(&store_path, &[StoredPasskey::of(&owner)]);
    let config = RemoteConfig {
        allowed_logins: vec![TailscaleLogin::parse(LOGIN).unwrap()],
        socket_path: path.clone(),
        poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
        allowed_hosts: options.allowed_hosts,
        allow_unlock: options.allow_unlock,
        auth: AuthConfig {
            rp_id: options.rp_id.clone(),
            allow_funnel: options.allow_funnel,
            credentials_path: Some(store_path.clone()),
        },
        alerts: soos_remote::config::AlertsConfig::default(),
        push: soos_remote::config::PushConfig::default(),
        camera: soos_remote::config::CameraConfig::default(),
        battery: soos_remote::config::BatteryConfig::default(),
        offline: soos_remote::config::OfflineConfig::default(),
    };
    let source = MockSource::unlocked();
    let clock = TestClock::new();
    let clock_fn = Arc::clone(&clock);
    let mut state = ServerState::new(config, UID, source.clone())
        .with_unix_clock(Arc::new(move || clock_fn.now_ms()))
        .with_credentials_path(store_path.clone())
        .with_file_owner_uid(own_uid());
    if let Some(app) = android {
        state = state.with_android_app(app);
    }
    let listener = UnixListener::bind(&path).unwrap();
    let (tx, rx) = oneshot::channel::<()>();
    let server = tokio::spawn(serve(listener, Arc::new(state), async move {
        let _ = rx.await;
    }));
    settle().await;
    Harness {
        dir: dir.path().to_path_buf(),
        _dir: dir,
        path,
        store_path,
        owner,
        source,
        clock,
        shutdown: Some(tx),
        server: Some(server),
        _frozen: frozen,
        secrets: Mutex::new(Vec::new()),
    }
}

fn expected_links() -> Vec<u8> {
    app_with(&[fp1()]).asset_links_json()
}

#[tokio::test(start_paused = true)]
async fn test_ana_asset_links_absent_is_byte_identical_404() {
    let h = start(Options::funnel(), None).await;
    for via in [Via::Tailnet, Via::funnel("198.51.100.7")] {
        for method in ["GET", "HEAD", "POST"] {
            let links = h.send(&via, method, ASSET_LINKS_PATH, &[], None).await;
            let unknown = h.send(&via, method, "/nope", &[], None).await;
            assert_eq!(links.status, 404, "{method} {via:?}");
            assert_eq!(links.headers, unknown.headers, "{method} {via:?}");
            assert_eq!(links.body, unknown.body, "{method} {via:?}");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn test_ana_asset_links_served_on_tailnet_and_funnel() {
    let h = start(Options::funnel(), Some(app_with(&[fp1()]))).await;
    let expected = expected_links();
    for via in [
        Via::Tailnet,
        Via::funnel("198.51.100.7"),
        Via::funnel("198.51.100.8").with_cookie("__Host-soos_session=garbage"),
    ] {
        let r = h.send(&via, "GET", ASSET_LINKS_PATH, &[], None).await;
        assert_eq!(r.status, 200, "{via:?}: {}", r.result_or_body());
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_eq!(r.body, expected);
        r.assert_mandatory_headers();
        assert!(r.header("set-cookie").is_none(), "{via:?}");
        let head = h.send(&via, "HEAD", ASSET_LINKS_PATH, &[], None).await;
        assert_eq!(head.status, 200);
        assert!(head.body.is_empty());
        assert_eq!(
            head.header("content-length"),
            Some(expected.len().to_string().as_str())
        );
        head.assert_mandatory_headers();
        let query = h
            .send(&via, "GET", &format!("{ASSET_LINKS_PATH}?a=1"), &[], None)
            .await;
        assert_eq!(query.body, expected);
    }
}

#[tokio::test(start_paused = true)]
async fn test_ana_asset_links_other_methods_and_hosts() {
    let h = start(Options::funnel(), Some(app_with(&[fp1()]))).await;
    for via in [Via::Tailnet, Via::funnel("198.51.100.7")] {
        let post = h.send(&via, "POST", ASSET_LINKS_PATH, &[], None).await;
        assert_eq!(post.status, 405, "{via:?}");
        assert_eq!(post.result(), "method_not_allowed");
        assert_eq!(post.header("allow"), Some("GET, HEAD"));
        assert!(post.header("set-cookie").is_none());
    }
    let wrong = h
        .raw(&raw_request(
            "GET",
            ASSET_LINKS_PATH,
            &[("Host", "example.com"), ("Tailscale-User-Login", LOGIN)],
        ))
        .await;
    assert_eq!(wrong.status, 421);
    let other_node = h
        .raw(&raw_request(
            "GET",
            ASSET_LINKS_PATH,
            &[
                ("Host", "localhost"),
                ("X-Forwarded-Host", "other.tail1234.ts.net"),
                ("X-Forwarded-Proto", "https"),
                ("Tailscale-Funnel-Request", "?1"),
            ],
        ))
        .await;
    assert_eq!(other_node.status, 421, "the Funnel host must be rp_id");
}

#[tokio::test(start_paused = true)]
async fn test_ana_asset_links_need_rp_id_at_runtime() {
    // A binding without rp_id (only reachable through the API, the file refuses it) never
    // serves the statement.
    let h = start(Options::default(), Some(app_with(&[fp1()]))).await;
    let r = h.get_via(&Via::Tailnet, ASSET_LINKS_PATH).await;
    assert_eq!(r.status, 404);
}

/// A login through `via` whose assertion carries `origin` and `extra` client-data members.
async fn login_with_origin(h: &Harness, via: &Via, origin: &str, extra: &str) -> HttpResponse {
    let options = h.login_options(via).await;
    let challenge = h.challenge_of(&options);
    let client = client_data("webauthn.get", &challenge, origin, extra);
    let ad = assertion_auth_data(HOST, h.owner.flags(), 0);
    let sig = h.owner.sign(&ad, &client);
    let body = assertion_body(
        &h.owner.credential_id,
        &client,
        &ad,
        &sig,
        Some(&OWNER_HANDLE),
    );
    h.post_login(via, &body).await
}

#[tokio::test(start_paused = true)]
async fn test_ana_login_accepts_the_configured_app_origin() {
    let h = start(Options::funnel(), Some(app_with(&[fp1(), fp2()]))).await;
    let via = Via::funnel("198.51.100.7");
    let extra = format!(",\"androidPackageName\":\"{PACKAGE}\"");
    let r = login_with_origin(&h, &via, &android_origin(&fp2()), &extra).await;
    assert_eq!(r.status, 200, "{}", r.result_or_body());
    assert_eq!(r.result(), "logged_in");
    let cookie = cookie_of(&r);
    let state = h.auth_state(&via.with_cookie(&cookie)).await;
    assert_eq!(state.json()["authenticated"], true);
    let status = h.get_via(&via.with_cookie(&cookie), "/api/status").await;
    assert_eq!(status.status, 200);
    // The web origin still works next to it.
    let web = login_with_origin(&h, &Via::funnel("198.51.100.8"), ORIGIN, "").await;
    assert_eq!(web.status, 200, "{}", web.result_or_body());
}

#[tokio::test(start_paused = true)]
async fn test_ana_login_refuses_app_origin_without_binding_or_other_digest() {
    let h = start(Options::funnel(), None).await;
    let r = login_with_origin(
        &h,
        &Via::funnel("198.51.100.7"),
        &android_origin(&fp1()),
        "",
    )
    .await;
    assert_eq!(r.status, 403);
    assert_eq!(r.result(), "passkey_rejected");
    assert!(r.header("set-cookie").is_none());

    let h = start(Options::funnel(), Some(app_with(&[fp1()]))).await;
    let other = login_with_origin(
        &h,
        &Via::funnel("198.51.100.7"),
        &android_origin(&fp2()),
        "",
    )
    .await;
    assert_eq!(other.status, 403);
    assert_eq!(other.result(), "passkey_rejected");
    let wrong_package = login_with_origin(
        &h,
        &Via::funnel("198.51.100.9"),
        &android_origin(&fp1()),
        ",\"androidPackageName\":\"org.evil.app\"",
    )
    .await;
    assert_eq!(wrong_package.status, 403);
    assert_eq!(wrong_package.result(), "passkey_rejected");
}

#[tokio::test(start_paused = true)]
async fn test_ana_unlock_and_registration_stay_web_origin_only() {
    let h = start(Options::funnel(), Some(app_with(&[fp1()]))).await;
    h.source.set_locked();
    // Unlock: the app origin is refused, the web origin works.
    let options = h.unlock_options(&Via::Tailnet).await;
    let challenge = h.challenge_of(&options);
    let client = client_data("webauthn.get", &challenge, &android_origin(&fp1()), "");
    let ad = assertion_auth_data(HOST, h.owner.flags(), 0);
    let sig = h.owner.sign(&ad, &client);
    let body = assertion_body(
        &h.owner.credential_id,
        &client,
        &ad,
        &sig,
        Some(&OWNER_HANDLE),
    );
    let refused = h.post_unlock(&Via::Tailnet, Some(&body)).await;
    assert_eq!(refused.status, 403);
    assert_eq!(refused.result(), "passkey_rejected");
    assert!(h.source.unlock_ids().is_empty(), "logind never reached");
    h.advance_ms(soos_remote::MIN_UNLOCK_INTERVAL_MS).await;
    let accepted = h.unlock_with(&Via::Tailnet, &h.owner.clone()).await;
    assert_eq!(accepted.status, 202, "{}", accepted.result_or_body());

    // Registration: an app origin is never accepted.
    h.write_code("ABCDEFGHJK", h.now_unix_s() + 60);
    let options = h.register_options("ABCDEFGHJK").await;
    let challenge = h.challenge_of(&options);
    let device = Authenticator::device(0x22, b"android-app-device-credential-01");
    let client = client_data("webauthn.create", &challenge, &android_origin(&fp1()), "");
    let body = registration_body(
        &device.credential_id,
        &client,
        &registration(&device, HOST, device.flags() | AT),
    );
    let r = h.register_verify(&body).await;
    assert_eq!(r.status, 403);
    assert_eq!(r.result(), "passkey_rejected");
    assert_eq!(
        h.store_json_value()["passkeys"].as_array().unwrap().len(),
        1,
        "nothing stored"
    );
}

/// The store path helper is used by the shared harness only; keeps the import set honest.
#[test]
fn test_ana_fixture_digests_differ() {
    assert_ne!(fp1(), fp2());
    assert!(Path::new(CREDENTIALS_FILE_NAME).extension().is_some());
}

/// ANA2: through `AuthState::check_assertion` (the single call site of every assertion), the
/// app origin passes for `Login` only; `Unlock` and `CameraView` refuse the same assertion.
#[test]
fn test_ana_check_assertion_accepts_the_app_origin_for_login_only() {
    use soos_remote::auth::{AssertionFailure, AuthState, DecodedAssertion};
    use soos_remote::challenge::{ChallengeBinding, ChallengePurpose};
    use soos_remote::credentials::CredentialStore;
    use soos_remote::identity::{ClientHint, PathClass};
    use zeroize::Zeroizing;

    let dir = tempfile::tempdir().unwrap();
    let store_path = dir.path().join(CREDENTIALS_FILE_NAME);
    let owner = Authenticator::owner();
    write_store(&store_path, &[StoredPasskey::of(&owner)]);
    let rp = rp_with_app();
    let cases = [
        (
            ChallengePurpose::Login,
            PathClass::Funnel,
            ChallengeBinding::Login(ClientHint::UNKNOWN),
            true,
        ),
        (
            ChallengePurpose::Unlock,
            PathClass::Tailnet,
            ChallengeBinding::None,
            false,
        ),
        (
            ChallengePurpose::CameraView,
            PathClass::Tailnet,
            ChallengeBinding::None,
            false,
        ),
    ];
    for (index, (purpose, class, binding, accepted)) in cases.into_iter().enumerate() {
        let mut auth = AuthState::new(Some(CredentialStore::new(store_path.clone(), own_uid())));
        let now = tokio::time::Instant::now();
        let bytes = [u8::try_from(index).unwrap() + 1; 32];
        auth.challenges
            .issue(now, class, purpose, binding.clone(), None, bytes)
            .unwrap();
        let client = client_data("webauthn.get", &b64url(&bytes), &android_origin(&fp1()), "");
        let ad = assertion_auth_data(HOST, owner.flags(), 0);
        let sig = owner.sign(&ad, &client);
        let assertion = DecodedAssertion {
            id: Zeroizing::new(owner.credential_id.clone()),
            client_data_json: Zeroizing::new(client),
            authenticator_data: Zeroizing::new(ad),
            signature: Zeroizing::new(sig),
            user_handle: Some(Zeroizing::new(OWNER_HANDLE.to_vec())),
        };
        let result = auth.check_assertion(&rp, class, purpose, &binding, &assertion, now);
        if accepted {
            assert!(result.is_ok(), "{purpose:?} accepts the app origin");
        } else {
            assert!(
                matches!(result, Err(AssertionFailure::Rejected)),
                "{purpose:?} refuses the app origin"
            );
        }
    }
}
