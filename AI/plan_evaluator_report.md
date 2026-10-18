# Plan Evaluation Report
- **Date**: 2026-10-18
- **Issue**: GitHub #439 (part of epic #357; GitHub-only, no backlog id) — read-only battery and offline-history cards in the native Android app
- **Branch**: `feat/android-cards`
- **Base commit**: `3f76090`
- **Plan evaluated**: `AI/architect_spec_android_cards.md` §0–§8
- **Mode**: self-check by the implementing agent. The independent candid review runs separately, before the push.

## 1. Coverage Matrix

| #439 acceptance line | Spec element | Status |
|---|---|---|
| `SoosApi.battery()` / `offline()` through the shared client, cookie, `login_required` drops the session | §3 (existing private `get()`/`send()`) | Covered |
| Strict, bounded `parseBattery` (every state incl. `disabled`) | §2.1 | Covered |
| Strict, bounded `parseOffline` (states, reason, monitor, intervals newest first) | §2.2 | Covered |
| Unknown enum, wrong type, negative or out-of-range number, > 32 intervals, > 16384 characters refuse the whole view, no "other" fallback | §2.1, §2.2 | Covered |
| Two read-only soos-brand cards, refresh on open, Refresh, pull to refresh | §5 | Covered |
| Fixed `strings.xml` texts, theme colours, no server/exception text, no logging, `java.time` + `ZoneId.systemDefault()` | §5 | Covered |
| Tour battery text names the new card | §6 (where line; the shared text is pinned byte-equal to `app.js`) | Covered with deviation, see F-1 |
| No permission, manifest, dependency, server or signing change | §0, §8 | Covered |
| JVM tests, add-only invariant extension, matrix ANA22+, ADR, §2j, walkthrough | §7 | Covered |

## 2. Facts Verified

| Claim | Evidence |
|---|---|
| Paths | `crates/remote/src/routes.rs:114` `BATTERY_PATH = "/api/battery"`, `:116` `OFFLINE_PATH = "/api/offline"`; dispatched at `server.rs:1400-1401` |
| Battery view shape and cross-field rules | `battery.rs:46-116` (`BatteryState`, `ChargeStatus`, `BatteryView`, `without_values`), `aggregate` `:295-319` (`charge: Some(..)` for `Present`), `parse_capacity` ≤ 100 `:216` |
| Offline view shape and cross-field rules | `offline.rs:817-879` (`OfflineState`, `OfflineUnavailableReason`, `NetworkMonitor`, `OfflineView`, `disabled`, `store_failed`), `view()` `:1090-1113`, `OfflineInterval` `:103-129`, store refuses `start > end` `:545` |
| Interval bound | `lib.rs:700` `MAX_OFFLINE_INTERVALS: usize = 32` |
| Auth of both routes | `Docs/REMOTE_COMPANION.md` §6 table: tailnet, or Funnel with a session, `403 login_required` otherwise |
| Transport bound | `UrlConnectionTransport.kt` `MAX_RESPONSE_BYTES = 16384` (pinned by ANA-S8) |
| `tour_battery_text` is pinned | `android_tour_contract.rs` ATR-S1 asserts byte equality with `app.js:2183`; ATR-S2 needs `web app` and `battery_status = false` in `tour_battery_where` |
| SoosApp constraints | ANA-S14 forbids `.message`, `localizedMessage`, `toString()}` and bare `Text(result` in `SoosApp.kt`; ATR-S5/S6 pin the tour wiring, which is not touched |

## 3. Pillar Analysis

- **Security**: read-only GETs; no new header, body, origin, host or storage; tokens select fixed texts only; the parser is total and bounded.
- **Bounded resources**: 16384 characters, 32 intervals, fixed-size views.
- **Panic/exception safety**: every `org.json` access type-checked; `JSONException` and an internal `Malformed` caught inside the parser; `DateTimeException` caught in the formatter.
- **Privacy**: battery level, charge state, mains presence, offline reasons and times only (what the web app shows); nothing logged.
- **Tests immutable**: only additions to `ApiJsonTest.kt`, `SoosApiTest.kt` and `android_native_contract.rs`, plus a new `CardModelTest.kt`.

## 4. Findings

- **F-1 (accepted deviation, documented)**: the task asked to adjust `tour_battery_text`. That text is byte-equal to the web tour and pinned by ATR-S1; changing it needs a change of `crates/remote/assets/app.js` (a server change, excluded) and of an existing test assertion (forbidden). The app-specific `tour_battery_where` line names the new Battery card instead, which ATR-S2 allows. Reported in the PR body and the walkthrough.
- **F-2 (accepted)**: strict parsing without an "other" fallback means a future server enum shows "could not be loaded" until the app is updated, as for the alerts (ADR 2026-10-17 item (3)).

VALIDATION_VERDICT: APPROVED
