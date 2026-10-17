//! Static contracts of the acknowledgement feedback of the alerts card in the native Android
//! app (GitHub #420, part of #357, follow-up of #404 / PR #406; matrix AAF1–AAF4).
//!
//! The Android tree cannot be built on the development host: these checks pin the wiring of
//! `ui/SoosApp.kt` to the pure helper `alerts/AlertsFeedback.kt`, whose decisions the JVM unit
//! tests of `AlertsFeedbackTest.kt` cover in the CI job `Android (debug build)`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use crate::remote_companion_contract::{exists, read};

const HELPER: &str = "android/app/src/main/java/org/soos/remote/alerts/AlertsFeedback.kt";
const UI: &str = "android/app/src/main/java/org/soos/remote/ui/SoosApp.kt";
const JVM_TEST: &str = "android/app/src/test/java/org/soos/remote/AlertsFeedbackTest.kt";
const DOC: &str = "Docs/REMOTE_COMPANION.md";
const MATRIX: &str = "AI/VERIFICATION_MATRIX.md";
const WALKTHROUGH: &str = "AI/walkthroughs/227_android_alerts_feedback.md";

/// The body of the first function of `src` whose declaration starts with `decl`, from its
/// opening brace to the matching closing brace (both included).
fn function_body<'a>(src: &'a str, decl: &str) -> &'a str {
    let start = src.find(decl).unwrap_or_else(|| panic!("{decl} declared"));
    let open = start + src[start..].find('{').expect("opening brace");
    let mut depth = 0usize;
    for (offset, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &src[open..=open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("{decl} has no closing brace");
}

/// The lines of `body` that are code (no blank line, no `//` comment line).
fn code_lines(body: &str) -> Vec<&str> {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//") && *l != "{" && *l != "}")
        .collect()
}

/// AAF1: the feedback decision is a pure helper (no Android, resource, network or logging
/// API), the token mapping lives there, and the JVM tests cover the four required paths.
#[test]
fn test_aaf_s1_pure_helper_and_jvm_tests() {
    assert!(exists(HELPER), "{HELPER}");
    let helper = read(HELPER);
    assert!(helper.starts_with("package org.soos.remote.alerts\n"));
    for line in helper.lines().filter(|l| l.starts_with("import ")) {
        assert!(
            line.starts_with("import org.soos.remote.net."),
            "the helper imports only the parsed API types: {line}"
        );
    }
    for forbidden in [
        "android.",
        "androidx.",
        "R.string",
        "Context",
        "suspend ",
        "Dispatchers",
        "Log.",
        "println(",
        "System.",
    ] {
        assert!(
            !helper.contains(forbidden),
            "the helper contains {forbidden}"
        );
    }
    for needle in [
        "enum class AckFeedback",
        "enum class RefreshOutcome",
        "AUTHENTICATED",
        "SIGNED_OUT",
        "UNREACHABLE",
        "data class FeedbackDisplay(val card: AckFeedback?, val notice: AckFeedback?)",
        "fun ofAck(result: ApiResult<*>): AckFeedback",
        "fun needsRefresh(result: ApiResult<*>): Boolean",
        "fun afterRefresh(carried: AckFeedback?, outcome: RefreshOutcome): FeedbackDisplay",
    ] {
        assert!(helper.contains(needle), "the helper needs {needle}");
    }
    for token in [
        "\"stale_view\"",
        "\"rate_limited\"",
        "\"alerts_disabled\"",
        "\"unavailable\"",
        "\"bad_request\"",
        "\"login_required\"",
    ] {
        assert!(helper.contains(token), "the helper maps {token}");
    }

    assert!(exists(JVM_TEST), "{JVM_TEST}");
    let test = read(JVM_TEST);
    for name in [
        "fun aSuccessfulAckThenARefreshClearsTheFeedback()",
        "fun staleViewKeepsItsTextAfterTheFollowUpRefresh()",
        "fun loginRequiredThenASignedOutRefreshStillShowsTheLoginRequiredText()",
        "fun anUnreachableAckShowsTheUnreachableText()",
    ] {
        assert!(test.contains(name), "AlertsFeedbackTest needs {name}");
    }
}

/// AAF2: every refresh drops the previous feedback before any request; the acknowledgement's
/// follow-up refresh carries its refusal; a signed-out refresh routes the login-required text to
/// the screen message; only fixed app strings are shown; the Acknowledge rule keeps web parity.
#[test]
fn test_aaf_s2_home_screen_wiring() {
    let ui = read(UI);
    assert!(ui.contains("import org.soos.remote.alerts.AlertsFeedback"));

    let refresh = function_body(&ui, "suspend fun refresh(carried: AckFeedback? = null)");
    let lines = code_lines(refresh);
    assert_eq!(
        lines.first().copied(),
        Some("ackFeedback = null"),
        "refresh() clears the feedback first"
    );
    let cleared = refresh.find("ackFeedback = null").unwrap();
    let first_call = refresh.find("api.").expect("refresh() calls the PC");
    assert!(
        cleared < first_call,
        "the feedback is cleared before any request"
    );
    let decided = refresh
        .find("AlertsFeedback.afterRefresh(carried, outcome)")
        .expect("refresh() asks the helper");
    assert!(decided > refresh.find("loadAlerts()").unwrap());
    assert!(refresh.contains("RefreshOutcome.UNREACHABLE"));
    assert!(refresh.contains("RefreshOutcome.AUTHENTICATED"));
    assert!(refresh.contains("RefreshOutcome.SIGNED_OUT"));
    assert!(refresh.contains("ackFeedback = shown.card"));
    assert!(
        refresh.contains("shown.notice?.let { message = activity.getString(ackFeedbackText(it)) }"),
        "the login-required notice reaches the screen message as a fixed string"
    );
    assert_eq!(
        refresh.matches("return").count(),
        0,
        "refresh() has a single exit, so the feedback decision always runs"
    );

    let acknowledge = function_body(&ui, "suspend fun acknowledge(view: AlertsView)");
    assert!(acknowledge.contains("val feedback = AlertsFeedback.ofAck(result)"));
    assert!(acknowledge
        .contains("if (AlertsFeedback.needsRefresh(result)) refresh(carried = feedback)"));
    assert!(
        !ui.contains("ackRefusalText"),
        "the token mapping lives in the helper"
    );

    let load = function_body(&ui, "suspend fun loadAlerts()");
    assert!(
        !load.contains("ackFeedback"),
        "loadAlerts() leaves the feedback to refresh()"
    );

    // Every other refresh call site carries no acknowledgement.
    let carried_calls = ui.matches("refresh(carried =").count();
    assert_eq!(carried_calls, 1, "only acknowledge() carries a feedback");
    assert!(
        ui.matches("refresh()").count() >= 5,
        "the plain refresh call sites"
    );
    assert_eq!(
        ui.matches("refresh(").count(),
        ui.matches("refresh()").count() + 2,
        "besides the declaration and acknowledge(), every refresh call is plain"
    );

    let mapping = function_body(
        &ui,
        "private fun ackFeedbackText(feedback: AckFeedback): Int",
    );
    for (entry, string) in [
        ("AckFeedback.DONE", "R.string.alerts_ack_done"),
        ("AckFeedback.STALE_VIEW", "R.string.alerts_ack_stale_view"),
        (
            "AckFeedback.RATE_LIMITED",
            "R.string.alerts_ack_rate_limited",
        ),
        ("AckFeedback.DISABLED", "R.string.alerts_ack_disabled"),
        ("AckFeedback.UNAVAILABLE", "R.string.alerts_ack_unavailable"),
        ("AckFeedback.BAD_REQUEST", "R.string.alerts_ack_bad_request"),
        (
            "AckFeedback.LOGIN_REQUIRED",
            "R.string.alerts_ack_login_required",
        ),
        ("AckFeedback.REFUSED", "R.string.alerts_ack_refused"),
        ("AckFeedback.UNREACHABLE", "R.string.alerts_ack_unreachable"),
    ] {
        assert!(
            mapping.contains(&format!("{entry} -> {string}")),
            "{entry} shows {string}"
        );
    }
    assert!(!mapping.contains("else"), "the mapping is exhaustive");

    // The Acknowledge rule is unchanged and documented as the web app's rule.
    let card = function_body(&ui, "private fun AlertsCard(");
    assert!(card.contains("if (pending && view.epoch != null) {"));
    assert!(card.contains("crates/remote/assets/app.js"));
    assert!(
        ui.contains("    feedback: AckFeedback?,\n    busy: Boolean,"),
        "AlertsCard takes the typed feedback"
    );
}

/// AAF3: §2j documents the feedback rules and the owner checks, matrix rows AAF1–AAF4, and
/// walkthrough 227.
#[test]
fn test_aaf_s3_docs_matrix_walkthrough() {
    let doc = read(DOC);
    let start = doc.find("## 2j. Native Android app").expect("section 2j");
    let end = start + doc[start..].find("\n## 3.").unwrap();
    let section = &doc[start..end];
    for needle in [
        "Every refresh clears the acknowledgement message",
        "Your session ended. Sign in again.",
        "AAF4",
    ] {
        assert!(section.contains(needle), "§2j must mention {needle}");
    }
    let matrix = read(MATRIX);
    for row in 1..=4 {
        assert!(
            matrix.contains(&format!("| AAF{row} |")),
            "matrix row AAF{row}"
        );
    }
    assert!(exists(WALKTHROUGH), "{WALKTHROUGH}");
    let walkthrough = read(WALKTHROUGH);
    for needle in ["#420", "#357", "#404", "AlertsFeedback", "app.js"] {
        assert!(
            walkthrough.contains(needle),
            "walkthrough 227 must mention {needle}"
        );
    }
}
