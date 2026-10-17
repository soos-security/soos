# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `feat/android-alerts`
- **Base (merge-base)**: `6f89f05`
- **Reviewed-Diff-Fingerprint**: `767f1b3040aa45841249deb8359ca9606bdca0b51b83efa5823fce3f9140dd23`
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

The rebased branch adds the failed-password alerts list and acknowledgement to the native
Kotlin/Compose app (GitHub #404, matrix ANA18-ANA21) on top of main, which already holds the
process-wide `SharedApi`, the opt-in heartbeat (#419), the GNOME extension (#407) and the phone
web-app tour (#418, §2k). The alerts use the single shared client
(`SharedApi.forOrigin` at `SoosApp.kt:297`; the only other user is `HeartbeatJobService.kt:39`; no
second `SoosApi` is constructed). `SoosApi.send` keeps main's `sent` snapshot and synchronized
cookie logic unchanged; the branch only appends the snapshot headers. The parser is strict and
bounded (16384 chars, 32 entries, `u32` counters, 16-hex epoch). Test files are add-only. The
`soos-invariants` suite passes (759 passed, 0 failed). Two MINOR items and one SUGGESTION; no
CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `android/app/src/test/java/org/soos/remote/ApiJsonTest.kt` (+159),
  `android/app/src/test/java/org/soos/remote/SoosApiTest.kt` (+140),
  `tests/invariants/src/android_native_contract.rs` (additions only).
- Removed or changed assertions (`^-` lines with `assert`/`#[test]`): **none**.
  `git diff 6f89f05 -- android/app/src/test` contains 0 removed lines.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance): **none**.
- Inline test modules changed: none.

New tests: ApiJsonTest (5 tests, ANA18), SoosApiTest (6 tests, ANA19), `test_ana_s14_alerts_contract`
and an add-only extension of `test_ana_s8_kotlin_hygiene` (ANA20). They would fail against
plausible wrong implementations (missing or duplicated header, uppercase epoch accepted, 33
entries accepted, 16385 chars accepted, a 2^32 counter accepted, a body sent on the ack).

## 3. Deep Reasoning Audit

### Logic & Architecture

- Shared client: searched for a second `SoosApi(` in production code: only the class declaration
  and `ApiHolder` create one. PASS.
- Session handling: `ackAlerts` goes through `send`, so the cookie is read once into `sent`, the
  `Set-Cookie` update and the `login_required` drop run under `synchronized(this)` and only drop
  the session when `session == sent`. A heartbeat request racing an ack cannot drop a freshly
  stored session. PASS.
- Header order and duplicates: `extra` is appended after `X-Soos-Action` and before
  `Content-Type` (absent, body null) and `Cookie`; `ackCarriesExactlyTheSnapshotHeadersAndNoBody`
  pins the exact list. PASS.
- Local snapshot refusal: an invalid epoch (length, case, non-hex, newline) or a negative
  `through` returns `Refused(400, "bad_request")` without touching the transport. PASS.
- Never acknowledging an undisplayed view: the button closure captures the `view` of the current
  composition; `acknowledge` sends that view's epoch/through; any non-`Ok` result triggers
  `refresh()` so the user sees the fresh view before a new acknowledgement; the button is
  disabled while `busy` (pull to refresh is also guarded by `!busy`). A `200` whose body is not a
  view is `Unreachable`, never a silent success. PASS.
- Compose structure: `HeartbeatCard` lives in `SettingsScreen` (main), `AlertsCard` in
  `HomeScreen`; they are never composed together and share no state. `PullToRefreshBox`
  (material3 1.3+, BOM 2025.10.01) wraps a `fillMaxSize().verticalScroll(...)` column, a valid
  nesting. PASS.
- Kotlin compile by reading: imports (`StringRes`, `rememberScrollState`, `verticalScroll`,
  `ExperimentalMaterial3Api`, `PullToRefreshBox`, `stringResource`, `java.time.*`,
  `AlertRecord`, `AlertsView`) are present and used; `when` over the sealed `ApiResult` is
  exhaustive; `is ApiResult.Refused, ApiResult.Unreachable ->` is a legal mixed branch;
  `records += ... ?: return null` on an `ArrayList` is valid; `java.time` is fine with
  `minSdk = 34`. PASS.
- FINDING (MINOR): `SoosApp.kt:377` `refresh()` after a refused acknowledgement reaches
  `clearAlerts()` (`SoosApp.kt:357`) when the refusal was `login_required` (the session is now
  dropped, `authenticated` false), so `R.string.alerts_ack_login_required` (`SoosApp.kt:530`) is
  set and immediately cleared, never shown. Fail-safe (the sign-in button appears); only an
  explanatory text is lost.
- FINDING (MINOR): `ackFeedback` survives later successful refreshes (only `clearAlerts` and a
  new click reset it), so "Acknowledged." can sit under a newer view with new pending attempts
  after a pull to refresh. Cosmetic; it does not enable a blind acknowledgement.

### PAM Concurrency & Deadlines

No PAM, daemon or Rust production code changed. Network calls run on `Dispatchers.IO` through the
unchanged `UrlConnectionTransport` (5 s connect/read timeouts from main). N/A / PASS.

### Panic Safety & Fail-Closed

`parseAlerts` catches its private `Malformed` and `JSONException`; `long()` rejects doubles,
`BigInteger` and strings (`1.5`, `18446744073709551615`, `"3"` tested); `alertTime` catches
`DateTimeException` for out-of-range instants. No `!!` added. Every malformed input yields
`null` → `Unreachable` → "Alerts could not be loaded". PASS.

### Test Integrity & Anti-Weakening

Section 2 listing: zero removed or modified assertions in Kotlin or Rust tests. PASS.

- FINDING (SUGGESTION): `tests/invariants/src/android_native_contract.rs:904` slices §2j up to
  `"\n## 3."`, so the slice now also spans §2k (feature tour); a needle such as `pull` or
  `time zone` could be satisfied by §2k text. Bounding the slice at the next `"\n## "` heading
  would make the check exact. All needles are currently genuinely present in the §2j additions.

### Memory, Bounds & Secrets

- 16 KiB: `MAX_ALERTS_BODY_CHARS = 16384` equals `MAX_RESPONSE_BYTES` (pinned by the invariant);
  the transport already throws above 16384 bytes and chars ≤ bytes, so the parser bound is a
  consistent second line. Boundary 16384/16385 tested. PASS.
- 32 entries: `MAX_ALERT_HISTORY = 32` pinned to `crates/remote/src/lib.rs`; 32 accepted, 33
  refused. PASS.
- 5 s timeouts: unchanged transport. PASS.
- Secrets: no password, cookie or server text displayed; only fixed `strings.xml` texts selected
  by enumerated tokens; the cookie stays in memory. PASS.

### Supply Chain & Automation

No `Cargo.*`, Gradle, `deny.toml`, workflow or script change. PASS.

### English-Only Policy

All code, comments, strings and documents in the diff are English. PASS.

### Documents

- `AI/DECISIONS.md`: one new ADR line, its title present exactly once, add-only. PASS.
- `AI/VERIFICATION_MATRIX.md`: ANA18-ANA21 each present once, add-only. PASS.
- `Docs/REMOTE_COMPANION.md`: the alerts bullet and the "Owner hardware check of the alerts
  (matrix ANA21)" sit inside "## 2j. Native Android app", immediately before "## 2k. Feature
  tour"; the only removed lines are the intro sentence, rewritten to drop "alerts" from the
  web-only list. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt:377` — after a
  `login_required` acknowledgement refusal, `refresh()` → `clearAlerts()` (line 357) erases the
  `alerts_ack_login_required` feedback; surface it through `message` instead, or keep the
  feedback when clearing on sign-out.
- **[MINOR]** `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt:681` — `ackFeedback` is not
  reset by a successful refresh; reset it in `loadAlerts` when a view with another `through`
  arrives.
- **[SUGGESTION]** `tests/invariants/src/android_native_contract.rs:904` — bound the §2j slice at
  the next `"\n## "` heading rather than `"\n## 3."` (add-only tightening in a follow-up).

## 5. Final Verdict

**VERDICT: APPROVED**
