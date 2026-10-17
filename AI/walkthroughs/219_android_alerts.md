# Walkthrough 219 — Failed-Password Alerts in the Native Android App

- **Date**: 2026-10-17
- **Issue**: GitHub #404, PR 2 of the epic #357 (native Android app). GitHub-only: no `AI/BACKLOG.md` entry, not
  registered in `scripts/sync_issue.py`; the commit carries `Closes #404` and `Refs #357`.
  **Branch**: `feat/android-alerts`.
- **ADR**: "[2026-10-17] Failed-Password Alerts in the Native Android App" (`AI/DECISIONS.md`), amending item (2) of
  "[2026-10-13] Native Android App for `soos-remote` (Kotlin and Jetpack Compose)".
- **Documents**: spec `AI/architect_spec_android_alerts.md`; plan self-check `AI/plan_evaluator_report.md`
  (APPROVED); tester contract `AI/tester_contract_android_alerts.md`; auditor constraints
  `AI/auditor_constraints_android_alerts.md` (C1–C10, CLEARED); user documentation `Docs/REMOTE_COMPANION.md` §2j.
- **Matrix criteria**: ANA18–ANA21 (ANA18 and ANA19 pending the CI run of `Android (debug build)`, ANA21 pending
  the owner hardware check).

## 1. Context & Objectives

The foundation PR (#391, walkthrough 208) gave the native Kotlin/Compose app the PC address, the Funnel passkey
sign-in, the status and the lock. The owner wants full parity with the web app (#357 comment of 2026-10-12). The
failed-password alerts are the next read-and-acknowledge feature that the existing session already covers: the
2026-10-13 ADR lists "status, events, lock, alerts, push, battery" for a Funnel web session, and `GET /api/alerts`
and `POST /api/alerts/ack` accept a tailnet identity or a web session. So this PR changes only the Android app and
its contract: no server change, no new dependency, no manifest or permission change, no signing key.

## 2. Architect Design

- **`net/ApiJson.kt`**: `AlertRecord` and `AlertsView` mirror `crates/remote/src/alerts.rs` (`AlertsView`,
  `AlertRecord`, serde snake case). `parseAlerts` is strict and bounded. The whole view is refused on an unknown
  enum value (state, reason, coverage, source, account, kind), a wrong JSON type, a negative or out-of-range number,
  an epoch that is not 16 lowercase hex characters, more than `MAX_ALERT_HISTORY` (32) entries, or a body over
  `MAX_ALERTS_BODY_CHARS` (16384, equal to the unchanged transport bound). Nullable members accept absent or `null`.
  Unknown extra members are ignored, like the existing parsers.
- **`net/SoosApi.kt`**: `alerts()` is a plain `GET` through the shared `get` path. `ackAlerts(epoch, through)`
  validates the snapshot locally (an invalid one is `Refused(400, "bad_request")` and is never sent). It then sends
  a body-less `POST` with `Accept`, `Origin`, `X-Soos-Action: alerts-ack`, one `X-Soos-Alerts-Epoch` and one
  `X-Soos-Alerts-Through`, plus the in-memory cookie. `send` gained an optional `extra` header list; the existing
  calls are unchanged. A `200` answer is parsed as the fresh view.
- **`ui/SoosApp.kt`**: the home screen content now scrolls inside a material3 `PullToRefreshBox`. Pull to refresh,
  the Refresh button and opening the screen all run the same `refresh()`, which loads the alerts after the status
  when the session is authenticated. The new `AlertsCard` shows one fixed sentence per state, the unacknowledged
  counts, the last attempt, the lock-screen coverage note, the history (time in the phone's time zone through
  `java.time` and `ZoneId.systemDefault()`, source, account class, count) and *Acknowledge*. The button appears only
  when an epoch is present and a counter is non-zero. A refused acknowledgement shows a fixed sentence and refreshes
  the view; it never acknowledges blindly.
- **`res/values/strings.xml`**: every alerts text, including `_one`/`_other` count texts (English only). Server
  tokens only select a resource.

## 3. Tester Phase (Red)

New JVM tests (CI only, no JDK or SDK on the development host): five `ApiJsonTest` methods (ANA18) and six
`SoosApiTest` methods (ANA19). They referenced the not-yet-existing `AlertsView`, `AlertRecord`,
`ApiJson.parseAlerts`, `SoosApi.alerts`, `SoosApi.ackAlerts` and `SoosApi.ACTION_ALERTS_ACK`, so they could not
compile before the developer phase. The invariant run
`cargo test -p soos-invariants --lib android_native_contract` failed for the right reasons:
`test_ana_s8_kotlin_hygiene` with `ACTION_ALERTS_ACK not found`, and the new `test_ana_s14_alerts_contract` with
`§2j must mention GET /api/alerts`. The 12 other tests of the module passed. No existing test or assertion was
changed. The Kotlin files gained imports and appended methods. The invariant file gained constants, two helpers, an
add-only block in ANA-S8 and a new test.

## 4. Auditor Constraints

C1 total parser (no `!!`, type-checked `org.json` access, one private exception caught locally); C2 bounds; C3 the
CSRF contract equals the server's; C4 the cookie stays in memory only; C5 no logging and no server or exception text
on screen; C6 no colour literal; C7 no dependency, manifest, permission or metadata change; C8 network calls on
`Dispatchers.IO`, with no concurrent pull-to-refresh while busy; C9 no Rust change; C10 no test weakened. CLEARED.

## 5. Developer Phase (Green)

- `cargo test -p soos-invariants --lib android_native_contract`: 14 passed.
- `cargo test -p soos-invariants`: 708 passed, 0 failed.
- During the green phase, the new ANA-S14 needle `Text(result` turned out to also match the existing, compliant
  `refusalText(result.status, result.result)` calls. The new check (never green before) was refined to reject only a
  bare `Text(result..)` call, which is not preceded by an identifier character. No pre-existing assertion was touched.
- `cargo fmt --check` and `cargo clippy -p soos-invariants --all-targets -- -D warnings`: clean.
- The Kotlin code is compiled, unit-tested and linted only by the CI job `Android (debug build)` (strict dependency
  verification, unchanged `aapt2` allowlist). That run is recorded in the pull request, and ANA18/ANA19 move to
  verified with it.

## 6. Security Notes

- No new route, origin, cookie or storage; the cookie is still dropped on `login_required`.
- The acknowledgement only ever names the epoch and `through` of the view on screen. `stale_view` triggers a fresh
  read, never a blind retry.
- Strict parsing means a newer server enum makes the card say "Alerts could not be loaded" until the app is updated
  (accepted in the ADR). This errs towards showing an explicit failure, never "no attempts".

## 7. Out of Scope

Battery and offline-history cards (next, read-only); push (needs its own owner/ADR decision); unlock and camera view
(need a server-origin ADR); Keystore device key (#354 part 2); release signing (#361). The opt-in PC-offline heartbeat (#415, part of #359)
landed first (PR #419): the alerts reuse its process-wide `SharedApi.forOrigin` client, so the session stays single.

## 8. Owner Check (ANA21)

Follow `Docs/REMOTE_COMPANION.md` §2j "Owner hardware check of the alerts": list, pull to refresh and acknowledge
over the tailnet and over Funnel, then the stale-view path after a service restart.
