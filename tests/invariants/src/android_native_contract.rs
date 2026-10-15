//! Static contracts of the native Android app foundation (GitHub #382, PR 1 of #357, part 2 of
//! #354; ADR "[2026-10-13] Native Android App for `soos-remote` (Kotlin and Jetpack Compose)";
//! architect spec `AI/architect_spec_android_native_app.md` §10; matrix ANA7–ANA12).
//!
//! The Android tree cannot be built on the development host (no JDK, no SDK by rule): these
//! checks pin what can be verified statically, the CI job `Android (debug build)` builds it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::collections::BTreeMap;
use std::process::Command;

use crate::remote_companion_contract::{
    exists, macro_invocations, production_part, read, strip_comments, workspace_root,
};

const ANDROID: &str = "android";
const CATALOG: &str = "android/gradle/libs.versions.toml";
const WRAPPER: &str = "android/gradle/wrapper/gradle-wrapper.properties";
const METADATA: &str = "android/gradle/verification-metadata.xml";
const GRADLE_PROPERTIES: &str = "android/gradle.properties";
const APP_GRADLE: &str = "android/app/build.gradle.kts";
const SETTINGS_GRADLE: &str = "android/settings.gradle.kts";
const MANIFEST: &str = "android/app/src/main/AndroidManifest.xml";
const NETWORK_CONFIG: &str = "android/app/src/main/res/xml/network_security_config.xml";
const COLORS: &str = "android/app/src/main/res/values/colors.xml";
const ICON_FOREGROUND: &str = "android/app/src/main/res/drawable/ic_launcher_foreground.xml";
const ICON_MONOCHROME: &str = "android/app/src/main/res/drawable/ic_launcher_monochrome.xml";
const ADAPTIVE_ICON: &str = "android/app/src/main/res/mipmap-anydpi-v26/ic_launcher.xml";
const KOTLIN_MAIN: &str = "android/app/src/main/java/org/soos/remote";
const SOOS_API: &str = "android/app/src/main/java/org/soos/remote/net/SoosApi.kt";
const TRANSPORT: &str = "android/app/src/main/java/org/soos/remote/net/UrlConnectionTransport.kt";
const SETTINGS_STORE: &str = "android/app/src/main/java/org/soos/remote/data/SettingsStore.kt";
const WORKFLOW: &str = ".github/workflows/android.yml";
const CI_WORKFLOW: &str = ".github/workflows/ci.yml";
const BUILD_SCRIPT: &str = "scripts/android_build.sh";
const SECRET_SCAN: &str = "scripts/secret_scan.sh";
const GATE: &str = "scripts/pr_checks_gate.sh";
const GUI_THEME: &str = "crates/gui/src/theme.rs";
const ICON_SVG: &str = "crates/remote/assets/icon.svg";
const REMOTE_LIB: &str = "crates/remote/src/lib.rs";
const DOC: &str = "Docs/REMOTE_COMPANION.md";
const DECISIONS: &str = "AI/DECISIONS.md";
const WALKTHROUGH: &str = "AI/walkthroughs/208_android_native_app.md";
const ADR_TITLE: &str =
    "[2026-10-13] Native Android App for `soos-remote` (Kotlin and Jetpack Compose)";
const JOB_NAME: &str = "Android (debug build)";

/// Every file under `android/` that git tracks or would add (ignored files excluded).
fn android_files() -> Vec<String> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            ANDROID,
        ])
        .current_dir(workspace_root())
        .output()
        .expect("git ls-files");
    assert!(output.status.success(), "git ls-files failed");
    let text = String::from_utf8(output.stdout).expect("UTF-8 paths");
    let files: Vec<String> = text
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .filter(|path| exists(path))
        .collect();
    assert!(
        files.len() >= 20,
        "the android/ tree is present ({} files)",
        files.len()
    );
    files
}

/// Text of every Kotlin source of the app (production).
fn kotlin_sources() -> Vec<(String, String)> {
    android_files()
        .into_iter()
        .filter(|p| p.starts_with(KOTLIN_MAIN) && p.ends_with(".kt"))
        .map(|p| {
            let text = read(&p);
            (p, text)
        })
        .collect()
}

/// `[section]` of a TOML-like file: `key = value` lines (comments dropped).
fn toml_section(text: &str, section: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == format!("[{section}]");
            continue;
        }
        if !inside || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').expect("key = value");
        out.insert(key.trim().to_string(), value.trim().to_string());
    }
    out
}

/// The value of `name = "value"` inside an inline table such as `{ group = "x", ... }`.
fn inline_field(table: &str, name: &str) -> Option<String> {
    let needle = format!("{name} = \"");
    let at = table.find(&needle)? + needle.len();
    let end = table[at..].find('"')?;
    Some(table[at..at + end].to_string())
}

/// The `0xRR, 0xGG, 0xBB` constant `name` of `crates/gui/src/theme.rs` as `RRGGBB`.
fn gui_color(theme: &str, name: &str) -> String {
    if name == "WHITE" {
        assert!(theme.contains("pub const WHITE: Color32 = Color32::WHITE;"));
        return "FFFFFF".to_string();
    }
    let needle = format!("pub const {name}: Color32 = Color32::from_rgb(");
    let at = theme
        .find(&needle)
        .unwrap_or_else(|| panic!("theme.rs has no {name}"))
        + needle.len();
    let end = theme[at..].find(')').unwrap();
    theme[at..at + end]
        .split(',')
        .map(|part| part.trim().trim_start_matches("0x").to_ascii_uppercase())
        .collect::<String>()
}

/// Every `d="..."` of `icon.svg`, in order.
fn svg_paths(svg: &str) -> Vec<String> {
    svg.match_indices(" d=\"")
        .map(|(at, needle)| {
            let start = at + needle.len();
            let end = svg[start..].find('"').unwrap();
            svg[start..start + end].to_string()
        })
        .collect()
}

/// Every `android:pathData="..."` of a vector drawable, in order.
fn vector_paths(xml: &str) -> Vec<String> {
    xml.match_indices("android:pathData=\"")
        .map(|(at, needle)| {
            let start = at + needle.len();
            let end = xml[start..].find('"').unwrap();
            xml[start..start + end].to_string()
        })
        .collect()
}

/// The string value of `pub const name: &str = "...";` in `src`.
fn rust_str_const(src: &str, name: &str) -> String {
    let needle = format!("pub const {name}: &str = \"");
    let at = src
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} not found"))
        + needle.len();
    let end = src[at..].find('"').unwrap();
    src[at..at + end].to_string()
}

/// The string value of `const val name = "...";` in Kotlin `src`.
fn kotlin_str_const(src: &str, name: &str) -> String {
    let needle = format!("const val {name} = \"");
    let at = src
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} not found"))
        + needle.len();
    let end = src[at..].find('"').unwrap();
    src[at..at + end].to_string()
}

/// ANA7: no signing key, password, local SDK path or Firebase file is tracked under
/// `android/`; `.gitignore` and `scripts/secret_scan.sh` cover the Android secret names.
#[test]
fn test_ana_s1_no_secret_files_under_android() {
    for path in android_files() {
        let name = path.rsplit('/').next().unwrap().to_ascii_lowercase();
        for suffix in [
            ".jks",
            ".keystore",
            ".p12",
            ".pfx",
            ".pem",
            ".key",
            ".apk",
            ".aab",
            ".jar",
        ] {
            assert!(!name.ends_with(suffix), "{path} must not be committed");
        }
        for forbidden in [
            "keystore.properties",
            "local.properties",
            "google-services.json",
            "googleservice-info.plist",
            "gradlew",
            "gradlew.bat",
        ] {
            assert_ne!(name, forbidden, "{path} must not be committed");
        }
        let text = read(&path);
        for needle in [
            "PRIVATE KEY",
            "storePassword",
            "keyPassword",
            "signingConfigs",
            "sdk.dir",
        ] {
            assert!(!text.contains(needle), "{path} contains {needle}");
        }
    }
    let ignore = read(".gitignore");
    for pattern in [
        "*.jks",
        "*.keystore",
        "*.p12",
        "keystore.properties",
        "local.properties",
        "google-services.json",
        "android/.gradle/",
        "android/app/build/",
        "*.apk",
        "*.aab",
    ] {
        assert!(
            ignore.lines().any(|l| l.trim() == pattern),
            ".gitignore must cover {pattern}"
        );
    }
    let scan = read(SECRET_SCAN);
    assert!(
        scan.contains("*.jks|*.keystore|"),
        "secret_scan rejects keystores"
    );
    assert!(
        scan.contains("keystore.properties|local.properties|google-services.json)"),
        "secret_scan rejects the Android configuration files"
    );
}

/// ANA8: exact version pins only; BOM-managed libraries are exactly the two Compose ones.
#[test]
fn test_ana_s2_versions_are_exact_pins() {
    let catalog = read(CATALOG);
    let versions = toml_section(&catalog, "versions");
    let expected = [
        ("agp", "8.13.2"),
        ("kotlin", "2.2.21"),
        ("composeBom", "2025.10.01"),
        ("activityCompose", "1.11.0"),
        ("coreKtx", "1.17.0"),
        ("credentials", "1.5.0"),
        ("coroutines", "1.10.2"),
        ("junit", "4.13.2"),
        ("orgJson", "20250517"),
    ];
    assert_eq!(versions.len(), expected.len(), "{versions:?}");
    for (key, value) in expected {
        assert_eq!(
            versions.get(key).map(String::as_str),
            Some(format!("\"{value}\"").as_str()),
            "[versions] {key}"
        );
    }
    for value in versions.values() {
        let bare = value.trim_matches('"');
        assert!(
            !bare.is_empty() && bare.chars().all(|c| c.is_ascii_digit() || c == '.'),
            "exact numeric version only: {value}"
        );
    }
    for forbidden in [
        "+\"",
        "latest",
        "SNAPSHOT",
        "strictly",
        "prefer",
        "require =",
    ] {
        assert!(!catalog.contains(forbidden), "catalog contains {forbidden}");
    }
    let libraries = toml_section(&catalog, "libraries");
    for (alias, table) in &libraries {
        let group = inline_field(table, "group").expect("group");
        if !table.contains("version.ref") {
            assert!(
                ["androidx.compose.ui", "androidx.compose.material3"].contains(&group.as_str()),
                "{alias} has no version and is not BOM-managed"
            );
        }
    }
    for (alias, table) in toml_section(&catalog, "plugins") {
        assert!(table.contains("version.ref"), "plugin {alias} is pinned");
    }
    let app = read(APP_GRADLE);
    assert!(app.contains("implementation(platform(libs.androidx.compose.bom))"));
    for line in app.lines().filter(|l| {
        let t = l.trim_start();
        t.starts_with("implementation(") || t.starts_with("testImplementation(")
    }) {
        assert!(
            line.contains("(libs.") || line.contains("(platform(libs."),
            "dependencies come from the catalog only: {line}"
        );
    }
    let settings = read(SETTINGS_GRADLE);
    assert!(settings.contains("RepositoriesMode.FAIL_ON_PROJECT_REPOS"));
    for forbidden in ["mavenLocal", "jitpack", "maven(\"", "maven {"] {
        assert!(!settings.contains(forbidden), "settings: {forbidden}");
    }
}

/// ANA8: the Gradle distribution is pinned by URL and SHA-256, and no wrapper binary exists.
#[test]
fn test_ana_s3_gradle_distribution_is_pinned() {
    let props = read(WRAPPER);
    assert!(props.contains(
        "distributionUrl=https\\://services.gradle.org/distributions/gradle-8.14.5-bin.zip"
    ));
    assert!(props.contains(
        "distributionSha256Sum=6f74b601422d6d6fc4e1f9a1ab6522f642c2fdcbc15ae33ebd30ba3d7198e854"
    ));
    assert!(!exists("android/gradle/wrapper/gradle-wrapper.jar"));
    assert!(!exists("android/gradlew"));
}

/// ANA8: dependency verification is on, never weakened, and (once populated) covers every
/// catalog coordinate, the Compose BOM and the three plugin markers with a SHA-256.
#[test]
fn test_ana_s4_dependency_verification_metadata() {
    let metadata = read(METADATA);
    assert!(metadata.contains("<verify-metadata>true</verify-metadata>"));
    for forbidden in [
        "<trusted-artifacts>",
        "<trust ",
        "<trust>",
        "<ignored-keys>",
    ] {
        assert!(
            !metadata.contains(forbidden),
            "metadata contains {forbidden}"
        );
    }
    let properties = read(GRADLE_PROPERTIES);
    assert!(
        !properties.contains("org.gradle.dependency.verification"),
        "verification mode is never relaxed in gradle.properties"
    );
    if !metadata.contains("<component ") {
        // Bootstrap pending (assumption A-4): the CI job then generates the file and fails.
        let script = read(BUILD_SCRIPT);
        assert!(script.contains("if ! grep -q '<component ' \"$METADATA\"; then"));
        return;
    }
    let catalog = read(CATALOG);
    let versions = toml_section(&catalog, "versions");
    let version = |key: &str| versions[key].trim_matches('"').to_string();
    let mut required: Vec<(String, String, Option<String>)> = Vec::new();
    for table in toml_section(&catalog, "libraries").values() {
        let group = inline_field(table, "group").unwrap();
        let name = inline_field(table, "name").unwrap();
        let pinned = inline_field(table, "version.ref").map(|key| version(&key));
        required.push((group, name, pinned));
    }
    for table in toml_section(&catalog, "plugins").values() {
        let id = inline_field(table, "id").unwrap();
        let pinned = inline_field(table, "version.ref").map(|key| version(&key));
        required.push((id.clone(), format!("{id}.gradle.plugin"), pinned));
    }
    for (group, name, pinned) in required {
        let mut found = false;
        for (at, _) in metadata.match_indices("<component ") {
            let end = metadata[at..].find("</component>").unwrap() + at;
            let component = &metadata[at..end];
            let matches_coordinate = component.contains(&format!("group=\"{group}\""))
                && component.contains(&format!("name=\"{name}\""))
                && pinned
                    .as_ref()
                    .is_none_or(|v| component.contains(&format!("version=\"{v}\"")));
            if matches_coordinate {
                assert!(
                    component.contains("<sha256 "),
                    "{group}:{name} has a SHA-256"
                );
                found = true;
            }
        }
        assert!(found, "verification metadata covers {group}:{name}");
    }
}

/// ANA9: no analytics, no Firebase, no Google Play services, no WebView anywhere in the app.
#[test]
fn test_ana_s5_no_analytics_firebase_play_services_or_webview() {
    for path in android_files() {
        if path.ends_with("verification-metadata.xml") {
            continue;
        }
        let lower = read(&path).to_ascii_lowercase();
        for forbidden in [
            "firebase",
            "google-services",
            "play-services",
            "com.google.android.gms",
            "crashlytics",
            "analytics",
            "webview",
            "credentials-play-services-auth",
        ] {
            assert!(!lower.contains(forbidden), "{path} mentions {forbidden}");
        }
    }
}

/// ANA9: manifest hardening.
#[test]
fn test_ana_s6_manifest_hardening() {
    let manifest = read(MANIFEST);
    let permissions: Vec<&str> = manifest
        .match_indices("<uses-permission")
        .map(|(at, _)| &manifest[at..at + manifest[at..].find("/>").unwrap()])
        .collect();
    assert_eq!(permissions.len(), 1, "exactly one permission");
    assert!(permissions[0].contains("android.permission.INTERNET"));
    for needle in [
        "android:allowBackup=\"false\"",
        "android:usesCleartextTraffic=\"false\"",
        "android:networkSecurityConfig=\"@xml/network_security_config\"",
        "android:dataExtractionRules=\"@xml/data_extraction_rules\"",
        "android:name=\".MainActivity\"",
        "android.intent.category.LAUNCHER",
    ] {
        assert!(manifest.contains(needle), "manifest needs {needle}");
    }
    assert_eq!(manifest.matches("android:exported=\"true\"").count(), 1);
    for forbidden in [
        "<service",
        "<receiver",
        "<provider",
        "android:autoVerify",
        "android:debuggable",
        "<data ",
        "android:sharedUserId",
        "usesCleartextTraffic=\"true\"",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "manifest contains {forbidden}"
        );
    }
    let network = read(NETWORK_CONFIG);
    assert!(network.contains("cleartextTrafficPermitted=\"false\""));
    assert!(network.contains("<certificates src=\"system\" />"));
    for forbidden in [
        "src=\"user\"",
        "<debug-overrides",
        "<domain-config",
        "<pin-set",
    ] {
        assert!(
            !network.contains(forbidden),
            "network config contains {forbidden}"
        );
    }
    let app = read(APP_GRADLE);
    for needle in [
        "applicationId = \"org.soos.remote\"",
        "minSdk = 34",
        "targetSdk = 36",
        "compileSdk = 36",
        "buildToolsVersion = \"35.0.0\"",
        "includeInApk = false",
    ] {
        assert!(app.contains(needle), "app/build.gradle.kts needs {needle}");
    }
}

/// ANA10: the only colour literals are `colors.xml`, equal to `crates/gui/src/theme.rs`; the
/// adaptive icon reuses the path data of `crates/remote/assets/icon.svg`.
#[test]
fn test_ana_s7_brand_colors_and_icon() {
    let theme = read(GUI_THEME);
    let colors = read(COLORS);
    let mut seen = 0;
    for (at, _) in colors.match_indices("<color name=\"soos_") {
        let line_end = colors[at..].find('\n').unwrap() + at;
        let line = &colors[at..line_end];
        let name = &line["<color name=\"soos_".len()..line.find("\">").unwrap()];
        let value = &line[line.find("\">#FF").unwrap() + 5..line.find("</color>").unwrap()];
        assert_eq!(
            value,
            gui_color(&theme, &name.to_ascii_uppercase()),
            "soos_{name} equals theme.rs"
        );
        seen += 1;
    }
    assert!(seen >= 8, "palette present ({seen})");
    for path in android_files() {
        if path.ends_with(".xml") && path != COLORS {
            let text = read(&path);
            let literal = text.match_indices('#').any(|(at, _)| {
                let rest = &text[at + 1..];
                rest.len() >= 6 && rest[..6].chars().all(|c| c.is_ascii_hexdigit())
            });
            assert!(!literal, "{path} carries a colour literal");
        }
    }
    for (path, text) in kotlin_sources() {
        for forbidden in [
            "Color(0x",
            "Color.Red",
            "Color.Blue",
            "Color.Black",
            "Color.White",
        ] {
            assert!(!text.contains(forbidden), "{path} uses {forbidden}");
        }
    }
    let svg = svg_paths(&read(ICON_SVG));
    assert_eq!(svg.len(), 4);
    for icon in [ICON_FOREGROUND, ICON_MONOCHROME] {
        assert_eq!(vector_paths(&read(icon)), svg, "{icon} reuses icon.svg");
    }
    let adaptive = read(ADAPTIVE_ICON);
    for needle in [
        "<background android:drawable=\"@color/soos_pale\" />",
        "<foreground android:drawable=\"@drawable/ic_launcher_foreground\" />",
        "<monochrome android:drawable=\"@drawable/ic_launcher_monochrome\" />",
    ] {
        assert!(adaptive.contains(needle), "{needle}");
    }
}

/// ANA11: Kotlin hygiene: no logging or printing, no redirect following, the action values
/// equal `crates/remote/src/lib.rs`, the session cookie is never persisted.
#[test]
fn test_ana_s8_kotlin_hygiene() {
    let sources = kotlin_sources();
    assert!(sources.len() >= 10, "{} Kotlin files", sources.len());
    for (path, text) in &sources {
        for forbidden in [
            "android.util.Log",
            "Log.",
            "println(",
            "print(",
            "printStackTrace",
            "System.err",
            "System.out",
            "Timber",
            "HttpURLConnection(",
            "http://",
            "addJavascriptInterface",
        ] {
            assert!(!text.contains(forbidden), "{path} contains {forbidden}");
        }
    }
    let api = read(SOOS_API);
    let lib = strip_comments(production_part(&read(REMOTE_LIB)));
    for (kotlin, rust) in [
        ("ACTION_LOCK", "ACTION_LOCK"),
        ("ACTION_LOGIN_OPTIONS", "ACTION_LOGIN_OPTIONS"),
        ("ACTION_LOGIN", "ACTION_LOGIN"),
        ("ACTION_LOGOUT", "ACTION_LOGOUT"),
    ] {
        assert_eq!(
            kotlin_str_const(&api, kotlin),
            rust_str_const(&lib, rust),
            "{kotlin} equals lib.rs"
        );
    }
    assert!(api.contains("headers += \"Origin\" to origin"));
    let transport = read(TRANSPORT);
    for needle in [
        "as? HttpsURLConnection",
        "instanceFollowRedirects = false",
        "useCaches = false",
        "const val MAX_RESPONSE_BYTES = 16384",
        "const val CONNECT_TIMEOUT_MS = 5000",
        "const val READ_TIMEOUT_MS = 5000",
    ] {
        assert!(transport.contains(needle), "transport needs {needle}");
    }
    let store = read(SETTINGS_STORE).to_ascii_lowercase();
    assert!(
        !store.contains("cookie\"") && !store.contains("session\""),
        "the session cookie is never stored"
    );
    assert!(!sources
        .iter()
        .any(|(_, t)| t.contains("SessionCookie") && t.contains("SharedPreferences")));
}

/// ANA12: the CI job is pinned, secret-free, path-filtered, strict or failing, not a CI
/// Success dependency and not advisory.
#[test]
fn test_ana_s9_ci_job() {
    let wf = read(WORKFLOW);
    for line in wf
        .lines()
        .filter(|l| l.trim_start().starts_with("- uses:") || l.trim_start().starts_with("uses:"))
    {
        let spec = line.split("uses:").nth(1).unwrap().trim();
        let (_, rest) = spec.split_once('@').expect("pinned action");
        let sha = rest.split_whitespace().next().unwrap();
        assert!(
            sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
            "action pinned by full SHA: {line}"
        );
    }
    for needle in [
        &format!("name: \"{JOB_NAME}\""),
        "runs-on: ubuntu-24.04",
        "permissions:\n  contents: read",
        "persist-credentials: false",
        "java-version: \"17.0.16+8\"",
        "\"platforms;android-36\" \"build-tools;35.0.0\"",
        "--licenses",
        "id: build",
        "run: ./scripts/android_build.sh ci",
        "if: failure() && steps.build.outputs.bootstrap == 'true'",
        "- \"android/**\"",
        "- \"scripts/android_build.sh\"",
        "concurrency:",
    ] {
        assert!(wf.contains(needle), "android.yml needs {needle}");
    }
    for forbidden in [
        "secrets.",
        "pull_request_target",
        "write-all",
        ": write",
        "-latest",
    ] {
        assert!(!wf.contains(forbidden), "android.yml contains {forbidden}");
    }
    let ci = read(CI_WORKFLOW);
    let needs = ci
        .lines()
        .find(|l| l.trim_start().starts_with("needs: [lint, clippy"))
        .expect("CI Success needs");
    assert!(
        !needs.contains("android"),
        "CI Success does not depend on the Android job"
    );
    assert!(!ci.contains("android_build.sh") && !ci.contains(JOB_NAME));
    assert!(
        !read(GATE).contains(JOB_NAME),
        "the Android job is not advisory"
    );
}

/// ANA12: the build script verifies the distribution and builds strictly.
#[test]
fn test_ana_s10_build_script() {
    let script = read(BUILD_SCRIPT);
    for needle in [
        "set -euo pipefail",
        "sha256sum --check --status",
        "--proto '=https' --proto-redir '=https'",
        "--max-redirs 5",
        "--dependency-verification strict",
        "--write-verification-metadata sha256",
        "readonly TASKS=(assembleDebug testDebugUnitTest lintDebug)",
        "readonly BUILD_TOOLS_VERSION=\"35.0.0\"",
        "\"android.permission.INTERNET\"",
        "echo \"bootstrap=true\" >> \"$GITHUB_OUTPUT\"",
        "check_permissions",
    ] {
        assert!(script.contains(needle), "android_build.sh needs {needle}");
    }
    // In ci mode, a bootstrap run always fails after writing its output.
    let ci = &script[script.rfind("    ci)").unwrap()..];
    let bootstrap = ci.find("run_bootstrap").unwrap();
    let exit = ci[bootstrap..].find("exit 1").unwrap();
    assert!(
        !ci[bootstrap..bootstrap + exit].contains("run_strict"),
        "bootstrap exits 1"
    );
    for forbidden in ["--no-verify", "lenient", "curl -k", "--insecure", "sudo "] {
        assert!(
            !script.contains(forbidden),
            "android_build.sh contains {forbidden}"
        );
    }
}

/// ANA6: `soos-remote` accepts the app origin for the login ceremony only, never logs the
/// binding, and keeps the asset-links route out of session refreshes.
#[test]
fn test_ana_s11_remote_binding_is_login_only_and_silent() {
    let auth = strip_comments(production_part(&read("crates/remote/src/auth.rs")));
    assert_eq!(auth.matches("OriginPolicy::WebOrAndroidApp").count(), 1);
    assert!(auth.contains("if purpose == ChallengePurpose::Login {"));
    let server = strip_comments(production_part(&read("crates/remote/src/server.rs")));
    assert!(
        !server.contains("WebOrAndroidApp"),
        "server never widens the policy"
    );
    assert!(server
        .contains("parse_client_data(rp, &registration.client_data_json, CeremonyType::Create)"));
    assert!(server.contains(
        "Route::Asset(_) | Route::AssetLinks | Route::NotFound | Route::MethodNotAllowed"
    ));
    for file in [
        "crates/remote/src/config.rs",
        "crates/remote/src/server.rs",
        "crates/remote/src/main.rs",
        "crates/remote/src/webauthn.rs",
        "crates/remote/src/auth.rs",
    ] {
        let code = strip_comments(production_part(&read(file)));
        for args in macro_invocations(&code, &["trace", "debug", "info", "warn", "error"]) {
            let lower = args.to_ascii_lowercase();
            for needle in ["android", "package", "fingerprint", "asset_links", "origin"] {
                assert!(!lower.contains(needle), "{file} logs {needle}: {args}");
            }
        }
    }
}

/// ANA5, ANA13: documentation, ADR and walkthrough; the pinned §9 needles of other contracts
/// stay in place.
#[test]
fn test_ana_s12_docs_adr_walkthrough() {
    let doc = read(DOC);
    let start = doc.find("## 2j. Native Android app").expect("section 2j");
    let end = start + doc[start..].find("\n## 3.").unwrap();
    let section = &doc[start..end];
    for needle in [
        "[android]",
        "sha256_cert_fingerprints",
        "assetlinks.json",
        "android:apk-key-hash:",
        "allow_funnel",
        "end of `remote.toml`",
        "debug.keystore",
        "keytool",
        "apksigner",
        "rotation",
        "accepted risk",
        "Google Password Manager",
        "debuggable",
        "verification-metadata.xml",
        "candid review",
        "--no-verify",
        "scripts/android_build.sh",
        JOB_NAME,
        "org.soos.remote",
    ] {
        assert!(section.contains(needle), "§2j must mention {needle}");
    }
    let out_of_scope = &doc[doc.find("## 9.").unwrap()..];
    for needle in [
        "native Android app",
        "notify.windows.com",
        "Tasker",
        "section 2i",
    ] {
        assert!(out_of_scope.contains(needle), "§9 keeps {needle}");
    }
    let decisions = read(DECISIONS);
    let adr = decisions
        .lines()
        .find(|l| l.contains(ADR_TITLE))
        .expect("ADR recorded");
    for needle in [
        "#382",
        "#357",
        "#354",
        "Trusted Web Activity",
        "get_login_creds",
        "Login",
        "trust on first use",
        "accepted risk",
        "Apache-2.0",
        "EPL-1.0",
        "Public Domain",
        "owner to confirm",
    ] {
        assert!(adr.contains(needle), "ADR must mention {needle}");
    }
    assert!(exists(WALKTHROUGH), "{WALKTHROUGH}");
}
