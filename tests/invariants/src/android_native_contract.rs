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
const REMOTE_ROUTES: &str = "crates/remote/src/routes.rs";
const API_JSON: &str = "android/app/src/main/java/org/soos/remote/net/ApiJson.kt";
const SOOS_APP: &str = "android/app/src/main/java/org/soos/remote/ui/SoosApp.kt";
const STRINGS: &str = "android/app/src/main/res/values/strings.xml";
const ALERTS_WALKTHROUGH: &str = "AI/walkthroughs/219_android_alerts.md";
const ALERTS_ADR_TITLE: &str = "[2026-10-17] Failed-Password Alerts in the Native Android App";
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

/// The integer value of `pub const name: usize = N;` in `src`.
fn rust_usize_const(src: &str, name: &str) -> u64 {
    let needle = format!("pub const {name}: usize = ");
    let at = src
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} not found"))
        + needle.len();
    let end = src[at..].find(';').unwrap();
    src[at..at + end].replace('_', "").parse().unwrap()
}

/// The integer value of `const val name = N` in Kotlin `src`.
fn kotlin_int_const(src: &str, name: &str) -> u64 {
    let needle = format!("const val {name} = ");
    let at = src
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} not found"))
        + needle.len();
    let end = src[at..].find('\n').unwrap();
    src[at..at + end].trim().replace('_', "").parse().unwrap()
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
    // Owner-approved contract migration (2026-10-17, #359): the opt-in PC-offline heartbeat
    // adds exactly POST_NOTIFICATIONS (runtime prompt) and RECEIVE_BOOT_COMPLETED (a persisted
    // JobScheduler job survives a reboot; no receiver is declared) to INTERNET.
    let mut names: Vec<&str> = permissions
        .iter()
        .map(|p| {
            let at = p.find("android:name=\"").expect("named permission") + "android:name=\"".len();
            &p[at..at + p[at..].find('"').expect("closing quote")]
        })
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "android.permission.INTERNET",
            "android.permission.POST_NOTIFICATIONS",
            "android.permission.RECEIVE_BOOT_COMPLETED",
        ],
        "exactly INTERNET, POST_NOTIFICATIONS and RECEIVE_BOOT_COMPLETED"
    );
    // The heartbeat runs in at most one platform JobService, never exported and callable only
    // by the system scheduler.
    let services: Vec<&str> = manifest
        .match_indices("<service")
        .map(|(at, _)| &manifest[at..at + manifest[at..].find('>').expect("service tag end")])
        .collect();
    assert!(services.len() <= 1, "at most one service");
    for service in &services {
        assert!(
            service.contains("android:permission=\"android.permission.BIND_JOB_SERVICE\""),
            "the service is guarded by BIND_JOB_SERVICE: {service}"
        );
        assert!(
            service.contains("android:exported=\"false\""),
            "the service is not exported: {service}"
        );
    }
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
    // Failed-password alerts (GitHub #404, matrix ANA20): the action, the paths, the snapshot
    // headers and the bounds equal the server.
    assert_eq!(
        kotlin_str_const(&api, "ACTION_ALERTS_ACK"),
        rust_str_const(&lib, "ACTION_ALERTS_ACK"),
        "ACTION_ALERTS_ACK equals lib.rs"
    );
    let routes = strip_comments(production_part(&read(REMOTE_ROUTES)));
    for name in ["ALERTS_PATH", "ALERTS_ACK_PATH"] {
        assert_eq!(
            kotlin_str_const(&api, name),
            rust_str_const(&routes, name),
            "{name} equals routes.rs"
        );
    }
    for name in ["ALERTS_EPOCH_HEADER", "ALERTS_THROUGH_HEADER"] {
        assert_eq!(
            kotlin_str_const(&api, name).to_ascii_lowercase(),
            rust_str_const(&lib, name),
            "{name} equals lib.rs"
        );
    }
    let api_json = read(API_JSON);
    for name in ["MAX_ALERT_HISTORY", "ALERTS_EPOCH_HEX_LEN"] {
        assert_eq!(
            kotlin_int_const(&api_json, name),
            rust_usize_const(&lib, name),
            "{name} equals lib.rs"
        );
    }
    assert_eq!(
        kotlin_int_const(&api_json, "MAX_ALERTS_BODY_CHARS"),
        kotlin_int_const(&read(TRANSPORT), "MAX_RESPONSE_BYTES"),
        "the alerts parser keeps the 16 KiB transport bound"
    );
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

/// ANA20: failed-password alerts in the app (GitHub #404, PR 2 of #357): §2j, the ADR entry and
/// walkthrough 219; every alerts text referenced by the UI is a `strings.xml` resource; times are
/// rendered in the phone's time zone; pull to refresh; no exception or server text is shown.
#[test]
fn test_ana_s14_alerts_contract() {
    let doc = read(DOC);
    let start = doc.find("## 2j. Native Android app").expect("section 2j");
    let end = start + doc[start..].find("\n## 3.").unwrap();
    let section = &doc[start..end];
    for needle in [
        "GET /api/alerts",
        "POST /api/alerts/ack",
        "X-Soos-Action: alerts-ack",
        "X-Soos-Alerts-Epoch",
        "X-Soos-Alerts-Through",
        "time zone",
        "pull",
        "stale_view",
        "ANA21",
    ] {
        assert!(section.contains(needle), "§2j must mention {needle}");
    }
    let decisions = read(DECISIONS);
    let adr = decisions
        .lines()
        .find(|l| l.contains(ALERTS_ADR_TITLE))
        .expect("alerts ADR recorded");
    for needle in [
        "#404",
        "#357",
        "alerts-ack",
        "no server change",
        "verification-metadata.xml",
        "strings.xml",
        "colors.xml",
        "ANA18",
    ] {
        assert!(adr.contains(needle), "alerts ADR must mention {needle}");
    }
    assert!(exists(ALERTS_WALKTHROUGH), "{ALERTS_WALKTHROUGH}");

    let app = read(SOOS_APP);
    let strings = read(STRINGS);
    let mut referenced = 0;
    for kind in ["R.string.", "R.plurals."] {
        for (at, _) in app.match_indices(kind) {
            let rest = &app[at + kind.len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.starts_with("alerts_") {
                continue;
            }
            let tag = if kind == "R.string." {
                "string"
            } else {
                "plurals"
            };
            assert!(
                strings.contains(&format!("<{tag} name=\"{name}\"")),
                "{kind}{name} is defined in strings.xml"
            );
            referenced += 1;
        }
    }
    assert!(
        referenced >= 10,
        "the alerts card uses strings.xml ({referenced})"
    );
    for needle in [
        "ZoneId.systemDefault()",
        "PullToRefreshBox",
        "api.alerts()",
        "api.ackAlerts(",
    ] {
        assert!(app.contains(needle), "SoosApp.kt needs {needle}");
    }
    for forbidden in [".message", "localizedMessage", "toString()}"] {
        assert!(!app.contains(forbidden), "SoosApp.kt shows {forbidden}");
    }
    // A server token only selects a fixed text (`refusalText(result..)`), never a bare
    // `Text(result..)` composable.
    for (at, _) in app.match_indices("Text(result") {
        let before = app[..at].chars().next_back().unwrap_or(' ');
        assert!(
            before.is_ascii_alphanumeric(),
            "SoosApp.kt displays a server value at byte {at}"
        );
    }
    let api = read(SOOS_API);
    assert!(
        api.contains("fun alerts(): ApiResult<AlertsView>"),
        "alerts() returns the parsed view"
    );
}

/// The block of `script` that starts at the line `start` and ends at the first following line
/// equal to `end` (both included).
fn script_block(script: &str, start: &str, end: &str) -> String {
    let mut out = Vec::new();
    let mut inside = false;
    for line in script.lines() {
        if !inside && line == start {
            inside = true;
        }
        if inside {
            out.push(line);
            if out.len() > 1 && line == end {
                return out.join("\n");
            }
        }
    }
    panic!("block `{start}` … `{end}` not found in {BUILD_SCRIPT}");
}

/// Runs the script's own `check_permissions` (extracted verbatim) against a fake `aapt2`
/// printing `dump`; returns `(success, stderr)`.
pub(crate) fn run_check_permissions(dump: &str, case: usize) -> (bool, String) {
    let script = read(BUILD_SCRIPT);
    let version = script
        .lines()
        .find(|l| l.starts_with("readonly BUILD_TOOLS_VERSION="))
        .expect("BUILD_TOOLS_VERSION");
    let allowed = script_block(&script, "readonly ALLOWED_PERMISSIONS=(", ")");
    let fail = script_block(&script, "fail() {", "}");
    let check = script_block(&script, "check_permissions() {", "}");
    let dir = std::env::temp_dir().join(format!("soos-ana17-{}-{case}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let tools_version = version
        .trim_start_matches("readonly BUILD_TOOLS_VERSION=")
        .trim_matches('"');
    assert!(
        !tools_version.is_empty() && !tools_version.contains('/'),
        "{version}"
    );
    let tools = dir.join("sdk/build-tools").join(tools_version);
    std::fs::create_dir_all(&tools).unwrap();
    let dump_file = dir.join("dump.txt");
    std::fs::write(&dump_file, dump).unwrap();
    let aapt2 = tools.join("aapt2");
    std::fs::write(
        &aapt2,
        format!(
            "#!/usr/bin/env bash\n[[ \"$1 $2\" == \"dump permissions\" ]] || exit 9\ncat '{}'\n",
            dump_file.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&aapt2, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let apk = dir.join("app-debug.apk");
    std::fs::write(&apk, b"apk").unwrap();
    let harness = format!(
        "set -euo pipefail\n{version}\nreadonly APK='{}'\n{allowed}\n{fail}\n{check}\ncheck_permissions\n",
        apk.display()
    );
    let output = Command::new("bash")
        .arg("-c")
        .arg(&harness)
        .env("ANDROID_HOME", dir.join("sdk"))
        .output()
        .expect("bash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// ANA17 (GitHub #398): the APK permission allowlist covers both `aapt2` request forms,
/// `uses-permission:` and `uses-permission-sdk-23:`, with exact names; any other
/// `uses-permission*` line fails closed.
#[test]
fn test_ana_s13_permission_check_covers_the_sdk_23_form() {
    let script = read(BUILD_SCRIPT);
    assert!(
        script.contains("uses-permission(-sdk-23)?: name="),
        "check_permissions must parse the uses-permission-sdk-23 form"
    );
    let base = "package: name='org.soos.remote'\n\
                uses-permission: name='android.permission.INTERNET'\n\
                permission: org.soos.remote.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION\n\
                uses-permission: name='org.soos.remote.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION'\n";
    let cases: [(String, bool, &str); 10] = [
        (base.to_string(), true, ""),
        (
            format!(
                "{base}uses-permission: name='android.permission.USE_FINGERPRINT' maxSdkVersion='28'\n\
                 uses-permission-sdk-23: name='android.permission.USE_BIOMETRIC'\n"
            ),
            true,
            "",
        ),
        (
            format!("{base}uses-permission-sdk-23: name='android.permission.USE_BIOMETRIC' maxSdkVersion='35'\n"),
            true,
            "",
        ),
        (
            format!("{base}uses-permission-sdk-23: name='android.permission.CAMERA'\n"),
            false,
            "unexpected permission in the APK: android.permission.CAMERA",
        ),
        (
            format!("{base}uses-permission: name='android.permission.READ_CONTACTS'\n"),
            false,
            "unexpected permission in the APK: android.permission.READ_CONTACTS",
        ),
        (
            format!("{base}uses-permission-sdk-23: name='android.permission.INTERNET.EXTRA'\n"),
            false,
            "unexpected permission in the APK: android.permission.INTERNET.EXTRA",
        ),
        (
            format!("{base}uses-permission-sdk-m: name='android.permission.CAMERA'\n"),
            false,
            "unrecognized aapt2 permission line",
        ),
        (
            format!("{base}  uses-permission-sdk-23: name='android.permission.CAMERA'\n"),
            false,
            "unrecognized aapt2 permission line",
        ),
        (
            format!("{base}uses-permission: name=''\n"),
            false,
            "unrecognized aapt2 permission line",
        ),
        (
            "package: name='org.soos.remote'\n\
             uses-permission-sdk-23: name='android.permission.USE_BIOMETRIC'\n"
                .to_string(),
            false,
            "android.permission.INTERNET not found",
        ),
    ];
    for (case, (dump, pass, message)) in cases.iter().enumerate() {
        let (ok, stderr) = run_check_permissions(dump, case);
        assert_eq!(ok, *pass, "case {case}: {stderr}\n{dump}");
        assert!(stderr.contains(message), "case {case}: {stderr}");
    }
}

// --- Battery and offline-history cards (GitHub #439, part of #357; matrix ANA22–ANA26) -------

const REMOTE_BATTERY: &str = "crates/remote/src/battery.rs";
const REMOTE_OFFLINE: &str = "crates/remote/src/offline.rs";
const PC_CARDS: &str = "android/app/src/main/java/org/soos/remote/ui/PcCards.kt";
const CARD_MODEL: &str = "android/app/src/main/java/org/soos/remote/cards/CardModel.kt";
const CARD_MODEL_TEST: &str = "android/app/src/test/java/org/soos/remote/CardModelTest.kt";
const CARDS_WALKTHROUGH: &str = "AI/walkthroughs/236_android_battery_offline_cards.md";
const CARDS_ADR_TITLE: &str =
    "[2026-10-18] Battery and Offline-History Cards in the Native Android App";
const MATRIX: &str = "AI/VERIFICATION_MATRIX.md";

/// The `serde` names of the unit variants of `pub enum name` in `src` (comments stripped),
/// which must be serialized with `rename_all = "snake_case"`.
fn rust_snake_variants(src: &str, name: &str) -> std::collections::BTreeSet<String> {
    let head = format!("pub enum {name} {{");
    let at = src
        .find(&head)
        .unwrap_or_else(|| panic!("pub enum {name} not found"));
    let before = &src[..at];
    let from = before.rfind(['}', ';']).map_or(0, |i| i + 1);
    assert!(
        before[from..].contains("#[serde(rename_all = \"snake_case\")]"),
        "{name} is serialized in snake_case"
    );
    let body_start = at + head.len();
    let body = &src[body_start..body_start + src[body_start..].find('}').unwrap()];
    let variants: std::collections::BTreeSet<String> = body
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|variant| {
            assert!(
                variant.chars().all(|c| c.is_ascii_alphanumeric()),
                "{name}::{variant} is a unit variant"
            );
            let mut snake = String::new();
            for (i, c) in variant.chars().enumerate() {
                if c.is_ascii_uppercase() {
                    if i > 0 {
                        snake.push('_');
                    }
                    snake.push(c.to_ascii_lowercase());
                } else {
                    snake.push(c);
                }
            }
            snake
        })
        .collect();
    assert!(!variants.is_empty(), "{name} has variants");
    variants
}

/// The members of `val name = setOf("a", "b")` in Kotlin `src`.
fn kotlin_string_set(src: &str, name: &str) -> std::collections::BTreeSet<String> {
    let head = format!("val {name} = setOf(");
    let at = src
        .find(&head)
        .unwrap_or_else(|| panic!("{name} not found"))
        + head.len();
    let body = &src[at..at + src[at..].find(')').unwrap()];
    let members: Vec<String> = body
        .split(',')
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(|m| {
            assert!(
                m.len() >= 2 && m.starts_with('"') && m.ends_with('"'),
                "{name}: {m}"
            );
            m[1..m.len() - 1].to_string()
        })
        .collect();
    let set: std::collections::BTreeSet<String> = members.iter().cloned().collect();
    assert_eq!(set.len(), members.len(), "{name} has no duplicate");
    set
}

/// The names `R.string.<prefix>…` referenced in `src`.
fn string_refs(src: &str, prefix: &str) -> std::collections::BTreeSet<String> {
    let needle = format!("R.string.{prefix}");
    src.match_indices(&needle)
        .map(|(at, _)| {
            src[at + "R.string.".len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect()
        })
        .collect()
}

/// The `<string name="<prefix>…">` resources of `strings.xml`.
fn string_names(strings: &str, prefix: &str) -> std::collections::BTreeSet<String> {
    let needle = format!("<string name=\"{prefix}");
    strings
        .match_indices(&needle)
        .map(|(at, _)| {
            let start = at + "<string name=\"".len();
            strings[start..start + strings[start..].find('"').unwrap()].to_string()
        })
        .collect()
}

/// ANA25: the battery and offline-history cards are pinned to the server (paths, enums, the
/// interval bound), read-only, bounded, built from fixed `strings.xml` texts and theme colours,
/// wired into every refresh, and documented (ADR, §2j, matrix, walkthrough 236).
#[test]
fn test_ana_s15_battery_and_offline_cards_contract() {
    // Wire contract equals the server.
    let api = read(SOOS_API);
    let routes = strip_comments(production_part(&read(REMOTE_ROUTES)));
    for name in ["BATTERY_PATH", "OFFLINE_PATH"] {
        assert_eq!(
            kotlin_str_const(&api, name),
            rust_str_const(&routes, name),
            "{name} equals routes.rs"
        );
    }
    for needle in [
        "fun battery(): ApiResult<BatteryView> = get(BATTERY_PATH) { ApiJson.parseBattery(it) }",
        "fun offline(): ApiResult<OfflineView> = get(OFFLINE_PATH) { ApiJson.parseOffline(it) }",
    ] {
        assert!(api.contains(needle), "SoosApi.kt needs {needle}");
    }
    let api_json = read(API_JSON);
    let battery = strip_comments(production_part(&read(REMOTE_BATTERY)));
    let offline = strip_comments(production_part(&read(REMOTE_OFFLINE)));
    for (kotlin, rust, src) in [
        ("BATTERY_STATES", "BatteryState", &battery),
        ("CHARGE_STATUSES", "ChargeStatus", &battery),
        ("OFFLINE_STATES", "OfflineState", &offline),
        (
            "OFFLINE_UNAVAILABLE_REASONS",
            "OfflineUnavailableReason",
            &offline,
        ),
        ("NETWORK_MONITORS", "NetworkMonitor", &offline),
        ("OFFLINE_REASONS", "OfflineReason", &offline),
    ] {
        assert_eq!(
            kotlin_string_set(&api_json, kotlin),
            rust_snake_variants(src, rust),
            "ApiJson.{kotlin} equals the serde names of {rust}"
        );
    }
    let lib = strip_comments(production_part(&read(REMOTE_LIB)));
    assert_eq!(
        kotlin_int_const(&api_json, "MAX_OFFLINE_INTERVALS"),
        rust_usize_const(&lib, "MAX_OFFLINE_INTERVALS"),
        "MAX_OFFLINE_INTERVALS equals lib.rs"
    );
    assert_eq!(
        kotlin_int_const(&api_json, "MAX_VIEW_BODY_CHARS"),
        kotlin_int_const(&read(TRANSPORT), "MAX_RESPONSE_BYTES"),
        "the card parsers keep the 16 KiB transport bound"
    );
    assert_eq!(kotlin_int_const(&api_json, "MAX_BATTERY_PERCENT"), 100);
    assert!(
        battery.contains("pub percent: Option<u8>,"),
        "the server percent is a u8"
    );
    for parser in [
        "fun parseBattery(body: String)",
        "fun parseOffline(body: String)",
    ] {
        let at = api_json
            .find(parser)
            .unwrap_or_else(|| panic!("ApiJson.kt needs {parser}"));
        let head = api_json[at..]
            .lines()
            .take(3)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            head.contains("if (body.length > MAX_VIEW_BODY_CHARS) return null"),
            "{parser} checks the body bound first"
        );
    }
    assert!(api_json.contains("if (intervals.length() > MAX_OFFLINE_INTERVALS) return null"));
    // No "other" fallback: a member outside a set refuses the view.
    assert!(!api_json.contains("?: \"other\""));

    // Pure model, JVM-tested.
    let model = read(CARD_MODEL);
    assert!(
        !model.contains("import android") && !model.contains("import androidx"),
        "the card model is pure Kotlin"
    );
    assert!(exists(CARD_MODEL_TEST), "{CARD_MODEL_TEST}");

    // Read-only cards: fixed texts, theme colours, no request, no server or exception text.
    let cards = read(PC_CARDS);
    let strings = read(STRINGS);
    for prefix in ["battery_", "offline_"] {
        let used = string_refs(&cards, prefix);
        let defined = string_names(&strings, prefix);
        assert!(
            used.len() >= 6,
            "the {prefix} card uses strings.xml ({used:?})"
        );
        assert_eq!(used, defined, "every {prefix} resource is defined and used");
    }
    for needle in [
        "fun BatteryCard(view: BatteryView?, failed: Boolean)",
        "fun OfflineCard(view: OfflineView?, failed: Boolean)",
        "MaterialTheme.colorScheme.",
        "CardModel.batteryLine(",
        "CardModel.batteryPower(",
        "CardModel.offlineDuration(",
        "timeText(",
    ] {
        assert!(cards.contains(needle), "PcCards.kt needs {needle}");
    }
    for forbidden in [
        "Text(\"",
        "text = \"",
        "colorResource",
        "Color(",
        "Color.",
        "#FF",
        ".message",
        "localizedMessage",
        "toString()",
        "api.",
        "SoosApi",
        "SharedApi",
        "Dispatchers",
        "launch",
        "Button(",
        "onClick",
    ] {
        assert!(
            !cards.contains(forbidden),
            "PcCards.kt contains {forbidden}"
        );
    }
    // Times: java.time in the phone's zone (the shared formatter of SoosApp.kt).
    let app = read(SOOS_APP);
    assert!(app.contains("internal fun alertTime(unixMs: Long): String?"));
    assert!(app.contains(".withZone(ZoneId.systemDefault())"));
    let squashed: String = app.split_whitespace().collect::<Vec<_>>().join(" ");
    for needle in [
        "var battery by remember { mutableStateOf<BatteryView?>(null) }",
        "var offline by remember { mutableStateOf<OfflineView?>(null) }",
        "withContext(Dispatchers.IO) { api.battery() }",
        "withContext(Dispatchers.IO) { api.offline() }",
        "loadAlerts() loadCards() RefreshOutcome.AUTHENTICATED",
        "BatteryCard(view = battery, failed = batteryFailed)",
        "OfflineCard(view = offline, failed = offlineFailed)",
    ] {
        assert!(squashed.contains(needle), "SoosApp.kt needs {needle}");
    }
    assert!(
        squashed.matches("clearCards()").count() >= 3,
        "the cards are cleared when unreachable and signed out"
    );

    // The battery tour step names the new card (the shared text stays equal to app.js).
    let where_battery = {
        let needle = "<string name=\"tour_battery_where\">";
        let at = strings.find(needle).expect("tour_battery_where") + needle.len();
        &strings[at..at + strings[at..].find("</string>").unwrap()]
    };
    assert!(
        where_battery.contains("Battery card") && !where_battery.contains("does not show"),
        "tour_battery_where names the Battery card: {where_battery}"
    );

    // ADR, operator documentation, matrix and walkthrough.
    let decisions = read(DECISIONS);
    let adr = decisions
        .lines()
        .find(|l| l.contains(CARDS_ADR_TITLE))
        .expect("cards ADR recorded");
    for needle in [
        "#439",
        "#357",
        "/api/battery",
        "/api/offline",
        "no server change",
        "verification-metadata.xml",
        "strings.xml",
        "tour_battery_text",
        "ANA22",
    ] {
        assert!(adr.contains(needle), "cards ADR must mention {needle}");
    }
    let doc = read(DOC);
    let start = doc.find("## 2j. Native Android app").expect("section 2j");
    let end = start + doc[start..].find("\n## 2k. ").unwrap();
    let section = &doc[start..end];
    for needle in [
        "GET /api/battery",
        "GET /api/offline",
        "Battery",
        "Offline history",
        "ZoneId.systemDefault()",
        "ANA26",
    ] {
        assert!(section.contains(needle), "§2j must mention {needle}");
    }
    assert!(
        !section.contains("the battery level and the offline history stay"),
        "§2j no longer says the app lacks these cards"
    );
    let matrix = read(MATRIX);
    for row in 22..=26 {
        let line = matrix
            .lines()
            .find(|l| l.starts_with(&format!("| ANA{row} |")))
            .unwrap_or_else(|| panic!("matrix row ANA{row}"));
        if row == 26 {
            assert!(line.contains("Pending owner verification"), "ANA26: {line}");
        }
    }
    assert!(exists(CARDS_WALKTHROUGH), "{CARDS_WALKTHROUGH}");
    let walkthrough = read(CARDS_WALKTHROUGH);
    for needle in ["#439", "#357", "/api/battery", "/api/offline", "ANA22"] {
        assert!(
            walkthrough.contains(needle),
            "walkthrough 236 mentions {needle}"
        );
    }
}
