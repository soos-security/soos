# Candid Review Report

- **Date**: 2026-10-07
- **Target Branch**: `feat/remote-live-camera` (GitHub #345, PR #347), round 1 of the pre-push review of the
  unpushed commits `fee8480`, `0a34872`, `1168206`, `bb1d19a`
- **Base (merge-base)**: `05001c9` (`origin/main`; full branch diff as computed by the pre-push hook)
- **Reviewed-Diff-Fingerprint**: `dd37774cccbd8ee0f9541ef3641e8416ab0dc3e5f952145f1863ffde808d0409`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `.github/workflows/ci.yml`,
  `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_live_camera.md`,
  `AI/auditor_constraints_remote_live_camera.md`, `AI/research_live_camera.md`,
  `AI/tester_contract_remote_live_camera.md`, `AI/walkthroughs/191_remote_live_camera.md`, `Cargo.lock`, `Cargo.toml`,
  `Docs/DAEMON.md`, `Docs/GUI_APPLICATION.md`, `Docs/IPC_PROTOCOL.md`, `Docs/REMOTE_COMPANION.md`,
  `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `crates/daemon/src/{config,dispatcher,lib,main,preview,preview_image,preview_peer,session,session_policy}.rs`,
  `crates/daemon/tests/{preview_authorization_tests,preview_remote_view_tests}.rs`, `crates/gui/src/ipc_camera.rs`,
  `crates/protocol/src/types.rs`, `crates/remote/Cargo.toml`, `crates/remote/assets/{app.js,style.css}`,
  `crates/remote/src/{audit,camera,camera_ipc,camera_jpeg,camera_slot,challenge,config,http,lib,main,routes,server}.rs`,
  `crates/remote/tests/{alerts_server_tests,camera_challenge_tests,camera_config_tests,camera_ipc_tests,camera_jpeg_tests,camera_routes_tests,camera_server_tests,camera_slot_tests,push_server_tests,server_tests}.rs`,
  `crates/remote/tests/common/{camera,harness}.rs`, `deny.toml`, `scripts/install_remote.sh`,
  `tests/invariants/src/{lib,remote_camera_contract,remote_companion_contract}.rs`

Scope note: commits `a2876c9` and `df4798e` are already on `origin/feat/remote-live-camera` and were reviewed in
earlier rounds; this round re-reads the whole frozen patch but concentrates on the four new commits
(`git diff df4798e HEAD`): stream `Content-Type` for iOS (M12), full screen and two orientation modes, removal of the
view cooldown (M13), removal of the camera-start Web Push (M14). `crates/remote/assets/sw.js` and
`crates/remote/src/push.rs` changed in these commits and were read in their changed regions.

## 1. Executive Summary

The four commits are owner-requested behavior changes in `soos-remote` only. They remove state (cooldown, camera
push), switch the stream head to `application/octet-stream` while keeping the `soosframe` part framing, and add a
client-side full-screen overlay plus a portrait/landscape CSS toggle. No PAM, daemon, protocol or dependency code
changed in these commits. Every removed assertion is covered by a recorded owner-approved migration (M12, M13, M14 in
`AI/tester_contract_remote_live_camera.md`) and is replaced by a stricter negative assertion (no cooldown, zero push
calls with a positive control, static absence checks). `cargo fmt --check`, `cargo clippy --workspace --all-targets
-- -D warnings`, `cargo test -p soos-remote` and `cargo test -p soos-invariants` (532 passed) are green. No CRITICAL or
MAJOR finding.

## 2. Test Changes (mechanical listing, step 3)

Removed/changed assertions found in `git diff df4798e HEAD -- crates/remote/tests tests/invariants`:

| Location | Change | Justification |
|---|---|---|
| `camera_config_tests.rs` | `CAMERA_VIEW_COOLDOWN_MS == 10_000`, `PUSH_CAMERA_TOPIC == "sooscamera"` removed | M13e / M14c; absence pinned by invariant tests 60 and 63 |
| `camera_push_tests.rs` (file deleted) | `test_rlc_camera_payload` and its payload assertions | M14b; nothing remains to test; absence pinned by test 63 |
| `camera_server_tests.rs` | cooldown state / `cooldown_ms` / `429 camera_cooldown` assertions → `idle`, no `cooldown_ms`, immediate restart | M13f-M13j; the new form asserts a real second and third view stream pixels |
| `camera_server_tests.rs` test 35 | one `sooscamera` delivery → zero transport calls during start, run, stop, end and failed first frame | M14a; positive control (test notification reaches the same spy) prevents a vacuous pass |
| `camera_slot_tests.rs` | `phase()` tuple → `SlotPhase`; `end(now, view, shown)` → `end(view)`; cooldown cases → immediate re-reserve | M13a-M13d; exhaustive `match` on `SlotPhase` / `SlotError` makes a reintroduced cooldown variant fail to compile |
| `camera_server_tests.rs` test 27 | expected head `Content-Type` `multipart/x-mixed-replace; boundary=soosframe` → `application/octet-stream` | M12 (iOS "Load failed"); part framing, mandatory headers, CSP and JPEG assertions unchanged |
| `remote_camera_contract.rs` `camera_functions_outside_modules` | `push.rs` camera functions no longer required | M14d; those functions no longer exist; `server.rs` / `http.rs` camera functions still scanned |

New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): none. New tests: invariant test
60 (`test_rlc_page_fullscreen_rotate_and_no_cooldown`) and test 63 (`test_remote_camera_sends_no_push`).

## 3. Deep Reasoning Audit

### Logic & Architecture
- Slot state machine without cooldown: `Idle → Pending → Starting → Streaming → Idle`; `end` of a stale view id is a
  no-op; an expired `Pending` reads as `Idle` in both `check` and `phase`. Tried: end of a non-current view while a new
  one streams (no-op, covered by test 14), stop while `Pending` (Idle, `true`), stop while Idle (`false`). PASS.
- `SlotPhase::Cooldown` and `SlotError::Cooldown` removed; every consumer (`camera.rs`, `server.rs`
  `camera_slot_refusal`, `camera_view_response`, `app.js` `CAMERA_REASONS` / `renderCamera`) updated; no dangling
  `cooldown_ms` in the JSON shape or page. PASS.
- Push removal: `Message::CameraView`, `camera_queued`, `queue_camera_view`, `send_camera`, `camera_payload`,
  `PUSH_CAMERA_TOPIC` and its const assertions are all gone; `send_test` folded back into a single path; the
  dispatcher loop is otherwise unchanged (test notification and alert retries untouched). `sw.js` no longer special-
  cases `kind: "camera"`. PASS.
- Stream head: `CAMERA_STREAM_CONTENT_TYPE = "application/octet-stream"` is the single source of truth in Rust; the page
  checks the same prefix. `X-Content-Type-Options: nosniff` is in `MANDATORY_HEADERS`, so a browser does not sniff the
  opaque body. PASS.
- Full screen: `enterCameraFullscreen` is a no-op without an open view (`cameraController === null`);
  `endCameraView` always calls `exitCameraFullscreen` and hides the stage; `fullscreenchange` / `webkitfullscreenchange`
  and Escape exit the overlay. Element fullscreen refusal (promise rejection or throw) is swallowed and the overlay
  remains. Orientation is a CSS class only, kept in memory, never stored. PASS.
- Architecture, ADR, verification matrix (RLC9, RLC10, RLC13, RLC15, RLC16), `Docs/REMOTE_COMPANION.md` and the
  walkthrough are updated consistently. Removing the cooldown is bounded by the unchanged per-view fresh passkey
  assertion, the single global slot, the token TTL and the shared failure lockout. Removing the camera push is an
  explicit owner decision recorded in the ADR; the audit lines and camera LED remain the awareness signals. PASS.

### PAM Concurrency & Deadlines
- No file under `crates/pam` changed in the reviewed commits (nor in the branch). PASS.

### Panic Safety & Fail-Closed
- New Rust code removes branches only; no `unwrap`/`expect`/indexing added in production. The removed
  `remaining_ms` helper used `unwrap_or`, not a panic. No path from an error to a stream or an unlock; slot refusals
  still map to `409` / `403`. PASS.

### Test Integrity & Anti-Weakening
- See §2. Each removed assertion maps to a recorded migration with owner approval; replacements are stronger
  negative checks. Test 35 can fail against a plausible wrong implementation (one `sooscamera` call is detected, as
  shown during this review on a stale build artifact; after a clean rebuild of the current sources it passes). PASS.

### Memory, Bounds & Secrets
- No new allocation in Rust. The page's bounded buffer (`CAMERA_MAX_BUFFER_BYTES`) and canvas-only rendering are
  unchanged; no `blob:` URL, no storage of the orientation mode. The camera push payload (which carried no token or
  image) is removed entirely. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change in the four reviewed commits. PASS.

### English-Only Policy
- Code, comments, docs, UI strings and commit subjects are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/remote/src/camera_slot.rs:8` — one module doc line ("ended, still behind a fresh passkey
  assertion. Every method receives the current time; ...") is much longer than the surrounding wrapped lines.
  Cosmetic; rewrap when next touched.
- **[SUGGESTION]** `crates/remote/assets/style.css` `.camera-canvas` — `width`/`height` are declared twice (percent then
  container units). This is a valid fallback for browsers without container query units; a short comment would make
  the intent explicit.
- **[SUGGESTION]** Process note: during this review `test_rlc_no_push_on_camera_view` first failed with one
  `sooscamera` delivery although no source contained the string; touching `crates/remote/src/*.rs` and rebuilding made
  it pass. Run gates from a clean incremental state after branch switches in this worktree.

## 5. Final Verdict

**VERDICT: APPROVED**
