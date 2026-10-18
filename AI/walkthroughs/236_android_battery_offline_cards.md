# Walkthrough 236 — Battery and Offline-History Cards in the Native Android App

- **Date**: 2026-10-18
- **Issue**: GitHub #439 (GitHub-only, no backlog id), part of epic #357 — **Branch**: `feat/android-cards`
- **Owner decisions**: none needed. Item (5) of ADR 2026-10-17 "Failed-Password Alerts in the
  Native Android App" names these read-only cards as the next PR.
- **Contract migration**: none. No existing test or assertion was edited. `ApiJsonTest.kt`,
  `SoosApiTest.kt` and `android_native_contract.rs` only gained lines, and `CardModelTest.kt`
  is new.
- **Matrix criteria**: ANA22–ANA26 (ANA26 is an owner hardware check, pending).

## 1. Context & Objectives

`soos-remote` already serves two read-only views:

- `GET /api/battery`: the level, charge state and mains presence (ADR 2026-10-07, §2g).
- `GET /api/offline`: the offline periods of the PC (ADR 2026-10-13, §2j).

The web app shows both. The native app (#357) showed neither: `SoosApi.kt` had no call for them
and `ApiJson.kt` had no view type. This PR adds both views to the app's home screen. It changes
nothing on the server.

## 2. Architect Design

The spec is `AI/architect_spec_android_cards.md`. The wire contract was checked against
`crates/remote/src/battery.rs`, `offline.rs`, `routes.rs` and `lib.rs`.

- **`net/ApiJson.kt`**
  - Adds three view types: `BatteryView`, `OfflineInterval` and `OfflineView`.
  - Adds six enum sets that equal the server's snake_case variants.
  - Adds the bounds `MAX_OFFLINE_INTERVALS = 32`, `MAX_BATTERY_PERCENT = 100` and
    `MAX_VIEW_BODY_CHARS = 16384`.
  - Adds two strict parsers. Like `parseAlerts`, they type-check every member and catch any
    malformed input. They also enforce the server's cross-field rules:
    - battery: values only with `present`, and `charge` always set with `present`;
    - offline: a `reason` exactly when `unavailable`; and unless `active`, the monitor is
      `unknown` and there are no intervals;
    - every interval: `end >= start`.
- **`net/SoosApi.kt`**: adds `battery()` and `offline()`. Both are plain GETs that go through
  the existing `get()`/`send()` path, so the session cookie and the `login_required` rule are
  unchanged.
- **`cards/CardModel.kt`** (pure Kotlin) picks what each card says. It applies the same rules as
  the web page's `batteryText` and `durationText`.
- **`ui/PcCards.kt`** holds the two cards, *Battery* and *Offline history*.
  - Every text comes from `strings.xml`, every colour from the theme, and each title is a
    heading.
  - The cards make no request and have no action.
- **`ui/SoosApp.kt`**: `refresh()` loads both views after the alerts in the signed-in branch
  (`loadCards()`). It clears them when the PC is unreachable or the user is signed out
  (`clearCards()`). The cards sit under the alerts card. The existing `alertTime` and `timeText`
  helpers are now `internal`, so the cards share the alerts' time formatter (`java.time`,
  `ZoneId.systemDefault()`).

## 3. Plan Evaluation

The plan check is `AI/plan_evaluator_report.md` (self-check): APPROVED, with two findings.

- **F-1:** the task asked for a change to `tour_battery_text`. That text must stay byte-equal to
  the web tour (`app.js`), and ATR-S1 enforces this. Changing it would need a server asset change
  and an edit of an existing test, and both are excluded. The app-specific `tour_battery_where`
  line names the Battery card instead, which ATR-S2 allows. `tour_offline_alerts_where` now names
  the Offline history card too.
- **F-2:** a newer server enum makes a card show "could not be loaded" until the app is updated.
  This is accepted, as it was for the alerts.

## 4. Test Contract

The contract is `AI/tester_contract_android_cards.md`.

- **ANA22 (battery parser)**: exact views, every state, cross-field rules, enums, types,
  percents 0, 100 and 101, and bodies of 16384 and 16385 characters.
- **ANA23 (offline parser)**: exact views, every state, monitor and reason, cross-field rules,
  negative and beyond-`Long` numbers, `end < start`, and 32 against 33 intervals.
- **ANA24 (client and model)**: the plain-GET headers, the cookie, `login_required`, the
  refusal mapping, and the duration boundaries up to `Long.MAX_VALUE`.
- **ANA25 (invariant)**: `android_native_contract::test_ana_s15_battery_and_offline_cards_contract`.
  Before the implementation it failed with `BATTERY_PATH not found`.

The JVM tests run only in the CI job `Android (debug build)`, because the development host has
no JDK or SDK by rule.

## 5. Audit Constraints

The constraints are in `AI/auditor_constraints_android_cards.md`, C1–C12, and the audit is
CLEARED. They cover the following:

- parsers that are total and bounded;
- read-only GETs;
- the cookie kept in memory only;
- fixed texts only, with no server or exception text and no logging;
- theme colours only;
- no dependency, manifest or permission change;
- network calls on `Dispatchers.IO`;
- a time formatter that cannot crash;
- no Rust change;
- the shared tour texts unchanged;
- no test weakened.

## 6. Implementation

- **Production code:**
  - Kotlin: `ApiJson.kt` (+ types, sets, bounds, parsers, `nullableBool`), `SoosApi.kt`
    (+ 2 calls, 2 paths), the new `cards/CardModel.kt` and `ui/PcCards.kt`, and `SoosApp.kt`
    (state, `loadCards`, `clearCards`, wiring, two helpers made `internal`).
  - `strings.xml`: 30 new `battery_*` and `offline_*` texts, plus two `where` lines reworded.
- **Unchanged:** the manifest, Gradle files, catalog, `verification-metadata.xml`,
  `scripts/android_build.sh` and `crates/**`.

## 7. Verification

These commands were run with the scoped environment from AGENTS.md
(`CARGO_TARGET_DIR=~/.cache/soos-targets/android-cards`):

- `cargo fmt -p soos-invariants -- --check`
- `cargo clippy -p soos-invariants --locked --all-targets --all-features -- -D warnings`
- `cargo test -p soos-invariants --locked -- --test-threads=3`: green, including
  `android_native_contract`, `android_tour_contract` and `android_alerts_feedback_contract`.

Not run locally: the Kotlin build, the JVM unit tests and lint. They run in the CI job
`Android (debug build)` on the pull request (ANA22–ANA24 stay pending until then).

## 8. Documentation

- **`Docs/REMOTE_COMPANION.md` §2j:**
  - the introduction now lists the two cards;
  - a new "Battery and offline history" bullet covers both GETs, the texts, the strict parser
    and `ZoneId.systemDefault()`;
  - the owner hardware check (matrix ANA26) is new;
  - the "Feature tour in the app" paragraph is updated.
- **`README.md`**: the native app row lists the cards.
- **`AI/DECISIONS.md`**: ADR 2026-10-18 "Battery and Offline-History Cards in the Native
  Android App".

## 9. Owner Hardware Check (pending, ANA26)

The steps are in `Docs/REMOTE_COMPANION.md` §2j, "Owner hardware check of the battery and
offline-history cards". Check on a phone, in light and dark mode, over the tailnet and over
Funnel:

- the Battery card against the web page and a charger change;
- the `battery_status = false` sentence;
- an offline period of more than a minute in the Offline history card;
- the `offline_alerts = false` sentence;
- sign-out hides both cards;
- the other features still work.
