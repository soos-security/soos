# Audit Constraints — GitHub #404: failed-password alerts in the native Android app (PR 2 of #357)

Inputs: architect spec `AI/architect_spec_android_alerts.md`, plan self-check (APPROVED), tester
contract `AI/tester_contract_android_alerts.md`, the files about to change under
`android/app/src/main/`.

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C1 | The parser is total: no `!!`, no unchecked cast, no uncaught exception; every `org.json` access is type-checked (no `opt*` coercion); any malformed input returns `null` | `ApiJson.kt::parseAlerts` | ANA18 JVM tests; code reading |
| C2 | Bounded input: body ≤ 16384 characters (equal to the transport bound, which stays unchanged), history ≤ 32 entries, every string a member of a fixed set or the exact 16-character lowercase-hex epoch, counters ≤ `u32::MAX`, ids and times ≥ 0 | `ApiJson.kt::parseAlerts` | ANA18; ANA-S8 pins the bounds to `lib.rs` and the transport |
| C3 | CSRF contract of the ack equals the server: exact action `alerts-ack`, `Origin` = the PC origin, exactly one epoch and one through header, no body, no `Content-Type`; a snapshot the server would refuse (`400`) is not sent at all | `SoosApi.kt::ackAlerts` | ANA19; ANA-S8 pins the action, paths and header names |
| C4 | The session cookie stays in memory only, sent through the shared `send` path; `login_required` still drops it; nothing new is persisted (`SettingsStore` unchanged) | `SoosApi.kt`, `SettingsStore.kt` | ANA19 cookie test; ANA-S8 cookie needles unchanged |
| C5 | No logging or printing, no exception or server text on screen: every user-visible alerts text is a fixed `strings.xml` resource; server tokens only select a fixed text; no status number in the alerts texts | `SoosApp.kt` | ANA-S8 forbidden list; ANA-S14 (`strings.xml` resolution, forbidden `.message`, `localizedMessage`) |
| C6 | No colour literal: theme colours or `colorResource(R.color.soos_*)` only | `SoosApp.kt` | ANA-S7 unchanged |
| C7 | No new dependency, manifest entry, permission, exported component or network host; `verification-metadata.xml`, `libs.versions.toml`, `build.gradle.kts`, the manifest and the `aapt2` allowlist are byte-identical | `android/**`, `scripts/android_build.sh` | `git diff --stat`; ANA-S2/S4/S6/S10 unchanged |
| C8 | Network calls run off the main thread (`Dispatchers.IO`), UI state on the main thread; a refresh while one is running is not started twice (`busy` flag) | `SoosApp.kt` | code reading; owner row ANA21 |
| C9 | No Rust change: `crates/**` byte-identical, RC-1 untouched | `crates/**` | `git diff --stat` |
| C10 | No existing test or assertion modified (Zero Test Weakening) | test files | `git diff` shows additions only in the test files |

### Pre-existing violations found (not introduced by this change)

None relevant. Accepted residual: an unknown enum value added by a newer server makes the alerts
card show "could not be loaded" until the app is updated (strict parsing requested by #404).

### Clearance: CLEARED
