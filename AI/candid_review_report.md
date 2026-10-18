# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `feat/android-cards`
- **Base (merge-base)**: `33b5672`
- **Reviewed-Diff-Fingerprint**: `f4819c726776ac52df7d652896d7c5d3cfdbf40c8358a402c04c7edd9ac54299`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_android_cards.md`,
  `AI/auditor_constraints_android_cards.md`, `AI/tester_contract_android_cards.md`,
  `AI/walkthroughs/236_android_battery_offline_cards.md`, `Docs/REMOTE_COMPANION.md`, `README.md`,
  `android/app/src/main/java/org/soos/remote/cards/CardModel.kt`,
  `android/app/src/main/java/org/soos/remote/net/ApiJson.kt`,
  `android/app/src/main/java/org/soos/remote/net/SoosApi.kt`,
  `android/app/src/main/java/org/soos/remote/ui/PcCards.kt`,
  `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt`,
  `android/app/src/main/res/values/strings.xml`,
  `android/app/src/test/java/org/soos/remote/ApiJsonTest.kt`,
  `android/app/src/test/java/org/soos/remote/CardModelTest.kt`,
  `android/app/src/test/java/org/soos/remote/SoosApiTest.kt`,
  `tests/invariants/src/android_native_contract.rs`

## 1. Executive Summary

Re-review after the branch was rebased from `3f76090` onto `origin/main` `33b5672` (which gained
#445, #440, #434 and #435). The frozen patch (`target/candid_diff.patch`, 1864 lines, 18 files)
adds read-only *Battery* and *Offline history* cards to the native Android app: two plain GETs
(`/api/battery`, `/api/offline`) on the shared `SoosApi`, two strict bounded parsers, a pure
`CardModel`, two Compose cards, fixed `strings.xml` texts, JVM tests and invariant ANA-S15.

Rebase integrity was checked mechanically against the pre-rebase commit `f237f1d`:

- Every Kotlin, resource, test, invariant, spec, walkthrough, README and `Docs/` file has the
  **same blob** at `HEAD` as at `f237f1d`; the file list of the diff is identical.
- `AI/VERIFICATION_MATRIX.md` has a different blob (main moved on) but its +/- hunk lines are
  identical to the pre-rebase diff (add-only, no removed line).
- `AI/DECISIONS.md` differs from main by exactly one added line, the branch ADR; the pre-rebase
  diff also added one blank line before it, which was dropped (whitespace only).
- Between the two bases, main did not touch `android/`, `crates/remote/src/battery.rs`,
  `offline.rs`, `routes.rs` or `assets/app.js`, so the pinned server contracts are unchanged.

No CRITICAL, MAJOR or MINOR finding.

## 2. Test Changes (mechanical listing)

- Test files touched: `ApiJsonTest.kt`, `SoosApiTest.kt` (append-only: imports plus new tests),
  `CardModelTest.kt` (new), `tests/invariants/src/android_native_contract.rs` (append-only, new
  ANA-S15 section after the last existing test).
- Removed or changed checks (`^-` with `assert`/`#[test]`/`@Test`/`proptest!`/`should_panic`):
  **none**.
- New escape hatches (`#[ignore]`, `@Ignore`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.
- All removed lines in the patch are documentation prose (README, `Docs/REMOTE_COMPANION.md`),
  KDoc comments (`ApiJson.Malformed`, `SoosApi`, `refresh`, `alertTime`), the visibility of
  `alertTime` / `timeText` (`private` to `internal`), and the two app-specific tour where lines
  (`tour_battery_where`, `tour_offline_alerts_where`), which ADR item (4) explicitly allows; the
  byte-equal tour texts pinned by ATR-S1 are untouched and the invariants pass.
- Matrix: rows ANA22 to ANA26 each appear once; a scan of every first-column ID of
  `AI/VERIFICATION_MATRIX.md` found no duplicate.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Client: `SoosApp.kt` still obtains `api` via `remember(pc.origin) { SharedApi.forOrigin(pc.origin) }`;
  `battery()` / `offline()` go through the existing `get()` helper, so the in-memory cookie and the
  `login_required` drop apply (`SoosApiTest.batteryAndOfflineSendTheSessionCookieAndDropItOnLoginRequired`).
- Parsers: tried `percent = 4294967296` (Long above 100, refused), `50.5`, `"50"`, values with a
  non-`present` state, `present` without charge, `unavailable` without reason, a reason without
  `unavailable`, non-`active` with a monitor other than `unknown` or with intervals, 33 intervals,
  inverted spans, a `u64::MAX` start, bodies of 16384 / 16385 chars: all refused or accepted as
  the server rules (`battery.rs`, `offline.rs`) require, and each case has a test.
- `CardModel.offlineDuration`: boundaries 59 999 / 60 000 / 3 599 999 / 3 600 000 / 172 799 999 /
  172 800 000 / `Long.MAX_VALUE` / negative are all tested; no overflow (division only).
- UI state: `clearCards()` runs on the unreachable and signed-out branches, `loadCards()` only on
  the authenticated branch; a failed load shows a fixed error text, never server text.
- PASS.

### PAM Concurrency & Deadlines
- No change under `crates/pam` or any Rust production crate. Network calls run on
  `Dispatchers.IO` through the existing transport and its unchanged bounds. PASS.

### Panic Safety & Fail-Closed
- `parseBattery` / `parseOffline` catch `Malformed` and `JSONException` and return `null`; array
  access is by `0 until length()` with an `as? JSONObject` check; `offlineReasonText` only sees
  parser-admitted reasons. No path grants anything: the cards are read-only with no action. PASS.

### Test Integrity & Anti-Weakening
- No pre-existing assertion removed or changed (section 2). New tests would fail against plausible
  wrong implementations (accepting values without `present`, an off-by-one interval bound,
  dropping the session cookie, sending `Origin` / `X-Soos-Action` on a GET). PASS.

### Memory, Bounds & Secrets
- Body length checked before `JSONObject` parsing (`MAX_VIEW_BODY_CHARS` = 16384, equal to the
  transport bound); intervals checked against `MAX_OFFLINE_INTERVALS` = 32 before the list is
  allocated, pinned to `crates/remote/src/lib.rs` by ANA-S15. Nothing logged; no server, status
  number or exception text displayed. PASS.

### Supply Chain & Automation
- No change to `AndroidManifest.xml`, Gradle files, the version catalog, `verification-metadata.xml`,
  `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/`; no new permission (the `aapt2`
  allowlist invariant passes). PASS.

### English-Only Policy
- All code, comments, strings and docs in the patch are English. PASS.

## 4. Detailed Findings & Action Items

- None.

Validation run on the rebased branch:
`LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_TARGET_DIR=~/.cache/soos-targets/android-cards CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=3 cargo test --locked -p soos-invariants -- --test-threads=3`
gave 810 passed, 0 failed, 0 ignored. The JVM tests (`ApiJsonTest`, `SoosApiTest`, `CardModelTest`)
were not run locally; their sources are byte-identical to the pre-rebase review and they run in
the CI job `Android (debug build)`.

## 5. Final Verdict

**VERDICT: APPROVED**
