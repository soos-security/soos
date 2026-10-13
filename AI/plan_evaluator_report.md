# Plan Evaluation Report — Round 2
- **Date**: 2026-10-13
- **Issue**: GitHub #379 (PR 1 of epic #363) — first-run onboarding tour with per-feature toggles in `soos-gui` (no `AI/BACKLOG.md` id)
- **Branch**: `feat/onboarding-tour`
- **Base commit**: `6f32ef8`
- **Plan**: `AI/architect_spec_onboarding_tour.md` with §9 (coverage map) and §10 Revision 1 (R-1..R-11)

## 1. Coverage Matrix

| Acceptance line (#363 / #379) | Spec element | Status |
|---|---|---|
| #363-1 First launch only, per-user state, replayable, skippable | §2.1 `Tour`, `TourStore`; §2.4 header button | Covered |
| #363-2 Current state from daemon / soos-remote / config; off switch through privileged paths | §2.1 state table + R-1 (udev rule), R-3 (`daemon.toml`), R-11 (masked); §2.2 executor | Covered |
| #363-3 Phone tour | §9: deferred to the next PR of #363 | Deferred (explicit) |
| #363-4 Brand, #358 motion, reduced motion | §9: brand tokens, no animation, `Tour::generation` hook only | Covered |
| #363-5 English UI, docs, ADR, matrix, walkthrough | §8, §9 (walkthrough 209 = orchestrator assignment; 200–208 held by parallel branches) | Covered |
| #379 daemon `[preview]` shown as key/command | R-3 routing: `Command` naming the `daemon.toml` key + daemon restart | Covered |

## 2. Round 1 Findings — Resolution

| # | Round 1 finding | Revision | Verified against code | Status |
|---|---|---|---|---|
| 1 | Wi-Fi "on" removes the rfkill rule; Unknown on Arch | R-1: `network_lock_rfkill_installed` probe; `install` Action only when the rfkill rule is known absent; otherwise `Command("sudo soos-admin network-lock install --rfkill")`; polkit-rule Unknown ⇒ only the off button | `/etc/udev/rules.d` is world-readable; `uninstall` removes both rules (`network_lock.rs:1004-1021`), stated in `when_off` | Resolved |
| 2 | Presence "on" no-op under the global flag | R-2: `Unavailable("Face unlock is off; turn it on first.")`; `FaceUnlock::when_off` mentions presence | `presence/switch.rs:30` | Resolved |
| 3 | Daemon config not a state source | R-3: `daemon_presence_enabled` (`[presence] enabled`, default `true`), `daemon_remote_view` (`[preview] enabled && remote_view && allowed_uids ∋ uid`, defaults false); command routing, never an edit | Semantics match `daemon/src/preview.rs:123-145` and `dispatcher.rs:1229`; `presence/config.rs:7` default enabled; the shared reader `soos_camera_v4l::daemon_config::read_daemon_config_text` exists (`daemon_config.rs:214`, `O_PATH` pin, regular file, 1 MiB) and `soos-camera-v4l` is already a `soos-gui` dependency | Resolved |
| 4 | Polkit exclusion not at the choke point | R-4: single `submit_task` wrapper | Draft `app.rs:361-365`; `self.tasks.submit` appears exactly once (inside the wrapper) | Resolved |
| 5 | FIFO / TOCTOU on user-file reads | R-5: `read_user_file_bounded` with `O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC` + `fstat` | Draft `onboarding.rs:762-764`; module keeps `#![forbid(unsafe_code)]` | Resolved (load stays on the UI thread before the first frame, now non-blocking and 4 KiB bounded — accepted) |
| 6 | "On" texts lack soos-remote prerequisites | R-6 per-switch prerequisite wording, tested | Matches `remote/src/config.rs:281-327,403-410,564-569,725` | Resolved |
| 7 | Refresh while busy; joining `Drop` | R-7: `refresh_pending` coalescing; no `Drop`, handle detached | Draft `app.rs:443-476` | Resolved |
| 8 | Stale header facts | R-8: base `6f32ef8`; walkthrough 209 justified by orchestrator assignment | — | Resolved |
| 9 | No coverage map | §9 | — | Resolved |
| 10 | Face unlock "On" overstated | R-10: wording + Overview footnote; probe keeps the daemon rule | `pam/src/config.rs:108-120` | Resolved |
| 11 | Masked unit | R-11: `UnitState::Masked`, Off, on = unmask+enable command, off = `Unavailable` | — | Resolved |

## 3. Pillar Analysis (re-run on the revised plan)

### Pillar 1 — Architecture & threat model
- Scenario: rfkill rule installed, polkit rule unreadable (Arch). Revised routing offers only "Turn off" (`uninstall`, admin password, removes both rules as `when_off` says) and shows the `--rfkill` install command for "on". No GUI click can silently weaken the lock. PASS.
- Scenario: `daemon.toml` symlinked by root to elsewhere — the shared reader follows it by design (root-owned `/etc/soos`, same as the daemon); only booleans are extracted, nothing logged. PASS.

### Pillar 2 — Concurrency / UI thread
- Scenario: `gui-tour.json` replaced by a FIFO before start-up → `O_NONBLOCK` open + `fstat` refuses it; UI thread does not block. Scenario: Refresh rejected as Busy → coalesced. PASS.

### Pillar 3 — Fail-closed
- Scenario: `daemon.toml` unreadable → `None` → Unknown, never On. Scenario: key `camera_view = true` but daemon side off → state Off, off button still offered (`DisableRemote(CameraView)`), so the exposure can still be reduced. PASS.

### Pillar 4 — Dependencies
- No new crate (`toml` 0.8.23 single version; `soos-camera-v4l` already a dependency). PASS.

### Pillar 5 — Data confidentiality
- Unchanged from round 1; command texts carry no user data (uid only compared, not printed). PASS.

### Pillar 6 — Test integrity
- Immutable tests (`responsiveness_tests`, `camera_source_tests` exhaustive matches, `test_app_ui_code_spawns_no_process`, `gcv_review_contract`, `program_path_tests`, `network_lock_contract`) remain compatible; the draft contract tests were revised before Phase 2 freeze, which is legitimate. PASS.

## 4. Findings

- **[MINOR / observation]** PresenceUnlock with both the daemon key off *and* `presence.disable` present: the "on" route is the `daemon.toml` command; after the user follows it, the GUI then offers the flag removal. Two steps, but each is correct and visible; no change required. The developer should ensure the routing test covers this combination so the order (daemon command first) is pinned.
- **[MINOR / observation]** `TourStore::load` stays on the UI thread (bounded, non-blocking); accepted as a deliberate trade-off documented in R-5.

No CRITICAL or MAJOR finding remains.

## 5. Verdict

VALIDATION_VERDICT: APPROVED
