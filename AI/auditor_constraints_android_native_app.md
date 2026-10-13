# Audit Constraints — GitHub #382: native Android app foundation (PR 1 of #357)

Inputs: architect spec (revisions 1–2), plan evaluation round 2 (APPROVED), tester contract
`AI/tester_contract_android_native_app.md`, the diff of `crates/remote/src/`, `android/`,
`scripts/android_build.sh`, `scripts/secret_scan.sh`, `.github/workflows/android.yml`.

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C1 | No `unwrap`/`expect`/`panic!`/indexing/unchecked arithmetic in production code; the digest parser uses `get_mut`, slice patterns and `checked_*` | `config.rs::{valid_android_package, parse_android_fingerprint, upper_hex, android_config}`, `webauthn.rs::classify_origin`, `server.rs::asset_links_response` | workspace clippy lints (`-D warnings`), grep of the diff (no hit) |
| C2 | `#![forbid(unsafe_code)]` unchanged in `soos-remote`; no new crate dependency (`serde_json`, `base64ct` already present) | `crates/remote` | `test_business_crates_forbid_unsafe_code`, `Cargo.lock` unchanged, `cargo deny` not needed |
| C3 | Fail closed: an invalid `[android]` table refuses the start (`EXIT_CONFIG`); without the table nothing changes (byte-identical `404`, web origin only) | `config.rs::parse_config_with_android`, `server.rs` | ANA1, ANA3 tests |
| C4 | Bounded inputs: package ≤ 128 bytes, ≤ 2 digests of exactly 95 bytes, generated body ≤ `MAX_ASSET_LINKS_BYTES`; `clientDataJSON` bound unchanged; the new `androidPackageName` member is read only for an app origin | `config.rs`, `webauthn.rs` | ANA1, ANA2, ANA3 tests |
| C5 | Origin widening limited to `ChallengePurpose::Login` and `CeremonyType::Get`; exact byte comparison against configured origins; `crossOrigin`/`topOrigin` still refused; registration, unlock and camera stay web-only | `auth.rs::check_assertion`, `webauthn.rs::classify_origin` | `test_ana_check_assertion_accepts_the_app_origin_for_login_only`, `test_ana_unlock_and_registration_stay_web_origin_only`, `test_ana_s11_remote_binding_is_login_only_and_silent` |
| C6 | No logging of the package, digests, origins or body; `AndroidAppConfig` `Debug` redacted; error texts value-free | `config.rs`, `main.rs`, `server.rs` | `test_ana_config_values_never_reach_debug_or_errors`, `test_ana_s11_...` |
| C7 | RC-1 / RMC-S2: no network socket; the route is served on the existing Unix socket, mandatory and hardening headers unchanged, no `Set-Cookie`, `Touch::Keep` | `server.rs` | ANA3 tests (`assert_mandatory_headers`), invariant S11 |
| C8 | Android: INTERNET only, no backup / device transfer, cleartext off, system CAs only, one exported component, no WebView, no analytics/Firebase/Play services, no logging/printing, HTTPS-only transport without redirects with 5 s timeouts and a 16 KiB body bound, cookie in memory only | `android/app/**` | invariants S1, S5, S6, S8; CI `aapt2` permission check |
| C9 | Supply chain: exact catalog pins, Gradle distribution SHA-256 checked before unpacking, `curl` HTTPS-only redirects, strict dependency verification; a bootstrap run never passes | `scripts/android_build.sh`, `android/gradle/**` | invariants S2–S4, S10 |
| C10 | CI: actions pinned by SHA, `contents: read`, no `secrets.`, no `pull_request_target`, no persisted credentials, untrusted event data never interpolated into `run:` (none used) | `.github/workflows/android.yml` | invariant S9 |
| C11 | Secrets never committed: keystores, keystore/local properties, Firebase files rejected by `.gitignore`, `secret_scan.sh` and the invariant | `.gitignore`, `scripts/secret_scan.sh` | invariant S1 |
| C12 | No existing test weakened; no contract amendment | all | `git diff --stat` of `crates/remote/tests` and `tests/invariants` (additions only) |

### Pre-existing violations found (not introduced by this change)

None relevant. Residual (documented, accepted in the ADR): the signing key of a configured app is
a login trust anchor (spec revision 1, M3); `setup-java` and `sdkmanager` downloads are verified
by their vendors only (m13); the debug APK is debuggable.

### Clearance: CLEARED
