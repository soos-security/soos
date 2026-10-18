//! Static contracts of the first-run feature tour of the native Android app (GitHub #427, PR 3
//! of #363; ADR "[2026-10-18] Phone Onboarding Tour in the Native Android App"; spec
//! `AI/architect_spec_android_onboarding_tour.md`; matrix ATR1–ATR9):
//!
//! - ATR-S1 the shared titles, explanations and "Off:" lines, the introduction title and the
//!   labels are byte-equal to the web tour of `crates/remote/assets/app.js`, and the step list
//!   of `strings.xml` and `TourStep` is the specified one;
//! - ATR-S2 every where line names the same `remote.toml` keys as the web line (so an edit never
//!   makes `soos-remote` exit 78), the app's own heartbeat is turned off in the app, and every
//!   tour text is plain English;
//! - ATR-S3 every tour text is a `strings.xml` resource, all of them are used, and the step map
//!   is one exhaustive `when`;
//! - ATR-S4 the only stored datum is the Boolean `tour_seen` of `SettingsStore`, in the private
//!   `soos_settings` file excluded from backup and device transfer;
//! - ATR-S5 the tour code makes no request, switches nothing, reads no server or user data and
//!   logs nothing; the model is pure Kotlin;
//! - ATR-S6 the wiring (once after the status view, never over the settings screen or the
//!   sign-in state, system back, Feature tour), theme roles, no animation, and the unchanged
//!   manifest, permissions and dependencies;
//! - ATR-S7 ADR, matrix rows, operator documentation and walkthrough.
//!
//! The Android tree cannot be built on the development host (no JDK, no SDK by rule): these
//! checks are static; the JVM tests run in the CI job `Android (debug build)`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::collections::{BTreeMap, BTreeSet};

use crate::remote_companion_contract::{exists, read, workspace_root};

const APP_JS: &str = "crates/remote/assets/app.js";
const STRINGS: &str = "android/app/src/main/res/values/strings.xml";
const TOUR_MODEL: &str = "android/app/src/main/java/org/soos/remote/tour/Tour.kt";
const TOUR_CARD: &str = "android/app/src/main/java/org/soos/remote/ui/TourCard.kt";
const SOOS_APP: &str = "android/app/src/main/java/org/soos/remote/ui/SoosApp.kt";
const SETTINGS_STORE: &str = "android/app/src/main/java/org/soos/remote/data/SettingsStore.kt";
const MANIFEST: &str = "android/app/src/main/AndroidManifest.xml";
const EXTRACTION_RULES: &str = "android/app/src/main/res/xml/data_extraction_rules.xml";
const CATALOG: &str = "android/gradle/libs.versions.toml";
const APP_GRADLE: &str = "android/app/build.gradle.kts";
const KOTLIN_MAIN: &str = "android/app/src/main/java/org/soos/remote";
const JVM_TEST: &str = "android/app/src/test/java/org/soos/remote/TourControllerTest.kt";
const DOC: &str = "Docs/REMOTE_COMPANION.md";
const DECISIONS: &str = "AI/DECISIONS.md";
const MATRIX: &str = "AI/VERIFICATION_MATRIX.md";
const WALKTHROUGH: &str = "AI/walkthroughs/230_android_onboarding_tour.md";
const ADR_TITLE: &str = "[2026-10-18] Phone Onboarding Tour in the Native Android App";

const RESTART: &str = "systemctl --user restart soos-remote.service";
const REMOTE_TOML: &str = "~/.config/soos/remote.toml";

/// The steps in order: `TourStep` entry, resource key, title of the matching web step (`None`
/// for the steps the web tour does not have).
const STEPS: [(&str, &str, Option<&str>); 10] = [
    ("INTRO", "intro", None),
    (
        "PHONE_COMPANION",
        "phone_companion",
        Some("Phone companion"),
    ),
    ("BATTERY", "battery", Some("Battery level")),
    ("REMOTE_LOCK", "remote_lock", Some("Remote lock and unlock")),
    (
        "INTERNET_ACCESS",
        "internet_access",
        Some("Internet access"),
    ),
    (
        "FAILED_PASSWORDS",
        "failed_passwords",
        Some("Failed-password history"),
    ),
    ("PUSH_ALERTS", "push_alerts", Some("Push alerts")),
    ("CAMERA_VIEW", "camera_view", Some("Live camera view")),
    ("OFFLINE_ALERTS", "offline_alerts", Some("Offline alerts")),
    ("PC_OFFLINE_ALERTS", "pc_offline_alerts", None),
];

/// Native label resource, web constant.
const LABELS: [(&str, &str); 5] = [
    ("tour_next", "TOUR_NEXT_LABEL"),
    ("tour_done", "TOUR_DONE_LABEL"),
    ("tour_back", "TOUR_BACK_LABEL"),
    ("tour_skip", "TOUR_SKIP_LABEL"),
    ("tour_replay", "TOUR_REPLAY_LABEL"),
];

/// Steps whose feature the app does not show itself: the where line points to the web app.
const WEB_APP_STEPS: [&str; 4] = ["battery", "remote_lock", "push_alerts", "camera_view"];

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// The value of `const NAME = "literal";` of `app.js` (exactly one declaration, one literal).
fn js_const(app: &str, name: &str) -> String {
    let head = format!("const {name} =");
    assert_eq!(app.matches(&head).count(), 1, "exactly one `{head}`");
    let rest = app[app.find(&head).unwrap() + head.len()..].trim_start();
    assert!(rest.starts_with('"'), "{name} is a string literal");
    let end = rest[1..].find('"').unwrap();
    assert!(
        rest[end + 2..].trim_start().starts_with(';'),
        "{name} is one literal"
    );
    rest[1..=end].to_string()
}

/// `TOUR_STEPS` of `app.js`: per step, `field -> literal` (the web contract PTR-S5 already
/// forbids escapes, braces and quotes inside the literals).
fn web_steps(app: &str) -> Vec<BTreeMap<String, String>> {
    let head = "const TOUR_STEPS = [";
    assert_eq!(app.matches(head).count(), 1, "exactly one `{head}`");
    let start = app.find(head).unwrap() + head.len();
    let end = start
        + app[start..]
            .find("\n];")
            .expect("TOUR_STEPS ends with `];`");
    let inner = &app[start..end];
    let mut steps = Vec::new();
    let mut rest = inner;
    while let Some(open) = rest.find('{') {
        let close = open + rest[open..].find('}').expect("step object closes");
        let body = &rest[open + 1..close];
        let mut fields = BTreeMap::new();
        let mut cursor = body;
        while let Some(colon) = cursor.find(':') {
            let name = cursor[..colon].trim().trim_start_matches(',').trim();
            let after = cursor[colon + 1..].trim_start();
            assert!(after.starts_with('"'), "`{name}` is a literal");
            let value_end = after[1..].find('"').unwrap();
            let value = &after[1..=value_end];
            assert!(!value.contains('\\'), "no escape in `{name}`");
            fields.insert(name.to_string(), value.to_string());
            cursor = &after[value_end + 2..];
        }
        steps.push(fields);
        rest = &rest[close + 1..];
    }
    steps
}

fn web_field<'a>(step: &'a BTreeMap<String, String>, name: &str) -> &'a str {
    step.get(name)
        .unwrap_or_else(|| panic!("web step field {name}: {step:?}"))
}

/// Decodes the body of an Android `<string>` resource: `\'`, `\"`, `\\` and the three XML
/// entities; any other escape, markup or a leading `@`/`?` is refused.
fn decode_resource(name: &str, raw: &str) -> String {
    assert!(
        !raw.starts_with('@') && !raw.starts_with('?'),
        "{name} is a literal"
    );
    assert!(!raw.contains('<'), "{name} carries no markup");
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('\'') => out.push('\''),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                other => panic!("{name}: unsupported escape \\{other:?}"),
            },
            '\'' => panic!("{name}: an apostrophe must be escaped"),
            '"' => panic!("{name}: a double quote is not allowed"),
            _ => out.push(c),
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Every `<string name="...">...</string>` of `strings.xml` (one per line), decoded.
fn resources() -> BTreeMap<String, String> {
    let xml = read(STRINGS);
    let mut out = BTreeMap::new();
    for line in xml.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("<string name=\"") else {
            continue;
        };
        let quote = rest.find('"').unwrap();
        let name = &rest[..quote];
        let tail = &rest[quote + 1..];
        assert!(
            tail.starts_with('>'),
            "{name}: no attribute besides the name ({tail})"
        );
        let body = tail[1..]
            .strip_suffix("</string>")
            .unwrap_or_else(|| panic!("{name}: one line"));
        let previous = out.insert(name.to_string(), decode_resource(name, body));
        assert!(previous.is_none(), "{name} is defined once");
    }
    out
}

fn tour_resources() -> BTreeMap<String, String> {
    resources()
        .into_iter()
        .filter(|(name, _)| name.starts_with("tour_"))
        .collect()
}

fn res<'a>(map: &'a BTreeMap<String, String>, name: &str) -> &'a str {
    map.get(name)
        .unwrap_or_else(|| panic!("strings.xml defines {name}"))
}

/// The set of `remote.toml` keys a text turns off (`key = false`).
fn keys_set_false(text: &str) -> BTreeSet<String> {
    text.match_indices(" = false")
        .map(|(at, _)| {
            text[..at]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_lowercase() || *c == '_')
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        })
        .inspect(|key| assert!(!key.is_empty(), "a key before ` = false` in {text}"))
        .collect()
}

/// Plain English the app may show: printable ASCII, no markup, quote, template or emphasis.
fn plain_text(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|b| {
            (0x20..=0x7e).contains(&b)
                && !matches!(
                    b,
                    b'\\' | b'`' | b'!' | b'<' | b'>' | b'{' | b'}' | b'$' | b'&' | b'"' | b'%'
                )
        })
        && text == text.trim()
        && !text.contains("  ")
}

/// Every Kotlin source of the app (production), path relative to the workspace.
fn kotlin_sources() -> Vec<(String, String)> {
    let root = workspace_root();
    let mut out = Vec::new();
    let mut stack = vec![root.join(KOTLIN_MAIN)];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path).expect("read_dir") {
            let p = entry.expect("dir entry").path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "kt") {
                let rel = p
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let text = std::fs::read_to_string(&p).expect("read Kotlin source");
                out.push((rel, text));
            }
        }
    }
    out.sort();
    assert!(out.len() >= 10, "{} Kotlin files", out.len());
    out
}

/// Kotlin source without `//` line comments and `/* */` block comments (strings kept; a `//`
/// inside a literal, such as `https://`, also cuts the rest of its line, which only removes
/// text).
fn kotlin_code(src: &str) -> String {
    let mut out = String::new();
    let mut rest = src;
    loop {
        let line = rest.find("//");
        let block = rest.find("/*");
        let line_first = match (line, block) {
            (None, None) => {
                out.push_str(rest);
                return out;
            }
            (Some(l), Some(b)) => l < b,
            (Some(_), None) => true,
            (None, Some(_)) => false,
        };
        if line_first {
            let l = line.unwrap();
            out.push_str(&rest[..l]);
            rest = rest[l..].find('\n').map_or("", |e| &rest[l + e..]);
        } else {
            let b = block.unwrap();
            out.push_str(&rest[..b]);
            let end = rest[b..].find("*/").expect("block comment closes");
            rest = &rest[b + end + 2..];
        }
    }
}

/// `text` with every run of whitespace replaced by one space.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The argument list of the first call `callee(` in `src`, up to its matching parenthesis.
fn call_block<'a>(src: &'a str, callee: &str) -> &'a str {
    let at = src
        .find(callee)
        .unwrap_or_else(|| panic!("call {callee} present"));
    let open = at + callee.len() - 1;
    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &src[open..=open + i];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced call {callee}");
}

/// The `when (step) { ... }` block of `TourCard.kt`.
fn step_map(card: &str) -> &str {
    let head = "when (step) {";
    assert_eq!(card.matches(head).count(), 1, "one step map");
    let start = card.find(head).unwrap();
    let mut depth = 0usize;
    for (i, c) in card[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &card[start..=start + i];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced step map");
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

/// The helpers parse what they claim.
#[test]
fn test_atr_helper_self_test() {
    assert_eq!(decode_resource("a", "this phone\\'s"), "this phone's");
    assert_eq!(decode_resource("a", "x &amp; y"), "x & y");
    assert_eq!(
        keys_set_false("set allow_funnel = false and camera_view_funnel = false in"),
        ["allow_funnel", "camera_view_funnel"]
            .into_iter()
            .map(String::from)
            .collect::<BTreeSet<_>>()
    );
    assert!(keys_set_false("no key here").is_empty());
    assert!(plain_text(
        "Off: the phone can still lock this PC, never unlock it."
    ));
    assert!(!plain_text("Step %1$d"));
    assert_eq!(
        kotlin_code("a // c\nb /* d */ e"),
        "a \nb  e",
        "comments removed"
    );
    assert_eq!(call_block("f(a, g(b)) + h", "f("), "(a, g(b))");
    let steps = web_steps(
        "const TOUR_STEPS = [\n  {\n    title: \"T\",\n    where:\n      \"W\",\n  },\n];",
    );
    assert_eq!(steps.len(), 1);
    assert_eq!(web_field(&steps[0], "title"), "T");
    assert_eq!(web_field(&steps[0], "where"), "W");
}

/// ATR-S1 (ATR2): the shared wording is byte-equal to the web tour; the step lists match.
#[test]
fn test_atr_s1_shared_wording_is_byte_equal_to_the_web_tour() {
    let app = read(APP_JS);
    let web = web_steps(&app);
    let strings = tour_resources();
    let web_titles: Vec<&str> = web.iter().map(|s| web_field(s, "title")).collect();
    let shared: Vec<&str> = STEPS.iter().filter_map(|s| s.2).collect();
    assert_eq!(
        web_titles, shared,
        "every web step is in the app, in the web order"
    );
    for (_, key, title) in STEPS {
        let Some(title) = title else { continue };
        let step = web.iter().find(|s| web_field(s, "title") == title).unwrap();
        for (field, suffix) in [("title", "title"), ("text", "text"), ("off", "off")] {
            assert_eq!(
                res(&strings, &format!("tour_{key}_{suffix}")),
                web_field(step, field),
                "tour_{key}_{suffix} equals the web {field} of {title}"
            );
        }
    }
    assert_eq!(
        res(&strings, "tour_intro_title"),
        js_const(&app, "TOUR_INTRO_TITLE")
    );
    for (name, constant) in LABELS {
        assert_eq!(res(&strings, name), js_const(&app, constant), "{name}");
    }
    assert_eq!(res(&strings, "tour_step_counter"), "Step %1$d of %2$d");

    // Exactly the specified resources.
    let mut expected: BTreeSet<String> = ["tour_intro_title", "tour_intro_text", "tour_intro_note"]
        .into_iter()
        .map(String::from)
        .collect();
    for (_, key, _) in &STEPS[1..] {
        for suffix in ["title", "text", "off", "where"] {
            expected.insert(format!("tour_{key}_{suffix}"));
        }
    }
    for (name, _) in LABELS {
        expected.insert(name.to_string());
    }
    expected.insert("tour_step_counter".to_string());
    assert_eq!(
        strings.keys().cloned().collect::<BTreeSet<_>>(),
        expected,
        "the tour resources of strings.xml"
    );

    // `TourStep` lists the same steps, in the same order, with the shared flag of the web tour.
    let model = kotlin_code(&read(TOUR_MODEL));
    let head = "enum class TourStep(val sharedWithWebTour: Boolean) {";
    let start = model.find(head).expect("enum TourStep") + head.len();
    let body = &model[start..start + model[start..].find('}').unwrap()];
    let entries: Vec<(String, String)> = body
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(|e| {
            let (name, flag) = e.split_once('(').expect("ENTRY(flag)");
            (
                name.trim().to_string(),
                flag.trim_end_matches(')').trim().to_string(),
            )
        })
        .collect();
    let expected_entries: Vec<(String, String)> = STEPS
        .iter()
        .map(|(entry, _, web)| (entry.to_string(), web.is_some().to_string()))
        .collect();
    assert_eq!(entries, expected_entries, "TourStep entries and flags");
    for (entry, key, _) in STEPS {
        assert_eq!(
            entry.to_ascii_lowercase(),
            key,
            "{entry} maps to tour_{key}_*"
        );
    }

    let intro = res(&strings, "tour_intro_text");
    assert!(
        intro.contains("changes no setting"),
        "the introduction says the tour switches nothing: {intro}"
    );
    let note = res(&strings, "tour_intro_note");
    assert!(
        note.contains("only remembers that you closed the tour") && note.contains("Feature tour"),
        "the introduction says what is remembered and how to reopen: {note}"
    );
}

/// ATR-S2 (ATR3): the where lines name the same `remote.toml` keys as the web tour; the app's
/// own heartbeat is turned off in the app; every text is plain English.
#[test]
fn test_atr_s2_off_paths_name_the_dependent_keys() {
    let app = read(APP_JS);
    let web = web_steps(&app);
    let strings = tour_resources();
    for (name, value) in &strings {
        if name == "tour_step_counter" {
            continue;
        }
        let plain = value.replace('\'', "");
        assert!(plain_text(&plain), "{name} is plain English: {value:?}");
    }
    for (_, key, title) in &STEPS[1..] {
        let off = res(&strings, &format!("tour_{key}_off"));
        assert!(off.starts_with("Off: "), "tour_{key}_off: {off}");
        let text = res(&strings, &format!("tour_{key}_text"));
        assert!(text.len() >= 80, "tour_{key}_text is a real explanation");
        let place = res(&strings, &format!("tour_{key}_where"));
        assert!(
            !place.contains("this page") && !place.contains("Show it on"),
            "tour_{key}_where speaks of the app, not of the web page: {place}"
        );
        let Some(title) = title else { continue };
        let step = web
            .iter()
            .find(|s| web_field(s, "title") == *title)
            .unwrap();
        let web_place = web_field(step, "where");
        let keys = keys_set_false(place);
        assert_eq!(
            keys,
            keys_set_false(web_place),
            "tour_{key}_where turns off the keys of the web tour"
        );
        assert_eq!(
            place.contains("[android]"),
            web_place.contains("[android]"),
            "tour_{key}_where names the [android] table like the web tour"
        );
        if !keys.is_empty() {
            assert!(
                place.contains(REMOTE_TOML),
                "tour_{key}_where: {REMOTE_TOML}"
            );
            assert!(place.contains(RESTART), "tour_{key}_where: {RESTART}");
        }
        for command in [
            "systemctl --user disable --now soos-remote.service",
            "soos-gui",
        ] {
            if web_place.contains(command) {
                assert!(place.contains(command), "tour_{key}_where: {command}");
            }
        }
    }
    for key in WEB_APP_STEPS {
        let place = res(&strings, &format!("tour_{key}_where"));
        assert!(
            place.contains("web app"),
            "the app has no such card: tour_{key}_where points to the web app: {place}"
        );
    }
    let internet = res(&strings, "tour_internet_access_where");
    assert!(
        internet.contains("Sign out"),
        "the app has Sign out: {internet}"
    );

    // The app's own opt-in heartbeat (ADR 2026-10-13 item (8)): off with the app's switch.
    assert_eq!(
        res(&strings, "tour_pc_offline_alerts_title"),
        "PC offline alerts"
    );
    let ui = read(SOOS_APP);
    assert!(
        ui.contains("\"PC offline alerts\""),
        "the switch the step names exists"
    );
    let place = res(&strings, "tour_pc_offline_alerts_where");
    assert!(place.contains("Settings") && place.contains("PC offline alerts"));
    assert!(keys_set_false(place).is_empty() && !place.contains("remote.toml"));
    let text = res(&strings, "tour_pc_offline_alerts_text");
    for needle in [
        "15 minutes",
        "two failed checks in a row",
        "off until you turn it on",
    ] {
        assert!(text.contains(needle), "heartbeat step: {needle}");
    }
}

/// ATR-S3 (ATR4): every tour text is a resource, every tour resource is used, one exhaustive
/// step map.
#[test]
fn test_atr_s3_every_text_is_a_used_resource() {
    let strings = tour_resources();
    let card = kotlin_code(&read(TOUR_CARD));
    let ui = kotlin_code(&read(SOOS_APP));
    let mut used = BTreeSet::new();
    for src in [&card, &ui] {
        for (at, _) in src.match_indices("R.string.tour_") {
            let name: String = src[at + "R.string.".len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            assert!(strings.contains_key(&name), "{name} is in strings.xml");
            used.insert(name);
        }
    }
    assert_eq!(
        used,
        strings.keys().cloned().collect::<BTreeSet<_>>(),
        "every tour resource is used"
    );
    assert!(ui.contains("R.string.tour_replay"), "Feature tour button");
    assert!(!card.contains("Text(\""), "no literal text in TourCard.kt");
    assert!(
        !card.contains("text = \""),
        "no literal text in TourCard.kt"
    );

    let map = step_map(&card);
    for (entry, key, _) in STEPS {
        let arm = format!("TourStep.{entry} ->");
        assert_eq!(map.matches(&arm).count(), 1, "{arm} once");
        let at = map.find(&arm).unwrap();
        let texts = call_block(&map[at..], "TourTexts(");
        for suffix in ["title", "text"] {
            assert!(
                texts.contains(&format!("R.string.tour_{key}_{suffix},")),
                "{arm} uses tour_{key}_{suffix}"
            );
        }
        let (third, fourth) = if key == "intro" {
            ("tour_intro_note,".to_string(), "null".to_string())
        } else {
            (
                format!("tour_{key}_off,"),
                format!("R.string.tour_{key}_where"),
            )
        };
        assert!(texts.contains(&third), "{arm} uses {third}");
        assert!(texts.contains(&fourth), "{arm} uses {fourth}");
    }
    assert!(!map.contains("else ->"), "the step map is exhaustive");
}

/// ATR-S4 (ATR5): one Boolean `tour_seen` in `SettingsStore`, nothing else stored.
#[test]
fn test_atr_s4_storage_is_one_boolean_in_settings_store() {
    let store = read(SETTINGS_STORE);
    for needle in [
        "class SettingsStore(context: Context) : HeartbeatStore, TourStore",
        "const val FILE_NAME = \"soos_settings\"",
        "context.getSharedPreferences(FILE_NAME, Context.MODE_PRIVATE)",
        "const val KEY_TOUR_SEEN = \"tour_seen\"",
        "override var tourSeen: Boolean",
        "prefs.getBoolean(KEY_TOUR_SEEN, false)",
        "prefs.edit().putBoolean(KEY_TOUR_SEEN, value).apply()",
    ] {
        assert!(store.contains(needle), "SettingsStore needs {needle}");
    }
    assert_eq!(
        store.matches("KEY_TOUR_SEEN").count(),
        3,
        "one key, read and written once"
    );
    for (path, text) in kotlin_sources() {
        if path == SETTINGS_STORE {
            continue;
        }
        for forbidden in ["tour_seen", "getSharedPreferences", "KEY_TOUR_SEEN"] {
            assert!(!text.contains(forbidden), "{path} contains {forbidden}");
        }
    }
    for path in [TOUR_MODEL, TOUR_CARD] {
        let code = kotlin_code(&read(path));
        for forbidden in [
            "SharedPreferences",
            "DataStore",
            "openFileOutput",
            "FileOutputStream",
            "File(",
            "SQLite",
            "Room",
            "rememberSaveable",
            "Bundle",
            "SettingsStore",
            "localStorage",
        ] {
            assert!(!code.contains(forbidden), "{path} contains {forbidden}");
        }
    }
    let manifest = read(MANIFEST);
    assert!(manifest.contains("android:allowBackup=\"false\""));
    assert!(manifest.contains("android:dataExtractionRules=\"@xml/data_extraction_rules\""));
    let rules = read(EXTRACTION_RULES);
    assert_eq!(
        rules.matches("<exclude domain=\"sharedpref\" />").count(),
        2,
        "the settings file is excluded from cloud backup and device transfer"
    );
}

/// ATR-S5 (ATR6): the tour makes no request, switches nothing, reads no server or user data and
/// logs nothing.
#[test]
fn test_atr_s5_tour_requests_nothing_and_switches_nothing() {
    for path in [TOUR_MODEL, TOUR_CARD] {
        let code = kotlin_code(&read(path));
        for forbidden in [
            "SoosApi",
            "SharedApi",
            "api.",
            "HttpTransport",
            "UrlConnectionTransport",
            "HttpsURLConnection",
            "openConnection",
            "URL(",
            "https://",
            "Dispatchers",
            "launch",
            "coroutines",
            "Heartbeat",
            "heartbeat",
            "settings.",
            "Intent",
            "startActivity",
            "Uri",
            "StatusView",
            "AuthStateView",
            "AlertsView",
            "ApiResult",
            "credentialId",
            "pcUrl",
            "Log.",
            "println(",
            "print(",
            "Toast",
            "Clipboard",
            "LocalContext",
            "Activity",
        ] {
            assert!(!code.contains(forbidden), "{path} contains {forbidden}");
        }
    }
    let model = read(TOUR_MODEL);
    assert!(
        !model.contains("import android") && !model.contains("import androidx"),
        "the tour model is pure Kotlin"
    );
    let model = kotlin_code(&model);
    assert!(model.contains("interface TourStore"));
    assert!(model.contains("class TourController(private val store: TourStore)"));
    // The store is written by `close` only (Skip tour, system back, and Done through `next`).
    assert_eq!(model.matches("store.tourSeen = true").count(), 1);
    assert!(!model.contains("store.tourSeen = false"));
    // The UI hands the card nothing but the tour state and three callbacks.
    let ui = kotlin_code(&read(SOOS_APP));
    let call = call_block(&ui, "TourCard(");
    for forbidden in [
        "api",
        "heartbeat",
        "settings",
        "status",
        "auth",
        "launch",
        "alerts",
    ] {
        assert!(
            !call.contains(forbidden),
            "the TourCard call passes {forbidden}"
        );
    }
    assert_eq!(ui.matches("TourCard(").count(), 1, "one tour card");
    let card = kotlin_code(&read(TOUR_CARD));
    assert!(card.contains(
        "fun TourCard(state: TourState, onNext: () -> Unit, onBack: () -> Unit, onSkip: () -> Unit)"
    ));
}

/// ATR-S6 (ATR7): wiring, theme roles, no animation; manifest, permissions and dependencies
/// unchanged.
#[test]
fn test_atr_s6_wiring_theme_and_unchanged_manifest() {
    let ui = kotlin_code(&read(SOOS_APP));
    for needle in [
        "val tour = remember { TourController(settings) }",
        "var tourState by remember { mutableStateOf(TourState()) }",
        "val homeReady = auth?.authenticated == true",
        "val tourShown = tourState.open && homeReady",
        "if (homeReady && status != null) onTourOffer()",
        "BackHandler(enabled = tourShown) { onTourClose() }",
        "if (tourShown) {",
        "tour.offer(tourState, statusShown = true)",
        "tour.replay(tourState)",
        "tour.next(tourState)",
        "tour.back(tourState)",
        "tour.close(tourState)",
        "scroll.scrollTo(0)",
    ] {
        assert!(ui.contains(needle), "SoosApp.kt needs {needle}");
    }
    // The settings screen never receives the tour.
    let settings = call_block(&ui, "SettingsScreen(\n");
    assert!(
        !settings.to_ascii_lowercase().contains("tour"),
        "no tour on the settings screen"
    );
    // The Feature tour button is shown on the home screen only when the tour may show.
    assert!(
        squash(&ui).contains(
            "if (homeReady) { TextButton(onClick = onTourReplay) { \
             Text(stringResource(R.string.tour_replay)) } }"
        ),
        "the replay button sits inside the homeReady guard"
    );

    for path in [TOUR_MODEL, TOUR_CARD] {
        let code = kotlin_code(&read(path));
        for forbidden in [
            "animate",
            "Animat",
            "tween(",
            "spring(",
            "Crossfade",
            "Transition",
            "colorResource",
            "Color(",
            "Color.",
            "#FF",
        ] {
            assert!(!code.contains(forbidden), "{path} contains {forbidden}");
        }
    }
    let card = kotlin_code(&read(TOUR_CARD));
    assert!(card.contains("MaterialTheme.colorScheme."), "theme roles");
    assert!(
        card.contains("semantics { heading() }"),
        "the title is a heading"
    );

    // Manifest and permissions: exactly the heartbeat-era set (ANA9, ANA17, AHB1).
    let manifest = read(MANIFEST);
    let mut names: Vec<&str> = manifest
        .match_indices("<uses-permission android:name=\"")
        .map(|(at, head)| {
            let start = at + head.len();
            &manifest[start..start + manifest[start..].find('"').unwrap()]
        })
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "android.permission.INTERNET",
            "android.permission.POST_NOTIFICATIONS",
            "android.permission.RECEIVE_BOOT_COMPLETED",
        ]
    );
    assert_eq!(manifest.matches("<uses-permission").count(), 3);
    assert_eq!(manifest.matches("android:exported=\"true\"").count(), 1);
    assert_eq!(manifest.matches("<service").count(), 1);
    let mut prompts = 0;
    for (_, text) in kotlin_sources() {
        prompts += text.matches("RequestPermission()").count()
            + text.matches("requestPermissions(").count();
    }
    assert_eq!(
        prompts, 1,
        "still exactly one runtime prompt (the heartbeat)"
    );

    // No new dependency (ANA8): the catalog libraries and the app dependencies are unchanged.
    let catalog = read(CATALOG);
    let libraries =
        &catalog[catalog.find("[libraries]").unwrap()..catalog.find("[plugins]").unwrap()];
    let names: Vec<&str> = libraries
        .lines()
        .filter(|l| l.contains(" = {"))
        .map(|l| l.split(" = {").next().unwrap().trim())
        .collect();
    assert_eq!(
        names,
        [
            "androidx-core-ktx",
            "androidx-activity-compose",
            "androidx-compose-bom",
            "androidx-compose-ui",
            "androidx-compose-material3",
            "androidx-credentials",
            "kotlinx-coroutines-android",
            "junit",
            "org-json",
        ]
    );
    let gradle = read(APP_GRADLE);
    assert_eq!(gradle.matches("    implementation(").count(), 7);
    assert_eq!(gradle.matches("    testImplementation(").count(), 2);
    assert!(exists(JVM_TEST), "{JVM_TEST}");
}

/// ATR-S7 (ATR8): ADR, matrix rows, operator documentation and walkthrough.
#[test]
fn test_atr_s7_documentation_adr_matrix_and_walkthrough() {
    let decisions = read(DECISIONS);
    let adr = decisions
        .lines()
        .find(|l| l.contains(ADR_TITLE))
        .unwrap_or_else(|| panic!("{DECISIONS}: ADR `{ADR_TITLE}`"));
    for needle in [
        "#427",
        "#363",
        "#357",
        "tour_seen",
        "soos_settings",
        "app.js",
        "exit 78",
        "no new permission",
        "ATR1",
    ] {
        assert!(adr.contains(needle), "the ADR mentions {needle}");
    }
    let doc = read(DOC);
    let start = doc.find("## 2j. Native Android app").expect("section 2j");
    let end = start + doc[start..].find("\n## 2k. ").expect("section 2k follows");
    let section = &doc[start..end];
    let tour = &section[section.find("### Feature tour").expect("§2j Feature tour")..];
    for needle in [
        "Feature tour",
        "Skip tour",
        "Done",
        "tour_seen",
        "soos_settings",
        "app.js",
        "PC offline alerts",
        "ATR9",
    ] {
        assert!(tour.contains(needle), "§2j Feature tour mentions {needle}");
    }
    let k = &doc[end..];
    let k = &k[..k[3..].find("\n## ").map_or(k.len(), |e| e + 3)];
    assert!(
        !k.contains("the app has no tour yet"),
        "§2k no longer says the app has no tour"
    );
    let matrix = read(MATRIX);
    for row in 1..=9 {
        assert!(
            matrix
                .lines()
                .any(|l| l.starts_with(&format!("| ATR{row} |"))),
            "matrix row ATR{row}"
        );
    }
    for row in 2..=8 {
        let line = matrix
            .lines()
            .find(|l| l.starts_with(&format!("| ATR{row} |")))
            .unwrap();
        assert!(
            line.contains("android_tour_contract::test_atr_s"),
            "ATR{row} cites the contract"
        );
    }
    assert!(exists(WALKTHROUGH), "{WALKTHROUGH}");
    let walkthrough = read(WALKTHROUGH);
    for needle in ["#427", "#363", "tour_seen", "app.js"] {
        assert!(
            walkthrough.contains(needle),
            "walkthrough 230 mentions {needle}"
        );
    }
}
