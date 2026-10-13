//! Static contracts of the direct biometric prompt of `soos-remote` (GitHub #354 sub-issue 1;
//! ADR "[2026-10-12] Named Credentials on Authenticated Passkey Ceremonies of
//! `soos-remote`"; architect spec `AI/architect_spec_remote_direct_biometric.md` §8.1; matrix
//! RDB3–RDB6, RDB9):
//!
//! - S-1 sign-in stays discoverable (`OptionsCredentials::Discoverable`, no allow list);
//! - S-2 only the unlock and camera handlers name the registered credentials;
//! - S-3 registration prefers the platform authenticator and sends the transports;
//! - S-4 the six transport names are single-sourced between the page and the store;
//! - S-5 the named options routes are never Funnel-public;
//! - S-6 documentation, ADR and walkthrough 195.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use crate::remote_companion_contract::{exists, production_part, read, strip_comments};

const APP_JS: &str = "crates/remote/assets/app.js";
const SERVER: &str = "crates/remote/src/server.rs";
const ROUTES: &str = "crates/remote/src/routes.rs";
const CREDENTIALS: &str = "crates/remote/src/credentials.rs";
const REMOTE_DOC: &str = "Docs/REMOTE_COMPANION.md";
const ADR_TITLE: &str =
    "[2026-10-12] Named Credentials on Authenticated Passkey Ceremonies of `soos-remote`";
const WALKTHROUGH: &str = "AI/walkthroughs/195_remote_direct_biometric.md";
/// The six WebAuthn L3 transport names, canonical order (spec §3.1, §3.5).
const TRANSPORTS: [&str; 6] = ["internal", "hybrid", "usb", "nfc", "ble", "smart-card"];

/// The top-level item of `src` starting at the first occurrence of `start`, up to and
/// including its closing `}` at column 0 (Rust functions and top-level JS functions of the
/// page are formatted that way).
fn top_level_item<'a>(src: &'a str, start: &str) -> &'a str {
    let at = src
        .find(start)
        .unwrap_or_else(|| panic!("`{start}` not found"));
    let rest = &src[at..];
    let end = rest
        .find("\n}")
        .unwrap_or_else(|| panic!("end of `{start}` not found"));
    &rest[..end + 2]
}

/// Every top-level `async fn <name><S: SessionSource>(` handler of `server.rs`: (name, item).
fn handlers(server: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (at, _) in server.match_indices("\nasync fn ") {
        let line_end = server[at + 1..]
            .find('\n')
            .map_or(server.len(), |e| at + 1 + e);
        let line = &server[at + 1..line_end];
        if !line.contains("<S: SessionSource>") {
            continue;
        }
        let name = line["async fn ".len()..]
            .split('<')
            .next()
            .expect("handler name")
            .to_string();
        let item = top_level_item(&server[at + 1..], "async fn ").to_string();
        out.push((name, item));
    }
    out
}

/// The text of the `## <number>.` section of a Markdown document (up to the next `## `).
fn section<'a>(doc: &'a str, number: &str) -> &'a str {
    let heading = format!("\n## {number} ");
    let at = doc
        .find(&heading)
        .unwrap_or_else(|| panic!("section ## {number} not found"));
    let rest = &doc[at + 1..];
    let end = rest[3..].find("\n## ").map_or(rest.len(), |e| e + 3);
    &rest[..end]
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The double-quoted string literals of `code`, in order (no escapes expected).
fn string_literals(code: &str) -> Vec<String> {
    code.split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s.to_string())
        .collect()
}

#[test]
fn test_rdb_top_level_item_and_section_helpers() {
    let src = "x\nfn a() {\n    if b {\n    }\n}\nfn c() {\n}\n";
    assert_eq!(
        top_level_item(src, "fn a("),
        "fn a() {\n    if b {\n    }\n}"
    );
    let doc = "intro\n## 2g. G\ng text\n## 2h. H\nh text\n## 3. Next\n";
    assert_eq!(section(doc, "2h."), "## 2h. H\nh text");
    assert_eq!(
        string_literals("[\"a\", \"b-c\"]"),
        vec!["a".to_string(), "b-c".to_string()]
    );
}

/// S-1 (RDB3, spec D-2, OD-3, PE-1): the login handler passes
/// `OptionsCredentials::Discoverable`; `requestLogin` calls `getAssertion(options.body,
/// false)`; `allowCredentials` occurs exactly once in `app.js`, inside `getAssertion`, which
/// sets it only when `nameCredentials` is true and the server sent a list.
#[test]
fn test_rdb_s1_login_ceremony_stays_discoverable() {
    let server = strip_comments(production_part(&read(SERVER)));
    let login = top_level_item(&server, "async fn login_options<");
    assert!(
        login.contains("OptionsCredentials::Discoverable"),
        "login_options passes OptionsCredentials::Discoverable"
    );
    assert!(!login.contains("OptionsCredentials::Registered"));
    assert!(
        server.contains("enum OptionsCredentials"),
        "the OptionsCredentials type (spec §3.3)"
    );

    let app = read(APP_JS);
    let request_login = top_level_item(&app, "function requestLogin(");
    assert!(
        request_login.contains("getAssertion(options.body, false)"),
        "sign-in never names a credential"
    );
    assert!(!request_login.contains("getAssertion(options.body, true)"));
    assert_eq!(
        app.matches("allowCredentials").count(),
        1,
        "allowCredentials exactly once"
    );
    let get_assertion = top_level_item(&app, "function getAssertion(options, nameCredentials)");
    assert!(
        get_assertion.contains("allowCredentials"),
        "the only allowCredentials is in getAssertion"
    );
    for needle in [
        "nameCredentials === true",
        "Array.isArray(options.allow_credentials)",
        "navigator.credentials.get",
        "userVerification: \"required\"",
    ] {
        assert!(
            get_assertion.contains(needle),
            "getAssertion must contain {needle}"
        );
    }
    assert_eq!(
        app.matches("navigator.credentials.get").count(),
        1,
        "a single assertion call site"
    );
}

/// S-2 (RDB1, RDB2, spec D-1, OD-4): `unlock_options` and `camera_options` pass
/// `OptionsCredentials::Registered`, no other handler does; `requestUnlock` and
/// `startCameraView` call `getAssertion(options.body, true)`, and nothing else names.
#[test]
fn test_rdb_s2_unlock_and_camera_name_the_registered_credentials() {
    let server = strip_comments(production_part(&read(SERVER)));
    let all = handlers(&server);
    assert!(all.len() >= 20, "handlers found: {}", all.len());
    let naming: Vec<&str> = all
        .iter()
        .filter(|(_, item)| item.contains("OptionsCredentials::Registered"))
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        naming,
        vec!["unlock_options", "camera_options"],
        "only the unlock and camera options name the credentials"
    );
    let discoverable: Vec<&str> = all
        .iter()
        .filter(|(_, item)| item.contains("OptionsCredentials::Discoverable"))
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(discoverable, vec!["login_options"]);
    assert!(
        squash(&server).contains(
            "async fn issue_options( &self, key: LimitKey, class: PathClass, purpose: ChallengePurpose, binding: ChallengeBinding, credentials: OptionsCredentials, ) -> Response"
        ),
        "issue_options takes the OptionsCredentials argument (spec §3.3)"
    );

    let app = read(APP_JS);
    for function in ["function requestUnlock(", "function startCameraView("] {
        let body = top_level_item(&app, function);
        assert!(
            body.contains("getAssertion(options.body, true)"),
            "{function} names the credentials"
        );
    }
    assert_eq!(app.matches("getAssertion(options.body, true)").count(), 2);
    assert_eq!(app.matches("getAssertion(options.body, false)").count(), 1);
    assert_eq!(
        app.matches("getAssertion(").count(),
        4,
        "the definition and exactly three call sites"
    );
}

/// S-3 (RDB5, spec D-4, §3.5): registration asks for the platform authenticator with the
/// `client-device` hint, keeps discoverable credentials, UV, attestation `none` and ES256,
/// and sends the bounded `getTransports()` list.
#[test]
fn test_rdb_s3_registration_prefers_the_platform_authenticator() {
    let app = read(APP_JS);
    let at = app
        .find("navigator.credentials.create({")
        .expect("the registration call");
    let create = &app[at..];
    let create = &create[..create.find("\n      });").expect("end of create")];
    for needle in [
        "authenticatorAttachment: \"platform\"",
        "hints: [\"client-device\"]",
        "residentKey: \"required\"",
        "requireResidentKey: true",
        "userVerification: \"required\"",
        "attestation: \"none\"",
        "alg: -7",
        "excludeCredentials",
    ] {
        assert!(
            create.contains(needle),
            "registration must contain {needle}"
        );
    }
    assert!(
        !create.contains("cross-platform"),
        "never a cross-platform attachment"
    );
    for needle in [
        "transports: credentialTransports(response)",
        "function credentialTransports(",
        "typeof response.getTransports === \"function\"",
    ] {
        assert!(app.contains(needle), "app.js must contain {needle}");
    }
    let helper = top_level_item(&app, "function credentialTransports(");
    for needle in ["try", "catch", "PASSKEY_TRANSPORTS"] {
        assert!(
            helper.contains(needle),
            "credentialTransports must contain {needle}"
        );
    }
}

/// S-4 (RDB6, spec §3.5, §4): `PASSKEY_TRANSPORTS` of `app.js` equals the `as_str` names of
/// `AuthenticatorTransport` in `credentials.rs`, in the same (canonical) order.
#[test]
fn test_rdb_s4_transport_names_single_sourced() {
    let app = read(APP_JS);
    let line = app
        .lines()
        .find(|l| l.starts_with("const PASSKEY_TRANSPORTS = ["))
        .expect("const PASSKEY_TRANSPORTS = [...] in app.js");
    assert_eq!(app.matches("const PASSKEY_TRANSPORTS").count(), 1);
    assert_eq!(string_literals(line), TRANSPORTS.to_vec());

    let credentials = strip_comments(production_part(&read(CREDENTIALS)));
    assert!(credentials.contains("pub enum AuthenticatorTransport"));
    let as_str = top_level_item(&credentials, "pub const fn as_str(self)");
    let as_str = &as_str[..as_str.find("\n    }").expect("end of as_str")];
    assert_eq!(
        string_literals(as_str),
        TRANSPORTS.to_vec(),
        "AuthenticatorTransport::as_str names, declaration order"
    );
    let all = &credentials[credentials
        .find("pub const ALL: [Self; 6] = [")
        .expect("AuthenticatorTransport::ALL")..];
    let all = &all[..all.find("];").expect("end of ALL")];
    let variants: Vec<String> = all
        .split("Self::")
        .skip(1)
        .map(|rest| {
            rest.chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect()
        })
        .collect();
    assert_eq!(
        variants,
        vec!["Internal", "Hybrid", "Usb", "Nfc", "Ble", "SmartCard"],
        "AuthenticatorTransport::ALL in canonical order"
    );
}

/// S-5 (RDB4, spec D-7, V-4): `is_funnel_public` lists neither `UnlockOptions` nor
/// `CameraOptions`; the server refuses a non-public route without a session before routing;
/// the only handlers that name credentials are reached from exactly those two routes.
#[test]
fn test_rdb_s5_options_routes_are_not_funnel_public() {
    let routes = strip_comments(production_part(&read(ROUTES)));
    let public = top_level_item(&routes, "pub fn is_funnel_public(");
    assert!(public.contains("Route::LoginOptions"), "login stays public");
    for route in ["UnlockOptions", "CameraOptions", "RegisterOptions"] {
        assert!(!public.contains(route), "{route} is never Funnel-public");
    }
    let server = strip_comments(production_part(&read(SERVER)));
    assert!(
        squash(&server).contains("!is_funnel_public(resolved) && session.is_none()"),
        "a non-public Funnel route without a session is refused before routing"
    );
    assert!(server.contains("Route::UnlockOptions => Some(unlock_options(&shared, &ctx).await)"));
    assert!(server.contains("Route::CameraOptions => Some(camera_options(&shared, &ctx).await)"));
    let naming = handlers(&server)
        .into_iter()
        .filter(|(_, item)| item.contains("OptionsCredentials::Registered"))
        .count();
    assert_eq!(
        naming, 2,
        "only the two non-public options handlers name credentials"
    );
}

/// S-6 (RDB9, spec §10, PE-5): §2h documents the preferred passkey service, the Proton Pass
/// re-enrollment on the phone and the RDB10 owner check (previous needles kept); §6 shows
/// `allow_credentials`; the ADR is registered; walkthrough 195 exists.
#[test]
fn test_rdb_s6_docs_adr_and_walkthrough() {
    let doc = read(REMOTE_DOC);
    let android = squash(section(&doc, "2h."));
    let lower = android.to_ascii_lowercase();
    assert!(
        lower.contains("preferred passkey"),
        "§2h: preferred passkey service"
    );
    assert!(
        lower.contains("already exists"),
        "§2h: the \"already exists\" case (PE-5)"
    );
    assert!(
        lower.contains("on the phone"),
        "§2h: enroll on the phone itself (PE-5)"
    );
    for needle in [
        "Proton Pass",
        "soos-remote enroll-code",
        "soos-remote passkeys remove",
        "RDB10",
        "Google Password Manager",
        "enroll-code",
        "fingerprint",
    ] {
        assert!(android.contains(needle), "§2h must contain {needle}");
    }
    let http = squash(section(&doc, "6."));
    assert!(
        http.contains("allow_credentials"),
        "§6 shows allow_credentials"
    );
    assert!(
        http.contains("transports"),
        "§6 shows the register transports"
    );

    let decisions = read("AI/DECISIONS.md");
    assert!(decisions.contains(ADR_TITLE), "the ADR is registered");
    let architecture = read("AI/ARCHITECTURE.md");
    assert!(
        architecture.contains("client-device"),
        "ARCHITECTURE §13 passkey row updated"
    );
    assert!(exists(WALKTHROUGH), "{WALKTHROUGH} exists");
}
