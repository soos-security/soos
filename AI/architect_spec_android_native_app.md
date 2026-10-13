# Architect Spec — GitHub #382: native Android app foundation (PR 1 of #357, part 2 of #354)

- **Branch**: `feat/android-native-app` (GitHub-only issue: not registered in `scripts/sync_issue.py`)
- **Epic**: #357 (sideloaded Android app), owner decision of 2026-10-12 overriding the epic's
  Trusted Web Activity design: the app is **100% native Kotlin + Jetpack Compose**, the web app
  stays and each user chooses. Also part 2 of #354 (direct biometric prompt on Android).
- **Matrix prefix**: `ANA` (Android Native App). **Walkthrough**: 208.

## 0. Owner decisions applied and assumptions

| # | Decision | Source |
|---|---|---|
| OD-1 | Native Kotlin + Jetpack Compose app; the web app stays; users choose | #357 comment 2026-10-12 |
| OD-2 | Sideloaded only, private; publisher identity `soos-security` | #357 comment 2026-10-12 |
| OD-3 | Login keeps the discoverable flow on the server: no credential id is ever named to an anonymous client | #354 comment, part 2 |
| OD-4 | Both device-bound and password-manager passkeys stay accepted (no `BE=0` enforcement) | #354 comment, part 1 |
| A-1 | **Assumed — owner to confirm**: application id `org.soos.remote` (the example of #357 open question 2) | spec |
| A-2 | **Assumed — owner to confirm**: `minSdk` 34 (Android 14). Credential Manager then uses the platform implementation, so the app needs **no** `credentials-play-services-auth` / Google Play services dependency (the passkey *provider*, e.g. Google Password Manager, is the user's choice, not a dependency of the app) | spec |
| A-3 | **Assumed — owner to confirm** (job name revised to `"Android (debug build)"`, revision 1 m5): the Android CI job is a separate, path-filtered workflow (`.github/workflows/android.yml`), not part of `CI Success`. It is not on the advisory list of `scripts/pr_checks_gate.sh` (MAC5 pins exactly one advisory check), so `scripts/pr_loop.sh` still blocks a pull request **that changes `android/`** while that job fails; pull requests that do not touch `android/` never run it | spec |
| A-4 | **Assumed — owner to confirm**: Gradle dependency verification is bootstrapped trust-on-first-use. The Android SDK and a JDK are not installed on the development host (agents never install them), so the first `verification-metadata.xml` cannot be generated locally: the committed file enables verification with an empty component list, the CI job detects the empty list, runs once in bootstrap mode (`--write-verification-metadata sha256`), uploads the generated file as an artifact and **fails** (revision 1, M1); the owner reviews and commits it on the branch; from then on every run is strict | spec |
| A-5 | **Assumed — owner to confirm** (revised after plan evaluation m1): the Digital Asset Links statement carries exactly one relation, `delegate_permission/common.get_login_creds` (what Credential Manager needs). If the owner hardware check ANA14 shows that a Credential Manager provider also needs `handle_all_urls`, that is a separate, owner-approved change | spec |

## 1. Scope & Blast Radius

### 1.1 Rust (`soos-remote`, opt-in, fail closed)

| File | Change |
|---|---|
| `crates/remote/src/lib.rs` | new constants (§3) |
| `crates/remote/src/config.rs` | `[android]` table: `AndroidAppConfig`, `LoadedConfig`, `parse_config_with_android`, `load_config_with_android`, five `ConfigError` variants; `parse_config` / `load_config` keep their signatures and also validate the table (they drop the value) |
| `crates/remote/src/webauthn.rs` | `RelyingParty::with_android_app`, `OriginPolicy`, `ClientOrigin`, `ClientData::origin`, `parse_client_data_with_policy`; `parse_client_data` = web origin only (unchanged behaviour) |
| `crates/remote/src/auth.rs` | `check_assertion` uses `OriginPolicy::WebOrAndroidApp` for `ChallengePurpose::Login` only, `WebOnly` otherwise |
| `crates/remote/src/routes.rs` | `ASSET_LINKS_PATH`, `Route::AssetLinks` (every method), `is_funnel_public(AssetLinks) == true` |
| `crates/remote/src/server.rs` | `ServerState::with_android_app`, `Shared::asset_links`, RP built with the Android origins, `Route::AssetLinks` handler, `Touch::Keep` for it |
| `crates/remote/src/main.rs` | `load_config_with_android`, `with_android_app` when the table is present; nothing about the table is logged |

Consumers: `soos-remote` is a leaf crate (no other crate depends on it). Existing tests build
`RemoteConfig` / `AuthConfig` with struct literals (`server_tests.rs`, `alerts_server_tests.rs`,
`android_push_tests.rs`, `battery_server_tests.rs`, `common/harness.rs`): **no field is added to
either struct**, the Android settings travel in the separate `LoadedConfig` /
`ServerState::with_android_app`, so no existing test changes.

### 1.2 Android (`android/`, new, outside the Cargo workspace)

```
android/
  settings.gradle.kts            repositories (google with content filter, mavenCentral, gradlePluginPortal for plugins only), FAIL_ON_PROJECT_REPOS
  build.gradle.kts               plugin aliases (apply false)
  gradle.properties              AndroidX, non-transitive R, no build cache, no configuration cache
  gradle/libs.versions.toml      exact versions (§3.3)
  gradle/verification-metadata.xml   verify-metadata=true, sha256, components (bootstrap: empty)
  gradle/wrapper/gradle-wrapper.properties   distributionUrl + distributionSha256Sum (single pin of the Gradle distribution)
  app/build.gradle.kts           namespace/applicationId org.soos.remote, compileSdk 36, buildToolsVersion 35.0.0, minSdk 34, targetSdk 36, Java/Kotlin 17, compose, no minify, dependenciesInfo off
  app/src/main/AndroidManifest.xml
  app/src/main/res/values/{colors,strings,themes}.xml
  app/src/main/res/xml/{network_security_config,data_extraction_rules}.xml
  app/src/main/res/drawable/{ic_launcher_foreground,ic_launcher_monochrome}.xml
  app/src/main/res/mipmap-anydpi-v26/ic_launcher.xml
  app/src/main/java/org/soos/remote/...   (§2.2)
  app/src/test/java/org/soos/remote/...   JVM unit tests (§7)
```

No `gradlew`, no `gradle-wrapper.jar` (no binary committed): `scripts/android_build.sh` downloads
the pinned distribution, checks its SHA-256 against `gradle-wrapper.properties` and runs it.

### 1.3 CI, invariants, docs

- `.github/workflows/android.yml` (new, §5), `scripts/android_build.sh` (new), `.gitignore` (Android section).
- `tests/invariants/src/android_native_contract.rs` (new) + `mod` line in `tests/invariants/src/lib.rs`.
- `AI/DECISIONS.md` (ADR), `Docs/REMOTE_COMPANION.md` (§2j new, §5, §6, §8, §9), `AI/VERIFICATION_MATRIX.md` (`ANA` rows), walkthrough `AI/walkthroughs/208_android_native_app.md`.

## 2. Types & Signatures

### 2.1 Rust

```rust
// config.rs
/// `[android]`: the one native app allowed to use the passkeys of `rp_id`. Public data
/// (package name, certificate digests), but nothing of it is ever logged: `Debug` is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct AndroidAppConfig { package: String, cert_fingerprints: Vec<[u8; 32]> }
impl AndroidAppConfig {
    pub fn package(&self) -> &str;
    pub fn cert_fingerprints(&self) -> &[[u8; 32]];   // 1..=MAX_ANDROID_CERT_FINGERPRINTS, distinct, file order
    pub fn origins(&self) -> Vec<String>;              // "android:apk-key-hash:" + base64url-unpadded(digest), same order
    pub fn asset_links_json(&self) -> Vec<u8>;         // §4.2, <= MAX_ASSET_LINKS_BYTES
}
impl fmt::Debug for AndroidAppConfig { /* "AndroidAppConfig(<redacted>)" */ }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedConfig { pub remote: RemoteConfig, pub android: Option<AndroidAppConfig> }

pub fn parse_config_with_android(text: &str, runtime_dir: Option<&Path>) -> Result<LoadedConfig, ConfigError>;
pub fn load_config_with_android(path: &Path, runtime_dir: Option<&Path>) -> Result<LoadedConfig, ConfigError>;
// parse_config(..) == parse_config_with_android(..).map(|l| l.remote); load_config likewise.

// FileConfig gains `android: Option<FileAndroid>`;
#[derive(Deserialize)] #[serde(deny_unknown_fields)]
struct FileAndroid { package: Option<String>, sha256_cert_fingerprints: Option<Vec<String>> }

// New ConfigError variants (fixed texts, never a value):
AndroidRequiresRpId            // "[android] requires rp_id"
InvalidAndroidPackage          // "[android] package is not a valid application id"
AndroidFingerprintCount { max } // "[android] sha256_cert_fingerprints needs 1 to {max} entries"
InvalidAndroidFingerprint { index } // "[android] invalid sha256_cert_fingerprints entry at index {index}"
```

Validation order (after every existing check, so existing error precedence is unchanged):
`[android]` present and `rp_id` absent → `AndroidRequiresRpId`; `package` absent or invalid →
`InvalidAndroidPackage`; `sha256_cert_fingerprints` absent, empty or longer than
`MAX_ANDROID_CERT_FINGERPRINTS` → `AndroidFingerprintCount`; an entry that is not exactly 32
upper-case hex pairs separated by `:` (95 bytes, `^([0-9A-F]{2}:){31}[0-9A-F]{2}$`), or that
repeats an earlier entry → `InvalidAndroidFingerprint { index }`. Unknown keys in the table →
`Syntax` (serde). A non-table `android = 1` → `Syntax`.

Package grammar (Android application id): 1..=`MAX_ANDROID_PACKAGE_LEN` bytes, at least two
`.`-separated segments, each segment non-empty, first byte ASCII letter, other bytes ASCII
letter, digit or `_`. No lowercasing (application ids are case-sensitive).

```rust
// webauthn.rs
impl RelyingParty {
    /// Adds the Android app origins accepted by `OriginPolicy::WebOrAndroidApp`.
    #[must_use] pub fn with_android_app(self, package: &str, cert_fingerprints: &[[u8; 32]]) -> Self;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum OriginPolicy { WebOnly, WebOrAndroidApp }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum ClientOrigin { Web, AndroidApp }
pub struct ClientData { pub challenge: [u8; CHALLENGE_BYTES], pub origin: ClientOrigin }
pub fn parse_client_data_with_policy(rp: &RelyingParty, raw: &[u8], expected: CeremonyType, policy: OriginPolicy) -> Result<ClientData, WebAuthnError>;
pub fn parse_client_data(rp, raw, expected) -> Result<ClientData, WebAuthnError>  // == with_policy(.., WebOnly)
```

`parse_client_data_with_policy` keeps the existing check order; the origin step becomes:
`origin == rp.origin()` → `Web`; else if `policy == WebOrAndroidApp && expected == Get` and
`origin` equals (byte for byte) one of the configured Android origins → `AndroidApp`, and then a
present `androidPackageName` member must equal the configured package
(else `OriginMismatch`); anything else → `OriginMismatch`. `crossOrigin: true` / `topOrigin` stay
refused for both kinds. A `RelyingParty` without an Android app never yields `AndroidApp`.
A registration (`Create`) never accepts an Android origin, whatever the policy.

```rust
// auth.rs, AuthState::check_assertion: the only call site change
let policy = if purpose == ChallengePurpose::Login { OriginPolicy::WebOrAndroidApp } else { OriginPolicy::WebOnly };
let client = parse_client_data_with_policy(rp, &assertion.client_data_json, CeremonyType::Get, policy)...
```

```rust
// routes.rs
pub const ASSET_LINKS_PATH: &str = "/.well-known/assetlinks.json";
Route::AssetLinks   // `route(any method, ASSET_LINKS_PATH)`; query ignored like every route
is_funnel_public(Route::AssetLinks) == true
// allow_header(ASSET_LINKS_PATH) == "GET, HEAD" (existing default branch)

// server.rs
impl<S> ServerState<S> { #[must_use] pub fn with_android_app(self, app: AndroidAppConfig) -> Self; }
// Shared: `asset_links: Option<Vec<u8>>` = Some(app.asset_links_json()) iff rp_id is Some and an app is set;
//         `rp` = RelyingParty::new(rp_id).with_android_app(..) when an app is set.
// Route::AssetLinks handler:
//   asset_links None                 -> Response::json(404, "not_found")   (byte-identical to Route::NotFound)
//   Some, method GET or HEAD         -> 200, content_type "application/json", the prebuilt body
//   Some, any other method           -> Response::json(405, "method_not_allowed") + Allow: GET, HEAD
// Funnel touch: Route::AssetLinks => Touch::Keep (like assets); no cookie read beyond the
// existing generic session validation, never a Set-Cookie.
```

### 2.2 Android (Kotlin, package `org.soos.remote`)

| File | Content |
|---|---|
| `config/PcUrl.kt` | `object PcUrl { fun parse(input: String): PcUrlResult }`; `sealed interface PcUrlResult { data class Valid(val origin: String, val host: String); data class Invalid(val reason: PcUrlError) }`; `enum class PcUrlError { EMPTY, TOO_LONG, NOT_HTTPS, USERINFO, PORT, PATH, QUERY_OR_FRAGMENT, INVALID_HOST, NOT_TS_NET }`. Rules mirror `validate_rp_id`: trimmed input ≤ `MAX_INPUT_LENGTH` (300); scheme `https://` (case-insensitive), one optional trailing `/`, no userinfo, no port, no path, no query or fragment, host lowercased, a valid DNS name (labels 1..=63 of `[a-z0-9-]`, no leading/trailing `-`, total ≤ 253), ending with `.ts.net` with at least two labels before it. `origin = "https://" + host` |
| `net/SessionCookie.kt` | `object SessionCookie { const val NAME = "__Host-soos_session"; fun fromSetCookie(headers: List<String>): CookieUpdate }`; `sealed interface CookieUpdate { object None; object Cleared; data class Set(val value: String) }`; a `Set` needs exactly 43 base64url characters; `Cleared` when the value is empty; any other shape → `None`. `fun header(value: String): String = "$NAME=$value"` |
| `net/HttpTransport.kt` | `data class HttpRequest(method, url, headers: List<Pair<String,String>>, body: ByteArray?)`, `data class HttpResponse(status: Int, body: ByteArray, setCookies: List<String>)`, `fun interface HttpTransport { fun execute(request: HttpRequest): HttpResponse }` (throws `java.io.IOException`) |
| `net/UrlConnectionTransport.kt` | `HttpsURLConnection` only (a non-HTTPS connection is refused), `instanceFollowRedirects = false`, `useCaches = false`, connect timeout `CONNECT_TIMEOUT_MS` 5000, read timeout `READ_TIMEOUT_MS` 5000, response body read through a bound of `MAX_RESPONSE_BYTES` (16 384; a longer body is an `IOException`), no cookie handler (cookies are handled by `SoosApi`) |
| `net/SoosApi.kt` | `class SoosApi(origin: String, transport: HttpTransport)`; in-memory session cookie (never persisted); calls `authState()`, `status()`, `lock()`, `loginOptions()`, `loginVerify(bodyJson)`, `logout()`. Every `POST` carries `Origin: <origin>` and the route's exact `X-Soos-Action` (`lock`, `login-options`, `login`, `logout`); the body route sends `Content-Type: application/json`; the cookie header is sent when a session exists; a `Set-Cookie` of the session cookie updates or clears it; `403 login_required` clears it. Results are sealed types (`ApiResult.Ok`, `.Refused(status, result)`, `.Unreachable`) |
| `net/ApiJson.kt` | pure parsers with `org.json`: `parseAuthState`, `parseStatus` (`state` ∈ locked/unlocked/no_session/unavailable, else `null`), `parseResult` (`{"result": <token>}`, token `[a-z_]{1,64}`), `parseLoginOptions(body, expectedRpId)` (challenge 43 base64url chars, `rp_id` equal to the configured host, `timeout_ms` 1..=600000) |
| `auth/PasskeyJson.kt` | `fun requestJson(options: LoginOptions, rememberedCredentialId: String?): String` (`challenge`, `rpId`, `timeout`, `userVerification: "required"`, and, only when a credential id is remembered, `allowCredentials: [{type: "public-key", id, transports: ["internal", "hybrid"]}]`); `fun assertionBody(responseJson: String): AssertionBody?` maps Credential Manager's `authenticationResponseJson` (`id`, `type == "public-key"`, `response.clientDataJSON`, `authenticatorData`, `signature`, optional `userHandle`) to the server body `{"id","client_data_json","authenticator_data","signature","user_handle"}`; inputs bounded by `MAX_RESPONSE_JSON_CHARS` (16 384); every field base64url (`[A-Za-z0-9_-]`, padding stripped); `AssertionBody(json: String, credentialId: String)` |
| `auth/PasskeySignIn.kt` | Android only: `CredentialManager.create(context).getCredential(activity, GetCredentialRequest(listOf(GetPublicKeyCredentialOption(requestJson))))`; maps `GetCredentialCancellationException` → `Cancelled`, `NoCredentialException` → `NoCredential`, other `GetCredentialException` → `Failed`, a non-`PublicKeyCredential` → `Failed` |
| `data/SettingsStore.kt` | `SharedPreferences` `MODE_PRIVATE`: the validated PC URL and the remembered credential id (public data, never a secret); clearing the URL also clears the credential id |
| `MainActivity.kt`, `ui/SoosApp.kt`, `ui/Screens.kt`, `ui/theme/Theme.kt` | Compose shell: settings screen (URL field, validation message from `PcUrlError`, save), home screen (status card, **Lock** button, **Sign in with passkey** when `mode == "funnel"` and not authenticated, **Sign out**, **Refresh**, **Settings**). Colours only through `colorResource(R.color.soos_*)`: no colour literal in Kotlin |

Sign-in sequence (Funnel; on the tailnet `GET /api/auth/state` already answers
`authenticated: true` and no sign-in is offered):
`POST /api/auth/login/options` → `PasskeyJson.requestJson(options, remembered)` → Credential
Manager (biometric prompt) → `PasskeyJson.assertionBody` → `POST /api/auth/login/verify` →
`200 logged_in` + cookie → remember `credentialId`. If the remembered credential is gone
(`NoCredential`), the remembered id is cleared and the next attempt is discoverable.

## 3. Constants & Config

### 3.1 Rust (`crates/remote/src/lib.rs`)

| Constant | Value | Meaning / bound |
|---|---|---|
| `MAX_ANDROID_PACKAGE_LEN` | 128 | longest accepted `[android] package` (bytes) |
| `MAX_ANDROID_CERT_FINGERPRINTS` | 2 | `sha256_cert_fingerprints` entries (current + rotation, or release + debug) |
| `ANDROID_CERT_FINGERPRINT_LEN` | 95 | `XX:` × 31 + `XX` |
| `ANDROID_ORIGIN_PREFIX` | `"android:apk-key-hash:"` | `clientDataJSON.origin` prefix of a native Android app |
| `ASSET_LINKS_RELATION` | `"delegate_permission/common.get_login_creds"` | the only statement relation (A-5) |
| `MAX_ASSET_LINKS_BYTES` | 1024 | upper bound of the generated body (max package + two fingerprints is about 560 bytes) |

`[android]` absent ⇒ no route (`404`), no Android origin accepted: every existing behaviour is
byte-identical. There is no `0`/empty sentinel: an empty list is an error.

### 3.2 Android constants

`CONNECT_TIMEOUT_MS = 5000`, `READ_TIMEOUT_MS = 5000`, `MAX_RESPONSE_BYTES = 16384`,
`MAX_RESPONSE_JSON_CHARS = 16384`, `PcUrl.MAX_INPUT_LENGTH = 300`, `TS_NET_SUFFIX = ".ts.net"`,
action values identical to `lib.rs` (`lock`, `login-options`, `login`, `logout`; invariant).

### 3.3 Pinned build inputs (exact, checked by invariants)

| Input | Version / value |
|---|---|
| Gradle distribution | 8.14.5, `gradle-8.14.5-bin.zip`, SHA-256 `6f74b601422d6d6fc4e1f9a1ab6522f642c2fdcbc15ae33ebd30ba3d7198e854` (services.gradle.org) |
| Android Gradle Plugin | 8.13.2 |
| Kotlin (android + compose compiler plugins) | 2.2.21 |
| Compose BOM | 2025.10.01 (ui 1.9.4, material3 1.4.0) |
| androidx.activity:activity-compose | 1.11.0 |
| androidx.core:core-ktx | 1.17.0 |
| androidx.credentials:credentials | 1.5.0 (no `credentials-play-services-auth`) |
| kotlinx-coroutines-android | 1.10.2 |
| junit:junit (test) | 4.13.2 |
| org.json:json (test only: the JVM unit tests need a real `org.json`) | 20250517 |
| JDK (CI) | Temurin `17.0.16+8` through `actions/setup-java` pinned by SHA |
| SDK (CI) | `platforms;android-36`, `build-tools;35.0.0` through the runner's `sdkmanager` |
| Runner | `ubuntu-24.04` |

## 4. Error Taxonomy and wire shapes

### 4.1 HTTP

| Request | `[android]` absent | `[android]` present |
|---|---|---|
| `GET /.well-known/assetlinks.json` (tailnet or Funnel, anonymous) | `404 {"result":"not_found"}` (identical bytes to any unknown path) | `200 application/json` §4.2 |
| `HEAD` same | `404` head | `200` head, same `Content-Length`, empty body |
| `POST`/other same | `404` | `405 method_not_allowed`, `Allow: GET, HEAD` |
| wrong effective host | `421` (existing, before routing) | `421` |
| Funnel host ≠ `rp_id` | `421` (existing) | `421` |
| login verify with `clientDataJSON.origin` = configured `android:apk-key-hash:` | `403 passkey_rejected` (counted failure, unchanged) | accepted (all other WebAuthn checks unchanged) |
| unlock / camera / registration with an Android origin | `403 passkey_rejected` / registration refusal | unchanged refusal (web origin only) |

### 4.2 Asset links body (compact JSON, key order fixed)

```json
[{"relation":["delegate_permission/common.get_login_creds"],"target":{"namespace":"android_app","package_name":"<package>","sha256_cert_fingerprints":["<AA:..>"]}}]
```

## 5. CI job (`.github/workflows/android.yml`)

- Triggers: `pull_request` and `push` to `main` with `paths: [android/**, scripts/android_build.sh, .github/workflows/android.yml]`, and `workflow_dispatch`. Never `pull_request_target`.
- (Superseded in part by revision 1: job name `"Android (debug build)"`, failing bootstrap, mandatory `aapt2` check.) `permissions: contents: read`; one job `android-debug`, `runs-on: ubuntu-24.04`, `timeout-minutes: 40`; no `secrets.` reference; not a `CI Success` dependency (different workflow).
- Steps: `actions/checkout` (same SHA as `ci.yml`, `persist-credentials: false`); `actions/setup-java@de7274f081f381c8f8158605e0321c36c376e2e6 # v6.0.1` Temurin `17.0.16+8`; pinned SDK packages through `sdkmanager --install`; `./scripts/android_build.sh ci` (debug build, unit tests, lint, APK permission check); on a bootstrap run only, `actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1` of `android/gradle/verification-metadata.xml` (7-day retention).
- `scripts/android_build.sh`: `set -euo pipefail`; reads `distributionUrl` / `distributionSha256Sum` from `gradle-wrapper.properties` (URL must match `https://services.gradle.org/distributions/gradle-<x.y.z>-bin.zip`, digest 64 lowercase hex); downloads with `curl --fail --proto '=https' --tlsv1.2` into `${SOOS_ANDROID_CACHE:-$HOME/.cache/soos-android}`; `sha256sum -c` before unpacking (mismatch: file deleted, exit 1); modes `verify` (default, `--dependency-verification strict`), `bootstrap` (`--write-verification-metadata sha256`), `ci` (bootstrap when the metadata lists no `<component`, with a `::warning::`, otherwise verify; writes `bootstrap=true|false` to `$GITHUB_OUTPUT` when set); tasks `assembleDebug testDebugUnitTest lintDebug`; then, when `aapt2` of build-tools 35.0.0 is available, `aapt2 dump permissions` of `app-debug.apk` must list no `uses-permission` other than `android.permission.INTERNET` and `org.soos.remote.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION` (added by androidx.core, signature-level, own package).

## 6. Invariants Touched

- RC-1 / RMC-S2: `soos-remote` still opens no network socket (the new route is served on the existing Unix socket; no new dependency).
- Existing trust rules unchanged: host check, Funnel host = `rp_id`, CSRF of every route, CSP and hardening headers, cookie attributes, UV required, ES256 only, attestation `none`, at most 4 passkeys; an Android origin is accepted **only** for the login ceremony and **only** when the owner configured the exact signing-certificate digest.
- No logging of the package, digests, origin or any request data (RC-5 / RMC log audit).
- Android: `INTERNET` only, `allowBackup=false`, cleartext off, system CAs only, only the launcher activity exported, no WebView, no analytics, no Firebase / Google Play services dependency, no colour literal outside `colors.xml` (values from `crates/gui/src/theme.rs`), no `android.util.Log` or `println`, the session cookie never written to storage, no secret file tracked under `android/`.

## 7. Test Hooks for Tester

- Rust: `parse_config_with_android` (pure), `AndroidAppConfig::{origins, asset_links_json}` (pure), `parse_client_data_with_policy` (pure), `AuthState::check_assertion` through the existing `tests/common` authenticator fixtures (an `Authenticator` whose `clientDataJSON.origin` can be overridden), the server harness with `ServerState::with_android_app` (tailnet and Funnel requests, existing `HttpResponse::assert_mandatory_headers`).
- Android: JVM unit tests under `android/app/src/test/java/org/soos/remote/` (JUnit 4): `PcUrlTest`, `SessionCookieTest`, `ApiJsonTest`, `PasskeyJsonTest`, `SoosApiTest` (fake `HttpTransport` recording requests). They run in CI only (no SDK on the development host).
- Invariants: `tests/invariants/src/android_native_contract.rs` (static checks of the `android/` tree, the workflow, the build script, `.gitignore`, docs and ADR).

## 8. Documentation Drift / ADR

ADR "[2026-10-13] Native Android App for `soos-remote` (Kotlin and Jetpack Compose)" amends the
TWA direction of #357, ADR 2026-10-09 (Android web parity; its out-of-scope "native Android app")
and §9 of `Docs/REMOTE_COMPANION.md`: records OD-1..OD-4 and A-1..A-5, the opt-in `[android]`
binding (route + login-only origin), why the web app stays the reference client, and that
release signing, distribution, push, camera, unlock, the Keystore device key (#354 part 2) and
the offline heartbeat (#359) are later PRs.

## 9. Revision 1 (answers to the plan evaluation of 2026-10-13, REVISION_REQUIRED)

| Finding | Resolution (normative, overrides the sections above where they differ) |
|---|---|
| M1 bootstrap passes green | The CI job **never passes in bootstrap mode**: `scripts/android_build.sh ci` with a metadata file listing no `<component` runs `--write-verification-metadata sha256`, uploads the generated file as an artifact, prints a `::error::` and **exits 1**. Pull-request and push runs are otherwise strict only. Because the job is path-filtered and `scripts/pr_checks_gate.sh` blocks on any failing non-advisory check, PR 1 cannot be merged before the owner downloads the artifact of the PR's own run, reviews it and commits it on the branch (trust on first use; the ADR records the procedure and the owner records the run id in the PR); a later PR that empties the file turns the job red again. Invariant `test_ana_verification_metadata_covers_the_catalog`: when the file lists components, every coordinate of `libs.versions.toml` (libraries **and** plugin markers `<id>:<id>.gradle.plugin`) has a component with a `sha256` entry; when it lists none, the build script is still in its failing bootstrap shape (checked statically). `workflow_dispatch` is kept only for re-runs on `main`. |
| M2 §9 needle | `Docs/REMOTE_COMPANION.md` §9 keeps the exact phrase "native Android app" (pinned by `remote_android_contract::test_ran_s9_android_is_documented`); no existing invariant or test is modified. Doc needles of the new invariant are listed in §10. |
| M3 signing key is a login trust anchor | Accepted risk recorded in the ADR and in §2j: whoever holds a private key listed in `[android]` and installs a matching APK on a phone where the owner then passes the passkey prompt obtains a Funnel web session (status, events, lock, alerts, push, battery, logout); unlock, camera and registration stay web-origin only. The docs require a **dedicated** signing key (never the default `~/.android/debug.keystore`, password `android`, never a CI key), show how to read its fingerprint (`keytool -list -v` or `apksigner verify --print-certs`, converted to `AA:BB:..`), how to rotate (two entries, then remove the old one) and how to switch the binding off (remove the table). Owner hardware row ANA14 covers the end-to-end sign-in. |
| M4 merge approval | Unchanged rule of this run: agents never commit, push or merge; the final report lists A-1..A-5 for the owner to confirm before any merge. |
| m1 | Single relation (A-5 revised). |
| m2 | New `ConfigError::AndroidRequiresFunnel` ("[android] requires allow_funnel"), checked right after `AndroidRequiresRpId`. |
| m3 | `curl --fail --location --proto '=https' --proto-redir '=https' --max-redirs 5 --tlsv1.2 --connect-timeout 20 --max-time 600 --retry 3`. |
| m4 | In `ci` mode the `aapt2` permission check is mandatory (missing `aapt2` or APK → exit 1). |
| m5 | Job name `"Android (debug build)"` (no "non-required" wording); the ADR and docs state that it is not a `CI Success` dependency but blocks `pr_loop.sh` for a pull request that touches its paths; it is never added to `ADVISORY_CHECKS`; workflow-level `concurrency` group `android-${{ github.ref }}` with cancel-in-progress for pull requests. |
| m6 | Kotlin parsers use explicit type checks (`get(..)` + `is String` / `is Int` / `is Boolean`), never `opt*` coercion; a duplicate key is irrelevant because the server never sends one and every value is re-validated. |
| m7 | Test list in §10. |
| m8 | §2j says the `[android]` table goes at the **end** of `remote.toml`. |
| m9 | `scripts/secret_scan.sh` also rejects `*.keystore`, `keystore.properties`, `local.properties` and `google-services.json`; `.gitignore` covers the same names plus Android build outputs. |
| m10 | The ADR lists the Gradle dependency licences (Apache-2.0 for AGP, Kotlin, AndroidX, kotlinx-coroutines; EPL-1.0 for JUnit 4 and the JSON License for `org.json`, both **test-only**, never shipped in the APK); all compatible with distributing the AGPL-3.0-or-later app. |
| m11 | Invariant also forbids `printStackTrace` and `System.err`/`System.out`; docs warn that the debug APK is debuggable (`adb` can inspect it on a phone with USB debugging). |
| m12 | Docs name Google Password Manager as the provider to validate first (Proton Pass and other providers are best effort); ANA14 records the provider. |
| m13 | The ADR records that `actions/setup-java` and `sdkmanager` download the JDK and SDK packages over HTTPS without our own checksums (vendor-verified); the job has an explicit `sdkmanager --licenses` step. |

## 10. Test plan (revision 1)

Rust (`crates/remote/tests/android_app_tests.rs`): constants; config without table unchanged;
valid table (two digests, origins exact, unpadded base64url); `AndroidRequiresRpId`;
`AndroidRequiresFunnel`; valid and invalid packages (each rule); fingerprint count (absent,
empty, three); every invalid fingerprint shape and a duplicate (index reported); unknown key /
non-table / wrong type → `Syntax`; existing errors keep precedence; `Debug` redacted and error
texts value-free; `load_config_with_android` and `load_config` agree; exact asset-links body and
size bound; origin policy (app origin only with `WebOrAndroidApp` + `Get`; web-only
`parse_client_data`; never for `Create`; never without an app; padded, standard-base64,
colon-form, upper-case prefix, trailing slash, truncated, other digest refused;
`androidPackageName` absent accepted, mismatch or non-string refused; `crossOrigin` /
`topOrigin` refused); proptest over arbitrary origins; route table (every method, query,
near paths, Funnel-public, `Allow`); server: byte-identical `404` without the table (GET, HEAD,
POST, tailnet and Funnel), `200` GET/HEAD with exact body and mandatory headers and no
`Set-Cookie` (tailnet, anonymous Funnel, Funnel with a bogus cookie), `405` with `Allow`, `421`
for a wrong host and for a Funnel host other than `rp_id`, no route without `rp_id`; login with
the app origin accepted and usable for status; refused without binding, for another digest and
for another package (`403 passkey_rejected`, counted); unlock and registration with the app
origin refused (`passkey_rejected`, logind never called, nothing stored) while web-origin unlock
works.

Kotlin (CI only): `PcUrlTest`, `SessionCookieTest`, `ApiJsonTest`, `PasskeyJsonTest`,
`SoosApiTest`.

Invariants (`tests/invariants/src/android_native_contract.rs`): no tracked secret file under
`android/`, `.gitignore` and `secret_scan.sh` cover the Android secret names; exact version pins;
Gradle distribution pinned by SHA-256 and equal in the script's expectations; verification
metadata enabled and covering the catalog when populated; no analytics / Firebase / Play
services; manifest hardening (INTERNET only, no backup, no cleartext, network security config,
one exported component, no service/receiver/provider, no `autoVerify`); system CAs only;
`colors.xml` values equal `crates/gui/src/theme.rs`; icon path data equal to `icon.svg`; Kotlin
hygiene (no `Log`, `println`, `printStackTrace`, `System.err`/`System.out`, `WebView`, colour
literal; action values equal `lib.rs`; no redirect following; cookie never persisted); CI job
pinned, secret-free, strict-or-failing bootstrap, not in `CI Success`, not advisory; build
script checks; `soos-remote` logs nothing about the binding and accepts the app origin for
`ChallengePurpose::Login` only; docs, ADR and walkthrough 208.

Matrix rows ANA1–ANA16 (ANA13–ANA16 owner hardware/CI rows: first green Android CI run after
the metadata commit; sign-in with the app origin end to end with the provider recorded;
adaptive icon, splash and system bars; status and lock from the app over tailnet and Funnel).

## 11. Revision 2 (round-2 evaluation APPROVED; minor findings n1–n8 folded in)

- **n1**: after committing the generated metadata, the owner re-runs the candid review for the
  new head (the pre-push hook requires a fresh APPROVED report bound to the diff); never
  `--no-verify`. Written in the ADR and §2j owner steps.
- **n2**: the build step has `id: build`; the script writes `bootstrap=true` to
  `$GITHUB_OUTPUT` before `exit 1`; the upload step runs with
  `if: failure() && steps.build.outputs.bootstrap == 'true'` (pinned by an invariant).
- **n3**: recovery for an incomplete metadata file: empty `<components>` again (next run is a
  bootstrap) or run `scripts/android_build.sh bootstrap` locally; the file is generated on
  Linux x86_64 (platform `aapt2` entries of other systems may be missing). Documented.
- **n4**: the metadata invariant (when populated) matches BOM-managed libraries by
  `group:name`, requires the BOM and the three plugin markers, and forbids `<trusted-artifacts>`,
  `<trust`, `<ignored-keys>` in the metadata and `org.gradle.dependency.verification` in
  `gradle.properties`.
- **n5**: §9 of `Docs/REMOTE_COMPANION.md` keeps every existing pin: `native Android app`,
  `notify.windows.com`, `Tasker`; one line holding both `live camera` and `recording`;
  `section 2i`; no line starting with `- **push notification`.
- **n6**: added tests: the app origin is refused for the `CameraView` purpose (unit test through
  `AuthState::check_assertion` is not public; covered by a server test with `camera_view` on, or,
  if the camera harness is too heavy, by the purpose-gating invariant plus the unlock test —
  the invariant pins `purpose == ChallengePurpose::Login` as the **only** condition that selects
  `WebOrAndroidApp`); a Funnel session keeps its idle timer on the asset-links route
  (`Touch::Keep`, pinned statically next to the other `Touch::Keep` routes).
- **n7**: licences: AGP, Kotlin, AndroidX, kotlinx-coroutines Apache-2.0 (shipped); test only:
  JUnit 4.13.2 EPL-1.0, `hamcrest-core` 1.3 BSD-3-Clause, `org.json` 20250517 Public Domain
  (POM checked 2026-10-13).
- **n8**: §0 A-3/A-4, §1.1 and §5 aligned with revision 1; the metadata file comment says the
  bootstrap run fails.
