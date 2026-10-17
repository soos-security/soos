# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `fix/android-alerts-feedback`
- **Base (merge-base)**: `74782b8`
- **Reviewed-Diff-Fingerprint**: `c88fbebee96e6346ff9803c3e270bdea812b88db6de871afed267012e2b07ef4`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/227_android_alerts_feedback.md`,
  `Docs/REMOTE_COMPANION.md`, `android/app/src/main/java/org/soos/remote/alerts/AlertsFeedback.kt`,
  `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt`,
  `android/app/src/test/java/org/soos/remote/AlertsFeedbackTest.kt`,
  `tests/invariants/src/android_alerts_feedback_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

Android-only client fix (GitHub #420). The acknowledgement feedback of the alerts card moves
from a `@StringRes Int?` decided inline in `HomeScreen` to a pure Kotlin helper
(`AlertsFeedback`: `ofAck`, `needsRefresh`, `afterRefresh`). `refresh()` now clears the feedback
before its first request, has a single exit, and re-applies only the feedback carried by the
acknowledgement's follow-up refresh; a `login_required` refusal followed by a signed-out
refresh is shown as the screen message. No server, protocol, PAM, dependency, permission or
string-resource change. The test side of the diff is purely additive. No CRITICAL or MAJOR
finding; two MINOR notes and one SUGGESTION.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `android/app/src/test/java/org/soos/remote/AlertsFeedbackTest.kt` (new),
  `tests/invariants/src/android_alerts_feedback_contract.rs` (new),
  `tests/invariants/src/lib.rs` (+8 lines: one new `mod` declaration with its doc comment).
- Removed or changed assertions / `#[test]` lines (`^-` grep): **none**.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.
- `git diff 74782b8 HEAD --stat -- tests 'crates/*/tests' android/app/src/test`: 384 insertions,
  0 deletions. No pre-existing assertion was removed, weakened or changed.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Stale "Acknowledged.": after `200`, `needsRefresh` is false and the card shows DONE; every
  later `refresh()` (Refresh button, pull to refresh, sign-in, sign-out, Lock PC,
  `LaunchedEffect`) passes `carried = null`, sets `ackFeedback = null` first, and
  `afterRefresh(null, *)` yields `NONE` for every outcome. PASS.
- `login_required` path: the Funnel gate (`crates/remote/src/server.rs` around line 1307) answers
  `403 login_required` for a non-public route without a valid session before routing, so `ofAck`
  gives `LOGIN_REQUIRED`; the follow-up `authState()` reports unauthenticated, `clearAlerts()`
  hides the card and `afterRefresh(LOGIN_REQUIRED, SIGNED_OUT)` sets `message` to the fixed
  `alerts_ack_login_required` string ("Your session ended. Sign in again.", verified in
  `strings.xml`). PASS.
- Unreachable during the follow-up refresh: the early `return` became an `if/else` expression;
  `busy = false` is always reached and the unreachable message is not overwritten (`NONE`). PASS.
- Token mapping in `ofAck` is identical to the removed `ackRefusalText` (six tokens, anything
  else or a null token → REFUSED). `ackFeedbackText` is an exhaustive `when` (no `else`) over the
  nine enum entries, each mapped to the same `R.string` as before. PASS.
- Server-behaviour claims (walkthrough §2, `AlertsCard` comment, §2j) verified against code:
  `RemoteAlerts::acknowledge` returns `Unavailable` only on a poisoned lock or a missing book
  (`alerts.rs` 1041–1046); `RemoteAlerts::view` returns `AlertsView::empty` (epoch `None`, zero
  counts) in both cases (`alerts.rs` 929–948); `AlertsView::disabled()` is `empty(...)`;
  `alerts_ack` answers `403 alerts_disabled` when `shared.alerts` is `None` (`server.rs`
  2150–2152); web rule `alertsAckButton.hidden = !(wrong > 0 || lockedOut > 0) || typeof
  view.epoch !== "string"` (`app.js` 820). `WEB_SESSION_IDLE_MS = 900_000` matches "more than 15
  minutes" in owner step 5; Funnel requests refresh the session (`Touch::Refresh`), so asking to
  keep PC offline alerts off during that step is correct. PASS.
- Re-entrancy: `acknowledge()` sets `busy = false` then calls `refresh()`, whose first two
  statements (`ackFeedback = null`, `busy = true`) run before any suspension point, so no other
  refresh can start in between. PASS.

### PAM Concurrency & Deadlines

Not affected: no file under `crates/pam`, the daemon or IPC changes. PASS (N/A).

### Panic Safety & Fail-Closed

No authorization decision is touched; the change is display-only. The Kotlin helper has no
`!!`, no indexing and no throw. The Rust code is test-only (`#[cfg(all(test, unix))]`) with the
same explicit `allow` list as the sibling contract modules. PASS.

### Test Integrity & Anti-Weakening

- The diff is additive (section 2).
- The JVM tests fail against plausible wrong implementations: carrying DONE through a plain
  refresh (`afterRefresh(null, *)` must be `NONE`), dropping the signed-out promotion, promoting
  any other feedback to the notice (`aSignedOutRefreshShowsOnlyTheLoginRequiredNotice` iterates
  every entry), or a token-mapping regression. The static AAF2 test pins "clear first", the
  single exit (`return` count 0), the only carried call site and the exhaustive mapping.
- The JVM tests could not be compiled on this host (no `java`, `kotlinc` or `gradle`); they rely
  on the CI job `Android (debug build)`, as matrix row AAF1 states (pending). By reading, the
  `AlertsView` constructor arguments match `net/ApiJson.kt`, `ApiResult.Refused(Int, String?)`
  and `Ok<T>` match `net/SoosApi.kt`, and `.entries` is available (Kotlin 2.2.21, already used
  by `HeartbeatTextsTest.kt`). PASS.

### Memory, Bounds & Secrets

No new I/O, allocation or network call. Only fixed `strings.xml` texts are displayed; a server
token only selects an enum value; no `.message`, `localizedMessage` or logging (the existing
`test_ana_s14_alerts_contract` and `test_ana_s8_kotlin_hygiene` still pass). No password, frame
or embedding is involved. PASS.

### Supply Chain & Automation

No `Cargo.*`, Gradle, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. PASS (N/A).

### English-Only Policy

All code, comments, docs and the commit message are English; non-ASCII characters are only
typographic (em dash, en dash, section sign, the matrix status symbols used elsewhere). The
commit subject `fix(android): keep alerts ack feedback in step with the displayed view` is a
single-scope Conventional Commit of 70 characters. PASS.

### Commands Re-run

With `LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_TARGET_DIR=~/.cache/soos-targets/android-alerts-feedback
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=3`:

- `cargo test -p soos-invariants --lib -- --test-threads=3`: 774 passed, 0 failed.
- `cargo clippy -p soos-invariants --all-targets -- -D warnings`: clean.
- `cargo fmt --check -p soos-invariants`: clean.

## 4. Detailed Findings & Action Items

- **[MINOR]** `AI/walkthroughs/227_android_alerts_feedback.md:90` — section 7 says the candid
  review was "Not run in this session" and that no `AI/candid_review_report.md` is written; with
  this report committed, the sentence is outdated. Either accept it as a record of the author
  session or update it (which changes the diff and needs a new review run).
- **[MINOR]** `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt:385` — on a refused
  acknowledgement, `ackFeedback = feedback` is reset to `null` by `refresh()` in the same frame
  and restored only after the follow-up round-trips, so the refusal text is invisible while that
  refresh is in flight. The final state is correct; the assignment only matters for `200`.
- **[SUGGESTION]** `tests/invariants/src/android_alerts_feedback_contract.rs:175-185` — the
  call-site counting (`refresh()` count `>= 5`, `refresh(` count equal to `refresh()` count + 2)
  is brittle against unrelated future edits (for example a comment mentioning `refresh(`).

## 5. Final Verdict

**VERDICT: APPROVED**
