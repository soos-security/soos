# Walkthrough 227 — Alerts Acknowledgement Feedback in Step With the Displayed View (Android)

- **Date**: 2026-10-17
- **Issue**: GitHub #420, sub-issue of the epic #357 (native Android app), follow-up of #404 (PR #406, `27d1e7c`).
  GitHub-only: no `AI/BACKLOG.md` entry, not registered in `scripts/sync_issue.py`; the commit carries
  `Closes #420` and `Refs #357`. **Branch**: `fix/android-alerts-feedback`.
- **ADR**: none. The change is a client bug fix inside the decisions of "[2026-10-17] Failed-Password Alerts in the
  Native Android App" (fixed `strings.xml` texts only, never server text, no server change); it decides nothing new.
- **Documents**: user documentation `Docs/REMOTE_COMPANION.md` §2j (feedback rules, owner check step 5).
- **Matrix criteria**: AAF1–AAF4 (AAF1 pending the CI run of `Android (debug build)`, AAF4 pending the owner hardware
  check).

## 1. Context & Objectives

The alerts card of the native app (#404) shows a fixed feedback line after *Acknowledge*. Two defects came from how
`HomeScreen` in `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt` managed that line (`ackFeedback`):

1. **Stale "Acknowledged."** `loadAlerts()` and `refresh()` never reset it; only `clearAlerts()` did. After a
   successful acknowledgement, a later *Refresh*, pull to refresh or sign-in refresh kept "Acknowledged." under a newer
   view that could hold new attempts.
2. **`alerts_ack_login_required` never shown.** A refused acknowledgement calls `refresh()`. A signed-out auth state
   sent `refresh()` to `clearAlerts()`, which nulled the feedback and hid the whole card (`view == null && !failed`),
   so "Your session ended. Sign in again." was lost.

Goal: fix both on the client only, with the decision testable on the JVM.

## 2. Architect Design

- **`alerts/AlertsFeedback.kt`** (new, package `org.soos.remote.alerts`, pure Kotlin, imports only
  `org.soos.remote.net.ApiResult`):
  - `enum class AckFeedback` (`DONE`, `STALE_VIEW`, `RATE_LIMITED`, `DISABLED`, `UNAVAILABLE`, `BAD_REQUEST`,
    `LOGIN_REQUIRED`, `REFUSED`, `UNREACHABLE`): the fixed feedback, mapped to `strings.xml` by the UI.
  - `enum class RefreshOutcome` (`AUTHENTICATED`, `SIGNED_OUT`, `UNREACHABLE`): what a refresh found.
  - `data class FeedbackDisplay(card, notice)`: `card` under the alerts card, `notice` as the screen message.
  - `AlertsFeedback.ofAck(result)`: the token mapping moved from the UI (`ackRefusalText`), unchanged.
  - `AlertsFeedback.needsRefresh(result)`: every answer but `200` (unchanged rule: never acknowledge blindly).
  - `AlertsFeedback.afterRefresh(carried, outcome)`: signed in, the carried feedback stays on the card; signed out,
    only `LOGIN_REQUIRED` moves to the notice; unreachable, nothing (the screen already says the PC is unreachable).
    `carried` is `null` for every refresh but the acknowledgement's follow-up, so such a refresh shows no feedback.
- **`ui/SoosApp.kt`**: `refresh(carried: AckFeedback? = null)` clears `ackFeedback` before any request, computes a
  `RefreshOutcome` in a single-exit `if`/`else` (the early `return` of the unreachable path is gone, so the decision
  always runs and `busy` is always reset), then applies `afterRefresh`: `ackFeedback = shown.card` and
  `shown.notice?.let { message = activity.getString(ackFeedbackText(it)) }`. `acknowledge()` calls
  `refresh(carried = feedback)` on a refusal. `ackFeedbackText` is an exhaustive `when` over `AckFeedback`.
- **Dropped hint ("hide Acknowledge unless active")**: based on a wrong premise and not applied. Server
  `RemoteAlerts::acknowledge` (`crates/remote/src/alerts.rs`) refuses `503 unavailable` only when its lock is
  poisoned or no book exists, and then `RemoteAlerts::view` is `AlertsView::empty` (no epoch, zero counts), so the
  button is already hidden. With a book, a `starting` or `unavailable` state with pending counts acknowledges with
  `200`. `disabled` is excluded by `pending` and answered `403 alerts_disabled` by `alerts_ack` in `server.rs`. The
  web app (`crates/remote/assets/app.js`, `alertsAckButton.hidden`) shows the button whenever counts are pending and
  an epoch exists. The rule is kept and a comment in `AlertsCard` documents the parity.

## 3. Plan Self-Check

- Server untouched: `GET /api/alerts`, `POST /api/alerts/ack`, tokens and status codes unchanged (verified against
  `server.rs`: an expired Funnel session gets `403 login_required` from the Funnel gate before the route).
- Existing invariants kept: `test_ana_s14_alerts_contract` (every `R.string.alerts_*` defined, at least 10
  references, no `.message`/`localizedMessage`, no bare `Text(result`), `test_ana_s8_kotlin_hygiene` (no logging),
  `test_ahb_s3_kotlin_heartbeat_contract` (shared client). The field is named `notice`, not `message`, to keep the
  `.message` ban meaningful.
- No new dependency, permission, manifest entry or string resource.

## 4. Tester Contract

- **JVM** (`android/app/src/test/java/org/soos/remote/AlertsFeedbackTest.kt`, CI only):
  `aSuccessfulAckThenARefreshClearsTheFeedback`, `staleViewKeepsItsTextAfterTheFollowUpRefresh`,
  `loginRequiredThenASignedOutRefreshStillShowsTheLoginRequiredText`, `anUnreachableAckShowsTheUnreachableText`,
  `everyRefusalTokenSelectsAFixedFeedback`, `aSignedOutRefreshShowsOnlyTheLoginRequiredNotice`. Red phase: the
  package `org.soos.remote.alerts` did not exist (compile failure); there is no JDK on the development host.
- **Static** (`tests/invariants/src/android_alerts_feedback_contract.rs`): `test_aaf_s1_pure_helper_and_jvm_tests`,
  `test_aaf_s2_home_screen_wiring`, `test_aaf_s3_docs_matrix_walkthrough`. Red phase confirmed locally: all three
  failed (helper missing, `SoosApp.kt` not importing it, §2j text missing).
- No existing test was modified.

## 5. Auditor Constraints

- C1: only fixed app strings are displayed; a server token only selects an `AckFeedback`.
- C2: the helper is pure (no Android, resource, coroutine, I/O or logging API).
- C3: `refresh()` has a single exit so `busy` is always reset and the feedback decision always runs.
- C4: the feedback is cleared before the first request of every refresh.
- C5: no `.message`, `localizedMessage` or exception text in `SoosApp.kt`; no logging anywhere.
- C6: the *Acknowledge* rule is unchanged (web parity).

## 6. Implementation

See section 2. `clearAlerts()` still resets the feedback (harmless, the refresh decision runs after it).

## 7. Candid Review

Not run in this session (the run stops at a local commit; no `AI/candid_review_report.md` is written).

## 8. Verification Results

- `cargo test -p soos-invariants --lib android -- --test-threads=3`: all Android invariants pass, including the three
  new AAF tests.
- `cargo fmt --check` and `cargo clippy -p soos-invariants --all-targets -- -D warnings`: clean.
- JVM tests: pending the CI job `Android (debug build)` (AAF1).

## 9. Known Limitations / Follow-ups

- AAF4 needs the owner's phone (Funnel session expiry path).
- The login-required screen message stays until the next sign-in or authenticated refresh, like the other screen
  messages.
