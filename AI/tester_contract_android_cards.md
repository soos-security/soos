# Tester Contract — GitHub #439: battery and offline-history cards in the native Android app

Inputs: `AI/architect_spec_android_cards.md` (plan self-check APPROVED). Every test below is new;
no existing test or assertion is modified (the three Kotlin test files and
`android_native_contract.rs` only gain lines). The JVM tests run only in the CI job
`Android (debug build)` (no JDK or SDK on the development host by rule).

| Matrix | Test | Red reason before the implementation |
|---|---|---|
| ANA22 | `ApiJsonTest::{batteryPresentViewIsParsedExactly,batteryEveryStateWithoutValues,batteryUnknownOrMalformedValuesAreRefused,batteryOversizedInputIsRefused}` | `BatteryView`, `ApiJson.parseBattery` and `ApiJson.MAX_VIEW_BODY_CHARS` do not exist (compile error of the test source set) |
| ANA23 | `ApiJsonTest::{offlineActiveViewIsParsedExactly,offlineDisabledAndUnavailableViews,offlineUnknownOrMalformedValuesAreRefused,offlineOversizedInputIsRefused}` | `OfflineView`, `OfflineInterval`, `ApiJson.parseOffline`, `ApiJson.MAX_OFFLINE_INTERVALS` do not exist |
| ANA24 | `SoosApiTest::{batteryAndOfflineArePlainGets,batteryAndOfflineSendTheSessionCookieAndDropItOnLoginRequired,batteryAndOfflineRefusalsAndUnreadableBodies}`; `CardModelTest::{batteryLineFollowsTheState,batteryPowerFollowsTheWebPage,offlineDurationBoundaries}` | `SoosApi.battery()`, `offline()`, `BATTERY_PATH`, `OFFLINE_PATH` and the `cards` package do not exist |
| ANA25 | `soos-invariants::android_native_contract::test_ana_s15_battery_and_offline_cards_contract` | run on the base: panics `BATTERY_PATH not found` (assertion on the specified API, verified locally) |

Boundaries encoded: percent 0/100 accepted, -1/101/50.5/"50"/2^32 refused; 32 intervals accepted,
33 refused; 16384 characters accepted, 16385 refused; `end == start` accepted, `end < start`
refused; a number beyond `Long` refused; every enum member accepted, case variants and unknown
members refused; the server's cross-field rules (values only with `present`, `charge` with
`present`, `reason` iff `unavailable`, monitor `unknown` and no interval unless `active`).
