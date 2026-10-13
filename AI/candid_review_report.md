# Candid Review Report

- **Date**: 2026-10-13
- **Target Branch**: `feat/android-native-app`
- **Base (merge-base)**: `6f32ef8`
- **Reviewed-Diff-Fingerprint**: `787be7203b79fd0c0c33e0f464c107a6fce01ea709e59e5b774f30905a4db4c3`
- **Audited Files**: `.github/workflows/android.yml`, `.gitignore`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_android_native_app.md`, `AI/auditor_constraints_android_native_app.md`, `AI/tester_contract_android_native_app.md`, `AI/walkthroughs/208_android_native_app.md`, `Docs/REMOTE_COMPANION.md`, `android/**` (Gradle project: `build.gradle.kts`, `settings.gradle.kts`, `gradle.properties`, `gradle/libs.versions.toml`, `gradle/verification-metadata.xml`, `gradle/wrapper/gradle-wrapper.properties`, `app/build.gradle.kts`, `AndroidManifest.xml`, 11 Kotlin sources, 10 resources, 5 JVM unit tests), `crates/remote/src/{auth,config,lib,main,routes,server,webauthn}.rs`, `crates/remote/tests/android_app_tests.rs`, `scripts/android_build.sh`, `scripts/secret_scan.sh`, `tests/invariants/src/android_native_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

First PR of #357 (GitHub #382, part 2 of #354): an opt-in `[android]` table in `soos-remote`, a
`/.well-known/assetlinks.json` route, acceptance of the `android:apk-key-hash:` origin for the
**Login** WebAuthn ceremony only, and a new native Kotlin/Compose app (settings, Funnel passkey
sign-in through Credential Manager, status, lock) with a path-filtered CI job.

The server changes are fail-closed and narrowly scoped: without `[android]` every behaviour is
byte-identical (asserted by tests), the app origin is matched byte for byte against origins
computed from parsed 32-byte digests, only for `CeremonyType::Get` with `ChallengePurpose::Login`;
unlock, camera view and registration remain web-origin only. The Android app was read (it has
never been compiled locally; the first CI run is a deliberate trust-on-first-use bootstrap that
fails by design). No CRITICAL or MAJOR finding; four MINOR/SUGGESTION items are listed.

Commands run in this worktree: `cargo test -p soos-remote --test android_app_tests` (30 passed),
`cargo test -p soos-remote` (all suites green, 0 failures), `cargo test -p soos-invariants`
(616 passed), `cargo clippy -p soos-remote --all-targets -- -D warnings` (clean). Gradle
distribution SHA-256 checked against services.gradle.org (match); the three action SHAs
resolved through the GitHub API to their tagged versions (match).

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: all **new** — `android/app/src/test/java/org/soos/remote/{ApiJson,PasskeyJson,PcUrl,SessionCookie,SoosApi}Test.kt`, `crates/remote/tests/android_app_tests.rs`, `tests/invariants/src/android_native_contract.rs`; `tests/invariants/src/lib.rs` only gains a `mod android_native_contract;` declaration.
- Removed/changed assertion lines (`^-` with assert/#[test]/...): two hits, both in `crates/remote/src/auth.rs` production code (the `parse_client_data` import and call replaced by `parse_client_data_with_policy`). Not test code.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` added/removed: none.
- Conclusion: **no pre-existing test was modified, weakened or deleted.**

## 3. Deep Reasoning Audit

### Logic & Architecture

- *Origin confusion*: tried padded (`...=`), standard-alphabet base64, upper-cased scheme,
  trailing `/`, truncated, colon-hex form, another digest → all `OriginMismatch`
  (`webauthn.rs` `classify_origin` compares bytes of origins built by `b64url_encode`, which is
  `Base64UrlUnpadded`; tests `test_ana_near_android_origins_are_refused`,
  `test_ana_arbitrary_origins_never_pass_as_android`). PASS.
- *Ceremony scope*: policy is `WebOrAndroidApp` only when `purpose == ChallengePurpose::Login`
  (`auth.rs:368`), and `classify_origin` additionally requires `CeremonyType::Get`; registration
  still calls `parse_client_data` (WebOnly). `verify_assertion` does not re-parse the client
  data (only hashes it), so no second, divergent origin check. Unlock/camera/registration with
  the app origin refused end to end (`test_ana_unlock_and_registration_stay_web_origin_only`,
  `test_ana_check_assertion_accepts_the_app_origin_for_login_only`). PASS.
- *Not configured*: `RelyingParty.android = None` → app origin refused even with the permissive
  policy; `asset_links = None` unless both `rp_id` and `[android]` exist. PASS.
- *`androidPackageName`*: absent accepted, string equal to the package accepted, any other value
  or type refused. A browser client data carrying the member is unaffected because the web
  origin returns before the member is examined. PASS.
- *Config*: `[android]` requires `rp_id` and `allow_funnel = true`; package grammar (>= 2
  segments, letter-first, `[A-Za-z0-9_]`, <= 128 bytes); 1..=2 fingerprints, exactly 95 chars,
  upper-case hex pairs (3k-1 = 95 forces k = 32, `get_mut` guards over-length), duplicates
  refused; `deny_unknown_fields`. Errors never echo values; `AndroidAppConfig` `Debug` is
  redacted. Every failure maps to `EXIT_CONFIG` (78) in `main.rs`. PASS.
- *Duplicated bounded read (RMC-S10)*: `load_config` is kept first (the existing invariant
  `check_not_root` before `load_config(` still holds); the second `load_config_with_android`
  read is compared on `RemoteConfig` and a difference refuses start-up with exit 78. Both reads
  are fully validated, so a change confined to `[android]` between the reads can only yield a
  validated table. PASS (see SUGGESTION 4).
- *Asset links route*: `route()` returns `AssetLinks` for every method before method dispatch;
  off → `Response::json(404, "not_found")`, same as `Route::NotFound`, same `Touch::Keep`
  session handling, same `accepts_body` (false), same Funnel-public classification, so headers
  and body are identical (`test_ana_asset_links_absent_is_byte_identical_404` over every method
  and both path classes). On: `200 application/json` on GET/HEAD, `405` + `Allow: GET, HEAD`
  otherwise, no `Set-Cookie`, Funnel host must equal `rp_id` (421 otherwise), single
  `delegate_permission/common.get_login_creds` relation. Funnel anonymous capacity still
  applies. PASS.
- *App/server contract*: options fields (`challenge`, `rp_id`, `timeout_ms`), verify body fields
  (`id`, `client_data_json`, `authenticator_data`, `signature`, `user_handle`, matching the
  `deny_unknown_fields` `AssertionBody`), `X-Soos-Action` tokens and `Origin: https://<host>`
  (accepted by `check_action_csrf`; no `Sec-Fetch-Site` sent, which is allowed) all match the
  server. `parseLoginOptions` pins `rp_id` to the configured host. PASS.

### PAM Concurrency & Deadlines

Not touched (`crates/pam` absent from the diff). The new route is synchronous and allocation is
a clone of a prebuilt <= ~450-byte body. PASS.

### Panic Safety & Fail-Closed

No `unwrap`/`expect`/`panic!`/indexing added in production Rust (`parse_android_fingerprint`
uses `get_mut`, `checked_*`, slice patterns). `asset_links_json` uses `unwrap_or_default` on an
infallible `serde_json::Value` serialization (SUGGESTION 3). No path turns an error into a
session: every parse failure is `AssertionFailure::Rejected`. PASS.

### Test Integrity & Anti-Weakening

See section 2. New tests can fail against plausible wrong implementations (prefix match, case
folding, padding tolerance, policy applied to unlock, 404 differing from the unknown path,
cookie on the asset links route). PASS.

### Memory, Bounds & Secrets

Server: fingerprint count and package length bounded, statement bounded by construction.
Android: response body capped at 16 KiB, Credential Manager response at 16384 chars, credential
id at 1368 chars; 5 s connect/read timeouts; HTTPS-only (`HttpsURLConnection` cast, network
security config without cleartext and system CAs only, `usesCleartextTraffic=false`);
`instanceFollowRedirects = false`; no cache, no cookie handler; session cookie
(`__Host-soos_session`, 43-char base64url only) held in memory in `SoosApi`, cleared on
sign-out, server clear and `login_required`; only the PC origin and the last successful
credential id are persisted, in private `SharedPreferences` excluded from backup and device
transfer. No `Log.`, `println`, WebView, analytics, Firebase or Play services. Manifest: only
`INTERNET`, one exported launcher activity; the CI permission check allows only `INTERNET` plus
the app's own androidx receiver permission. `.gitignore` and `secret_scan.sh` now cover
`*.keystore`, `keystore.properties`, `local.properties`, `google-services.json`. PASS.

### Supply Chain & Automation

- `android.yml`: `permissions: contents: read`, `persist-credentials: false`, no secrets, no
  `${{ github.event.* }}` in `run:`, actions pinned by full SHA (verified against v7.0.1 /
  v6.0.1 / v7.0.1 tags), JDK and SDK packages pinned, path-filtered but blocking through
  `scripts/pr_checks_gate.sh` (every non-advisory failing check blocks). PASS.
- Gradle: no wrapper jar/gradlew committed; distribution URL regex-pinned and SHA-256 verified
  before unpacking (value checked against services.gradle.org); curl HTTPS-only including
  redirects; exact version pins only; repository content filters; `FAIL_ON_PROJECT_REPOS`;
  no build or configuration cache. PASS.
- *Bootstrap (TOFU)*: `verification-metadata.xml` starts empty; `ci` mode writes it, uploads it
  and **exits 1**, so an unverified build can never pass or merge; after the owner commits the
  metadata, every run is `--dependency-verification strict`. Documented in the ADR (assumption
  A-4), the script header, the workflow header and the metadata file. Acceptable for a
  first-PR foundation given no local JDK/SDK; the residual risk is that the first resolution
  (from Google Maven / Maven Central over HTTPS, on a GitHub runner) is trusted. Note that
  committing the metadata changes the diff, so a fresh candid review is required before merge
  (stated in the ADR). `verify-signatures` is off (SUGGESTION 5). PASS.

### English-Only Policy

Added lines scanned for French words and accented characters: none. Code, comments, docs,
UI strings and script output are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt:257` — a server refusal
  (`passkey_rejected`, e.g. after `soos-remote passkeys` revoked that credential while it still
  exists in the phone's provider) keeps the remembered credential id, so every later attempt
  goes straight to the same rejected passkey (and accrues rate-limited failures); only
  `NoCredentialException` clears it. Clear `settings.credentialId` on `passkey_rejected` (or
  offer "use another passkey") in a follow-up.
- **[MINOR]** `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt:193` — `SoosApi` (and
  thus the in-memory session) is held by `remember`, so an activity recreation (rotation,
  dark-mode switch) silently drops the session and asks for a new passkey sign-in. Safe
  (fail-closed), but a `ViewModel`-scoped holder would keep the in-memory session across
  configuration changes without persisting it.
- **[SUGGESTION]** `crates/remote/src/config.rs:431` — `asset_links_json` falls back to an empty
  body on a serialization error, which would be served as `200`; it cannot fail today, but
  building the body once at parse time and refusing the configuration (or asserting
  `<= MAX_ASSET_LINKS_BYTES`) would make the bound and the failure mode explicit.
- **[SUGGESTION]** `crates/remote/src/main.rs:188` — the second read compares only
  `RemoteConfig`; the `[android]` table itself is not compared between the two reads. Harmless
  (both reads are fully validated, owner-controlled file), but a single
  `load_config_with_android` call kept textually after `check_not_root` would remove the
  window entirely.
- **[SUGGESTION]** `android/gradle/verification-metadata.xml:16` — consider enabling
  `verify-signatures` (with a reviewed `trusted-keys` list) in a later PR to reduce reliance on
  the trust-on-first-use SHA-256 snapshot.

## 5. Final Verdict

No CRITICAL or MAJOR finding after attacking origin matching, ceremony scoping, the off-state
404 identity, Funnel exposure, configuration fail-closed paths, the Android network/storage
surface and the CI supply chain. The Android project is uncompiled locally; its first CI run is
an intentional, documented, blocking bootstrap, and a fresh candid review is required once the
generated metadata is committed.

**VERDICT: APPROVED**
