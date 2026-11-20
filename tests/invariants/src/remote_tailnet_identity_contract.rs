//! Static contracts of security review 2026-11-10 batch 2 (GitHub #479; ADR "[2026-11-20]
//! Per-Login Tailnet Limiters, Sign-Out Push Clean-Up and the Tailnet Local Listener
//! Residual"; spec `AI/architect_spec_remote_tailnet_identity.md`; rows RTI4, RTI6-RTI9):
//!
//! - RTI4 (RWEB-02, #616): *Sign out* drops this phone's push subscription, browser and PC,
//!   before the logout request, best effort and bounded;
//! - RTI6 (PTSN-02, #583): the revocation steps name the notification devices;
//! - RTI7 (RIDN-01, #601): the trust model states the tailnet local-listener gap;
//! - RTI8 (RPSHB-01, #605): the sender isolation sentences state the TCP path;
//! - RTI9: per-login limiter documentation, ADR, walkthrough.
//!
//! The behaviour of the sign-out clean-up is proven by the browser bench (RTI5,
//! `tests/browser/specs/signin.spec.mjs`); the limiter behaviour by
//! `crates/remote/tests/tailnet_caller_tests.rs` (RTI1-RTI3).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use crate::remote_companion_contract::{exists, read};

const APP_JS: &str = "crates/remote/assets/app.js";
const REMOTE_DOC: &str = "Docs/REMOTE_COMPANION.md";
const DECISIONS: &str = "AI/DECISIONS.md";
const WALKTHROUGH: &str = "AI/walkthroughs/258_remote_tailnet_identity.md";
const ADR_TITLE: &str = "[2026-11-20] Per-Login Tailnet Limiters, Sign-Out Push Clean-Up and the Tailnet Local Listener Residual";

/// The source of `function <name>(` in `code`, up to its matching closing brace (`None` when
/// absent). The functions checked here hold no brace inside a string literal.
fn js_function<'a>(code: &'a str, name: &str) -> Option<&'a str> {
    let start = code.find(&format!("\nfunction {name}("))?;
    let open = start + code[start..].find('{')?;
    let mut depth = 0usize;
    for (offset, ch) in code[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&code[start..=open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The markdown section that starts with the heading line `heading`, up to the next `## `.
fn section<'a>(doc: &'a str, heading: &str) -> &'a str {
    let start = doc
        .find(&format!("\n{heading}\n"))
        .unwrap_or_else(|| panic!("{REMOTE_DOC} has the heading {heading}"));
    let body = &doc[start + 1..];
    let end = body[heading.len()..]
        .find("\n## ")
        .map_or(body.len(), |i| i + heading.len());
    &body[..end]
}

/// RTI4 (RWEB-02, GitHub #616): `requestLogout` first runs `dropPushSubscription`, which takes
/// this browser's own subscription, starts its `unsubscribe()` in the background (never awaited,
/// failures ignored) and posts its endpoint to the unsubscribe route; it adds no timer (the timer
/// set of `app.js` is pinned by RMO-S9); the logout request is always sent afterwards and the
/// sign-in screen always shown.
#[test]
fn test_rti4_sign_out_drops_the_push_subscription_first() {
    let app = read(APP_JS);
    let drop = js_function(&app, "dropPushSubscription")
        .expect("app.js defines function dropPushSubscription");
    for needle in [
        "pushSupported()",
        "registerServiceWorker()",
        "getSubscription()",
        "subscription.unsubscribe().catch(",
        "return postJson(PUSH_UNSUBSCRIBE_PATH, \"push-unsubscribe\", { endpoint: endpoint })",
        ".catch(",
    ] {
        assert!(drop.contains(needle), "dropPushSubscription: {needle}");
    }
    assert!(
        !drop.contains("return subscription.unsubscribe()")
            && !drop.contains("subscription.unsubscribe().then("),
        "the browser unsubscribe is never awaited (a slow push service never delays the logout)"
    );
    assert!(
        drop.find(".unsubscribe()").unwrap() < drop.find("PUSH_UNSUBSCRIBE_PATH").unwrap(),
        "the browser subscription is dropped first, then the PC record"
    );
    for forbidden in ["LOGOUT_PATH", "setTimeout(", "setInterval("] {
        assert!(
            !drop.contains(forbidden),
            "dropPushSubscription must not contain {forbidden}"
        );
    }

    let logout = js_function(&app, "requestLogout").expect("app.js defines requestLogout");
    let cleanup = logout
        .find("dropPushSubscription()")
        .expect("requestLogout runs the push clean-up");
    let request = logout
        .find("postJson(LOGOUT_PATH, \"logout\")")
        .expect("requestLogout posts the logout");
    assert!(cleanup < request, "the clean-up runs before the logout");
    assert!(
        logout.contains(".finally(") && logout.contains("showLogin(\"Signed out\")"),
        "the sign-in screen is always shown"
    );
}

/// RTI6 (PTSN-02, GitHub #583): the revocation steps of section 2a name the notification
/// devices (`push list`, `push remove N`, `push reset`), and section 8 says that every
/// signed-in caller can register a device.
#[test]
fn test_rti6_revocation_names_the_notification_devices() {
    let doc = read(REMOTE_DOC);
    let unlock = section(
        &doc,
        "## 2a. Remote unlock (off by default, on after `scripts/setup.sh`)",
    );
    let revoke = &unlock[unlock
        .find("To revoke the capability at once")
        .expect("section 2a revocation paragraph")..];
    for needle in [
        "`soos-remote push list`",
        "`soos-remote push remove N`",
        "`soos-remote push reset`",
        "#583",
    ] {
        assert!(revoke.contains(needle), "section 2a revocation: {needle}");
    }
    let residual = section(&doc, "## 8. Residual limitations");
    assert!(
        residual.contains(
            "any signed-in caller (an allowed tailnet identity or a Funnel session) can register a device"
        ),
        "section 8 names every signed-in caller"
    );
    assert!(
        !residual.contains("a stolen Funnel session can register a device"),
        "the narrower wording is gone"
    );
}

/// RTI7 (RIDN-01, GitHub #601): section 2 states that `tailscale serve` stamps the owner's
/// identity on connections other local accounts open to the PC's own Tailscale address, what
/// they reach, what stays protected, and the interim advice.
#[test]
fn test_rti7_trust_model_states_the_local_listener_gap() {
    let doc = read(REMOTE_DOC);
    let trust = section(
        &doc,
        "## 2. Trust model (read this before exposing anything)",
    );
    let gap = &trust[trust
        .find("**Known gap: other local accounts")
        .expect("section 2 names the local-account gap")..];
    for needle in [
        "#601",
        "own Tailscale addresses",
        "other local account",
        "push sender",
        "remote lock",
        "passkey assertion",
        "userspace-networking",
        "`soos-remote push list`",
    ] {
        assert!(gap.contains(needle), "section 2 gap paragraph: {needle}");
    }
}

/// RTI8 (RPSHB-01, GitHub #605): the push sender isolation sentences say that the file-system
/// sandbox does not stop TCP to the PC's own Tailscale address, loopback or the LAN, and that
/// the egress filter is an open owner decision.
#[test]
fn test_rti8_sender_isolation_states_the_tcp_path() {
    let doc = read(REMOTE_DOC);
    let push = section(
        &doc,
        "## 2d. Push notifications (off by default, on after `scripts/setup.sh`)",
    );
    let tcp = &push[push
        .find("These are file-system limits")
        .expect("section 2d states the limit of the sandbox")..];
    for needle in [
        "over TCP",
        "own Tailscale address",
        "loopback",
        "`IPAddressDeny=`",
        "#605",
    ] {
        assert!(tcp.contains(needle), "section 2d: {needle}");
    }
}

/// RTI9: the limiter wording names one key per allowed tailnet login, the sign-out clean-up is
/// documented, and the ADR and the walkthrough exist with the deferred owner questions.
#[test]
fn test_rti9_documentation_adr_and_walkthrough() {
    let doc = read(REMOTE_DOC);
    let funnel = section(
        &doc,
        "## 2b. Internet access through Tailscale Funnel, and passkeys",
    );
    assert!(
        funnel.contains("per limiter key (each allowed tailnet login,"),
        "section 2b: one key per allowed tailnet login"
    );
    assert!(
        funnel.contains("**Sign out** also turns off this phone's notifications"),
        "section 2b documents the sign-out clean-up"
    );
    let decisions = read(DECISIONS);
    let adr = &decisions[decisions.find(ADR_TITLE).expect("ADR present")..];
    let adr = &adr[..adr.find("\n* **[").unwrap_or(adr.len())];
    for needle in [
        "#584",
        "#616",
        "#583",
        "#601",
        "#605",
        "test_rmc_s21_forwarded_for_is_read_only_by_client_hint",
        "test_pss1_system_units_are_exact",
        "MAX_PENDING_TAILNET_CHALLENGES",
        "RMO-S9",
    ] {
        assert!(adr.contains(needle), "ADR: {needle}");
    }
    assert!(exists(WALKTHROUGH), "{WALKTHROUGH}");
}
