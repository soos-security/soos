# Walkthrough 208 — Native Android App Foundation (Kotlin and Jetpack Compose)

- **Date**: 2026-10-13
- **Issue**: GitHub #382 = PR 1 of epic #357, also part 2 of #354 (GitHub-only; no `AI/BACKLOG.md`
  entry, not registered in `scripts/sync_issue.py`; commits say `Closes #382`, `Refs #357`, `Refs #354`).
  **Branch**: `feat/android-native-app`. Nothing is committed, pushed, installed, enabled or restarted
  by the agents. Merge only after explicit owner approval.
- **ADR**: "[2026-10-13] Native Android App for `soos-remote` (Kotlin and Jetpack Compose)"
  (`AI/DECISIONS.md`). It amends the Trusted Web Activity design of #357, ADR 2026-10-09 (Android web
  parity) and §9 of `Docs/REMOTE_COMPANION.md`.
- **Documents**: spec `AI/architect_spec_android_native_app.md` (revisions 1 and 2); plan evaluation
  `AI/plan_evaluator_report.md` (round 1 REVISION_REQUIRED, round 2 APPROVED); tester contract
  `AI/tester_contract_android_native_app.md`; auditor constraints
  `AI/auditor_constraints_android_native_app.md` (C1–C12, CLEARED).
- **Matrix criteria**: ANA1–ANA16 (component `android-native-app`; ANA13–ANA16 pending: first CI runs
  and owner hardware checks).

## 1. Context & Objectives

Epic #357 proposed a Trusted Web Activity shell. On 2026-10-12 the owner decided that the Android app
is 100% native Kotlin + Jetpack Compose, redesigned in the soos brand, and that the web app stays so
that each user chooses. Issue #354 asks Android to go straight to the device biometric prompt. This
first PR lays the foundation:

- a Gradle project under `android/` with minimal, pinned and verified dependencies;
- a settings screen for the PC address;
- Funnel sign-in through Credential Manager passkeys;
- a read-only status screen and the lock action;
- the opt-in server binding that a native passkey client needs.

## 2. Server side (`soos-remote`)

- **`[android]` table** (`crates/remote/src/config.rs`). `package` and 1–2
  `sha256_cert_fingerprints`. It requires `rp_id` and `allow_funnel` and is checked after every
  existing key. Five new fixed-text `ConfigError` variants. `AndroidAppConfig` has a redacted `Debug`.
  `parse_config` and `load_config` keep their signatures and validate the table too.
  `parse_config_with_android` and `load_config_with_android` return the binding. No existing struct
  gained a field, so no existing test changed.
- **Origin policy** (`crates/remote/src/webauthn.rs`). `RelyingParty::with_android_app` precomputes
  `android:apk-key-hash:<unpadded base64url digest>` origins. `parse_client_data_with_policy` with
  `OriginPolicy::WebOrAndroidApp` accepts one of them for `webauthn.get` only. When
  `androidPackageName` is present it must equal the package. `crossOrigin` and `topOrigin` are still
  refused. `parse_client_data` stays web-only. A browser's client data is read as before, because the
  new member accepts any JSON type and is checked only for an app origin.
- **Login only** (`crates/remote/src/auth.rs`). `AuthState::check_assertion`, the single call site of
  every assertion, selects `WebOrAndroidApp` only for `ChallengePurpose::Login`. Unlock, camera view
  and registration stay web-origin only.
- **Route** (`routes.rs`, `server.rs`). `/.well-known/assetlinks.json` maps to `Route::AssetLinks`
  for every method and is Funnel-public, with `Touch::Keep`.
  - Without a binding, the answer is the exact unknown-path `404`.
  - With a binding: the body is prebuilt at start-up (one statement, the relation `get_login_creds`,
    at most 1024 bytes) and served on `GET`/`HEAD`; other methods get `405`.
- **`main.rs`** keeps `load_config` right after the root check, which invariant RMC-S10 pins. It
  then reads the binding with `load_config_with_android`. If the second bounded read differs from
  the first (the file changed during start-up), the start is refused with exit status 78. The
  binding goes to `ServerState::with_android_app` and is never logged.

## 3. Android app (`android/`)

- **Build inputs.**
  - Exact pins in `gradle/libs.versions.toml`. The Gradle distribution is pinned by URL and SHA-256
    in `gradle/wrapper/gradle-wrapper.properties`; no `gradlew` and no wrapper jar are committed.
  - `scripts/android_build.sh` downloads Gradle over HTTPS (HTTPS-only redirects), checks the hash
    before unpacking, and runs `assembleDebug testDebugUnitTest lintDebug` with strict dependency
    verification. In `ci` mode it also runs the mandatory `aapt2` permission check.
  - `verification-metadata.xml` starts empty. This is the trust-on-first-use bootstrap described in
    §2j and in the ADR: the CI run generates the file, uploads it and fails.
- **Manifest.** Only `INTERNET`. `allowBackup=false`, and the data-extraction rules exclude
  everything. Cleartext is off and only system CAs are trusted. The launcher activity is the only
  component. `minSdk` 34 means the platform Credential Manager is used, with no Google Play services
  library.
- **Code** (`org.soos.remote`).
  - `config/PcUrl` applies the `rp_id` rule to the typed address.
  - `net/UrlConnectionTransport` is HTTPS only, with no redirects, 5 s timeouts and a 16 KiB bound.
  - `net/SoosApi` sends `Origin` and `X-Soos-Action` on every POST and keeps the session cookie in
    memory only.
  - `net/ApiJson` uses explicit type checks.
  - `auth/PasskeyJson` maps to and from Credential Manager JSON and names the remembered credential.
  - `auth/PasskeySignIn` calls `CredentialManager.getCredential`.
  - `data/SettingsStore` stores the PC address and the remembered credential id.
  - The Compose UI has a settings screen and a home screen with status, **Lock PC**, **Sign in with
    passkey**, **Sign out**, **Refresh** and **Settings**.
- **Brand.**
  - `colors.xml` holds the only colour literals, equal to `crates/gui/src/theme.rs`.
  - The adaptive icon reuses the four path data strings of `crates/remote/assets/icon.svg`, scaled
    into the safe zone, with a monochrome layer.
  - Blue header and status bar, ink in dark mode.

## 4. Direct biometric prompt (#354)

The server keeps the discoverable login (owner decision part 2: no credential id is ever sent to an
anonymous client). After a successful sign-in, the app remembers the id of the passkey that signed.
The next sign-in names it in `allowCredentials` with `internal` and `hybrid` transports, so Credential
Manager opens the provider's biometric or screen-lock prompt directly. If that passkey is gone, the
remembered id is cleared and the next attempt is discoverable.

## 5. CI

`.github/workflows/android.yml` runs the job `Android (debug build)`.

- It is path-filtered, uses SHA-pinned actions (checkout as in `ci.yml`, `setup-java` v6.0.1 with
  Temurin `17.0.16+8`, `upload-artifact` v7.0.1), runs on `ubuntu-24.04` with `contents: read`, and
  uses no secrets.
- It accepts the SDK licences, then installs `platforms;android-36` and `build-tools;35.0.0`.
- It runs `./scripts/android_build.sh ci`. On a bootstrap run only, the artifact upload step runs
  (`if: failure() && steps.build.outputs.bootstrap == 'true'`).
- It is not part of `CI Success` and not advisory. `pr_checks_gate.sh` blocks a pull request that
  touches its paths while it fails. MAC5 still pins exactly one advisory check.

## 6. Verification evidence (development host, private cargo target)

| Gate | Command | Result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | pass |
| Lint | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | pass (after allowing `cast_possible_wrap` in the new test file, like the sibling suites, and a `format!` without arguments) |
| Tests | `cargo test --locked --workspace --all-targets --all-features -- --test-threads=4` | pass, 3591 tests, 0 failed (`--no-fail-fast`); a first run caught `remote_companion_contract::test_rmc_s10_main_refuses_root_before_loading_configuration`, which pins the `load_config(` call in `main.rs`: `main` keeps it and reads the binding with a second bounded read |
| New contract tests | `cargo test --locked -p soos-remote --all-features --test android_app_tests` | 30 passed |
| New invariants | `cargo test --locked -p soos-invariants --all-features android_native` | 12 passed |
| Candid review layer 1 | `./scripts/candid_review.sh` | PASSED (layer 2 not run by the agents) |
| Supply chain | `cargo deny --locked check` | not run: no Cargo dependency or lockfile change |
| Android | `scripts/android_build.sh ci` | **not run locally** (no JDK/SDK by rule); first run in CI (ANA13) |

All gates above were run with a private cargo target directory (`CARGO_BUILD_JOBS=4`).

Red evidence: in the stub phase, 17 of 29 new contract tests failed and 12 fail-closed guards passed
(tester contract). The new invariants failed on the missing §2j and on a stray `"+"` in a catalog
comment.

## 7. What could not be verified here

The Kotlin sources, the Gradle build, lint, the JVM unit tests and the `aapt2` check were never
compiled or run on this host. The Android SDK and JDK are absent, and agents must not install them.
They are checked statically by `soos-invariants::android_native_contract`, and dynamically only by
the CI job once the verification metadata exists. Expect the first strict CI run to need follow-up
fixes.

## 8. Assumptions for the owner to confirm

- **A-1**: application id `org.soos.remote`.
- **A-2**: `minSdk` 34.
- **A-3**: the CI job is blocking for pull requests that touch Android paths.
- **A-4**: the trust-on-first-use metadata bootstrap.
- **A-5**: the single `get_login_creds` relation.

## 9. Owner steps

1. Open the pull request and let the bootstrap run fail.
2. Download the `android-verification-metadata` artifact, review it, commit it on the branch, re-run
   the candid review (never `--no-verify`) and note the run id in the pull request.
3. Create a dedicated signing key and add `[android]` at the end of `remote.toml` (`Docs/REMOTE_COMPANION.md` §2j).
4. Run the hardware checks ANA14–ANA16.
