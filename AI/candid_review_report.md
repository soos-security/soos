# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `fix/android-alerts-feedback`
- **Base (merge-base)**: `74782b8`
- **Reviewed-Diff-Fingerprint**: `c88fbebee96e6346ff9803c3e270bdea812b88db6de871afed267012e2b07ef4`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/227_android_alerts_feedback.md`,
  `Docs/REMOTE_COMPANION.md`, `android/app/src/main/java/org/soos/remote/alerts/AlertsFeedback.kt`,
  `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt`,
  `android/app/src/test/java/org/soos/remote/AlertsFeedbackTest.kt`,
  `tests/invariants/src/android_alerts_feedback_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

Independent second review (no prior context) of the Android-only client fix for GitHub #420. The
acknowledgement feedback of the alerts card becomes a typed `AckFeedback` decided by the pure
helper `AlertsFeedback` (`ofAck`, `needsRefresh`, `afterRefresh`). `HomeScreen.refresh()` clears
the feedback before its first request, has a single exit, and re-applies only the feedback carried
by the acknowledgement's follow-up refresh; a `login_required` refusal followed by a signed-out
refresh is routed to the screen message as a fixed `strings.xml` text. No server, protocol, PAM,
dependency, permission or string-resource change. Test side purely additive. Re-run locally:
`cargo test -p soos-invariants --lib -- --test-threads=3` 774 passed / 0 failed;
`cargo clippy -p soos-invariants --all-targets -- -D warnings` clean; `cargo fmt --check -p
soos-invariants` clean (scoped env: `LIBONNXRUNTIME_NO_PKG_CONFIG=1`, dedicated
`CARGO_TARGET_DIR`, `CARGO_BUILD_JOBS=3`). The Kotlin JVM tests cannot run on this host (no JDK);
they run only in CI `Android (debug build)` (AAF1 is honestly marked pending). No CRITICAL or
MAJOR finding; two MINOR notes and one SUGGESTION.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch` and
`git diff $(git merge-base origin/main HEAD) HEAD -- tests 'crates/*/tests' android/app/src/test`:

- Files: `android/app/src/test/java/org/soos/remote/AlertsFeedbackTest.kt` (new, +121),
  `tests/invariants/src/android_alerts_feedback_contract.rs` (new, +255),
  `tests/invariants/src/lib.rs` (+8: one `mod` declaration with its doc comment).
- Removed or changed lines in test paths: **0** (`3 files changed, 384 insertions(+)`).
- Removed/changed assertions, `#[test]`, `@Test`: none.
- New escape hatches (`#[ignore]`, `@Ignore`, `should_panic`): none.
- Inline `mod tests` changes: none.

No pre-existing assertion was removed, weakened or changed. No contract migration needed.

## 3. Deep Reasoning Audit

### Logic & Architecture

Scenarios attempted:

- *Successful ack, then Refresh / pull / sign-in / sign-out / Lock PC / screen opening*: every
  call site (`LaunchedEffect`, `PullToRefreshBox.onRefresh`, Refresh button, `signIn()`, Sign out,
  Lock PC) calls plain `refresh()`, so `carried = null`, `ackFeedback = null` first and
  `afterRefresh(null, _) = NONE` for every outcome. "Acknowledged." cannot survive. PASS.
- *`stale_view` / `rate_limited` refusal*: `acknowledge()` -> `refresh(carried = feedback)` ->
  `AUTHENTICATED` -> `card = carried`. Text kept under the fresh view. PASS.
- *`login_required` refusal over Funnel, session expired*: the Funnel gate (`server.rs` near line
  1308) answers `403 login_required`; the follow-up refresh finds `authenticated = false` (or a
  refused auth state, `auth = null`) -> `SIGNED_OUT` -> `clearAlerts()` then
  `notice = LOGIN_REQUIRED` -> `message = activity.getString(R.string.alerts_ack_login_required)`.
  The message line renders outside the authenticated block, so it is visible while the card is
  hidden. PASS.
- *Unreachable ack, PC still unreachable*: `UNREACHABLE` -> `NONE`; the screen's own unreachable
  message is set in that branch. PASS.
- *Stale notice*: the login-required message is cleared by `signIn()` (`message = null`) and by any
  authenticated refresh whose status call succeeds. PASS (documented in walkthrough §9).
- *Single exit*: the old early `return` is gone; `busy = false` always runs; the invariant pins
  zero `return` in `refresh()`. PASS.
- *Exhaustiveness*: `ackFeedbackText` is a `when` over all nine `AckFeedback` entries with no
  `else`; `ofAck` covers the sealed `ApiResult` (`sealed interface`, `SoosApi.kt:6`). PASS.
- *Token mapping unchanged*: the seven branches of the removed `ackRefusalText` map 1:1 to the new
  enum and back to the same string ids. PASS.
- *Acknowledge rule*: unchanged (`pending && view.epoch != null`). Comment claims verified:
  `RemoteAlerts::acknowledge` (`alerts.rs:1035`) returns `Unavailable` only on a poisoned lock or
  no book, and `view()` then yields `AlertsView::empty` (`epoch: None`, zero counts,
  `alerts.rs:645`); `book.acknowledge` does not check the state, so `starting`/`unavailable` with a
  book answers 200; `alerts_ack` returns `403 alerts_disabled` when no runtime (`server.rs:2151`);
  web app `alertsAckButton.hidden = !(wrong > 0 || lockedOut > 0) || typeof view.epoch !==
  "string"` (`app.js:820`). PASS.
- *Doc claims*: §2j owner step 5 "more than 15 minutes" matches `WEB_SESSION_IDLE_MS = 900_000`
  (`crates/remote/src/lib.rs:236`); "with the PC offline alerts off" is justified because the
  heartbeat reuses the shared in-memory client session (`SoosApp.kt` comment,
  `Heartbeat.classify(ApiResult<StatusView>)`) and every Funnel API request uses `Touch::Refresh`
  (`server.rs:1302`). Matrix "rows ANA18–ANA21 above" exist at `VERIFICATION_MATRIX.md:2682-2685`.
  PASS.

### PAM Concurrency & Deadlines

No change under `crates/pam` or any Rust production crate. N/A, PASS.

### Panic Safety & Fail-Closed

Kotlin: no `!!`, no `error()`, no `throw` introduced; `activity.getString` takes a compile-time
resource id. No path toward any authorization decision (the server owns acknowledgement; the
client never acknowledges blindly, `needsRefresh` keeps the "everything but 200 refreshes" rule).
The Rust invariant file uses `unwrap`/`expect`/`panic!` only under a scoped `#![allow]` with
`reason`, as the other invariant modules do. PASS.

### Test Integrity & Anti-Weakening

Section 2: zero pre-existing line touched. New JVM tests would fail against plausible wrong
implementations (e.g. `afterRefresh` returning `carried` on `SIGNED_OUT`, or keeping `DONE` after
a plain refresh: `aSuccessfulAckThenARefreshClearsTheFeedback` iterates every outcome with
`carried = null`; `aSignedOutRefreshShowsOnlyTheLoginRequiredNotice` iterates every
`AckFeedback`). The static invariants pin the wiring that the JVM tests cannot reach (clear
before the first `api.` call, decision after `loadAlerts()`, exactly one carried call site, plain
call count arithmetic). Existing `test_ana_s14_alerts_contract`, `test_ana_s8_kotlin_hygiene` and
the heartbeat contract still pass in the 774-test run. Kotlin compile-level details checked by
reading: integer literals into `Long` parameters of `AlertsView` are accepted; `entries` needs
Kotlin 1.9+, the project uses 2.2.21. PASS.

### Memory, Bounds & Secrets

No new I/O, buffer, logging or storage. Only fixed app strings are displayed; a server token only
selects an enum value. No password, frame or embedding involved. PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/`, `.githooks/`, Gradle or manifest change. N/A,
PASS.

### English-Only Policy

All code, comments, docs, walkthrough and commit subjects in English. Non-ASCII limited to
typographic dashes, `§` and the matrix status glyphs already used in the file. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `tests/invariants/src/android_alerts_feedback_contract.rs:55,120,225` — the doc
  comments label `test_aaf_s1` as "AAF1", `test_aaf_s2` as "AAF2" and `test_aaf_s3` as "AAF3",
  while `AI/VERIFICATION_MATRIX.md` maps AAF1 to the JVM tests, AAF2 to `test_aaf_s1` and AAF3 to
  `test_aaf_s2` + `test_aaf_s3`. Traceability only; align the doc comments with the matrix in a
  follow-up (comment-only, no assertion change).
- **[MINOR]** `AI/walkthroughs/227_android_alerts_feedback.md` §7 — says the candid review was not
  run and no report is written, which is stale now that `AI/candid_review_report.md` exists for
  this fingerprint. Update when the walkthrough is next touched.
- **[SUGGESTION]** `AI/walkthroughs/227_android_alerts_feedback.md` — numbers 224–226 are absent
  on this branch and on `origin/main`; presumably reserved by other open branches. Confirm no
  collision at merge time.

## 5. Final Verdict

**VERDICT: APPROVED**
