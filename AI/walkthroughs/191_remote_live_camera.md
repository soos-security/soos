# Walkthrough 191 — Live Camera View in `soos-remote` Through the Daemon Preview Channel

- **Date**: 2026-10-07
- **Issue**: GitHub #345 (GitHub-only, no backlog id, like #339; not registered in `BRANCH_TO_ISSUE` of
  `scripts/sync_issue.py`, the PR body says `Closes #345`). **Branch**: `feat/remote-live-camera` (base `05001c9`).
  Nothing is committed, pushed, deployed, installed or restarted by the agents.
- **ADR**: "[2026-10-07] Live Camera View in `soos-remote` Through the Daemon Preview Channel" (`AI/DECISIONS.md`,
  amended by D-1 and D-2 during the plan evaluation).
- **Documents**: research `AI/research_live_camera.md`; spec `AI/architect_spec_remote_live_camera.md` (round 2);
  plan evaluation `AI/plan_evaluator_report.md`; tester contract `AI/tester_contract_remote_live_camera.md`; auditor
  constraints `AI/auditor_constraints_remote_live_camera.md` (C1–C36, N1–N3); candid review
  `AI/candid_review_report.md` (round 3).
- **Matrix criteria**: RLC1–RLC16 (RLC16 is the owner's hardware check, pending).

## 1. Context & Objectives

The owner wants to look at the PC's camera from the iPhone home-screen app of `soos-remote`: to see who is in front
of the PC, or that nobody is. The camera is owned exclusively by the root `soos-daemon` (architectural invariant), so
the companion must never open `/dev/video*`. The view reuses the daemon preview channel that `soos-gui` already uses
(`RequestKind::PreviewFrame` over `/run/soos/daemon.sock`), converts each preview frame to a baseline JPEG in pure
Rust and streams it to the page as `soosframe` multipart parts served as `application/octet-stream` (section 10). Face unlock, presence auto-unlock and the view share
one capture.

Goals: off by default on both sides (double opt-in), a fresh Face ID per view, one bounded view at a time, no
recording, no pixel or token logging, awareness through the camera LED, a push notification and audit lines, and no
impact on face unlock at the lock screen when `soos-gui` is closed.

### Owner decisions

| Id | Decision |
|---|---|
| LC-1 | `soos-remote` may depend on `soos-protocol` and on no other soos crate (besides `soos-push-protocol`). The leaf-crate invariant RMC-S4 (`test_rmc_s4_remote_is_a_leaf_crate`) is migrated with the owner's approval to allow exactly that crate (migration M1). |
| LC-2 | JPEG in pure Rust with `jpeg-encoder` `=0.7.1` (default features, never `simd`). `deny.toml` allows `IJG` through a crate-scoped exception for `jpeg-encoder` only; `IJG` stays out of the global allow list. No image decoder in `soos-remote`. |
| LC-3 | Daemon: (a) the preview session check is tightened to a local seat session for **every** unprivileged peer (`soos-gui` included); (b) new `[preview] remote_view` (default `false`) keyed on the peer cgroup `soos-remote.service`; one `info` line without pixels on the first frame of a companion connection. Documented as an administrative opt-in, not a security boundary. |
| LC-4 | `camera_view` (default `false`, requires `rp_id`); tailnet only unless `camera_view_funnel = true` (requires `allow_funnel`). A fresh UV passkey assertion of purpose `CameraView` per view on every path. |

## 2. Architect Design

- **Protocol** (`crates/protocol/src/types.rs`): `PREVIEW_FORMAT_RGB24/GREY/YUYV/NV12/MJPEG/EMPTY` (0, 1, 2, 3, 4,
  255) become public constants; the daemon's `PREVIEW_FORMAT_EMPTY` and the GUI's private copy alias them. No wire
  change.
- **Daemon**: `PreviewConfig.remote_view` (default `false`); new `crates/daemon/src/preview_peer.rs` with
  `PreviewPeerOrigin`, `PreviewPeerError`, `REMOTE_COMPANION_UNIT` and the pure `classify_preview_peer_cgroup`;
  `SessionRecord::is_local_seat_session_of(uid)` is defined as `check_local_seat_session_of(uid).is_ok()` (the root
  `Auth` predicate, S-7: `UID`, active, `REMOTE=0` exactly, non-empty `SEAT`, `CLASS=user`);
  `SessionValidator::has_local_seat_session`; the dispatcher classifies the peer once per connection, lazily on its
  first `PreviewFrame` (S-6), and fails closed on an unreadable or malformed cgroup.
- **Remote** (four flat modules, S-1): `camera_slot.rs` (pure view slot state machine and the 256-bit single-use
  `ViewToken`), `camera_jpeg.rs` (pure Grey/YUYV/RGB24 conversion, optional 2x box downscale, bounded `JpegSink`),
  `camera_ipc.rs` (`DaemonPreviewClient`: one connection, graceful replacement, bounded replies and time),
  `camera.rs` (runtime and view loop). New config `CameraConfig` with six keys, `ChallengePurpose::CameraView` with
  its own pools, five routes (`/api/auth/camera/options`, `/api/camera/start`, `/api/camera/stream/<token>`,
  `/api/camera/stop`, plus the method table), `Message::CameraView` push and three audit lines.
- **Spec-level decisions** S-1..S-10: notably the `200` head is sent only after the first JPEG is ready (S-2), the
  page builds the camera card at runtime without new ids (S-3), the camera challenge shares the unlock limiter keys
  (one lockout, S-5), refused-view audit lines are rate gated (S-9), conversion and encoding run in
  `spawn_blocking` (S-10).
- **New invariants** RLC-S1..RLC-S13 (spec §15): single camera owner, double opt-in fail closed, fresh UV assertion
  per view, token secrecy, one view and one daemon connection, bounded everything, no recording or pixel logging, no
  decoder, no stale replay, drop-guarded termination, awareness never blocks, page hygiene, administrative cgroup
  classification.
- **Residual risks** R-1..R-6 (documented in `Docs/REMOTE_COMPANION.md`): code running as the owner can read the GUI
  preview anyway or impersonate the unit; iCloud Keychain compromise equals passkey compromise; with `soos-gui` and a
  view both open a same-uid lock screen face request is refused at admission and falls back to the password; pixels
  transit `tailscaled`; the LED is the only local indicator; PID reuse only affects the administrative
  classification.

## 3. Plan Evaluation

Round 1: REVISION_REQUIRED (F1–F3 MAJOR: absent or malformed `REMOTE` must refuse; a durable stop; non-terminal
events must not cancel a frame; plus MINOR F5, F6, F8–F12). Round 2: **VALIDATION_VERDICT: APPROVED**, every round-1
finding resolved in the spec text. Non-blocking observations O1 (the frame-step future owns its source), O2 (retry
uses the remaining IO budget), O3 (`watch` sender drop is terminal) and O4 (editorial, `STREAM_SINK_BYTES`) were
carried to the developer.

## 4. Tester Contract

Tests live in new files only (`test_rlc_` prefix, `test_rlc_s<N>_` for static invariants):

- `crates/remote/tests/camera_jpeg_tests.rs` (9, spec 1–9) → RLC9, RLC11, RLC13;
- `crates/remote/tests/camera_slot_tests.rs` (7, spec 10–16) → RLC8, RLC9, RLC14;
- `crates/remote/tests/camera_ipc_tests.rs` (11, spec 17–22 and 62) → RLC12;
- `crates/remote/tests/camera_server_tests.rs` (14, spec 23–35 and 61) → RLC1, RLC6–RLC10, RLC12, RLC14, RLC15;
- `crates/remote/tests/camera_config_tests.rs`, `camera_routes_tests.rs`, `camera_push_tests.rs`,
  `camera_challenge_tests.rs` (spec 36–40 plus `test_rlc_camera_constants_match_the_spec`) → RLC1, RLC6, RLC8, RLC13,
  RLC15;
- `crates/daemon/tests/preview_remote_view_tests.rs` (13, spec 41–47, 49 and 63, plus peer-source wiring tests) →
  RLC1, RLC3–RLC5, RLC11;
- `tests/invariants/src/remote_camera_contract.rs` (11, spec 50–59 plus a recording-scanner self-test) → RLC2, RLC3,
  RLC10, RLC13, RLC15.

Red evidence: every new suite failed to compile (E0432 on the specified but missing API) before the implementation;
the invariant module gave 1 pass (the scanner self-test) and 10 assertion failures. The auditor's first pass found
test-contract defects B1–B11 (power and determinism issues); the tester fixed them in new test code only, and the
auditor re-checked each one.

**Migrated existing tests** (tester contract §5): M1 owner-approved assertion migration of RMC-S4 (LC-1, stricter:
the exact workspace line and every `soos-*` key in {`soos-protocol`, `soos-push-protocol`}); M2–M7 setup only (a
`camera:` field, `remote_view: false`, and the seat fixture keys `REMOTE=0`/`SEAT`/`CLASS=user`); M8–M10 lint-only
`#![allow(..)]` additions in new test files; M11, see below. No other existing test was modified.

### Owner-approved installer-test amendment (M11)

Invariant test 58 `test_rlc_s9_installer_template` originally rejected any non-comment, non-`echo` line of
`scripts/install_remote.sh` that contains `/etc`. That was a false positive on the read-only
`target="$(readlink -f /etc/resolv.conf 2>/dev/null || true)"` of `warn_resolver_stub`, which is already on `main`;
the candid review round 2 raised it as MAJOR (red test). The owner approved, relayed by the workflow orchestrator, an
amendment that is **stricter** than the original:

- every non-comment line goes through `installer_line_touches_etc`: a `>`/`>>` redirection or a `tee` stage towards
  `/etc` is rejected on every line, `echo` lines included (they were exempt before);
- any non-`echo` line naming `/etc` is still rejected (covers `cp`, `install`, `mv`, `ln`, `sed -i`, `mkdir`, `rm`,
  `chmod`, `chown`), except the exact constant `INSTALLER_RESOLVER_PROBE`;
- a self-check asserts that the detector rejects 16 synthetic write lines (including `readlink` on another `/etc`
  file and the probe with a redirection) and accepts only the probe and a print-only `echo`, so the scan can never
  pass vacuously;
- the `sudo` assertion is unchanged and the installer script itself is unchanged (no path obfuscation).

The approval was relayed by the orchestrator and is not recorded elsewhere in the repository; the owner should
confirm it before the merge (candid review NOTE).

## 5. Auditor Constraints

Round 1 clearance BLOCKED on B1–B11 (test contract); round 2 **CLEARED** with constraints C1–C36 and notes N1–N3.
How the main ones were met:

- C1/C2: no `unwrap`/`expect`/panic or unchecked index in new production code; `#![forbid(unsafe_code)]` kept in
  `soos-remote` and `soos-protocol`, no new `unsafe` in the daemon.
- C3/C4/C5/C6: `jpeg-encoder = { version = "=0.7.1" }` in the workspace, crate-scoped `IJG` exception, IJG
  attribution in `Docs/REMOTE_COMPANION.md` §2f, no decoder and no device access (RLC2, `cargo deny` clean).
- C7–C10: the daemon reply length is checked before allocation, at most one `UnixStream` with `shutdown(Write)` and a
  bounded close wait before reconnecting, fail-closed reply mapping, cancel-safe exchanges (RLC12).
- C11–C14: geometry validated before allocation, `Zeroizing` buffers preallocated at final capacity, `TooLarge`
  recorded by the sink, encoding in `spawn_blocking` (RLC11, RLC13).
- C15–C17: token from the CSPRNG, constant-time comparison, never traced; fixed-text audit lines (RLC8, RLC13,
  RLC15).
- C18–C21: drop-guarded slot, durable stop, one whole-part bounded write per frame, first frame before the head
  (RLC9, RLC14).
- C22–C28: gate order and CSRF, fresh `CameraView` assertion, opt-in and Funnel exposure, permits, unchanged CSP,
  best-effort image-free push, page hygiene (RLC6, RLC7, RLC10, RLC15).
- C29–C32: pure cgroup classifier, default peer source independent of session enforcement, one `info` line, PAM
  latency and per-UID admission unchanged (RLC3–RLC5).
- C33–C36: no recording path, installer prints and comments only, `[profile.dev.package.jpeg-encoder] opt-level = 3`,
  residuals documented.

## 6. Implementation

- Protocol: `crates/protocol/src/types.rs` (format constants).
- Daemon: `crates/daemon/src/{preview.rs, preview_peer.rs (new), session_policy.rs, session.rs, dispatcher.rs,
  preview_image.rs, config.rs, main.rs, lib.rs}`.
- GUI: `crates/gui/src/ipc_camera.rs` (protocol constant, no behavior change).
- Remote: `crates/remote/src/{camera.rs, camera_ipc.rs, camera_jpeg.rs, camera_slot.rs}` (new) and
  `{lib.rs, config.rs, challenge.rs, routes.rs, http.rs, server.rs, push.rs, audit.rs, main.rs}`;
  `crates/remote/assets/{app.js, style.css, sw.js}` (runtime-built camera card, canvas reader, `kind === "camera"`
  notification tag); `crates/remote/Cargo.toml`.
- Workspace: `Cargo.toml`, `Cargo.lock`, `deny.toml`; `scripts/install_remote.sh` (commented template keys and a
  printed `daemon.toml` snippet, never written).
- Docs: `Docs/REMOTE_COMPANION.md` §2f (new) and §1/§5/§6/§8/§9, `Docs/DAEMON.md` §1.5, `Docs/IPC_PROTOCOL.md`,
  `Docs/GUI_APPLICATION.md` (seat-session requirement for the GUI preview), `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`
  (IJG exception, preview consumers), `AI/ARCHITECTURE.md`, the dev-workflow project facts, and the matrix rows
  RLC1–RLC16 in `AI/VERIFICATION_MATRIX.md`.
- `packaging/soos-remote.service` is unchanged: the daemon socket is an `AF_UNIX` connect by a process in group
  `soos`.

Behavior change to note: the `soos-gui` preview now also needs a local seat session (LC-3a); a preview from an
SSH-only login or a lingering user manager is refused.

## 7. Candid Review

Round 3, fingerprint `43826ff3efe128f0157a35a71b4e47f2527284f9a4a4f4c2a2a4515aa48e6069`, **VERDICT: APPROVED**.
Round 2 had one MAJOR finding (test 58 red), closed by the owner-approved migration M11. No CRITICAL, MAJOR or MINOR
finding remains. Open items:

- SUGGESTION: `camera_ipc.rs` `decode_reply` does not match a `PreviewResponse` to the request id (safe today: serial
  exchanges on one connection); match it if the preview wire format ever carries the id.
- SUGGESTION: `CAMERA_DAEMON_SOCKET_PATH` repeats `/run/soos/daemon.sock`, and the daemon's `wire_format_code` and the
  GUI's `pixel_format_from_wire` still hard-code the 0–4 codes; use the shared constants in a follow-up.
- NOTE: the owner should confirm the M11 approval before the merge.

## 8. Verification Results

- `cargo test --locked --all-features -p soos-remote -p soos-daemon -p soos-protocol -p soos-invariants
  --no-fail-fast`: exit 0, 1635 passed, 0 failed; the 71 `test_rlc_*` tests all pass.
- `cargo deny --locked check licenses bans sources`: bans ok, licenses ok, sources ok.
- `cargo test --locked -p soos-invariants` after the matrix and walkthrough edits: green (see the final run of this
  phase), including `test_matrix_claimed_rows_cite_only_existing_evidence` over the new RLC rows.
- `python3 scripts/sync_issue.py --check`: `[OK] sync_issue self-check: 49 GitHub targets, 55 branches, backlog
  headings and sub-issues unique.` GitHub-only issue: no backlog tick, no sub-issue, no GitHub call;
  `--print-github-issue` prints nothing for this unregistered branch.
- Reported by the candid reviewer on the frozen tree: `cargo fmt --all -- --check` and `cargo clippy --workspace
  --all-targets --all-features -- -D warnings` exit 0.

## 9. Known Limitations / Follow-ups

- **RLC16 pending owner verification** (hardware; agents never install, configure or restart anything). Steps
  (`Docs/REMOTE_COMPANION.md` §2f):
  1. Make passkeys work first (`rp_id`).
  2. As root, add `[preview] enabled = true`, `allowed_uids = [<uid>]`, `remote_view = true` to
     `/etc/soos/daemon.toml` and restart `soos-daemon`; set `camera_view = true` in `remote.toml` (and
     `camera_view_funnel = true` for Funnel) and restart the `soos-remote` user service.
  3. From the iPhone home-screen app on the tailnet: *Start camera view*, pass Face ID; live video appears within
     about 1 s at about 5 fps for up to 120 s, and the camera LED is on.
  4. With `soos-gui` closed, face unlock at the PC's lock screen still works during the view.
  5. The push "camera view started" arrives (when Web Push is set up).
  6. *Stop camera view* ends the view at once; a new start right after it succeeds (no cooldown since section 11).
  6a. A tap on the image toggles full screen (✕, Escape or a second tap exits); *Rotate* turns the image by 90°.
  7. After a full logout at the PC, a start is refused (`camera_refused`).
  8. Optionally, over Funnel with `camera_view_funnel = true`, repeat steps 3 and 6.
- The owner should confirm the M11 installer-test amendment before the merge.
- Follow-ups from the candid review: match preview replies to the request id if the wire format gains it; use the
  shared socket path and format constants everywhere.
- Residuals R-1..R-6 stay documented; with `soos-gui` and a view both open a same-uid lock screen face request falls
  back to the password (close `soos-gui` or raise `max_connections_per_uid` to 3).
- Out of scope (each needs its own ADR): recording, snapshots, audio, an on-screen indicator, any other live-camera
  transport.

## 10. Hardware Finding: iOS and `multipart/x-mixed-replace` (owner-approved amendment)

The owner's first iPhone check (RLC16, 2026-10-07, over Funnel with `camera_view_funnel = true`) started a view, the
daemon served the first preview frame, and `soos-remote` logged `live view closed: client gone` about one second
later while the page showed "Load failed". The iOS network stack handles a response labelled
`multipart/x-mixed-replace` itself (it splits it into separate responses), so the page's `fetch` + `ReadableStream`
reader fails. The owner approved changing the head's `Content-Type` to `application/octet-stream`
(`CAMERA_STREAM_CONTENT_TYPE`); the body keeps the exact `soosframe` framing that the page already parses itself.
Test 27 (`test_rlc_stream_is_multipart_jpeg`) now expects `application/octet-stream`; every other assertion is
unchanged. The ADR transport item, `Docs/REMOTE_COMPANION.md` §2f/§6, `AI/ARCHITECTURE.md` §13 and matrix row RLC10
were updated accordingly.

## 11. Owner Request: Full Screen, Rotation and No Cooldown (2026-10-07)

Once the live view worked on the owner's iPhone, the owner asked for a tap-to-full-screen view with a way out, a
portrait/landscape rotation, and no countdown between stopping and restarting a view.

- **No cooldown (migration M13).** `CAMERA_VIEW_COOLDOWN_MS`, the slot's `cooldown_until` state, `SlotPhase::Cooldown`,
  `SlotError::Cooldown` and the `429 camera_cooldown` answer are removed. `ViewSlot::end(view)` returns the slot to
  `Idle` at once and `ViewSlot::phase` returns a bare `SlotPhase`. `GET /api/camera` keeps its JSON shape minus
  `cooldown_ms`; `state` is `disabled|idle|pending|starting|streaming`. A new view still needs its own fresh Face ID
  assertion, there is still one global view slot, and the shared failure lockout is unchanged. The owner approved
  turning the cooldown tests into "no cooldown" assertions; the old and new rules are listed as M13a–M13l in
  `AI/tester_contract_remote_live_camera.md`.
- **Full screen.** The canvas now sits in a runtime-built `.camera-stage` (no new markup id). A tap (or Enter/Space on
  the focused image) toggles the `camera-full` class: a fixed full-viewport overlay in the brand ink
  (`--text` in light, `--page` in dark, both ink), padded by `env(safe-area-inset-*)`, the image scaled with
  `object-fit: contain`. Where `requestFullscreen` / `webkitRequestFullscreen` exist (iPad, desktop) the stage also
  enters element fullscreen; iPhone Safari has none, so the overlay alone covers the home-screen app. Exit: a second
  tap, the **✕** button (`aria-label` "Exit full screen"), Escape, the browser's own exit (`fullscreenchange` /
  `webkitfullscreenchange`), and always in `endCameraView` (stop, maximum duration, error, page hidden, `pagehide`).
- **Rotation.** **Rotate** cycles 0/90/180/270 degrees by toggling `camera-rot-90|180|270` on the stage (classList
  only; no inline style, CSSOM style or storage, CSP unchanged). The stage is a CSS size container; turned by 90 or
  270 degrees the canvas box takes the container's height x width before the `rotate()` transform, so the image
  still fits, and the card stage switches from 4:3 to 3:4 (at most 70vh). The rotation is kept in memory for the
  page lifetime (the next view starts with the same orientation) and is never persisted (RLC-S12).
- **Service worker.** `sw.js` only shows notifications and versions no asset cache, so there is nothing to bump; the
  page and its assets are served with `Cache-Control: no-store`.
- **Tests.** New invariant test 60 `test_rlc_page_fullscreen_rotate_and_no_cooldown`; migrated slot, config and
  server tests per M13. Docs: ADR item (5) amendment note, `Docs/REMOTE_COMPANION.md` §2f/§6/§7, matrix RLC9/RLC16,
  `AI/ARCHITECTURE.md` §13.
