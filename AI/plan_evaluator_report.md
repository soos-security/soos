# Plan Evaluation Report
- **Date**: 2026-10-17
- **Issue**: GitHub-only (no backlog id) — failed-password alerts in the native Android app (GitHub #404, PR 2 of epic #357)
- **Branch**: `feat/android-alerts`
- **Base commit**: `9e82795`
- **Plan evaluated**: `AI/architect_spec_android_alerts.md` §0–§8
- **Mode**: self-check by the implementing agent (the independent candid review follows separately)

## 1. Coverage Matrix

| Acceptance line (issue #404 / triage) | Spec element | Status |
|---|---|---|
| `SoosApi`: paths, `ACTION_ALERTS_ACK`, `alerts()`, `ackAlerts(epoch, through)`, no-body POST with action, one epoch, one through, `Origin` | §2, §2.2 | Covered |
| `ApiJson.parseAlerts`: four states, unknown state fails, counters, last time and source, `through`, epoch, `history`, capped entries and string lengths, 16 KiB bound kept | §2.1 rules 1–8 (strings are bounded by set membership and the exact 16-char epoch) | Covered |
| Alerts card: fixed sentence per state, times in the phone's time zone, Acknowledge, refresh on open, pull to refresh, fixed error texts, colours from `colors.xml`, strings in `strings.xml` | §3 | Covered |
| JVM tests `ApiJsonTest` / `SoosApiTest` | §7, tester contract | Covered |
| ANA-S8 pins the action to `lib.rs`; alerts routes added to the allowed path set if one exists | §6 (no path set exists; paths pinned to `routes.rs` instead) | Covered |
| Docs §2j, ADR, matrix ANA18+, owner hardware row, walkthrough 219 | §8 | Covered |
| `verification-metadata.xml`, manifest, `aapt2` allowlist unchanged; no logging; cookie in memory | §1 "Not touched"; ANA-S6/S8/S10 unchanged | Covered |
| No Rust server change, RC-1 untouched | §1 | Covered |

## 2. Facts Verified

| Fact | Check | Actual | Match |
|---|---|---|---|
| `ACTION_ALERTS_ACK` | `crates/remote/src/lib.rs` | `"alerts-ack"` | Yes |
| Header names | `lib.rs` `ALERTS_EPOCH_HEADER`, `ALERTS_THROUGH_HEADER` | `x-soos-alerts-epoch`, `x-soos-alerts-through` | Yes |
| Paths | `crates/remote/src/routes.rs` `ALERTS_PATH`, `ALERTS_ACK_PATH` | `/api/alerts`, `/api/alerts/ack` | Yes |
| View shape | `crates/remote/src/alerts.rs` `AlertsView`, `AlertRecord` (serde snake_case, `Option` → `null`) | members as in spec §2 | Yes |
| Enum values | `AlertsState`, `UnavailableReason`, `LockScreenCoverage`, `AttemptKind` (`alerts.rs`), `SourceClass`, `AccountClass` (`journal.rs`) | as in spec §2 | Yes |
| History cap | `MAX_ALERT_HISTORY` | 32 | Yes |
| Counters | `u32` saturating | ≤ 4 294 967 295 | Yes |
| Ack `200` body | `server.rs` `alerts_ack` → `alerts_response(200, &view)` | full view JSON | Yes |
| Ack CSRF | `routes.rs` `check_action_csrf`: exact action, optional `Sec-Fetch-Site`, `Origin` absent or `https://<host>` | the app's `Origin` = PC origin | Yes |
| Size | 32 records × ~150 bytes + header ≈ 5 KiB | < 16 KiB transport bound | Yes |
| `PullToRefreshBox` | material3 1.4.0 already in `verification-metadata.xml` | no new artifact | Yes |

## 3. Pillars

1. **Security invariants**: no new route, no server change, no new permission, the cookie path is the
   existing in-memory one; no logging; fixed texts only. Pass.
2. **Panic / totality**: the parser never throws (every `org.json` access is type-checked, one
   internal exception is caught locally); `ackAlerts` validates its input before sending. Pass.
3. **Bounds**: body length, history count, enum membership, exact epoch length, counter ranges. Pass.
4. **Test integrity**: only new test methods and add-only invariant assertions; no existing
   assertion edited. Pass.
5. **Supply chain**: no catalog change. Pass.

## 4. Findings

- n1 (minor, accepted): the JVM tests cannot run on the development host (no JDK/SDK by rule); they
  run in the CI job `Android (debug build)`. The Red phase is shown for the invariant tests locally
  and argued for the JVM tests (unresolved symbols before the developer phase).
- n2 (minor, accepted): unknown enum values make the whole view unreadable instead of falling back
  to "other" as the web app does; this is the strict behaviour the issue asks for, and the card
  then shows a fixed "could not be loaded" sentence.

VALIDATION_VERDICT: APPROVED
