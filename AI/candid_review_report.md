# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `feat/android-alerts`
- **Base (merge-base)**: `e216ac1`
- **Reviewed-Diff-Fingerprint**: `0df40a80b2f183769a888199f5bc66bd66c16df9352f0f39d2cc9de3b1e9873a`
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

The change adds the failed-password alerts list and its acknowledgement to the native Android app
(GitHub #404, matrix ANA18 to ANA21), rebased onto a main that already has the opt-in PC-offline
heartbeat with a process-wide `SharedApi`. The client-only change uses the single shared
`SoosApi` (`remember(pc.origin) { SharedApi.forOrigin(pc.origin) }`). No second `SoosApi` is
created in production code, since `SharedApi.kt:16` is the only constructor call. The change
keeps main's `sent`/`synchronized` session logic as it is and only appends the snapshot headers
through a new defaulted `extra` parameter. The parser is strict and bounded to 16384 chars and 32
entries. The transport stays at 5 s connect/read and 16384 bytes. Every test change is add-only.
`soos-invariants` passes: 747 passed, 0 failed. No CRITICAL or MAJOR defect was found. One MINOR
UX item and one SUGGESTION remain.

## 2. Test Changes

Mechanical listing of the frozen patch:

- Test files touched: `ApiJsonTest.kt` (+159/-0), `SoosApiTest.kt` (+140/-0),
  `tests/invariants/src/android_native_contract.rs` (+160/-0). `git diff e216ac1 -- android/app/src/test tests/`
  shows insertions only.
- Removed or changed assertions (`^-` with assert/#[test]/proptest/should_panic): **none**.
- New escape hatches (`#[ignore]`, `@Ignore`, `should_panic`, tolerance/epsilon): **none**.
- Inline test modules changed: none.
- New tests can fail against plausible wrong implementations. Examples: header order and count
  (`ackCarriesExactlyTheSnapshotHeadersAndNoBody`), no network call on an invalid snapshot, the
  boundaries 32/33 entries and 16384/16385 chars, `4294967295` accepted and `4294967296` refused,
  and a `200` without a view returning `Unreachable`. The invariant test pins the Kotlin constants
  to `crates/remote/src/lib.rs` and `routes.rs`, and it pins `MAX_ALERTS_BODY_CHARS` to the
  transport's `MAX_RESPONSE_BYTES`.

## 3. Deep Reasoning Audit

### Logic & Architecture
- **Wire contract.** I compared each field and enum of `crates/remote/src/alerts.rs` and
  `journal.rs` (`serde(rename_all = "snake_case")`) with `ApiJson.ALERT_*`: states, reasons,
  coverage, sources, accounts and kinds match exactly. The server sends counters as `u32`, which
  matches `MAX_ALERT_COUNT`. `through`/`id`/times are `u64`. A value above `Long.MAX_VALUE` makes
  the whole view `null`, so the app fails safe: no ack is possible. PASS.
- **Ack refusal tokens.** I mapped them to `server.rs:2145-2175`: `403 alerts_disabled`,
  `400 bad_request`, `429 rate_limited`, `503 unavailable`, `409 stale_view`, plus the generic
  `403 forbidden`, which falls back to `alerts_ack_refused`. PASS.
- **No ack of a view that is not displayed.** The button passes the `view` of the composition
  that drew it. `acknowledge()` takes `epoch` and `through` from that same object and returns
  early when `epoch == null`. The button only shows when `pending && view.epoch != null`. After a
  refusal, `refresh()` reloads the view and never retries the ack. A `200` body that does not
  parse becomes `Unreachable`, so the UI does not show "Acknowledged". PASS.
- **SharedApi coexistence.** `HomeScreen` uses the shared client already introduced on main
  (line 297, outside the diff). `HeartbeatCard` lives in `SettingsScreen`, so the two screens
  never compose at the same time and do not conflict. PASS.
- **Compose layout.** `PullToRefreshBox(fillMaxSize)` wraps `Column(verticalScroll)` inside the
  bounded `Box(fillMaxSize)` of `SoosApp`. No parent scrolls, so there is no
  infinite-height-constraint crash. PASS.
- **Compile correctness (by reading).** I checked the imports: `StringRes`, `rememberScrollState`,
  `verticalScroll`, `ExperimentalMaterial3Api`, `PullToRefreshBox` (material3 >= 1.3, and BOM
  2025.10.01 gives 1.4.x), `stringResource`, `java.time.*`, `AlertRecord`/`AlertsView`,
  `org.soos.remote.R` (already present), and `JSONArray`/`JSONException`/`JSONObject` in
  `ApiJson`. `return null` inside named arguments within `try` is valid Kotlin. Integer literals
  are passed for `Long` parameters in the tests, which Kotlin converts. The `when` blocks over the
  sealed `ApiResult` are exhaustive. `warningsAsErrors = false`, so an opt-in marker on a
  stabilized API cannot break the build. PASS.

### PAM Concurrency & Deadlines
- Not applicable: this is Android client code only. No `crates/pam` change. PASS.

### Panic Safety & Fail-Closed
- `parseAlerts` catches its private `Malformed` exception and `JSONException`. `alertTime`
  catches `DateTimeException`. I/O runs on `Dispatchers.IO`, and every `IOException` becomes
  `Unreachable`. No `!!` appears in the production hunks. When a load fails, the card shows a
  fixed "could not be loaded" text and never shows stale counts, because `alerts` is set to
  `null`. PASS.

### Test Integrity & Anti-Weakening
- See §2: the changes are add-only and have no escape hatches. PASS.

### Memory, Bounds & Secrets
- **Bounds.** The transport bound is 16384 bytes. The parser bound is 16384 chars, checked
  before `JSONObject` parses. The history holds at most 32 entries, checked before the
  `ArrayList` is allocated. Timeouts stay at 5000 ms (`UrlConnectionTransport.kt:47-48`). PASS.
- **Session.** `send` captures `sent` before the request. A `403 login_required` clears the
  session only when `session == sent`, so a session stored by the heartbeat in the meantime
  survives. The snapshot headers are appended before `Content-Type`/`Cookie`, and the test pins
  the exact header list. The cookie stays in memory only. PASS.
- **Secrets and text.** No server text is rendered: every token only selects a fixed
  `strings.xml` resource. No logging is added. The alerts carry no password text. PASS.

### Supply Chain & Automation
- No dependency, Gradle, workflow or script changes. PASS.

### English-Only Policy
- Code, comments, strings, docs, ADR and matrix lines are in English. PASS.

### Documentation registers
- `AI/DECISIONS.md` gains one add-only ADR line, and its title occurs once. `AI/VERIFICATION_MATRIX.md`
  gains four add-only lines, ANA18 to ANA21, with no duplicate ANA id in the file. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt:314`: `loadAlerts()`
  (called by every refresh and pull-to-refresh) never clears `ackFeedback`. An "Acknowledged."
  message therefore stays under a later view that may already show new unacknowledged attempts.
  It only clears on the next ack tap (line 495) or when the card is cleared. This is a cosmetic
  issue and not a security one: counts and the button still reflect the fresh view. Suggested
  fix: reset `ackFeedback` whenever a refresh, other than the one that follows a refusal, loads a
  new view.
- **[SUGGESTION]** `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt:362`: an ack attempt
  on a view whose state is `unavailable` but whose counts are still pending reaches the server
  only to get `503 unavailable`. The button could be hidden for `state != "active"`. This is
  optional.

## 5. Final Verdict

No CRITICAL or MAJOR finding.

**VERDICT: APPROVED**
