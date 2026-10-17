# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `feat/android-alerts`
- **Base (merge-base)**: `f1e7616`
- **Reviewed-Diff-Fingerprint**: `3d9e0556a33dd94f50a774cd5b1ee2c891b07d1b51bfe351304768abce4fea15`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_android_alerts.md`,
  `AI/auditor_constraints_android_alerts.md`, `AI/tester_contract_android_alerts.md`,
  `AI/walkthroughs/219_android_alerts.md`, `Docs/REMOTE_COMPANION.md`,
  `android/app/src/main/java/org/soos/remote/net/ApiJson.kt`,
  `android/app/src/main/java/org/soos/remote/net/SoosApi.kt`,
  `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt`,
  `android/app/src/main/res/values/strings.xml`,
  `android/app/src/test/java/org/soos/remote/ApiJsonTest.kt`,
  `android/app/src/test/java/org/soos/remote/SoosApiTest.kt`,
  `tests/invariants/src/android_native_contract.rs`

## 1. Executive Summary

Review of the failed-password alerts list with acknowledgement in the native Android app
(GitHub #404, matrix ANA18-ANA21), rebased onto `origin/main` `f1e7616`, which added the opt-in
PC-offline heartbeat (#419) and the process-wide `SharedApi`. The rebase is clean: the alerts go
through the single shared client obtained in `HomeScreen` with
`remember(pc.origin) { SharedApi.forOrigin(pc.origin) }`; no second `SoosApi` is created anywhere
in the diff. `SoosApi.send` keeps main's `sent` snapshot and `synchronized` cookie/`login_required`
logic unchanged and only appends the snapshot headers before `Content-Type`/`Cookie`. The parser is
strict and bounded (16384 chars, 32 entries, `u32` counters, 16-hex epoch); transport timeouts
remain 5 s connect / 5 s read with the 16 KiB read bound. Test files are add-only.
`soos-invariants` passes (736 passed, 0 failed). Two MINOR UX findings on the acknowledgement
feedback text; no CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `android/app/src/test/java/org/soos/remote/ApiJsonTest.kt`,
  `android/app/src/test/java/org/soos/remote/SoosApiTest.kt`,
  `tests/invariants/src/android_native_contract.rs`.
- Removed or changed lines in test files (`^-` inside those hunks): **none**. Every hunk is
  additions only (new imports, new helpers `rust_usize_const`/`kotlin_int_const`, new assertions
  appended to `test_ana_s8_kotlin_hygiene`, new test `test_ana_s14_alerts_contract`, new Kotlin
  tests for ANA18/ANA19).
- New escape hatches (`#[ignore]`, `@Ignore`, `should_panic`, tolerance, epsilon): **none**.
- Inline test modules changed: none.

Justification: no existing assertion was modified, weakened or deleted; nothing to justify.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Shared client: `HomeScreen` uses `SharedApi.forOrigin`; `alerts()`/`ackAlerts()` are methods
  of that client, so the in-memory session cookie stays single for the screens and the heartbeat
  job. Tried: searched the diff for any `SoosApi(` construction in production code: none. PASS.
- Compose coexistence: `HeartbeatCard` lives only in `SettingsScreen` (unchanged by this diff);
  `HomeScreen` now wraps its content in `PullToRefreshBox` + `Column(fillMaxSize().verticalScroll())`
  (correct modifier order: viewport sized before the scroll). The Settings button stays in the
  scrollable row, so the heartbeat settings remain reachable. PASS.
- Never acknowledging an undisplayed view: the button passes the exact `AlertsView` rendered in
  that composition; `acknowledge` sends its `epoch`/`through`; the button is hidden without an
  epoch or when nothing is pending, and disabled while `busy` (refresh, lock, ack). A refused or
  unreachable ack triggers `refresh()` and never retries automatically. Tried a concurrent pull to
  refresh during an ack: `onRefresh` is ignored while `busy`. PASS.
- Server agreement: field names match `crates/remote/src/alerts.rs::AlertsView`/`AlertRecord`
  (snake_case serde); ack refusals (`403 forbidden/alerts_disabled`, `400 bad_request`,
  `429 rate_limited`, `409 stale_view`, `503 unavailable`) match `server.rs::alerts_ack`; the
  CSRF path is the same `check_action_csrf` as `lock`, which the app already satisfies. The
  invariant test pins the action, paths, headers and bounds against `lib.rs`/`routes.rs`. PASS.
- `login_required`: the 403 handling in `send` is untouched and covers the ack and the GET; the
  `session == sent` guard prevents dropping a session stored meanwhile by another caller. PASS
  (see MINOR 2 on the feedback text).

### PAM Concurrency & Deadlines

Not affected (Android app only; no `crates/pam` change). Network calls run on `Dispatchers.IO`
with the unchanged 5 s connect/read timeouts of `UrlConnectionTransport`. PASS.

### Panic Safety & Fail-Closed

`parseAlerts` catches its private `Malformed` and `JSONException`; `history.get(index)` is bounded
by `length()`; casts use `as?`. `alertTime` catches `DateTimeException` for instants that cannot
be formatted. Any malformed `200` becomes `Unreachable`, shown as the fixed "Alerts could not be
loaded". An invalid snapshot is refused locally without a network call. PASS.

### Test Integrity & Anti-Weakening

New tests can fail against plausible wrong implementations: exact header list and order for the
ack, header counts of 1, no body, no `Content-Type`; 32 vs 33 entries; 16384 vs 16385 chars;
`4294967295` vs `4294967296`; uppercase, short, long and non-hex epochs; an invalid snapshot sends
nothing; the cookie is carried on both routes. PASS.

### Memory, Bounds & Secrets

- 16 KiB: `MAX_ALERTS_BODY_CHARS = 16384` is checked before `JSONObject` parsing and pinned to
  `MAX_RESPONSE_BYTES = 16384` by the invariant test. PASS.
- 32 entries: checked before allocating `ArrayList(history.length())`. PASS.
- No server text is displayed: every value selects a fixed `strings.xml` resource; no exception
  message is shown (the invariant test forbids `.message`/`localizedMessage`). The view carries no
  password or typed text. PASS.

### Supply Chain & Automation

No dependency, Gradle, workflow or script change. `PullToRefreshBox` exists in the material3
version of Compose BOM `2025.10.01` and is opted in with `@OptIn(ExperimentalMaterial3Api::class)`.
PASS.

### English-Only Policy

All added code, comments, strings and docs are English. PASS.

### Kotlin Compile Correctness (by reading)

Imports are present for `StringRes`, `rememberScrollState`, `verticalScroll`, `PullToRefreshBox`,
`ExperimentalMaterial3Api`, `stringResource`, the `java.time` types, `AlertRecord`, `AlertsView`
and `org.soos.remote.R`; the helpers `string`, `long`, `array` and `objectOf` exist in `ApiJson`.
`when` branches over the sealed `ApiResult` are exhaustive (`is ApiResult.Refused,
ApiResult.Unreachable ->` is legal). `stringResource(id, Long)` with `%d` formats a `Long`. Local
functions mutating `by remember` state inside a composable are valid. No compile issue found; not
verified by a build (no JDK locally; the CI `Android (debug build)` job is the authority).

## 4. Detailed Findings & Action Items

- **[MINOR]** `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt:314` — `loadAlerts()`
  does not clear `ackFeedback`, so after a successful acknowledgement a later Refresh or pull to
  refresh that shows new pending attempts still displays "Acknowledged." under the new counts.
  Suggest clearing the feedback on any user-initiated refresh (keeping it only for the refresh
  that follows a refused acknowledgement). Not security relevant: the counts and the button are
  correct.
- **[MINOR]** `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt:374` — on an ack refused
  with `login_required`, `refresh()` finds the session gone and calls `clearAlerts()`
  (`SoosApp.kt:311`), which resets `ackFeedback`, and the card is hidden while signed out; the
  `alerts_ack_login_required` text is therefore never shown. The user still sees "Signed out" and
  the passkey button, so nothing misleading is displayed; the string is effectively dead.

## 5. Final Verdict

No CRITICAL or MAJOR finding. Shared client, session logic, bounds and test integrity verified.

**VERDICT: APPROVED**
