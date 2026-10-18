# Architect Spec — GitHub #427: first-run feature tour in the native Android app

- **Date**: 2026-10-18
- **Issue**: GitHub #427 (GitHub-only, no backlog id) = PR 3 of epic #363; follows the PC tour
  (#379, PR #389) and the phone web app tour (#414, PR #418); native-app parity with #357.
- **Branch**: `feat/android-onboarding-tour`
- **Owner decisions used**: those of 2026-10-12 and 2026-10-17 on #363 (a tour on PC and phone,
  explained features, off paths). No new owner decision is needed: the app already has a private
  settings store excluded from backup and device transfer, and the tour adds one Boolean to it.
- **Contract migration**: none. No existing test is modified.

## 0. Goal

The native Kotlin/Compose app (`android/`) shows the same kind of short, skippable tour as the web
app: an introduction, then one step per feature the phone can reach, what it exposes, what turning
it off changes and where to turn it off. It opens once, after a PC address is saved and the first
status view is loaded; it never shows over the settings screen or the passkey sign-in state; it is
reopened from a **Feature tour** button of the home screen.

## 1. Types (pure Kotlin, `org.soos.remote.tour`, file `tour/Tour.kt`, no `android.` import)

```kotlin
enum class TourStep(val sharedWithWebTour: Boolean) {
    INTRO(false),
    PHONE_COMPANION(true),
    BATTERY(true),
    REMOTE_LOCK(true),
    INTERNET_ACCESS(true),
    FAILED_PASSWORDS(true),
    PUSH_ALERTS(true),
    CAMERA_VIEW(true),
    OFFLINE_ALERTS(true),
    PC_OFFLINE_ALERTS(false),
}

interface TourStore { var tourSeen: Boolean }

data class TourState(val open: Boolean = false, val index: Int = 0, val offered: Boolean = false) {
    val step: TourStep        // TourStep.entries[index clamped to 0..LAST]
    val number: Int           // index + 1, for "Step n of TOTAL"
    val isFirst: Boolean      // index == 0
    val isLast: Boolean       // index >= LAST
    companion object { val TOTAL: Int /* 10 */; val LAST: Int /* 9 */ }
}

class TourController(private val store: TourStore) {
    fun offer(state: TourState, statusShown: Boolean): TourState
    fun replay(state: TourState): TourState
    fun next(state: TourState): TourState
    fun back(state: TourState): TourState
    fun close(state: TourState): TourState
}
```

Transitions (the controller is stateless; Compose holds the immutable `TourState`):

| Call | Precondition | Result | Store |
|---|---|---|---|
| `offer(s, false)` | any | `s` | not read |
| `offer(s, true)` | `s.offered` or `s.open` | `s` | not read |
| `offer(s, true)` | otherwise, `tourSeen == false` | `open, index 0, offered` | read only |
| `offer(s, true)` | otherwise, `tourSeen == true` | `closed, index 0, offered` | read only |
| `replay(s)` | any | `open, index 0, offered` | untouched |
| `next(s)` | closed | `s` | untouched |
| `next(s)` | open, not last | `index + 1` | untouched |
| `next(s)` (Done) | open, last | `close(s)` | `tourSeen = true` |
| `back(s)` | closed or first | `s` | untouched |
| `back(s)` | open, not first | `index - 1` | untouched |
| `close(s)` (Skip tour, system back) | closed | `s` | untouched |
| `close(s)` | open | `closed, index 0, offered` | `tourSeen = true` |

No error type: nothing can fail (no I/O but one preference write, which `SharedPreferences.apply`
performs asynchronously and never reports).

## 2. Storage

`SettingsStore` implements `TourStore` with one key `tour_seen` (`KEY_TOUR_SEEN`), read with
`prefs.getBoolean(KEY_TOUR_SEEN, false)` and written with `putBoolean(...).apply()`, in the
existing private file `soos_settings` (`Context.MODE_PRIVATE`). The manifest keeps
`android:allowBackup="false"` and `data_extraction_rules.xml` already excludes `sharedpref` from
cloud backup and device transfer. No new file, DataStore, database or other storage API; the tour
sources never touch `SharedPreferences`. The heartbeat key needles (AHB3) and the no-cookie /
no-session rule of the store (ANA11) stay true; `tour_seen` holds neither.

## 3. UI (`ui/TourCard.kt`, wired in `ui/SoosApp.kt`)

- `SoosApp` holds `var tourState by remember { mutableStateOf(TourState()) }` and one
  `TourController(settings)`, and passes the state and callbacks to `HomeScreen`. The settings
  screen never receives them, so the tour never shows over it.
- `HomeScreen`: `homeReady = auth?.authenticated == true` (a tailnet identity or a Funnel session).
  `LaunchedEffect(homeReady, status != null)` calls `onTourOffer()` when `homeReady && status !=
  null` (the first status view is loaded). The card is the first item of the scrolled column,
  rendered only when `tourState.open && homeReady` (so never over the passkey sign-in state); on
  open the column scrolls to the top without animation. `BackHandler(enabled = shown)` closes the
  tour (system back). The **Feature tour** `TextButton` is shown when `homeReady` and calls
  `onTourReplay`.
- `TourCard(state, onNext, onBack, onSkip)`: counter "Step n of 10", title (semantics heading),
  explanation, the "Off:" line (or the introduction note), the where line (feature steps only),
  then **Back** (not on the introduction), **Next** / **Done** (last step), **Skip tour** (not on
  the last step). Colours: theme roles only (`MaterialTheme.colorScheme`, built from
  `colors.xml` by `Theme.kt`); no colour literal; no animation (nothing to reduce).
- Every text is a `strings.xml` resource; the step-to-resource map is one exhaustive `when` in
  `TourCard.kt`. The tour code makes no request, calls no `SoosApi`/`SharedApi`/heartbeat method,
  and shows no server value.

## 4. Texts (`res/values/strings.xml`, names `tour_*`)

For each step `<k>` in `intro`, `phone_companion`, `battery`, `remote_lock`, `internet_access`,
`failed_passwords`, `push_alerts`, `camera_view`, `offline_alerts`, `pc_offline_alerts`: `tour_<k>_title`,
`tour_<k>_text`, `tour_<k>_off` (the introduction has `tour_intro_note` instead) and, for the
nine feature steps, `tour_<k>_where`. Labels: `tour_next` "Next", `tour_done` "Done",
`tour_back` "Back", `tour_skip` "Skip tour", `tour_replay` "Feature tour" (all equal to
`app.js`), `tour_step_counter` "Step %1$d of %2$d".

- The eight shared steps: title, text and off are byte-equal to `TOUR_STEPS` of
  `crates/remote/assets/app.js` (after Android resource unescaping); `tour_intro_title` equals
  `TOUR_INTRO_TITLE`.
- The where lines are app-specific (no "Notifications card" or "this page" of the web app; the app
  has no unlock, push, camera or battery card, so those steps point to the web app or `soos-gui`)
  but name exactly the same set of `remote.toml` keys `<key> = false` as the web line, the
  `[android]` table when the web line does, `remote.toml` and the restart command whenever a key
  is named, so following them never makes `soos-remote` exit 78.
- `pc_offline_alerts` (the app's own heartbeat, ADR 2026-10-13 item (8) amended 2026-10-17):
  title "PC offline alerts" (the switch label), the off path is the app's own switch in
  Settings; it names no `remote.toml` key.
- Plain printable ASCII, no markup, no `"`/`<`/`&`/`$`/`{`; apostrophes written `\'`.

## 5. Tests

- **JVM** `android/app/src/test/java/org/soos/remote/TourControllerTest.kt` (CI job
  `Android (debug build)` only; no JDK on the host): the step order and shared flags, the full
  transition table of §1, TOTAL/LAST/number/isFirst/isLast, and that only closing writes the store.
- **Invariant** `tests/invariants/src/android_tour_contract.rs` (ATR-S1 to ATR-S7):
  - ATR-S1 wording parity with `app.js` (titles, texts, off lines, intro title, labels) and the
    exact step list of `strings.xml`;
  - ATR-S2 where lines: same `remote.toml` key set and `[android]` mention as the web, restart
    command, heartbeat step off in the app, plain text;
  - ATR-S3 every `R.string.tour_*` used by the tour code exists, every `tour_*` resource is used,
    the step map is exhaustive;
  - ATR-S4 storage confined to `SettingsStore` (`tour_seen`, `getBoolean(KEY_TOUR_SEEN, false)`,
    `: TourStore`), no other storage API in the tour code, backup exclusions unchanged;
  - ATR-S5 no request, action, heartbeat call, server value or logging in the tour code; pure
    model without `android.` import;
  - ATR-S6 wiring: offer only on the home screen after the status view, render gated by the
    sign-in state, `BackHandler`, Feature tour button, no animation API, theme roles only;
    manifest permissions and the dependency catalog unchanged by this PR;
  - ATR-S7 ADR, matrix rows ATR1–ATR9, §2j subsection, walkthrough 230, §2k updated.

## 6. Invariants touched

ANA9/ANA12/ANA17/AHB1 (manifest, permissions, one exported component, one guarded job service,
one runtime prompt): unchanged and still asserted by their own tests. ANA8 (catalog and
verification metadata): unchanged. ANA10 (no colour literal): the tour uses theme roles only.
ANA11 (no logging): the tour has none. ANA20: alerts texts untouched.

## 7. Out of scope

A tour language other than English; animations; tour steps for features the app may gain later
(unlock, push, camera and battery in the app need their own PRs and update the where lines and the
contract then); the Keystore sign-in (#354 part 2).
