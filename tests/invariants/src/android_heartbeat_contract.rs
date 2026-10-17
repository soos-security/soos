//! Static contracts of the opt-in PC-offline heartbeat of the native Android app (GitHub #415,
//! part of #359 Part B tier 3, refs #357; owner decision of 2026-10-17 on #359; ADR
//! "[2026-10-13] Native Android App for `soos-remote` (Kotlin and Jetpack Compose)" item (8),
//! amended 2026-10-17; architect spec `AI/architect_spec_android_offline_heartbeat.md`; matrix
//! AHB1–AHB8).
//!
//! The Android tree cannot be built on the development host: these checks pin what can be
//! verified statically. The CI job `Android (debug build)` builds the app, runs the JVM unit
//! tests of the state machine and checks the APK permissions with `aapt2`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use crate::android_native_contract::run_check_permissions;
use crate::remote_companion_contract::{exists, read};

const BUILD_SCRIPT: &str = "scripts/android_build.sh";
const MANIFEST: &str = "android/app/src/main/AndroidManifest.xml";
const KOTLIN_MAIN: &str = "android/app/src/main/java/org/soos/remote";
const HEARTBEAT_DIR: &str = "android/app/src/main/java/org/soos/remote/heartbeat";
const POLICY: &str = "android/app/src/main/java/org/soos/remote/heartbeat/Heartbeat.kt";
const CONTROLLER: &str =
    "android/app/src/main/java/org/soos/remote/heartbeat/HeartbeatController.kt";
const JOB_SERVICE: &str =
    "android/app/src/main/java/org/soos/remote/heartbeat/HeartbeatJobService.kt";
const SCHEDULER: &str =
    "android/app/src/main/java/org/soos/remote/heartbeat/PlatformHeartbeatScheduler.kt";
const NOTIFIER: &str = "android/app/src/main/java/org/soos/remote/heartbeat/HeartbeatNotifier.kt";
const SHARED_API: &str = "android/app/src/main/java/org/soos/remote/net/SharedApi.kt";
const SETTINGS_STORE: &str = "android/app/src/main/java/org/soos/remote/data/SettingsStore.kt";
const UI: &str = "android/app/src/main/java/org/soos/remote/ui/SoosApp.kt";
const MAIN_ACTIVITY: &str = "android/app/src/main/java/org/soos/remote/MainActivity.kt";
const CATALOG: &str = "android/gradle/libs.versions.toml";
const APP_GRADLE: &str = "android/app/build.gradle.kts";
const JVM_TESTS: [&str; 4] = [
    "android/app/src/test/java/org/soos/remote/HeartbeatPolicyTest.kt",
    "android/app/src/test/java/org/soos/remote/HeartbeatControllerTest.kt",
    "android/app/src/test/java/org/soos/remote/HeartbeatTextsTest.kt",
    "android/app/src/test/java/org/soos/remote/SharedApiTest.kt",
];
const DOC: &str = "Docs/REMOTE_COMPANION.md";
const DECISIONS: &str = "AI/DECISIONS.md";
const MATRIX: &str = "AI/VERIFICATION_MATRIX.md";
const WALKTHROUGH: &str = "AI/walkthroughs/223_android_offline_heartbeat.md";
const ADR_TITLE: &str =
    "[2026-10-13] Native Android App for `soos-remote` (Kotlin and Jetpack Compose)";

/// The exact APK permission allowlist, in order (owner decisions of 2026-10-13, 2026-10-15 and
/// 2026-10-17).
const EXPECTED_ALLOWLIST: [&str; 6] = [
    "android.permission.INTERNET",
    "org.soos.remote.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION",
    "android.permission.USE_BIOMETRIC",
    "android.permission.USE_FINGERPRINT",
    "android.permission.POST_NOTIFICATIONS",
    "android.permission.RECEIVE_BOOT_COMPLETED",
];

/// The lines strictly between `readonly ALLOWED_PERMISSIONS=(` and the closing `)`.
fn allowlist_lines(script: &str) -> Vec<String> {
    let mut lines = script.lines();
    assert!(
        lines.any(|l| l == "readonly ALLOWED_PERMISSIONS=("),
        "{BUILD_SCRIPT} declares ALLOWED_PERMISSIONS"
    );
    let mut out = Vec::new();
    for line in lines {
        if line == ")" {
            return out;
        }
        out.push(line.to_string());
    }
    panic!("ALLOWED_PERMISSIONS is not closed");
}

/// Kotlin sources of `dir` (production), with their paths.
fn kotlin_files(dir: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![crate::remote_companion_contract::workspace_root().join(dir)];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path).expect("read_dir") {
            let entry = entry.expect("dir entry");
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "kt") {
                let text = std::fs::read_to_string(&p).expect("read Kotlin source");
                out.push((p.display().to_string(), text));
            }
        }
    }
    out.sort();
    out
}

/// AHB1: the APK permission allowlist is exactly the six owner-approved names, one quoted name
/// per line, and the script's own check accepts the two heartbeat permissions while still
/// refusing the permissions a WorkManager, foreground-service, network-constraint or alarm
/// design would add.
#[test]
fn test_ahb_s1_apk_permission_allowlist_is_pinned() {
    let script = read(BUILD_SCRIPT);
    let lines = allowlist_lines(&script);
    let names: Vec<String> = lines
        .iter()
        .map(|line| {
            let inner = line
                .strip_prefix("    \"")
                .and_then(|rest| rest.strip_suffix('"'))
                .unwrap_or_else(|| panic!("one quoted name per line: {line:?}"));
            assert!(
                inner
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_'),
                "plain permission name: {inner:?}"
            );
            inner.to_string()
        })
        .collect();
    assert_eq!(names, EXPECTED_ALLOWLIST, "ALLOWED_PERMISSIONS is pinned");

    let base = "package: name='org.soos.remote'\n\
                uses-permission: name='android.permission.INTERNET'\n\
                permission: org.soos.remote.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION\n\
                uses-permission: name='org.soos.remote.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION'\n";
    let heartbeat = format!(
        "{base}uses-permission: name='android.permission.POST_NOTIFICATIONS'\n\
         uses-permission: name='android.permission.RECEIVE_BOOT_COMPLETED'\n"
    );
    let (ok, stderr) = run_check_permissions(&heartbeat, 100);
    assert!(ok, "the heartbeat permissions pass: {stderr}");
    let (ok, stderr) = run_check_permissions(
        &format!(
            "{heartbeat}uses-permission-sdk-23: name='android.permission.USE_BIOMETRIC'\n\
             uses-permission: name='android.permission.USE_FINGERPRINT' maxSdkVersion='28'\n"
        ),
        101,
    );
    assert!(ok, "the full allowlist passes: {stderr}");
    for (case, name) in [
        "android.permission.ACCESS_NETWORK_STATE",
        "android.permission.WAKE_LOCK",
        "android.permission.FOREGROUND_SERVICE",
        "android.permission.FOREGROUND_SERVICE_DATA_SYNC",
        "android.permission.SCHEDULE_EXACT_ALARM",
        "android.permission.USE_EXACT_ALARM",
        "android.permission.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS",
        "android.permission.POST_NOTIFICATIONS.EXTRA",
        "android.permission.post_notifications",
        "android.permission.RECEIVE_BOOT_COMPLETE",
    ]
    .into_iter()
    .enumerate()
    {
        let (ok, stderr) = run_check_permissions(
            &format!("{heartbeat}uses-permission: name='{name}'\n"),
            110 + case,
        );
        assert!(!ok, "{name} is refused");
        assert!(
            stderr.contains(&format!("unexpected permission in the APK: {name}")),
            "{name}: {stderr}"
        );
    }
}

/// AHB2: the manifest declares the heartbeat as one platform job service, never exported and
/// bound only by the system scheduler; no boot receiver, no WorkManager component, no
/// foreground service.
#[test]
fn test_ahb_s2_manifest_declares_one_guarded_job_service() {
    let manifest = read(MANIFEST);
    let at = manifest
        .find("<service")
        .expect("the job service is declared");
    let tag = &manifest[at..at + manifest[at..].find('>').unwrap()];
    for needle in [
        "android:name=\".heartbeat.HeartbeatJobService\"",
        "android:exported=\"false\"",
        "android:permission=\"android.permission.BIND_JOB_SERVICE\"",
    ] {
        assert!(
            tag.contains(needle),
            "the service tag needs {needle}: {tag}"
        );
    }
    assert_eq!(manifest.matches("<service").count(), 1);
    for forbidden in [
        "<receiver",
        "<provider",
        "android.intent.action.BOOT_COMPLETED",
        "foregroundServiceType",
        "androidx.work",
        "androidx.startup",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "manifest contains {forbidden}"
        );
    }
    // The only intent filter is the launcher's: the job service has none.
    assert_eq!(manifest.matches("<intent-filter").count(), 1);
    assert_eq!(manifest.matches("<action ").count(), 1);
    assert!(manifest.contains("<action android:name=\"android.intent.action.MAIN\" />"));
}

/// AHB3: Kotlin contract of the heartbeat: the owner defaults, a persisted periodic platform
/// job without a network constraint, no WorkManager or boot handling code, the status route as
/// the only call, the session never stored, fixed notification texts, no logging, and the
/// notification prompt only where the heartbeat is turned on.
#[test]
fn test_ahb_s3_kotlin_heartbeat_contract() {
    let policy = read(POLICY);
    for needle in [
        "const val JOB_ID = 359",
        "const val INTERVAL_MS = 900_000L",
        "const val FLEX_MS = 300_000L",
        "const val FAILURES_BEFORE_ALERT = 2",
        "val JOB = HeartbeatJobSpec(JOB_ID, INTERVAL_MS, FLEX_MS, persisted = true)",
    ] {
        assert!(policy.contains(needle), "Heartbeat.kt needs {needle}");
    }
    let pure = [read(POLICY), read(CONTROLLER), read(SHARED_API)];
    for text in &pure {
        assert!(
            !text.contains("import android."),
            "the state machine, controller and shared client stay pure Kotlin"
        );
    }
    let scheduler = read(SCHEDULER);
    for needle in [
        "JobInfo.Builder(spec.jobId, ComponentName(context, HeartbeatJobService::class.java))",
        ".setPeriodic(spec.intervalMs, spec.flexMs)",
        ".setPersisted(spec.persisted)",
        "JobScheduler.RESULT_SUCCESS",
        ".cancel(jobId)",
        ".getPendingJob(jobId) != null",
    ] {
        assert!(scheduler.contains(needle), "scheduler needs {needle}");
    }
    let job = read(JOB_SERVICE);
    for needle in [
        "class HeartbeatJobService : JobService()",
        "Settings.Global.AIRPLANE_MODE_ON",
        "SharedApi.forOrigin(",
        ".status()",
        "jobFinished(params, false)",
        "HeartbeatPolicy.classify(",
    ] {
        assert!(job.contains(needle), "job service needs {needle}");
    }
    let stop = &job[job.find("override fun onStopJob").expect("onStopJob")..];
    let stop = &stop[..stop.find("\n    }").expect("onStopJob end")];
    assert!(
        stop.contains("return false"),
        "a stopped run is never rescheduled with backoff"
    );
    let heartbeat = kotlin_files(HEARTBEAT_DIR);
    assert_eq!(heartbeat.len(), 5, "five heartbeat sources");
    for (path, text) in &heartbeat {
        // The only network call is `GET /api/status`.
        for call in [
            ".lock(",
            ".authState(",
            ".loginOptions(",
            ".loginVerify(",
            ".logout(",
            "UrlConnectionTransport(",
            "openConnection",
            "URL(",
        ] {
            assert!(!text.contains(call), "{path} calls {call}");
        }
        for forbidden in [
            "SessionCookie",
            "Cookie",
            "SharedPreferences",
            "setRequiredNetwork",
            "NETWORK_TYPE",
            "setExpedited",
            "setImportantWhileForeground",
            "AlarmManager",
            "startForeground",
            "BOOT_COMPLETED",
            "BroadcastReceiver",
            "registerReceiver",
            "BackoffPolicy",
            "setBackoffCriteria",
        ] {
            assert!(!text.contains(forbidden), "{path} contains {forbidden}");
        }
    }
    let notifier = read(NOTIFIER);
    for needle in [
        "Manifest.permission.POST_NOTIFICATIONS",
        "PackageManager.PERMISSION_GRANTED",
        "PendingIntent.FLAG_IMMUTABLE",
        "HeartbeatTexts.of(notice)",
        "HeartbeatTexts.CHANNEL_ID",
    ] {
        assert!(notifier.contains(needle), "notifier needs {needle}");
    }
    for forbidden in [
        "setLargeIcon",
        "setSubText",
        "addAction",
        "setStyle",
        "host",
    ] {
        assert!(
            !notifier.contains(forbidden),
            "notifier contains {forbidden}"
        );
    }
    // Fixed English texts: no string template anywhere in the texts object.
    let texts = &policy[policy
        .find("object HeartbeatTexts")
        .expect("HeartbeatTexts")..];
    assert!(!texts.contains('$'), "notification texts are fixed");
    for sentence in [
        "\"PC went offline\"",
        "\"soos could not reach your PC in the last two checks.\"",
        "\"PC back online\"",
        "\"soos can reach your PC again.\"",
        "\"Sign-in needed\"",
        "\"Open soos and sign in with your passkey to see the PC status.\"",
    ] {
        assert!(texts.contains(sentence), "HeartbeatTexts has {sentence}");
    }

    // No WorkManager, no new dependency anywhere in the app.
    for (path, text) in kotlin_files(KOTLIN_MAIN) {
        for forbidden in ["androidx.work", "WorkManager", "BOOT_COMPLETED"] {
            assert!(!text.contains(forbidden), "{path} contains {forbidden}");
        }
    }
    for file in [CATALOG, APP_GRADLE] {
        let text = read(file);
        for forbidden in ["androidx.work", "work-runtime", "androidx.startup"] {
            assert!(!text.contains(forbidden), "{file} contains {forbidden}");
        }
    }

    // The session stays in memory: the shared client holds no storage.
    let shared = read(SHARED_API);
    for forbidden in ["SharedPreferences", "Context", "File", "DataStore"] {
        assert!(
            !shared.contains(forbidden),
            "SharedApi.kt contains {forbidden}"
        );
    }
    let store = read(SETTINGS_STORE);
    for needle in [
        "\"heartbeat_enabled\"",
        "\"heartbeat_failures\"",
        "\"heartbeat_offline_notified\"",
        "\"heartbeat_sign_in_notified\"",
        "HeartbeatPolicy.sanitize(",
        ": HeartbeatStore",
    ] {
        assert!(store.contains(needle), "SettingsStore needs {needle}");
    }
    assert!(
        store.contains("getBoolean(KEY_HEARTBEAT_ENABLED, false)"),
        "the heartbeat is off by default"
    );

    // The runtime prompt exists exactly once, in the UI, behind the heartbeat switch.
    let mut prompts = 0;
    for (path, text) in kotlin_files(KOTLIN_MAIN) {
        let count = text.matches("RequestPermission()").count()
            + text.matches("requestPermissions(").count();
        if count > 0 {
            assert!(path.ends_with("ui/SoosApp.kt"), "{path} prompts");
        }
        prompts += count;
    }
    assert_eq!(prompts, 1, "one runtime prompt site");
    // A force stop drops the persisted job: opening the app schedules it again.
    assert!(read(MAIN_ACTIVITY).contains("heartbeat.ensureScheduled()"));
    let ui = read(UI);
    assert_eq!(
        ui.matches("launch(Manifest.permission.POST_NOTIFICATIONS)")
            .count(),
        1
    );
    assert!(ui.contains("SharedApi.forOrigin(pc.origin)"));
    assert!(
        !ui.contains("SoosApi(pc.origin"),
        "the UI uses the shared client"
    );

    for test in JVM_TESTS {
        assert!(exists(test), "{test}");
    }
}

/// AHB7: ADR item (8) amendment, documentation, matrix rows and walkthrough.
#[test]
fn test_ahb_s4_docs_adr_matrix_walkthrough() {
    let decisions = read(DECISIONS);
    let adr = decisions
        .lines()
        .find(|l| l.contains(ADR_TITLE))
        .expect("ADR recorded");
    let item = &adr[adr.find("(8) **App hardening.**").expect("item (8)")..];
    let item = &item[..item.find("(9) ").expect("item (9) follows")];
    for needle in [
        "amended 2026-10-17",
        "#359",
        "#415",
        "POST_NOTIFICATIONS",
        "RECEIVE_BOOT_COMPLETED",
        "BIND_JOB_SERVICE",
        "setPersisted(true)",
        "WorkManager",
        "ACCESS_NETWORK_STATE",
        "accepted risk",
        "one exported component",
    ] {
        assert!(item.contains(needle), "ADR item (8) must mention {needle}");
    }

    let doc = read(DOC);
    let start = doc.find("## 2j. Native Android app").expect("section 2j");
    let end = start + doc[start..].find("\n## 3.").unwrap();
    let section = &doc[start..end];
    for needle in [
        "PC offline alerts",
        "15 minutes",
        "two checks in a row",
        "PC back online",
        "Sign-in needed",
        "POST_NOTIFICATIONS",
        "RECEIVE_BOOT_COMPLETED",
        "airplane mode",
        "Doze",
        "Tailscale",
        "force stop",
        "AHB",
    ] {
        assert!(section.contains(needle), "§2j must mention {needle}");
    }
    let eight = doc.find("\n## 8. ").expect("## 8.");
    let nine = doc.find("\n## 9. ").expect("## 9.");
    assert!(
        doc[eight..nine].contains("heartbeat"),
        "§8 lists the heartbeat limitations"
    );
    assert!(
        !doc[nine..].contains("the Android heartbeat need their own ADR"),
        "§9 no longer defers the Android heartbeat"
    );

    let matrix = read(MATRIX);
    for row in 1..=8 {
        assert!(
            matrix.contains(&format!("| AHB{row} |")),
            "matrix row AHB{row}"
        );
    }
    assert!(exists(WALKTHROUGH), "{WALKTHROUGH}");
    let walkthrough = read(WALKTHROUGH);
    for needle in ["#415", "#359", "test_ana_s6_manifest_hardening"] {
        assert!(walkthrough.contains(needle), "walkthrough names {needle}");
    }
}
