# Plan Evaluation Report — Round 2
- **Date**: 2026-10-13
- **Issue**: GitHub #381 — Reproducible README screenshot capture (PR 1 of epic #356; GitHub-only, no `AI/BACKLOG.md` entry)
- **Branch**: `test/readme-capture`
- **Base commit**: `6f32ef8` (`origin/main`)
- **Plan evaluated**: `AI/architect_spec_readme_capture.md`, round 2 (§10 "Revision 2" overrides §1–§9)
- **Owner decisions**: OD-1 to OD-4 remain "assumed — owner to confirm" (authorised run; not a finding by themselves).

## 1. Coverage Matrix

The round 1 matrix still holds: every PR 1 acceptance line of #356/#381 maps to a spec element. Changes in round 2:

| Acceptance line / round 1 finding | Spec element (round 2) | Status |
|---|---|---|
| No username other than `alice` (F-1) | §10 F-1: `shown_identities` exact state check, then a whole-word scan of the rendered text with a documented skip list; RAC3 | Covered |
| Drift check (F-3, F-4) | §10 F-3: tolerance 16 / 0.00002. §10 F-4: update mode refused under `CI`, failure message names the script, ADR states that regeneration is not weakening; RAC1 | Covered |
| Script "runs inside Docker" deviation (F-5) | §10 F-5: recorded in §9 and the ADR | Covered |
| Web stub bodies match real server types (F-6) | §10 F-6: keys pinned; `test_rac_web_fixture_keys_match_the_remote_types` | Covered |
| Metadata stripped from browser output (F-7) | §10 F-7: `strip_png_metadata`, `--strip-png`, viewport 390×844 @2, light/dark | Covered |
| Loopback-only fixture server (F-8) | §10 F-8: static check in `readme_assets_contract.rs` | Covered |
| Dev-only tooling rule (F-10) | §10 F-10: exact path list | Covered |
| Honest captions (F-12) | §10 F-12: `Docs/README_ASSETS.md`, walkthrough, PR 2 caption rule | Covered |

## 2. Facts Verified Against Code (round 2 deltas)

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `StatusView` keys | `crates/remote/src/status.rs:29-37` | `state, active, idle, idle_since_unix_s, checked_unix_ms` | Yes |
| `AlertsView` keys | `crates/remote/src/alerts.rs:619-639` | 10 keys as listed | Yes |
| `AlertRecord` serialized keys | `alerts.rs:312-332` | `id, first_unix_ms, last_unix_ms, source, account, kind, count`; `last_seq` and `first_us` are `#[serde(skip)]` | Yes (the draft `struct_fields` honours `#[serde(skip` lines) |
| `BatteryView` keys | `crates/remote/src/battery.rs:77-86` | `state, percent, charge, external_power` | Yes |
| `PushView::disabled()` | `crates/remote/src/push.rs:1119-1129` | `state: Disabled, reason: None, public_key: None, subscriptions: 0, devices: [], last_delivery: None, sender: Unknown` | Yes |
| Model id and version constants | `crates/enrollment-cli/src/service.rs:58,63` | `MODEL_ID_EMBEDDING`, `EMBEDDING_MODEL_VERSION = "2.0.0"` | Yes |
| `CameraManager` can be implemented by a thread-free stub | `crates/camera-v4l/src/manager.rs:56-85` | `latest_frame`, `is_ready`, `notify_activity`, `stop` are required; `health` and `status` have defaults | Yes |
| Manifest changes are dev-only | `git diff crates/gui/Cargo.toml Cargo.lock` | `[features] readme-capture = []`; `png`, `soos-frame` and `soos-test-fixtures` only under `[dev-dependencies]`; lock adds edges only, no new package | Yes |
| GUI strings contain no `/run/`, `@`, `/var/lib/soos` or `/dev/` that would trip RAC3 | grep on `crates/gui/src/{app,header,widgets}.rs` | none found | Yes |
| Draft RAC1 refuses update mode under CI | `crates/gui/tests/readme_snapshots.rs:66-67` | `assert!(!(update && CI))`, message names `scripts/capture_readme_assets.sh` | Yes |
| Draft RAC3 | `readme_snapshots.rs:123-176` | exact state checks plus a whole-word scan, `ORDINARY_WORD_LOGINS` skip list, forbidden fragments, fixed telemetry text | Yes |
| Draft F-7 test | `tests/fixtures/tests/readme_fixture_tests.rs:370-401` | strips ancillary chunks, idempotent, refuses APNG, GIF, truncated input and a missing IEND | Yes |
| Draft F-8 check | `tests/invariants/src/readme_assets_contract.rs:986-990` | requires `"127.0.0.1:0"`, refuses `0.0.0.0`, `[::]`, `::/`, `"::"` | Yes |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Scenario: the fixture server is reachable from the LAN. It now binds loopback only, and a static check enforces this (F-8). `soos-remote` stays untouched (RAC15 rule F-10). The fixture server now lives in the dev-only `soos-test-fixtures` lib (`serve`) plus the example; `fixtures_contract` keeps that crate out of runtime dependencies.
- Result: PASS.

### Pillar 2 — PAM deadline & concurrency
- N/A for PAM (`crates/pam` untouched). Inside the capture, the only thread is the `DaemonMonitor` with a fixed probe, joined on drop, and its first probe runs immediately. The mock camera thread is gone (F-2). The RAC9 server test binds an ephemeral loopback port, so parallel CI tests do not collide.
- Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- N/A for PAM. `route`, `strip_png_metadata` and `create_placeholder_face_frame` are total and refuse malformed input. Panics are confined to test code.
- Result: PASS.

### Pillar 4 — Dependencies
- No new crate: the lock diff only adds dependency edges for already-locked packages. `png` stays dev-only. The hidden `_new_kittest` API is pinned by `eframe = "0.35"` plus `--locked`.
- Result: PASS.

### Pillar 5 — Data confidentiality
- Scenario: the host login shows in a screenshot. The exact state checks now catch it whatever the host (F-1). Whole-word matching removes the false positive on `root`/"root-protected" and the prefix match on `ali`.
- Residual: a host login equal to a word the UI itself renders, and not in the skip list (e.g. a login named `camera` or `daemon`), makes RAC3 fail loudly. It never passes silently, so the cost is a false alarm, not a leak. This is acceptable and noted as an observation.
- Browser PNGs are re-encoded without metadata (F-7). Captions must state that the shown values come from mock models (F-12).
- Result: PASS.

### Pillar 6 — Test integrity
- Scenario: a one-label text change hides under the tolerance. The fraction is now 0.00002 (at most 17 pixels) with a per-channel tolerance of 16. A changed glyph moves many pixels by far more than 16 levels, so RAC1 now catches it.
- Scenario: platform `powf` ±1 differences between the developer host and the CI runner cause false drift. These differences are absorbed by the channel tolerance of 16.
- Scenario: update mode left on in CI. It is refused (F-4).
- Regeneration is declared not to be test weakening, and the tolerance constants are the immutable part.
- No existing test is modified.
- Result: PASS.

## 4. Findings

None of severity CRITICAL, MAJOR or MINOR. All round 1 findings (F-1 MAJOR; F-2 to F-12 MINOR) are resolved by §10 and reflected in the draft tests.

Observations (no plan change required):
- RAC3's skip list is fixed. A host login that equals another rendered UI word gives a loud false failure, not a silent pass. If this ever happens, the remedy is to run the tests under another account, not to edit the test.
- The tight drift tolerance means the first CI run must confirm that assets generated on the developer host (Arch, newer glibc) match the Ubuntu runner render within 16 levels and 17 pixels. If they don't, the assets must be regenerated in an environment that matches, not by loosening the constants.
- `CAPTURE_PIXELS_PER_POINT = 1.0` is still low-density for HiDPI displays; this is a cosmetic choice for the architect.

## 5. Verdict
The round 2 plan resolves every round 1 finding and introduces no new CRITICAL or MAJOR issue.

VALIDATION_VERDICT: APPROVED
