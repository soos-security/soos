# Candid Review Report

- **Date**: 2026-10-15
- **Target Branch**: `feat/android-native-app` (PR #391, rebased onto `origin/main`)
- **Base (merge-base)**: `85a7f42`
- **Reviewed-Diff-Fingerprint**: `8ff37e77b38e9c987ceab52c9f1e62a87a427467204947dab48cf9aafa26e902`
- **Audited Files**: `.github/workflows/android.yml`, `.gitignore`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_android_native_app.md`, `AI/auditor_constraints_android_native_app.md`, `AI/tester_contract_android_native_app.md`, `AI/walkthroughs/208_android_native_app.md`, `Docs/REMOTE_COMPANION.md`, `android/app/build.gradle.kts`, `android/app/src/main/AndroidManifest.xml`, `android/app/src/main/java/org/soos/remote/MainActivity.kt`, `android/app/src/main/java/org/soos/remote/auth/PasskeyJson.kt`, `android/app/src/main/java/org/soos/remote/auth/PasskeySignIn.kt`, `android/app/src/main/java/org/soos/remote/config/PcUrl.kt`, `android/app/src/main/java/org/soos/remote/data/SettingsStore.kt`, `android/app/src/main/java/org/soos/remote/net/ApiJson.kt`, `android/app/src/main/java/org/soos/remote/net/HttpTransport.kt`, `android/app/src/main/java/org/soos/remote/net/SessionCookie.kt`, `android/app/src/main/java/org/soos/remote/net/SoosApi.kt`, `android/app/src/main/java/org/soos/remote/net/UrlConnectionTransport.kt`, `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt`, `android/app/src/main/java/org/soos/remote/ui/theme/Theme.kt`, `android/app/src/main/res/drawable/ic_launcher_foreground.xml`, `android/app/src/main/res/drawable/ic_launcher_monochrome.xml`, `android/app/src/main/res/mipmap-anydpi-v26/ic_launcher.xml`, `android/app/src/main/res/values-night/themes.xml`, `android/app/src/main/res/values/colors.xml`, `android/app/src/main/res/values/strings.xml`, `android/app/src/main/res/values/themes.xml`, `android/app/src/main/res/xml/data_extraction_rules.xml`, `android/app/src/main/res/xml/network_security_config.xml`, `android/app/src/test/java/org/soos/remote/ApiJsonTest.kt`, `android/app/src/test/java/org/soos/remote/PasskeyJsonTest.kt`, `android/app/src/test/java/org/soos/remote/PcUrlTest.kt`, `android/app/src/test/java/org/soos/remote/SessionCookieTest.kt`, `android/app/src/test/java/org/soos/remote/SoosApiTest.kt`, `android/build.gradle.kts`, `android/gradle.properties`, `android/gradle/libs.versions.toml`, `android/gradle/verification-metadata.xml`, `android/gradle/wrapper/gradle-wrapper.properties`, `android/settings.gradle.kts`, `crates/remote/src/auth.rs`, `crates/remote/src/config.rs`, `crates/remote/src/lib.rs`, `crates/remote/src/main.rs`, `crates/remote/src/routes.rs`, `crates/remote/src/server.rs`, `crates/remote/src/webauthn.rs`, `crates/remote/tests/android_app_tests.rs`, `scripts/android_build.sh`, `scripts/secret_scan.sh`, `tests/invariants/src/android_native_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

Context-free review of the raw diff (merge-base `85a7f42` to the working tree) after the rebase onto main,
with focus on the reconciliation of the `[android]` binding (`AndroidAppConfig`, `/.well-known/assetlinks.json`,
`ServerState.android`) with main's offline-alerts feature (`OfflineConfig`, `/api/offline`, `ServerState.offline`).
The merge is clean: both features are independently opt-in, validation ordering is camera, then offline, then
`[android]`, every new error is a fixed value-free `ConfigError` mapped to exit 78, the route table carries both
`Route::Offline` and `Route::AssetLinks`, only `AssetLinks` is Funnel-public, and an absent binding yields the exact
unknown-path `404`. No CRITICAL or MAJOR finding; one MINOR robustness note on the CI permission check.

## 2. Test Changes

Mechanical listing (step 3 of the procedure) on `target/candid_diff.patch`:

- Test files touched (all **new** relative to the merge-base, nothing pre-existing modified):
  `android/app/src/test/java/org/soos/remote/{ApiJsonTest,PasskeyJsonTest,PcUrlTest,SessionCookieTest,SoosApiTest}.kt`,
  `crates/remote/tests/android_app_tests.rs`, `tests/invariants/src/android_native_contract.rs`,
  `tests/invariants/src/lib.rs` (module registration only).
- Removed/changed assertions: none. The two `^-` hits (patch lines 6415, 6426) are production imports/calls in
  `crates/remote/src/auth.rs` (`parse_client_data` replaced by `parse_client_data_with_policy`), not checks.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, `cfg(any())`): none.
- Inline test modules changed: none.
- Rebase migration: `crates/remote/tests/android_app_tests.rs:682` adds `offline: soos_remote::config::OfflineConfig::default()`
  to a `RemoteConfig` struct literal, the same setup-only migration main applied to its other harnesses after adding the
  field. No assertion touched; justified (a new required field, the default is `offline_alerts` off, which preserves the
  test's original premise).

## 3. Deep Reasoning Audit

### Logic & Architecture
- `parse_config_with_android`: camera keys, then `OfflineRequiresPush`, then `android_config` — an `[android]` error never
  masks an earlier error (existing precedence kept, ANA1). `parse_config`/`load_config` delegate to the `_with_android`
  variants, so a bad table is refused on every load path. PASS.
- `android_config`: requires `rp_id` then `allow_funnel`; package rule (≥ 2 segments, letter-first, `[A-Za-z0-9_]`, ≤ 128 bytes);
  1–2 digests; each digest exactly 95 bytes, 32 upper-case hex pairs; duplicates refused. Tried `"AA:BB"` short, lower-case,
  `"AAA:..."` (length 95 but a 3-byte pair → `let [high, low]` fails), 33 pairs (length mismatch), `..` empty segment,
  `1abc.def`. All refused. PASS.
- Independence: no code path reads `offline` from the android code or vice versa; `[android]` without `offline_alerts` and the
  reverse both validate. PASS.
- Routing: `route()` returns `AssetLinks` for every method on the exact path (query stripped); `Offline` unchanged (GET/HEAD,
  else 405). `is_funnel_public` adds `AssetLinks` only; `Offline` stays session-gated on Funnel. PASS.
- `asset_links_response`: absent binding → `Response::json(404, "not_found")`, the same constructor as `Route::NotFound`, for
  every method; no body read since `accepts_body` is false for the path (same as an unknown path). Present → 200 on GET/HEAD,
  405 with `Allow: GET, HEAD` otherwise. Body size: 128-byte package + two 95-byte digests ≈ 470 bytes < `MAX_ASSET_LINKS_BYTES`. PASS.
- Origin policy: `AuthState` widens to `WebOrAndroidApp` only for `ChallengePurpose::Login`; `classify_origin` additionally
  requires `CeremonyType::Get` and a configured `RelyingParty.android`; non-string or mismatching `androidPackageName` refused;
  `crossOrigin`/`topOrigin` checks still run after the origin step. Registration and unlock stay web-only. Matches ADR
  2026-10-13 item (4). PASS.
- `main.rs` double bounded read with `loaded.remote == config` equality: a file swapped between the two reads refuses the
  start (exit 78) rather than mixing configurations. PASS.
- Docs: two `## 2j.` headings in `Docs/REMOTE_COMPANION.md` (lines 985 and 1063) are imposed by the literal headings the two
  immutable invariant suites look up; cosmetic only, accepted.

### PAM Concurrency & Deadlines
- No file under `crates/pam` changes. N/A — PASS.

### Panic Safety & Fail-Closed
- No `unwrap`/`expect`/indexing in the new Rust: `serde_json::to_vec(..).unwrap_or_default()` (infallible for a `Value`,
  and an empty body would only break the opt-in Asset Links, never widen access); `out.get_mut(index)?` bounds the digest
  write; checked arithmetic in `upper_hex`. Every new config error returns `EXIT_CONFIG` (78). No path turns an error
  into an accepted origin: every branch of `classify_origin` that is not an exact match returns `OriginMismatch`. PASS.

### Test Integrity & Anti-Weakening
- See §2: no pre-existing test or assertion modified; the single rebase migration is setup-only. ANA1–ANA4 tests exercise
  absent/present binding, near-form origins, wrong package, wrong digest, unlock/registration refusal; a plausible wrong
  implementation (e.g. widening for every purpose, or a non-identical 404) would fail them. PASS.

### Memory, Bounds & Secrets
- `AndroidAppConfig` `Debug` is `AndroidAppConfig(<redacted>)`; `LoadedConfig` derives `Debug` but only through that impl.
  `ConfigError` messages carry only an index or the constant max, never a value. No `tracing` call names the package, the
  digests or the origins; `debug!(?resolved)` prints the route variant only. Allocation bounded (≤ 2 digests, ≤ 128-byte package,
  config ≤ `MAX_CONFIG_BYTES`). Kotlin transport: HTTPS-only cast, no redirects, 5 s timeouts, 16 KiB bounded read, cookie
  in memory only. PASS.

### Supply Chain & Automation
- `.github/workflows/android.yml`: actions pinned by full SHA, `permissions: contents: read`, `persist-credentials: false`,
  no secrets, no `${{ github.event.* }}` in `run:`. `scripts/android_build.sh`: Gradle URL regex-restricted to
  services.gradle.org, SHA-256 checked before unpack, HTTPS-only curl with redirect protocol pinned, strict dependency
  verification, bootstrap run always fails. `secret_scan.sh` gains `*.keystore`, `keystore.properties`, `local.properties`,
  `google-services.json`. PASS with one MINOR note (§4).

### English-Only Policy
- No non-ASCII French characters in added lines; all code, comments, docs and messages are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `scripts/android_build.sh:120` — `check_permissions` extracts only lines starting with
  `uses-permission: name=`; `aapt2 dump permissions` prints `uses-permission-sdk-23: name='...'` for
  `<uses-permission-sdk-23>` entries, which would bypass the allowlist. Low risk today (the app manifest declares
  `INTERNET` only and the invariant suite checks the manifest), but a merged library could add one unnoticed. Suggested
  correction: match `^uses-permission(-sdk-23)?: name='...'`. Non-blocking.

## 5. Final Verdict

**VERDICT: APPROVED**
