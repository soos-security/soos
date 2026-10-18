# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `feat/android-cards`
- **Base (merge-base)**: `3f76090`
- **Reviewed-Diff-Fingerprint**: `8e844103d82b5c23706ed442aaef387609cd60797af6cac75d6c2e25a13478b7`
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
  (`AI/plan_evaluator_report.md` is excluded from the fingerprint by the script.)

## 1. Executive Summary

One commit (`36ef844`) adds read-only *Battery* and *Offline history* cards to the native
Android app (GitHub #439, part of #357): two plain GETs (`/api/battery`, `/api/offline`) through
the existing `SoosApi.get()/send()` path, two strict bounded parsers, a pure card model, a
Compose card file with fixed `strings.xml` texts, wiring into `HomeScreen.refresh()`, docs, ADR,
matrix rows ANA22–ANA26 and invariant ANA-S15. No Rust production code, no server change, no
manifest/Gradle/catalog change. Every wire claim was checked against `crates/remote/src/battery.rs`,
`offline.rs`, `routes.rs`, `lib.rs`, `server.rs` and `assets/app.js`. No CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing over `target/candid_diff.patch` and
`git diff $(git merge-base origin/main HEAD) HEAD -- tests 'crates/*/tests' android/app/src/test`:

- Test files touched: `ApiJsonTest.kt` (+216/-0), `CardModelTest.kt` (new, +58),
  `SoosApiTest.kt` (+92/-0), `tests/invariants/src/android_native_contract.rs` (+326/-0).
- Removed or changed lines in test files: **none** (`grep '^-[^-]'` over the test diff is empty;
  the removed-assertion grep over the whole patch is empty).
- New escape hatches (`#[ignore]`, `@Ignore`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.

All changes are additions; no pre-existing assertion was removed or altered.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Battery wire contract: `BatteryView` serializes four keys with `Option` as JSON `null`
  (no `skip_serializing`), `percent: Option<u8>` capped at 100 by `parse_capacity`, `charge`
  `Some` iff `Present` (`aggregate`/`without_values`). The parser enforces exactly this
  (`present && charge == null` refused, any value without `present` refused). PASS.
- Offline wire contract: `OfflineView::view()` returns `store_failed` (reason set, monitor
  unknown, no intervals), `disabled`, or `active` with newest-first history; the parser's
  cross-field rules match the server constructors; `MAX_OFFLINE_INTERVALS = 32` equals `lib.rs`;
  `end >= start` matches `OfflineInterval::new`. `u64` values above `Long.MAX_VALUE` are parsed by
  org.json as a non-`Long` number, which `long()` rejects (tested with `18446744073709551615`). PASS.
- `CardModel.batteryPower` reproduces `app.js batteryText` precedence (charge status first,
  `external_power == true` only completes `unknown`); `offlineDuration` reproduces
  `durationText` boundaries (60 s, 60 min, 48 h, days floored). Tried 3 599 999, 172 799 999,
  `Long.MAX_VALUE`, negative: all consistent with the web page. PASS.
- Wiring: `loadCards()` runs only in the authenticated branch after `loadAlerts()`;
  `clearCards()` in the unreachable and signed-out branches, so no stale card survives sign-out.
  A `403 login_required` on a card GET drops the session through the unchanged `send()` rule and
  shows the fixed "could not be loaded" text. PASS.
- Divergences from the web page are deliberate and documented: `disabled` shows a sentence
  instead of hiding the card; all up to 32 periods are listed (the web page shows 10). PASS.

### PAM Concurrency & Deadlines
- No change under `crates/pam`. Network calls run on `Dispatchers.IO` through the existing
  bounded transport. N/A / PASS.

### Panic Safety & Fail-Closed
- Parsers catch `Malformed` and `JSONException`; `as?` casts everywhere; no `!!`, no indexing
  outside `0 until length()`. `alertTime` already catches formatter failures. A malformed or
  newer-server body yields `null` → "could not be loaded", never a misleading state. No
  authorization decision is made in the app. PASS.

### Test Integrity & Anti-Weakening
- See §2: additions only. New tests would fail against plausible wrong implementations (e.g.
  accepting values without `present`, an "other" fallback, 33 intervals, `end < start`,
  fractional or string numbers, missing cookie, cookie kept after `login_required`).
- Re-ran `cargo test -p soos-invariants -- --test-threads=3` with the scoped environment
  (`CARGO_TARGET_DIR=~/.cache/soos-targets/android-cards`): 788 passed, 0 failed (includes
  ANA-S15 and the unchanged `android_tour_contract`). `cargo clippy -p soos-invariants
  --all-targets -- -D warnings`: clean. `cargo fmt --check -p soos-invariants`: clean.
- The JVM tests (`ApiJsonTest`, `SoosApiTest`, `CardModelTest`) could not be run here (no JDK on
  the host); they run in CI `Android (debug build)`, and matrix rows ANA22–ANA24 correctly stay
  pending. Imports and test helpers (`Fake`, `answer`, `token`, `headerCount`, `hasSession`,
  `assertNotNull`) were checked to exist. PASS (pending CI).

### Memory, Bounds & Secrets
- Body bound `MAX_VIEW_BODY_CHARS = 16384` checked first in both parsers, equal to
  `UrlConnectionTransport.MAX_RESPONSE_BYTES` (pinned by ANA-S15); interval count checked before
  allocation (`ArrayList(intervals.length())` after the ≤ 32 check). No logging, no exception or
  server text displayed; the session cookie stays in memory only. No biometric data involved. PASS.

### Supply Chain & Automation
- No `Cargo.*`, Gradle, catalog, `verification-metadata.xml`, manifest, `.github/` or
  `scripts/` change. PASS.

### English-Only Policy
- Code, comments, strings, docs, ADR, walkthrough and commit subject are English. PASS.

### Factual Accuracy of Doc Claims
- `403 login_required` for a GET without session over Funnel: `server.rs` (non-public routes
  without a session). Verified.
- "30 new `battery_*` and `offline_*` texts": 12 + 18 = 30. Verified.
- "the web page does show the battery in its status card": `app.js showBattery`. Verified.
- "no event-stream request, the battery card changes on a refresh": no SSE call added. Verified.
- README row and §2j intro updated consistently; the old "battery level and the offline history
  stay in the web app" sentence removed (pinned by ANA-S15). Verified.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `android/app/src/main/java/org/soos/remote/ui/PcCards.kt:403` — the card lists
  all (up to 32) periods while the web page shows the newest 10 (`OFFLINE_HISTORY_SHOWN`). Bounded
  and acceptable; consider matching the web page in a later PR if the card gets long.
- **[SUGGESTION]** `AI/walkthroughs/236_android_battery_offline_cards.md` — number 236 skips
  232–235 (the last on `origin/main` is 231); acceptable if those numbers are reserved by
  parallel branches, otherwise renumber before merge.

## 5. Final Verdict

**VERDICT: APPROVED**
