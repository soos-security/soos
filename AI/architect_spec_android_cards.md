# Architect Spec — Battery and Offline-History Cards in the Native Android App

- **Issue**: GitHub #439 (part of epic #357; GitHub-only, no backlog id, so the branch is not
  registered in `scripts/sync_issue.py` and the commit carries `Closes #439`)
- **Branch**: `feat/android-cards`
- **Base**: `origin/main` 3f76090
- **Follows**: ADR 2026-10-17 "Failed-Password Alerts in the Native Android App" item (5) (the
  battery and offline-history cards are the next read-only PR); ADR 2026-10-07 "Live Battery Level
  in `soos-remote`"; ADR 2026-10-13 "Offline History and Back-Online Push Notifications in
  `soos-remote`".
- **Walkthrough**: 236. **Matrix**: ANA22–ANA26.

## 0. Scope

Android-only. The server already serves both routes (`crates/remote/src/routes.rs`
`BATTERY_PATH = "/api/battery"`, `OFFLINE_PATH = "/api/offline"`; `Docs/REMOTE_COMPANION.md`
§2g, §2j and the §6 route table). Both answer `200` with the view, tailnet identity or Funnel web
session required (`403 login_required` otherwise). No server change, no new route, no manifest or
permission change (the `aapt2` allowlist is unchanged), no new dependency or catalog entry
(`verification-metadata.xml` unchanged), no signing. RC-1 untouched.

Out of scope (unchanged): the `event: battery` stream (the app makes no SSE request), push, unlock,
camera, release signing.

## 1. Server wire contract (verified against the code)

### 1.1 `GET /api/battery` — `crates/remote/src/battery.rs` `BatteryView` (exactly four keys)

| Key | Rust | Values |
|---|---|---|
| `state` | `BatteryState` (`snake_case`) | `disabled`, `unavailable`, `no_battery`, `present` |
| `percent` | `Option<u8>` | `null`, or `0..=100` only when `present` (`parse_capacity` ≤ 100, weighted or mean) |
| `charge` | `Option<ChargeStatus>` (`snake_case`) | `Some` iff `present`: `charging`, `discharging`, `full`, `not_charging`, `unknown` |
| `external_power` | `Option<bool>` | `null`, or a boolean only when `present` |

`disabled()`, `unavailable()` and `no_battery()` set the three values to `None`; `aggregate`
sets `charge: Some(..)` for `present`.

### 1.2 `GET /api/offline` — `crates/remote/src/offline.rs` `OfflineView` (exactly four keys)

| Key | Rust | Values |
|---|---|---|
| `state` | `OfflineState` | `disabled`, `active`, `unavailable` |
| `reason` | `Option<OfflineUnavailableReason>` | `Some(store_failed)` iff `unavailable` |
| `network_monitor` | `NetworkMonitor` | `unknown`, `active`, `unavailable`; `unknown` unless `active` |
| `intervals` | `Vec<OfflineInterval>` | newest first, empty unless `active`, at most `MAX_OFFLINE_INTERVALS` = 32 (`lib.rs`) |

`OfflineInterval` = `{reason, start_unix_ms, end_unix_ms, duration_ms}`: `reason` in
`network_lost`, `wifi_disabled`, `sleep`, `shutdown_or_crash`; `u64` times; `start <= end`
(`OfflineInterval::new` and the store loader, which refuses `start > end`). The body is at most
32 × ~120 bytes, far below the 16384-byte transport bound.

## 2. Kotlin types (`android/app/src/main/java/org/soos/remote/net/ApiJson.kt`)

```kotlin
data class BatteryView(val state: String, val percent: Int?, val charge: String?, val externalPower: Boolean?)
data class OfflineInterval(val reason: String, val startUnixMs: Long, val endUnixMs: Long, val durationMs: Long)
data class OfflineView(val state: String, val reason: String?, val networkMonitor: String, val intervals: List<OfflineInterval>)
```

Constants in `ApiJson` (pinned by `soos-invariants`):

- `BATTERY_STATES`, `CHARGE_STATUSES`, `OFFLINE_STATES`, `OFFLINE_UNAVAILABLE_REASONS`,
  `NETWORK_MONITORS`, `OFFLINE_REASONS`: `setOf(...)` equal to the snake_case variants of the
  six Rust enums.
- `const val MAX_OFFLINE_INTERVALS = 32` (equals `lib.rs`).
- `const val MAX_BATTERY_PERCENT = 100`.
- `const val MAX_VIEW_BODY_CHARS = 16384` (equals `UrlConnectionTransport.MAX_RESPONSE_BYTES`).

### 2.1 `parseBattery(body: String): BatteryView?`

`null` (whole view refused) when: body longer than `MAX_VIEW_BODY_CHARS`; not a JSON object;
`state` missing, not a string or not in `BATTERY_STATES`; `percent` neither absent/`null` nor an
integer in `0..MAX_BATTERY_PERCENT`; `charge` neither absent/`null` nor a member of
`CHARGE_STATUSES`; `external_power` neither absent/`null` nor a boolean; a value present while
`state != present`; `charge` absent/`null` while `state == present`. Unknown extra members are
ignored, an absent nullable member reads as `null` (same rules as `parseAlerts`).

### 2.2 `parseOffline(body: String): OfflineView?`

`null` when: body longer than `MAX_VIEW_BODY_CHARS`; not an object; `state` not in
`OFFLINE_STATES`; `reason` neither absent/`null` nor in `OFFLINE_UNAVAILABLE_REASONS`; `reason`
`null` while `unavailable` or non-`null` otherwise; `network_monitor` missing or not in
`NETWORK_MONITORS`, or not `unknown` while `state != active`; `intervals` missing or not an
array; more than `MAX_OFFLINE_INTERVALS` entries; non-empty while `state != active`; an entry that
is not an object, has a reason outside `OFFLINE_REASONS`, a time or duration that is not an
integer ≥ 0 (a JSON number beyond `Long` is not an `Int`/`Long` in `org.json`, so it is refused),
or `end_unix_ms < start_unix_ms`. Order is kept as received (newest first); it is not re-checked
(the server's order is its insertion order).

No "other" fallback: a newer server enum shows the card's "could not be loaded" text until the
app is updated (accepted, same as the alerts ADR item (3)).

## 3. Client (`net/SoosApi.kt`)

```kotlin
fun battery(): ApiResult<BatteryView> = get(BATTERY_PATH) { ApiJson.parseBattery(it) }
fun offline(): ApiResult<OfflineView> = get(OFFLINE_PATH) { ApiJson.parseOffline(it) }
const val BATTERY_PATH = "/api/battery"
const val OFFLINE_PATH = "/api/offline"
```

Plain GETs through the existing private `get()`: `Accept: application/json`, no `Origin`, no
action header, no body, the in-memory session cookie when present; a `403 login_required` sent
with the current session drops it (existing `send()` rule); a non-`200` is `Refused(status,
token)`, an unreadable `200` is `Unreachable`. The screens use the process-wide client of
`SharedApi.forOrigin` (unchanged).

## 4. Pure card model (`cards/CardModel.kt`, pure Kotlin, JVM-tested)

```kotlin
enum class BatteryLine { DISABLED, UNAVAILABLE, NO_BATTERY, LEVEL, LEVEL_UNKNOWN }
enum class BatteryPower { CHARGING, ON_BATTERY, FULL, PLUGGED_IN_NOT_CHARGING, PLUGGED_IN }
sealed interface OfflineDuration { UnderMinute; Minutes(m); Hours(h); HoursMinutes(h, m); Days(d) }
object CardModel {
    fun batteryLine(view: BatteryView): BatteryLine
    fun batteryPower(view: BatteryView): BatteryPower?
    fun offlineDuration(durationMs: Long): OfflineDuration
}
```

Same rules as the web page (`crates/remote/assets/app.js` `batteryText`, `durationText`):

- `batteryLine`: `disabled` → DISABLED, `unavailable` → UNAVAILABLE, `no_battery` → NO_BATTERY,
  `present` with a percent → LEVEL, else LEVEL_UNKNOWN.
- `batteryPower` (only for `present`): `charging` → CHARGING, `discharging` → ON_BATTERY,
  `full` → FULL, `not_charging` → PLUGGED_IN_NOT_CHARGING, `unknown` with `external_power ==
  true` → PLUGGED_IN, otherwise `null`.
- `offlineDuration`: `< 60 000` ms → UnderMinute; `< 60` min → Minutes; `< 48` h → Hours (whole
  hours) or HoursMinutes; else Days (`hours / 24`).

## 5. UI (`ui/PcCards.kt`, wired in `ui/SoosApp.kt`)

- `BatteryCard(view: BatteryView?, failed: Boolean)` and `OfflineCard(view: OfflineView?,
  failed: Boolean)`: Material3 `Card`s like the alerts card, shown while signed in (tailnet
  identity or Funnel session) under the alerts card; nothing shown before the first load.
- Texts: fixed `strings.xml` resources only (`battery_*`, `offline_*`); a server value only
  selects a resource; colours from `MaterialTheme.colorScheme` (the theme built from
  `colors.xml`); no colour literal, no `colorResource`.
- Times: the existing `alertTime` formatter (made `internal`): `java.time`,
  `DateTimeFormatter.ofLocalizedDateTime(FormatStyle.SHORT)`, `ZoneId.systemDefault()`;
  `DateTimeException` → the fixed "unknown time" text.
- Refresh: `HomeScreen.refresh()` loads both views after the alerts in the authenticated
  branch (open, *Refresh*, pull to refresh, sign-in, sign-out, *Lock PC*, acknowledgement
  refresh) and clears them in the unreachable and signed-out branches.
- Nothing logged, no exception or server text displayed.

## 6. Tour

`tour_battery_text` is byte-equal to the web tour (`app.js` `TOUR_STEPS`), pinned by
`android_tour_contract::test_atr_s1_shared_wording_is_byte_equal_to_the_web_tour`: changing it
would need an `app.js` (server asset) change and an edit of that test, both excluded. The app
names its new card in the app-specific `tour_battery_where` line instead ("In this app the battery
level is the Battery card of the home screen; the web app on this phone shows it too. On the PC:
..."), which keeps the `web app` mention and the `battery_status = false` key required by ATR-S2.
`tour_offline_alerts_where` names the Offline history card as well.

## 7. Tests (contract)

| Test | Covers |
|---|---|
| `ApiJsonTest.battery*` | every state, exact present view, nulls/absent, cross-field rules, enums, types, 0/100/101/-1, 16384/16385 |
| `ApiJsonTest.offline*` | every state/monitor/reason, exact active view, cross-field rules, types, negative, `end < start`, beyond `Long`, 32/33 intervals, 16384/16385 |
| `SoosApiTest.batteryAndOffline*` | plain GET headers and URL, cookie sent, `login_required` drops the session, refusal and unreadable mapping |
| `CardModelTest` | `batteryLine`, `batteryPower`, `offlineDuration` boundaries |
| `android_native_contract::test_ana_s15_battery_and_offline_cards_contract` (add-only) | paths = `routes.rs`; six enum sets = Rust snake_case variants; `MAX_OFFLINE_INTERVALS` = `lib.rs`; body bound = transport; SoosApi signatures; card texts are used `strings.xml` resources; no literal text, colour literal or exception text; `ZoneId.systemDefault()`; wiring in `refresh()`; tour where line; ADR, §2j, matrix rows and walkthrough 236 |

## 8. Invariants touched

No logging (ANA-S8 unchanged), HTTPS transport unchanged, cookie in memory only, no new
permission or runtime prompt (ATR-S6 unchanged), no new dependency (ATR-S6, ANA-S4 unchanged).
