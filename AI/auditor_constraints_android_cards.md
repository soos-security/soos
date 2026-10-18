# Audit Constraints — GitHub #439: battery and offline-history cards in the native Android app

Inputs: architect spec `AI/architect_spec_android_cards.md`, plan self-check (APPROVED), tester
contract `AI/tester_contract_android_cards.md`, the files about to change under
`android/app/src/main/`.

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C1 | The parsers are total: no `!!`, no unchecked cast, no uncaught exception; every `org.json` access type-checked (no `opt*` coercion); malformed input returns `null` | `ApiJson.kt::parseBattery`, `parseOffline` | ANA22, ANA23; code reading |
| C2 | Bounded input: body ≤ 16384 characters checked before parsing, ≤ 32 intervals checked before iterating, every string a member of a fixed set, percent `0..100`, times and durations integers ≥ 0, `end >= start` | `ApiJson.kt` | ANA22, ANA23; ANA-S15 pins the bounds to `lib.rs` and the transport |
| C3 | Read-only: both calls are plain GETs through the existing `get()`/`send()` path; no `Origin`, action header or body; no new route, header or host | `SoosApi.kt::battery`, `offline` | ANA24; ANA-S15 signature needles |
| C4 | The session cookie stays in memory only; `login_required` still drops it; nothing new is persisted (`SettingsStore` unchanged) | `SoosApi.kt`, `SettingsStore.kt` | ANA24 cookie test; ANA-S8 needles unchanged |
| C5 | No logging or printing; no server, status-number or exception text on screen: each text is a fixed `strings.xml` resource selected by a server token through an exhaustive mapping; unknown values never reach the UI (refused by the parser) | `PcCards.kt`, `SoosApp.kt` | ANA-S8, ANA-S14, ANA-S15 forbidden lists |
| C6 | No colour literal and no `colorResource` in the cards: `MaterialTheme.colorScheme` only | `PcCards.kt` | ANA-S15; ANA-S7 unchanged |
| C7 | No new dependency, manifest entry, permission, exported component or runtime prompt; `verification-metadata.xml`, `libs.versions.toml`, `build.gradle.kts`, the manifest and the `aapt2` allowlist byte-identical | `android/**`, `scripts/android_build.sh` | `git diff --stat`; ATR-S6, ANA-S2/S4/S6/S10 unchanged |
| C8 | Network calls on `Dispatchers.IO`, UI state on the main thread; the cards load inside the existing `busy` refresh, so no concurrent duplicate refresh | `SoosApp.kt::refresh` | code reading; owner row ANA26 |
| C9 | Time formatting cannot crash: `DateTimeException` caught, fixed "unknown time" text; durations computed with non-overflowing integer division | `SoosApp.kt::alertTime`, `CardModel.kt::offlineDuration` | ANA24 `offlineDurationBoundaries` (Long.MAX_VALUE) |
| C10 | No Rust production change: `crates/**` byte-identical, RC-1 untouched | `crates/**` | `git diff --stat` |
| C11 | Shared tour texts stay byte-equal to `app.js` (`tour_battery_text` unchanged); only app-specific `where` lines change, keeping the keys of ATR-S2 | `strings.xml` | `android_tour_contract` unchanged and green |
| C12 | No existing test or assertion modified (Zero Test Weakening) | test files | `git diff --numstat` shows additions only |

### Pre-existing violations found (not introduced by this change)

None relevant. Accepted residual: an unknown enum value added by a newer server makes a card show
"could not be loaded" until the app is updated (strict parsing requested by #439, as for #404).

### Clearance: CLEARED
