# Architect Spec — GitHub #404: failed-password alerts in the native Android app (PR 2 of #357)

- **Branch**: `feat/android-alerts` (GitHub-only issue: not registered in `scripts/sync_issue.py`)
- **Epic**: #357. Foundation: GitHub #382 / PR #391, ADR 2026-10-13 "Native Android App for
  `soos-remote` (Kotlin and Jetpack Compose)" (merged).
- **Matrix prefix**: `ANA` continued (rows ANA18–ANA21; ANA1–ANA17 exist on `origin/main`).
  **Walkthrough**: 219.

## 0. Owner decisions applied and assumptions

| # | Decision | Source |
|---|---|---|
| OD-1 | The Android app is 100% native (Kotlin + Jetpack Compose), soos brand, full feature parity with the web app, sideloaded only, private | #357 comment 2026-10-12 |
| OD-2 | Push, camera view, unlock, the Keystore key and the heartbeat are later PRs | ADR 2026-10-13 item (2) |
| OD-3 | A Funnel web session already covers "status, events, lock, alerts, push, battery" | ADR 2026-10-13 item (5) |
| F-1 | `GET /api/alerts` and `POST /api/alerts/ack` are authenticated routes (tailnet identity or web session), never Funnel-public; the server checks the ack `Origin` against `https://<normalized host>` (`check_action_csrf`, `crates/remote/src/routes.rs`), which is exactly the origin the app already sends for `POST /api/lock` | code, verified |
| F-2 | `200` of the ack is the fresh `AlertsView` JSON (`alerts_response(200, &view)`, `crates/remote/src/server.rs`), not a `{"result"}` body; refusals are `{"result": ...}` with `403 forbidden`, `403 alerts_disabled`, `400 bad_request`, `429 rate_limited`, `409 stale_view`, `503 unavailable`, and the generic `403 login_required` of the session gate | code, verified |

No new owner decision is needed: the change is client-only and uses routes the session already
covers (OD-3). No server change, no new dependency, no manifest change, no permission change, no
signing key.

## 1. Scope & Blast Radius

- `android/app/src/main/java/org/soos/remote/net/SoosApi.kt`: `alerts()`, `ackAlerts(epoch, through)`,
  constants; `send` gains an optional list of extra headers (existing calls unchanged).
- `android/app/src/main/java/org/soos/remote/net/ApiJson.kt`: `AlertsView`, `AlertRecord`,
  `parseAlerts`, the enum sets and caps.
- `android/app/src/main/java/org/soos/remote/ui/SoosApp.kt`: an Alerts card on the home screen,
  pull to refresh around the home screen, refresh on open.
- `android/app/src/main/res/values/strings.xml`: the new texts (strings and plurals).
- JVM tests: `ApiJsonTest.kt`, `SoosApiTest.kt` (new test methods only; existing ones untouched).
- `tests/invariants/src/android_native_contract.rs`: ANA-S8 extended (add-only); new test
  `test_ana_s14_alerts_contract` (docs, ADR, walkthrough, strings, no colour literal).
- Docs: `Docs/REMOTE_COMPANION.md` §2j, `AI/DECISIONS.md` (new ADR entry), matrix ANA18–ANA21,
  walkthrough 219.
- **Not touched**: any Rust crate (`crates/remote` byte-identical, RC-1 untouched), the manifest,
  `libs.versions.toml`, `verification-metadata.xml`, `build.gradle.kts`, the CI workflow, the
  build script and its `aapt2` allowlist, `colors.xml`, the transport.

## 2. Types & Signatures (Kotlin, package `org.soos.remote.net`)

```kotlin
/** One history record of `GET /api/alerts` (newest first). */
data class AlertRecord(
    val id: Long, val firstUnixMs: Long, val lastUnixMs: Long,
    val source: String,   // in ApiJson.ALERT_SOURCES
    val account: String,  // in ApiJson.ALERT_ACCOUNTS
    val kind: String,     // in ApiJson.ALERT_KINDS
    val count: Long,      // 1..MAX_ALERT_COUNT
)

/** `GET /api/alerts` and the `200` body of `POST /api/alerts/ack`. */
data class AlertsView(
    val state: String,            // in ApiJson.ALERT_STATES
    val reason: String?,          // null or in ApiJson.ALERT_REASONS
    val epoch: String?,           // null or 16 lowercase hex
    val lockScreen: String?,      // null or in ApiJson.LOCK_SCREEN_COVERAGE
    val unacknowledgedWrongPassword: Long,  // 0..MAX_ALERT_COUNT
    val unacknowledgedLockedOut: Long,      // 0..MAX_ALERT_COUNT
    val lastUnixMs: Long?,        // null or >= 0
    val lastSource: String?,      // null or in ALERT_SOURCES
    val through: Long,            // >= 0
    val history: List<AlertRecord>, // at most MAX_ALERT_HISTORY
)

object ApiJson {
    val ALERT_STATES = setOf("disabled", "starting", "active", "unavailable")
    val ALERT_REASONS = setOf("no_journal_access", "journal_reader_failed", "owner_unresolved", "overflow", "rng_failed")
    val LOCK_SCREEN_COVERAGE = setOf("monitored", "not_configured")
    val ALERT_SOURCES = setOf("lock_screen", "sudo", "login", "other")
    val ALERT_ACCOUNTS = setOf("owner", "root", "other")
    val ALERT_KINDS = setOf("wrong_password", "locked_out")
    const val MAX_ALERT_HISTORY = 32          // equals crates/remote/src/lib.rs MAX_ALERT_HISTORY
    const val ALERTS_EPOCH_HEX_LEN = 16       // equals lib.rs ALERTS_EPOCH_HEX_LEN
    const val MAX_ALERT_COUNT = 4_294_967_295L // u32::MAX (saturating counters)
    const val MAX_ALERTS_BODY_CHARS = 16384   // = UrlConnectionTransport.MAX_RESPONSE_BYTES
    fun parseAlerts(body: String): AlertsView?
    fun isAlertsEpoch(value: String): Boolean
}

class SoosApi {
    fun alerts(): ApiResult<AlertsView>
    fun ackAlerts(epoch: String, through: Long): ApiResult<AlertsView>
    companion object {
        const val ACTION_ALERTS_ACK = "alerts-ack"            // = lib.rs ACTION_ALERTS_ACK
        const val ALERTS_PATH = "/api/alerts"                 // = routes.rs ALERTS_PATH
        const val ALERTS_ACK_PATH = "/api/alerts/ack"         // = routes.rs ALERTS_ACK_PATH
        const val ALERTS_EPOCH_HEADER = "X-Soos-Alerts-Epoch"     // lowercased = lib.rs
        const val ALERTS_THROUGH_HEADER = "X-Soos-Alerts-Through" // lowercased = lib.rs
    }
}
```

### 2.1 Parser rules (`parseAlerts`, strict, total, never throws)

1. `body.length > MAX_ALERTS_BODY_CHARS` → `null` (the transport already bounds bytes; this keeps
   the parser bounded when called directly).
2. Not a JSON object → `null`.
3. Every enum value must be in its set; an unknown `state`, `reason`, `lock_screen`, `last_source`,
   `source`, `account` or `kind` → `null` (the whole view; no fallback label is invented).
4. Nullable members (`reason`, `epoch`, `lock_screen`, `last_unix_ms`, `last_source`) accept
   absent or JSON `null`; any other type → `null`.
5. Counters are JSON integers (`Int`/`Long`) in `0..MAX_ALERT_COUNT`; `through`, ids and times are
   integers `>= 0`; `count` is in `1..MAX_ALERT_COUNT`. A string, a float, a boolean or a value out
   of range → `null`. (`through` beyond `Long.MAX_VALUE` is a `BigInteger` in `org.json` → `null`.)
6. `epoch` must be exactly 16 characters of `[0-9a-f]` when present.
7. `history` is a JSON array of at most `MAX_ALERT_HISTORY` objects; more entries, or any entry that
   is not a valid object → `null`.
8. Unknown extra members are ignored (forward compatible, like the existing parsers).

### 2.2 Client rules

- `alerts()`: `GET /api/alerts`, headers `Accept` and the session `Cookie` only (no `Origin`, no
  action header, no body), `200` → `parseAlerts` (malformed → `Unreachable`), other status →
  `Refused(status, result)`.
- `ackAlerts(epoch, through)`: when `!isAlertsEpoch(epoch) || through < 0` → `Refused(400,
  "bad_request")` **without sending anything**. Otherwise `POST /api/alerts/ack` with `Accept`,
  `Origin: <origin>`, `X-Soos-Action: alerts-ack`, exactly one `X-Soos-Alerts-Epoch: <epoch>` and
  one `X-Soos-Alerts-Through: <through in decimal>`, the session cookie, no `Content-Type`, body
  `null`. `200` → `parseAlerts` of the body (malformed → `Unreachable`); other → `Refused`.
- `login_required` and `Set-Cookie` handling are the shared `send` path (unchanged).

## 3. UI (Compose, `ui/SoosApp.kt`)

- The home screen content sits in a vertically scrolling `Column` inside a material3
  `PullToRefreshBox` (material3 1.4.0, already resolved; `@OptIn(ExperimentalMaterial3Api::class)`).
  Pull to refresh and the Refresh button run the same `refresh()`; `LaunchedEffect(pc.origin)`
  refreshes on open.
- `refresh()` fetches `alerts()` after a successful status when the session is authenticated; a
  signed-out state clears the alerts view.
- Alerts card (only while authenticated): title "Password alerts"; one fixed sentence per state
  (`disabled`, `starting`, `active` with/without unacknowledged attempts, `unavailable` plus a fixed
  reason sentence per known reason); the lock-screen coverage note for `not_configured`; the history
  (each row: time in the phone's time zone through `java.time` `ZoneId.systemDefault()` and
  `DateTimeFormatter.ofLocalizedDateTime(FormatStyle.SHORT)`, source, account, kind and count as
  fixed strings); an **Acknowledge** button shown only when an epoch is present and at least one
  counter is non-zero; its outcome as a fixed sentence.
- Ack outcomes: `200` → new view + "Acknowledged."; `stale_view` → fixed sentence + refetch;
  `rate_limited`, `alerts_disabled`, `unavailable`, `bad_request`, `forbidden`, `login_required` →
  fixed sentences; anything else → a fixed generic sentence; `Unreachable` → fixed sentence. A
  fetch failure shows "Alerts could not be loaded." Never any server text, status number or
  exception text.
- Colours: theme colours only (`MaterialTheme.colorScheme`, built from `colors.xml`); an attention
  accent uses `colorResource(R.color.soos_danger_text)` / `soos_warn_text`. No colour literal.
- All new texts in `res/values/strings.xml` (`alerts_*` names; plurals for counts).

## 4. Constants & Config

No configuration change. Kotlin constants above; pinned to Rust by ANA-S8.

## 5. Error taxonomy

`ApiResult` unchanged: `Ok(AlertsView)`, `Refused(status, result?)`, `Unreachable`.

## 6. Invariants touched (add-only)

- ANA-S8 (`test_ana_s8_kotlin_hygiene`): the (kotlin, rust) action pair list gains
  `("ACTION_ALERTS_ACK", "ACTION_ALERTS_ACK")`; new assertions pin `ALERTS_PATH`,
  `ALERTS_ACK_PATH` to `crates/remote/src/routes.rs`, the two header names (lowercased) to
  `lib.rs`, and `MAX_ALERT_HISTORY` / `ALERTS_EPOCH_HEX_LEN` to `lib.rs`. All existing needles stay.
  No allowed-path set exists in the contract, so none is extended.
- New `test_ana_s14_alerts_contract`: §2j mentions the alerts routes and headers; the ADR entry
  exists with its needles; walkthrough 219 exists; every `alerts_` string referenced by
  `SoosApp.kt` exists in `strings.xml`; the alerts UI reads no `Color(` literal (already covered by
  ANA-S7) and shows no `result.result`/`e.message` text.
- ANA-S1…S7, S9…S13 unchanged; RC-1 and every `remote_*` contract unchanged.

## 7. Test hooks

The fake `HttpTransport` of `SoosApiTest` records requests; `ApiJson.parseAlerts` is pure.

## 8. Documentation / ADR

- `Docs/REMOTE_COMPANION.md` §2j: the first-version paragraph now lists alerts as delivered; a new
  "Failed-password alerts" bullet and owner check steps.
- New ADR entry "[2026-10-17] Failed-Password Alerts in the Native Android App (GitHub #404 = PR 2
  of #357)".
- Matrix ANA18 (JVM parser tests), ANA19 (JVM client tests), ANA20 (invariants), ANA21 (owner
  hardware: list and acknowledge over Funnel and over the tailnet, pending).
- Walkthrough `AI/walkthroughs/219_android_alerts.md`.
