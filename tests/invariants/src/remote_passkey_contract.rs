//! Static contracts of Tailscale Funnel access and in-house passkey authentication in
//! `soos-remote` (ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey
//! Authentication for `soos-remote`", architect spec §10.8 RMC-S13–RMC-S21, tests 46–53a,
//! matrix RMC39):
//!
//! - RMC-S13 pure-Rust passkey dependencies (`p256 0.13.2`, RustCrypto), no OpenSSL, `ring`,
//!   `aws-lc`, `rand` or `webauthn-rs` (exact crate names; `rand_core` allowed);
//! - RMC-S14 the one documented `der@0.7.10` skip;
//! - RMC-S15 no passkey material in any tracing field or inline capture;
//! - RMC-S16 `getrandom::fill` is the only CSPRNG, in `auth.rs` only;
//! - RMC-S17 the `__Host-` cookie name and attributes, never a `Domain`;
//! - RMC-S18 the page uses modal WebAuthn with UV and stores nothing;
//! - RMC-S19 Funnel and passkeys are documented, the ADR is registered, the unit is
//!   unchanged;
//! - RMC-S20 the new subcommands refuse root first and never print with `print!`;
//! - RMC-S21 `X-Forwarded-For` is read only by `client_hint`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::process::Command;

use crate::remote_companion_contract::{
    exists, field_keys, macro_invocations, production_part, read, rust_files, strip_comments,
    toml_table, workspace_root, TRACING_MACROS,
};

/// Every production source of `crates/remote/src`, with the passkey modules required.
fn passkey_sources() -> Vec<(String, String)> {
    let files = rust_files("crates/remote/src");
    for module in [
        "auth.rs",
        "challenge.rs",
        "credentials.rs",
        "enroll.rs",
        "webauthn.rs",
        "websession.rs",
    ] {
        assert!(
            files
                .iter()
                .any(|(rel, _)| rel == &format!("crates/remote/src/{module}")),
            "crates/remote/src/{module} must exist (spec §1.1)"
        );
    }
    files
}

/// Exact crate names refused in the normal dependency tree of `soos-remote` (spec §3.4).
const FORBIDDEN_CRATES: [&str; 7] = [
    "openssl",
    "openssl-sys",
    "native-tls",
    "ring",
    "aws-lc-rs",
    "aws-lc-sys",
    "rand",
];

fn is_forbidden_crate(name: &str) -> bool {
    FORBIDDEN_CRATES.contains(&name) || name.starts_with("webauthn-rs")
}

/// The dependency keys of a manifest table (`name = …` or `name.workspace = true`).
fn dependency_keys(table: &[&str]) -> Vec<String> {
    table
        .iter()
        .filter_map(|line| {
            let key = line.split('=').next()?.trim();
            let key = key.split('.').next()?.trim().trim_matches('"');
            (!key.is_empty()).then(|| key.to_string())
        })
        .collect()
}

/// Test 46 (RMC-S13, F-3): the workspace pins `p256 0.13.2` without default features; the
/// remote manifest takes every passkey crate from the workspace; no forbidden crate is a
/// dependency key or, when `cargo` is available, a crate of the normal tree (exact names,
/// `rand_core` allowed).
#[test]
fn test_rmc_s13_passkey_dependencies_are_pure_rust() {
    assert!(is_forbidden_crate("rand"));
    assert!(is_forbidden_crate("webauthn-rs-core"));
    assert!(is_forbidden_crate("openssl-sys"));
    assert!(
        !is_forbidden_crate("rand_core") && !is_forbidden_crate("rand_chacha_free"),
        "only exact names are refused"
    );
    let root = read("Cargo.toml");
    let workspace = toml_table(&root, "workspace.dependencies");
    assert!(
        workspace.contains(
            &"p256 = { version = \"0.13.2\", default-features = false, features = [\"ecdsa\"] }"
        ),
        "[workspace.dependencies] pins p256 0.13.2 with only the ecdsa feature"
    );
    let manifest = read("crates/remote/Cargo.toml");
    let deps = toml_table(&manifest, "dependencies");
    for name in [
        "p256",
        "sha2",
        "ciborium",
        "getrandom",
        "base64ct",
        "subtle",
    ] {
        assert!(
            deps.contains(&format!("{name} = {{ workspace = true }}").as_str())
                || deps.contains(&format!("{name}.workspace = true").as_str()),
            "crates/remote [dependencies] takes {name} from the workspace: {deps:?}"
        );
    }
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        for key in dependency_keys(&toml_table(&manifest, table)) {
            assert!(
                !is_forbidden_crate(&key),
                "crates/remote [{table}] must not depend on {key}"
            );
        }
    }

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    if Command::new(&cargo).arg("--version").output().is_err() {
        return;
    }
    let output = Command::new(&cargo)
        .current_dir(workspace_root())
        .args([
            "tree",
            "--offline",
            "--locked",
            "-p",
            "soos-remote",
            "-e",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ])
        .output()
        .expect("cargo tree runs");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8_lossy(&output.stdout);
    let names: Vec<&str> = tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    for name in &names {
        assert!(
            !is_forbidden_crate(name),
            "soos-remote's normal dependency tree contains {name}"
        );
    }
    for required in [
        "p256",
        "ecdsa",
        "sha2",
        "ciborium",
        "getrandom",
        "base64ct",
        "subtle",
    ] {
        assert!(
            names.contains(&required),
            "{required} is a normal dependency of soos-remote"
        );
    }
}

/// Test 47 (RMC-S14): exactly one `der@0.7.10` skip, naming its dependents.
#[test]
fn test_rmc_s14_deny_skip_for_der_is_documented() {
    let deny = read("deny.toml");
    let lines: Vec<&str> = deny
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && l.contains("der@0.7.10"))
        .collect();
    assert_eq!(lines.len(), 1, "exactly one der@0.7.10 skip: {lines:?}");
    for needle in [
        "crate = \"der@0.7.10\"",
        "ecdsa",
        "sec1",
        "ureq",
        "reason =",
    ] {
        assert!(lines[0].contains(needle), "{needle} in {}", lines[0]);
    }
}

/// Test 48 (RMC-S15, D-H): no tracing field key or inline capture names passkey material or
/// the client address.
#[test]
fn test_rmc_s15_auth_logging_hygiene() {
    let forbidden = [
        "challenge",
        "challenges",
        "token",
        "token_hash",
        "cookie",
        "credential",
        "credential_id",
        "credential_hash",
        "public_key",
        "key",
        "user_handle",
        "handle",
        "code",
        "enroll_code",
        "code_hash",
        "assertion",
        "attestation",
        "attestation_object",
        "signature",
        "client_data",
        "client_data_json",
        "authenticator_data",
        "hint",
        "client_hint",
        "ip",
        "addr",
        "address",
        "forwarded_for",
        "rp_id",
    ];
    for (rel, content) in passkey_sources() {
        let code = strip_comments(production_part(&content));
        for args in macro_invocations(&code, &TRACING_MACROS) {
            for key in field_keys(&args) {
                assert!(
                    !forbidden.contains(&key.as_str()),
                    "{rel}: field `{key}` must never be logged (D-H): {args}"
                );
            }
            for capture in forbidden {
                for form in [format!("{{{capture}}}"), format!("{{{capture}:")] {
                    assert!(
                        !args.contains(&form),
                        "{rel}: inline capture `{form}` must never be logged (D-H): {args}"
                    );
                }
            }
        }
    }
}

/// Test 49 (RMC-S16): `getrandom::fill` is the only CSPRNG and is called from `auth.rs`
/// only; no `rand` API anywhere in the crate.
#[test]
fn test_rmc_s16_csprng_is_getrandom_only() {
    let mut fill_sites = Vec::new();
    for (rel, content) in passkey_sources() {
        let code = strip_comments(production_part(&content));
        if code.contains("getrandom::fill") {
            fill_sites.push(rel.clone());
        }
        for needle in [
            "rand::",
            "thread_rng",
            "OsRng",
            "rand_core::",
            "getrandom::getrandom",
            "SmallRng",
            "StdRng",
        ] {
            assert!(!code.contains(needle), "{rel} must not use {needle}");
        }
    }
    assert_eq!(
        fill_sites,
        vec!["crates/remote/src/auth.rs".to_string()],
        "getrandom::fill is called from auth.rs only"
    );
}

/// Test 50 (RMC-S17): the cookie constants are the specified ones and no `Domain`
/// attribute exists anywhere in the crate or its assets.
#[test]
fn test_rmc_s17_cookie_attributes() {
    let lib = strip_comments(&read("crates/remote/src/lib.rs"));
    assert!(lib.contains("pub const SESSION_COOKIE_NAME: &str = \"__Host-soos_session\";"));
    assert!(lib.contains(
        "pub const SESSION_COOKIE_ATTRIBUTES: &str = \"Path=/; Secure; HttpOnly; SameSite=Strict\";"
    ));
    for (rel, content) in passkey_sources() {
        let code = strip_comments(production_part(&content)).to_ascii_lowercase();
        assert!(
            !code.contains("domain="),
            "{rel} never sets a cookie Domain"
        );
    }
    for asset in ["app.js", "index.html"] {
        let text = read(&format!("crates/remote/assets/{asset}")).to_ascii_lowercase();
        assert!(!text.contains("domain="), "{asset}");
    }
}

/// Test 51 (RMC-S18, S-3): the page drives modal WebAuthn with user verification, discoverable
/// credentials, attestation `none` and ES256, never conditional mediation, never storage,
/// never `allowCredentials`.
#[test]
fn test_rmc_s18_page_uses_modal_webauthn_without_storage() {
    let app = read("crates/remote/assets/app.js");
    for needle in [
        "navigator.credentials.get",
        "navigator.credentials.create",
        "userVerification: \"required\"",
        "residentKey: \"required\"",
        "attestation: \"none\"",
        "alg: -7",
        "/api/auth/login/options",
        "/api/auth/login/verify",
        "/api/auth/unlock/options",
        "/api/auth/register/options",
        "/api/auth/register/verify",
        "/api/auth/logout",
        "/api/auth/state",
        "\"X-Soos-Action\": \"unlock\"",
        "window.confirm(",
    ] {
        assert!(app.contains(needle), "app.js must contain {needle}");
    }
    for forbidden in [
        "conditional",
        "localStorage",
        "sessionStorage",
        "indexedDB",
        "document.cookie",
        "allowCredentials",
        "innerHTML",
        "eval(",
        "serviceWorker",
    ] {
        assert!(
            !app.contains(forbidden),
            "app.js must not contain {forbidden}"
        );
    }
    let html = read("crates/remote/assets/index.html");
    for id in [
        "id=\"login\"",
        "id=\"login-button\"",
        "id=\"enroll\"",
        "id=\"enroll-code\"",
        "id=\"enroll-button\"",
        "id=\"logout\"",
        "id=\"unlock\"",
    ] {
        assert!(html.contains(id), "index.html must carry {id}");
    }
}

/// Test 52 (RMC-S19): the operator documentation covers Funnel and passkeys, the ADR is
/// registered, and the user unit keeps `AF_UNIX` only without making `~/.config`
/// read-only.
#[test]
fn test_rmc_s19_funnel_and_passkeys_are_documented() {
    let docs = read("Docs/REMOTE_COMPANION.md");
    for needle in [
        "tailscale funnel --bg unix:",
        "allow_funnel",
        "rp_id",
        "soos-remote enroll-code",
        "soos-remote passkeys",
        "Tailscale-Funnel-Request",
        "passkey_required",
        "login_required",
        "port 443",
        "Accepted risk",
    ] {
        assert!(
            docs.contains(needle),
            "Docs/REMOTE_COMPANION.md must mention {needle}"
        );
    }
    let decisions = read("AI/DECISIONS.md");
    assert!(decisions
        .contains("Tailscale Funnel Access and In-House Passkey Authentication for `soos-remote`"));
    let unit = read("packaging/soos-remote.service");
    assert!(unit.contains("RestrictAddressFamilies=AF_UNIX"));
    for line in unit.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("ProtectHome=") {
            assert!(
                !["yes", "true", "read-only", "tmpfs"].contains(&value.trim()),
                "ProtectHome={value} would make the credential store unwritable"
            );
        }
    }
    assert!(exists("AI/architect_spec_remote_passkey_funnel.md"));
}

/// Test 53 (RMC-S20): `main.rs` dispatches `enroll-code` and `passkeys` only after the root
/// refusal, and never prints with `print!`/`println!`.
#[test]
fn test_rmc_s20_subcommands_refuse_root_and_never_print() {
    let main = strip_comments(production_part(&read("crates/remote/src/main.rs")));
    assert!(
        main.contains("EnrollCode") || main.contains("\"enroll-code\""),
        "main.rs names the enroll-code subcommand"
    );
    assert!(
        main.contains("Passkeys") || main.contains("\"passkeys\""),
        "main.rs names the passkeys subcommand"
    );
    let refuse = main
        .find("check_not_root(")
        .expect("check_not_root( in main.rs");
    let mut arms = 0;
    let mut offset = 0;
    for line in main.lines() {
        if (line.contains("EnrollCode") || line.contains("Passkeys")) && line.contains("=>") {
            arms += 1;
            assert!(
                offset > refuse,
                "subcommand dispatch before check_not_root(: {line}"
            );
        }
        offset += line.len() + 1;
    }
    assert!(arms >= 2, "both subcommands are dispatched by a match arm");
    for forbidden in ["println!", "print!(", "eprintln!", "eprint!(", "dbg!("] {
        assert!(
            !main.contains(forbidden),
            "main.rs must not use {forbidden}"
        );
    }
    assert!(
        main.contains("writeln!"),
        "CLI output goes through writeln!"
    );
}

/// Body of the first `fn <name>` in `code` (balanced braces, strings skipped).
fn fn_body<'a>(code: &'a str, name: &str) -> Option<&'a str> {
    let start = code.find(&format!("fn {name}("))?;
    let open = start + code[start..].find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in code[open..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&code[open..=open + i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Test 53a (RMC-S21, F-2): `X-Forwarded-For` is a rate-limit hint only: its constant is
/// defined once in `lib.rs`, named only inside `identity::client_hint`, and its literal
/// appears nowhere else; `classify_request`, `authorize` and `check_host` never name it.
#[test]
fn test_rmc_s21_forwarded_for_is_read_only_by_client_hint() {
    for (rel, content) in passkey_sources() {
        let code = strip_comments(production_part(&content));
        let lowered = code.to_ascii_lowercase();
        let literal_count = lowered.matches("x-forwarded-for").count();
        let uses = code.matches("FORWARDED_FOR_HEADER").count();
        if rel.ends_with("/lib.rs") {
            assert_eq!(literal_count, 1, "lib.rs defines the literal once");
            assert!(code.contains("pub const FORWARDED_FOR_HEADER: &str = \"x-forwarded-for\";"));
            assert_eq!(uses, 1, "lib.rs only defines the constant");
        } else if rel.ends_with("/identity.rs") {
            assert_eq!(
                literal_count, 0,
                "identity.rs uses the constant, not the literal"
            );
            let hint = fn_body(&code, "client_hint").expect("fn client_hint in identity.rs");
            let inside = hint.matches("FORWARDED_FOR_HEADER").count();
            assert!(inside >= 1, "client_hint reads FORWARDED_FOR_HEADER");
            let imports = code
                .lines()
                .filter(|l| l.trim_start().starts_with("use ") || l.trim().ends_with(','))
                .filter(|l| l.contains("FORWARDED_FOR_HEADER"))
                .count();
            assert!(
                uses <= inside + imports.max(1),
                "identity.rs names FORWARDED_FOR_HEADER only in client_hint (and its import)"
            );
            for function in ["classify_request", "authorize", "check_host"] {
                let body = fn_body(&code, function)
                    .unwrap_or_else(|| panic!("fn {function} in identity.rs"));
                assert!(
                    !body.contains("FORWARDED_FOR_HEADER") && !body.contains("client_hint"),
                    "{function} never reads X-Forwarded-For"
                );
            }
        } else {
            assert_eq!(literal_count, 0, "{rel} never names x-forwarded-for");
            assert_eq!(uses, 0, "{rel} never names FORWARDED_FOR_HEADER");
        }
    }
}

/// Auditor addition A-T4 (C8, C13): `main.rs` wires the resolved credential store path and
/// never a test hook (the RNG stays `getrandom`, the owner uid the process uid, the clock
/// `SystemTime`); the server and the auth module never block the runtime with
/// `thread::sleep`.
#[test]
fn test_rmc_s22_production_wiring_has_no_test_hooks() {
    let main = strip_comments(production_part(&read("crates/remote/src/main.rs")));
    assert!(
        main.contains("with_credentials_path("),
        "main.rs wires the resolved credential store path"
    );
    for hook in ["with_random(", "with_file_owner_uid(", "with_unix_clock("] {
        assert!(
            !main.contains(hook),
            "main.rs never calls the test hook {hook}"
        );
    }
    for rel in ["crates/remote/src/server.rs", "crates/remote/src/auth.rs"] {
        let code = strip_comments(production_part(&read(rel)));
        assert!(
            !code.contains("thread::sleep"),
            "{rel} never blocks the current-thread runtime"
        );
    }
}

/// Auditor addition A-T1, second part (C14, D-H): the passkey types without a redacted
/// `Debug` implementation have no `Debug` at all (no derive), so they can never be formatted
/// into a log line or a diagnostic.
#[test]
fn test_rmc_s23_secret_types_without_redaction_have_no_debug() {
    for (rel, types) in [
        (
            "crates/remote/src/webauthn.rs",
            &["ClientData", "NewCredential", "StoredCredential"][..],
        ),
        (
            "crates/remote/src/auth.rs",
            &["Verified", "DecodedAssertion", "DecodedRegistration"][..],
        ),
    ] {
        let code = strip_comments(production_part(&read(rel)));
        for name in types {
            let declaration = code
                .find(&format!("pub struct {name}"))
                .unwrap_or_else(|| panic!("{rel} declares {name}"));
            let before = &code[..declaration];
            let attributes = before
                .rfind(['}', ';'])
                .map_or(before, |end| &before[end + 1..]);
            assert!(
                !attributes.contains("Debug"),
                "{rel}: {name} must not derive Debug: {attributes:?}"
            );
            assert!(
                !code.contains(&format!("Debug for {name}")),
                "{rel}: {name} must not implement Debug"
            );
        }
    }
}
