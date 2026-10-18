# Plan Evaluation Report
- **Date**: 2026-10-18
- **Issue**: GitHub-only (no backlog id) — first-run feature tour in the native Android app (GitHub #427, PR 3 of epic #363)
- **Branch**: `feat/android-onboarding-tour`
- **Base commit**: `74782b8`
- **Plan evaluated**: `AI/architect_spec_android_onboarding_tour.md` §0–§7
- **Mode**: self-check by the implementing agent (phases run in one session as the dev-workflow skill allows). The
  independent candid review still runs separately before any push.

## 1. Coverage Matrix

| #363 PR 3 / #427 acceptance line | Spec element | Status |
|---|---|---|
| Tour in `ui/SoosApp.kt`, once after the PC address is saved and the first status view loads | §3 (`offer` from `HomeScreen` when `homeReady && status != null`) | Covered |
| Never over the sign-in or the settings screen | §3 (render gated by `homeReady`; the settings screen gets no tour state) | Covered |
| Skip tour / Done / back; reopened from Feature tour | §1 table (`close`, `next` on last), §3 (`BackHandler`, `TextButton`) | Covered |
| Intro plus one step per feature, the heartbeat with its own switch as off path | §1 `TourStep`, §4 | Covered |
| Every text a `strings.xml` resource | §3, §4, ATR-S3 | Covered |
| Shared titles, explanations, "Off:" lines byte-equal to `app.js`, enforced by an invariant | §4, ATR-S1 | Covered |
| Off paths name the dependent `remote.toml` keys (never exit 78) | §4, ATR-S2 | Covered |
| No request, switches nothing, reads no server or user data | §3, ATR-S5 | Covered |
| One Boolean `tour_seen` in `soos_settings`, no new storage API | §2, ATR-S4 | Covered |
| Manifest, permissions, runtime prompt, dependencies, verification metadata unchanged | §6, ATR-S6, ANA-S4/S6, AHB-S3 | Covered |
| No logging; theme role colours; no animation | §3, ATR-S5, ATR-S6, ANA-S7/S8 | Covered |
| JVM tests, invariant, ADR, §2j, matrix rows, walkthrough 230, owner hardware row | §5, ATR-S7 | Covered |

## 2. Facts Verified

| Fact | Check | Actual | Match |
|---|---|---|---|
| Web tour strings | `crates/remote/assets/app.js` lines 2167–2244 | 8 steps with `title`, `text`, `off`, `where`, `target`; labels Next, Done, Back, Skip tour, Feature tour | Yes |
| Web strings have no `"`, `<`, `&`, `\` | `plain_text` of `remote_tour_contract` (PTR-S5) | enforced on main | Yes |
| One apostrophe in the web texts, in a where line only | grep `'` in `TOUR_STEPS` | Internet access `where` ("this phone's") | Yes |
| The app shows no battery, unlock, push or camera | `SoosApp.kt`, `Docs/REMOTE_COMPANION.md` §2j intro | "Unlock, the live camera view, push notifications, the battery level and the offline history stay in the web app for now" | Yes |
| Tailnet requests are authenticated | `crates/remote/src/server.rs::auth_state_view` | `PathClass::Tailnet => true`, Funnel needs a session | Yes |
| Settings store file and backup exclusion | `SettingsStore.kt`, manifest, `data_extraction_rules.xml` | `soos_settings`, `MODE_PRIVATE`, `allowBackup="false"`, `sharedpref` excluded from cloud backup and device transfer | Yes |
| Existing store needles | AHB-S3, ANA-S8 | heartbeat keys, `getBoolean(KEY_HEARTBEAT_ENABLED, false)`, no `cookie"`/`session"` | Kept |
| Heartbeat package has exactly five sources | AHB-S3 | the tour lives in `tour/` and `ui/` | Kept |
| `BackHandler` available without a new dependency | `libs.versions.toml` | `activity-compose` 1.11.0 (`androidx.activity.compose.BackHandler`) | Yes |
| CI runs the JVM tests | `scripts/android_build.sh` | `assembleDebug testDebugUnitTest lintDebug` | Yes |
| Matrix prefix `ATR` unused | grep `AI/VERIFICATION_MATRIX.md` | 0 rows | Yes |
| Walkthrough 230 free | `ls AI/walkthroughs` | highest 223 | Yes |
| Docs §2j / §2k | `Docs/REMOTE_COMPANION.md` | "## 2j. Native Android app" then "## 2k. Feature tour (phone web app)", whose last bullet says the app has no tour yet | Update the bullet, keep its "native Android app" needle (PTR-S10) |

## 3. Pillar Analysis

- **Architecture alignment**: Android-only; no server, route, CSP or `remote.toml` change. The PAM, daemon and
  IPC invariants are not involved.
- **Test integrity**: no existing test is edited; the new invariant module is registered in `lib.rs` (addition
  only). No contract migration.
- **Fail-closed / truthfulness**: the where lines must not claim the app shows a card it does not have; they point
  to the web app or `soos-gui` for unlock, push, camera and battery. Key sets are pinned to the web lines.
- **Privacy**: the only new stored datum is a Boolean, excluded from backup and transfer.
- **Supply chain**: no new Gradle dependency; std only in the invariant.

## 4. Findings

| # | Severity | Finding | Resolution |
|---|---|---|---|
| P-1 | MINOR | The byte-equal web texts say "this page" (Battery off line, Failed-password history text) | Accepted: byte equality is the acceptance criterion; the app-specific where lines say where the feature lives in the app or in the web app |
| P-2 | MINOR | `remember` (not `rememberSaveable`) loses an open tour on an activity recreation (rotation) | Accepted, as for every other screen state of the app; the tour then opens again at the introduction because it was not closed |
| P-3 | MAJOR → mitigated | No JDK/SDK on the host: the Kotlin is compiled and the JVM tests run only in the CI job `Android (debug build)` | Use only APIs already used by the app or shipped by the pinned libraries; matrix rows of the JVM tests stay "Pending CI" until a green run id is recorded |
| P-4 | MINOR | The step map from `TourStep` to resources could drift from `strings.xml` | ATR-S3 checks every referenced and every defined `tour_*` resource, and the exhaustive `when` fails compilation on a missing step |

VALIDATION_VERDICT: APPROVED
