# Tester Contract — GitHub #345: Live Camera View in `soos-remote`

- **Date**: 2026-10-07
- **Branch**: `feat/remote-live-camera` (Phase 2, TDD Red; nothing committed)
- **Inputs**: `AI/architect_spec_remote_live_camera.md` (round 2, approved), `AI/plan_evaluator_report.md`
  (APPROVED, developer notes O1–O4), ADR "[2026-10-07] Live Camera View in `soos-remote` Through the Daemon Preview
  Channel" (`AI/DECISIONS.md`), `AI/research_live_camera.md`.
- **Matrix rows**: RLC1–RLC16 (RLC16 is the owner's manual hardware check, no automated test).
- Every test below is a contract for `developer-agent`: it may not be modified, weakened or deleted.

## 1. Placement decisions

- Tests 36–40 are mapped by the spec to existing files (`config_tests.rs`, `routes_tests.rs`, `push_tests.rs`,
  `auth_store_tests.rs`). They use only new API, so they live in **new** files (`camera_config_tests.rs`,
  `camera_routes_tests.rs`, `camera_push_tests.rs`, `camera_challenge_tests.rs`) with the **spec names unchanged**.
  The existing suites therefore keep compiling until the migrations of §5 are applied. The spec explicitly allows a
  new file for test 40.
- Test 49 (`test_rlc_preview_format_codes`) lives in `crates/daemon/tests/preview_remote_view_tests.rs`, as the spec
  allows: the protocol crate cannot see `soos_daemon::preview::PREVIEW_FORMAT_EMPTY`.
- Test 20 is split into five functions that share the spec name as a prefix
  (`test_rlc_client_holds_at_most_one_connection*`), so a `grep test_rlc_client_holds_at_most_one_connection` finds
  all of them.
- The tester-owned `ScriptedPreviewSource` (records started, completed and dropped `next_frame` calls) and the camera
  config helper live in the new shared fixture `crates/remote/tests/common/camera.rs`.

## 2. Test-facing API the developer must provide (beyond the spec text)

The spec leaves a few signatures implicit. The tests fix them as follows (all are minimal and consistent with the
spec; none is a production behaviour change):

| Item | Fixed by the tests | Why |
|---|---|---|
| `ViewSlot::new() -> ViewSlot` | `camera_slot_tests.rs` | §7.1 defines the struct but no constructor |
| `ViewSlot` time type | `tokio::time::Instant` (as `ChallengeStore`) | §7.1 says "`Instant` passed in"; `ViewTicket.ends_at` feeds `sleep_until` |
| `ViewSlot::begin(now, owner: ViewOwner, &ViewToken, max_view: Duration)` | owner **by value** (like `reserve`) | §7.2 writes `begin(now, owner, &presented, max_view)` |
| `ViewSlot::phase(&self, now) -> (SlotPhase, Option<u64>)` | `&self` | "read-only view" |
| `camera_jpeg::encode_preview_jpeg_into(format, width, height, data, settings, sink: JpegSink) -> Result<JpegFrame, FrameError>` (`#[doc(hidden)]`) | `camera_jpeg_tests.rs` test 7 | §13.2 test 7 needs `encode_preview_jpeg` to run over a `JpegSink::with_capacity_for_tests` sink; the bound `MAX_CAMERA_JPEG_BYTES` is unreachable with a valid frame (measured: worst case 640x480 noise at q85 ≈ 301 KB), so `TooLarge` is only testable through an injected sink. `encode_preview_jpeg(..)` must equal `encode_preview_jpeg_into(.., JpegSink::new())`. |
| `JpegSink::with_capacity_for_tests(usize) -> JpegSink` (`#[doc(hidden)]`) | test 7 | named by the spec |
| `PixelKind`, `FrameError`, `JpegSettings` derive `Debug, PartialEq` | assertions | §6.1 already derives them |

(Server-side and IPC test hooks: see §3.4 and §3.5.)

## 3. Test table

Red evidence was produced with `cargo test --locked -p <pkg> --all-features --test <target> --no-run` (compile) or a
targeted run (assertions). "E0432" = unresolved import of the specified-but-missing API.

### 3.1 `crates/remote/tests/camera_jpeg_tests.rs` (9 tests, spec §13.2)

| Test | Acceptance / matrix | Red evidence |
|---|---|---|
| `test_rlc_grey_frame_encodes_to_baseline_jpeg` (1) | RLC11 | E0432 `soos_remote::camera_jpeg`, `config::CameraWidth`, camera constants |
| `test_rlc_yuyv_conversion_shuffles_chroma` (2) | RLC11 | same |
| `test_rlc_rgb24_frame_encodes` (3) | RLC11 | same |
| `test_rlc_half_width_box_average` (4, F6) | RLC9 | same |
| `test_rlc_frame_geometry_bounds` (5, F6) | RLC9, RLC-S6 | same |
| `test_rlc_unsupported_formats_refused` (6) | RLC11, RLC-S8 | same |
| `test_rlc_jpeg_sink_never_grows` (7) | RLC-S6, RLC13 | same |
| `test_rlc_quality_changes_quantization` (8) | RLC11 | same |
| `test_rlc_convert_never_panics` (9, proptest) | RLC11, RLC-S6 | same |

### 3.2 `crates/remote/tests/camera_slot_tests.rs` (7 tests, spec §13.3)

| Test | Acceptance / matrix | Red evidence |
|---|---|---|
| `test_rlc_slot_reserve_begin_stream_end_cycle` (10) | RLC9 | E0432 `soos_remote::camera_slot`, `CAMERA_VIEW_*` constants |
| `test_rlc_slot_single_global_view` (11) | RLC9, RLC-S5 | same |
| `test_rlc_slot_token_single_use_and_ttl` (12) | RLC8, RLC-S4 | same |
| `test_rlc_slot_token_bound_to_owner` (13) | RLC8 | same |
| `test_rlc_slot_no_cooldown_after_any_view` (14; was `test_rlc_slot_cooldown_only_after_shown_view`, M13) | RLC9 | same |
| `test_rlc_slot_stop` (15, F3 durable flag) | RLC14, RLC-S10 | same |
| `test_rlc_view_token_format_and_redaction` (16) | RLC8, RLC-S4 | same |

Power check (outside the repository, scratch crate in the session scratchpad): with a reference implementation of
§6/§7 all 16 tests of 3.1–3.2 pass; a truncating box average plus "even width for every format" fails tests 4 and 5;
a `mark_streaming` that clears the stop flag fails test 15. This also caught and fixed two tester mistakes before
hand-off (a tuple arity and the all-0xFF token encoding).

### 3.3 Remote additions in new files (spec §13.6)

| Test (path) | Acceptance / matrix | Red evidence |
|---|---|---|
| `camera_config_tests.rs::test_rlc_camera_config_keys` (36) | RLC1 | E0432 `config::{CameraConfig, CameraWidth}`; E0599 the 7 new `ConfigError` variants; E0609 `RemoteConfig.camera` |
| `camera_config_tests.rs::test_rlc_camera_constants_match_the_spec` (extra, not numbered) | §3.2 constants (RLC1, RLC-S6) | E0432 on the 43 new `soos_remote::*` constants |
| `camera_routes_tests.rs::test_rlc_camera_route_table` (37) | RLC8, RLC6 | E0432 `routes::{camera_stream_token, check_camera_csrf, CAMERA_*_PATH, CAMERA_STREAM_PREFIX}`, `camera_slot`; E0599 `Route::Camera*` |
| `camera_routes_tests.rs::test_rlc_camera_csrf_rules` (38) | RLC6 | same |
| `camera_push_tests.rs::test_rlc_camera_payload` (39; deleted by M14, see below) | RLC13, RLC15 | E0432 `push::camera_payload`, `PUSH_CAMERA_TOPIC` |
| `camera_challenge_tests.rs::test_rlc_camera_view_challenge_pool` (40) | RLC6, RLC-S3 | E0432 `ChallengePurpose::CameraView` |

### 3.4 `crates/remote/tests/camera_ipc_tests.rs` (11 functions = tests 17–22, 62; spec §13.4)

| Test | Acceptance / matrix | Red evidence |
|---|---|---|
| `test_rlc_client_sends_preview_request` (17) | RLC12 | E0432 `soos_remote::camera_ipc`, 7 `CAMERA_DAEMON_*` constants; E0433 `soos_protocol` (not yet a dependency of `soos-remote`, added by the developer per §1.1); `nix::time` (feature `time` added by the developer per §1.1) |
| `test_rlc_client_maps_daemon_replies` (18) | RLC12 | same |
| `test_rlc_client_bounds_replies` (19) | RLC12, RLC-S6 | same |
| `test_rlc_client_holds_at_most_one_connection` (20) | RLC12, RLC-S5 | same |
| `test_rlc_client_holds_at_most_one_connection_reconnects_after_lifetime` (20, F5 ordering) | RLC12 | same |
| `test_rlc_client_holds_at_most_one_connection_bounded_close_wait` (20, F5 200 ms bound) | RLC12 | same |
| `test_rlc_client_holds_at_most_one_connection_request_cap` (20) | RLC12 | same |
| `test_rlc_client_holds_at_most_one_connection_after_cancel` (20) | RLC12 | same |
| `test_rlc_client_refuses_unexpected_daemon_uid` (21) | RLC12, RLC-S5 | same |
| `test_rlc_client_exchange_is_time_bounded` (22) | RLC12, RLC-S6 | same |
| `test_rlc_client_retries_once_after_idle_close` (62, F12) | RLC12 | same |

The fake daemon is a tokio `UnixListener` on a tempdir socket speaking `soos-protocol`; the client is built with
`DaemonPreviewClient::new(path, getuid()).with_expected_daemon_uid(getuid())` except in test 21. Power check (scratch
crate): against a wrong stub all 11 fail on assertions; against a reference implementation of §5.2 all 11 pass, 10
consecutive runs green; dropping the old connection without the graceful close fails the three F5 tests.
**Deviation**: the spec asks for paused tokio time in tests 20 and 22; tokio auto-advance fires timers while real
socket I/O is pending, which made the tests fail for the wrong reason. They use the real clock; the 25 s lifetime is
skipped with `pause`/`advance`/`resume` only while no exchange is in flight.

### 3.5 `crates/remote/tests/camera_server_tests.rs` (14 tests = 23–35, 61; spec §13.5)

| Test | Acceptance / matrix | Red evidence |
|---|---|---|
| `test_rlc_camera_disabled_by_default` (23) | RLC1, RLC-S2 | 7 compile errors: E0432 `soos_remote::camera`, `camera_ipc`, `config::{CameraConfig, CameraWidth}`, camera constants; E0425 `MAX_CAMERA_JPEG_BYTES`; `with_camera` |
| `test_rlc_start_requires_fresh_uv_camera_assertion` (24) | RLC6, RLC-S3 | same |
| `test_rlc_start_gate_order` (25) | RLC6 | same |
| `test_rlc_funnel_is_tailnet_only_by_default` (26) | RLC7 | same |
| `test_rlc_stream_is_multipart_jpeg` (27) | RLC10 | same |
| `test_rlc_stream_token_single_use_and_owner_bound` (28) | RLC8, RLC-S4 | same |
| `test_rlc_one_view_and_no_cooldown` (29; was `test_rlc_one_view_and_cooldown`, M13) | RLC9 | same |
| `test_rlc_view_end_conditions` (30, F3 a/b/c) | RLC14, RLC-S10 | same |
| `test_rlc_frame_rate_and_no_replay` (31, F11) | RLC9, RLC-S9 | same |
| `test_rlc_non_terminal_arms_never_cancel_a_frame` (61, F2) | RLC14, RLC-S10 | same |
| `test_rlc_first_frame_failures_are_json_errors` (32) | RLC12 | same |
| `test_rlc_rate_limited_backs_off_without_ending` (33) | RLC12 | same |
| `test_rlc_camera_audit_lines` (34) | RLC15, RLC-S11 | same |
| `test_rlc_no_push_on_camera_view` (35; was `test_rlc_push_on_view_start_is_best_effort`, M14) | RLC15, RLC-S11 | same |

Fixture: `crates/remote/tests/common/camera.rs` (`SourceSpy`/`ScriptedPreviewSource` recording factory calls, started,
completed and cancelled `next_frame` calls with virtual times and dropped sources; `start_camera(..)` building its own
`RemoteConfig` with `camera` and calling `ServerState::with_camera(CameraSettings::from_config(..), spy.factory())`,
optionally `with_push`; a strict multipart `ViewClient` that panics on any misaligned part). The target also includes
`common/harness.rs`, so it needs migration M2 to compile once the API exists.
Power check (throwaway scratch copy with API stubs and M2 applied): the file compiles, clippy `-D warnings` is clean,
and all 14 tests fail on assertions (camera routes answer 404) without hanging.
Assumptions fixed by these tests where the spec is silent: `CameraSettings` lives in `soos_remote::camera`; every camera
constant is exported at the crate root; the `GET /api/camera` body has exactly the 7 keys of §8.3; `view_ready` has
exactly `result`, `stream_path`, `token_ttl_ms`, `max_view_s`, `fps`, `width`; `HEAD /api/camera` is `200` with an
empty body; a body on the stop route is `413`; `POST` on a well-formed stream path is `405` with `Allow: GET`; a failed
first frame answers the JSON error then closes; UV-cleared and replayed assertions each count one failure, and the 6th
attempt is `429 rate_limited` on camera options, camera start and unlock options (shared lockout, S-5).
Approximations: part timing is sampled in 20 ms virtual steps (F11 allows at most one gap under 100 ms after the slow
exchange); the rate bound is checked over a 3 s window; a blocked part write is inferred from the source not being
asked for frames across two 300 ms steps (≈ 100 KiB noise JPEGs against the default Unix socket buffer); 61(ii) relies
on 1.4 s paused-reader windows covering the 5 s session-check ticks. Encoding waits on the real blocking pool, so
these tests can be slow in debug builds; flakiness is unchecked until the implementation exists.

### 3.6 `crates/daemon/tests/preview_remote_view_tests.rs` (9 tests, spec §13.7 + test 49)

| Test | Acceptance / matrix | Red evidence |
|---|---|---|
| `test_rlc_preview_format_codes` (49) | RLC2 (S-8) | E0432 `soos_protocol::types::PREVIEW_FORMAT_*` (6 constants) |
| `test_rlc_classify_preview_peer_cgroup` (41, F11) | RLC4 | E0432 `soos_daemon::preview_peer` |
| `test_rlc_remote_view_refused_unless_enabled` (42) | RLC4, RLC-S2 | E0560/E0609 `PreviewConfig.remote_view`; E0599 `with_preview_peer_source` |
| `test_rlc_unreadable_peer_cgroup_refuses_preview` (43) | RLC4 | same |
| `test_rlc_preview_requires_local_seat_session` (44, F1) | RLC3 | E0599 `is_local_seat_session_of`, `has_local_seat_session`, `with_preview_peer_source` |
| `test_rlc_remote_first_frame_logged_once_per_connection` (45) | RLC5 | same family |
| `test_rlc_remote_view_config_key` (46) | RLC1 | `remote_view` field |
| `test_rlc_local_peer_unaffected_by_remote_view` (47) | RLC4 | same family |
| `test_rlc_view_connection_leaves_room_for_auth` (63, F11) | RLC4 (ADR item 1) | same family |

25 compile errors in total, all on the API above. The other daemon test targets (`preview_authorization_tests`,
`session_policy_tests`, `peer_limits_tests`) still compile. Note: test 63 uses only existing admission behaviour
besides `remote_view` and the new hook; it is red now only because the file does not compile and is expected to pass
as soon as it does (a regression contract for R-3). Interpretations: §9.2 "first non-`.slice` component" means a
`soos-remote.service` directly under `user@<u>.service` is the companion; a `soos-gui.service` user unit is `Local`.
Dispatcher tests that need an unprivileged peer return early when run as root (same pattern as the existing preview
tests).

### 3.7 `tests/invariants/src/remote_camera_contract.rs` (10 tests, spec §13.8; `mod` added in `lib.rs`)

`cargo test --locked -p soos-invariants --all-features remote_camera_contract`: **0 passed, 10 failed**, each on an
assertion message (no bare `unwrap` panic):

| Test | Acceptance / matrix | Red evidence (assertion) |
|---|---|---|
| `test_rlc_s1_remote_camera_dependencies` (50) | RLC2, RLC-S1 | `crates/remote/Cargo.toml` must declare exactly `soos-protocol = { workspace = true }` |
| `test_rlc_s2_ijg_is_allowed_for_jpeg_encoder_only` (51) | RLC2 | `deny.toml` must contain the `jpeg-encoder`/`IJG` exception |
| `test_rlc_s3_camera_modules_never_log` (52) | RLC13, RLC-S7 | `crates/remote/src/camera.rs` must exist |
| `test_rlc_s4_no_recording_path` (53) | RLC13, RLC-S7 | `crates/remote/src/camera.rs` must exist |
| `test_rlc_s5_camera_types_redacted` (54) | RLC13, RLC-S4 | `crates/remote/src/camera_slot.rs` must exist |
| `test_rlc_s6_page_reads_the_stream_into_a_canvas` (55) | RLC10, RLC-S12 | `app.js` must contain `/api/auth/camera/options` (the pinned CSP sub-check already passes: it guards a change) |
| `test_rlc_s7_daemon_remote_view_gate` (56) | RLC3 | `crates/daemon/src/preview_peer.rs` must exist |
| `test_rlc_s8_camera_is_documented` (57) | RLC15 | `Docs/REMOTE_COMPANION.md` has no `## 2f.` section |
| `test_rlc_s9_installer_template` (58) | RLC15 | template must contain `# camera_view = false` |
| `test_rlc_s10_no_decoder_and_no_device_access_in_remote` (59) | RLC2, RLC-S8 | `crates/remote/src/camera_jpeg.rs` must exist (guards against a vacuous pass) |

Every scan of camera modules first asserts that the four files exist, so no test passes vacuously. Interpretations:
s2 requires the comment before `exceptions` to name "ADR" or "2026-10-07" and `"IJG"` to appear exactly once in
`deny.toml`; s5 accepts `impl Drop` or `ZeroizeOnDrop` for `ViewToken` and forbids any `Debug` (derived or manual) on
`PreviewFrame`, `JpegFrame`, `JpegSink`; s7 checks the body of `handle_preview_request` only and requires
`pub mod preview_peer;` and `pub fn classify_preview_peer_cgroup(`; s9 bans `/etc` on non-`echo`, non-comment lines
and `sudo` on every non-comment line. The `cargo tree` part of s1 needs `jpeg-encoder` fetched (`cargo fetch` once
after the manifest change).

### 3.8 Test 60 (migrated, not written by the tester)

`test_rmc_s4_remote_is_a_leaf_crate` — owner-approved assertion migration (LC-1), applied by the orchestrator (§5).

## 4. Dependencies and manifests left to the developer

The spec assigns every manifest change to the developer (§1.1), so the tester changed none:
`crates/remote/Cargo.toml` (+ `soos-protocol`, + `jpeg-encoder`, `nix` feature `time`), root `Cargo.toml`
(`jpeg-encoder = { version = "=0.7.1" }`), `deny.toml` (crate-scoped `IJG` exception). The red state of
`camera_ipc_tests.rs` therefore also includes "`soos_protocol` not found" and "`nix::time` not found".

## 5. Migrated existing tests (applied by the orchestrator, NOT by the tester)

| # | Test / file:line | Old | New | Kind / mandate |
|---|---|---|---|---|
| M1 | `tests/invariants/src/remote_companion_contract.rs:644` in `test_rmc_s4_remote_is_a_leaf_crate` (forbidden list l.636–650) | `"soos-protocol",` entry of the `for forbidden in [...]` list | remove that entry; **add** after the loop: the manifest contains exactly the line `soos-protocol = { workspace = true }`, and every dependency key of `crates/remote/Cargo.toml` starting with `soos-` is `soos-protocol` or `soos-push-protocol` | Assertion change, owner-approved (LC-1, spec §12) |
| M2 | `crates/remote/tests/common/harness.rs:783-784` | `push: soos_remote::config::PushConfig::default(),` then `};` | insert `camera: soos_remote::config::CameraConfig::default(),` before `};` | Setup-only (compile) |
| M3 | `crates/remote/tests/server_tests.rs:774-775` | `push: soos_remote::config::PushConfig::default(),` then `};` | insert `camera: soos_remote::config::CameraConfig::default(),` before `};` | Setup-only (compile) |
| M4 | `crates/remote/tests/alerts_server_tests.rs:219-220` | `push: soos_remote::config::PushConfig::default(),` then `};` | insert `camera: soos_remote::config::CameraConfig::default(),` before `};` | Setup-only (compile) |
| M5 | `crates/remote/tests/push_server_tests.rs:204-205` | `push,` then `};` | insert `camera: soos_remote::config::CameraConfig::default(),` before `};` | Setup-only (compile) |
| M6 | `crates/daemon/tests/preview_authorization_tests.rs:511` (literal l.508–512) | `max_requests_per_sec: 2,` then `}),` | add `remote_view: false,` after `max_requests_per_sec: 2,` | Setup-only (compile) |
| M7 | `crates/daemon/tests/preview_authorization_tests.rs:476` in `test_preview_frame_allowed_for_configured_uid_serves_frames` | `format!("UID={current_uid}\nACTIVE=1\nSTATE=active\n"),` | `format!("UID={current_uid}\nACTIVE=1\nSTATE=active\nREMOTE=0\nSEAT=seat0\nCLASS=user\n"),` | Setup-only (fixture; assertions unchanged) |
| M8 | `crates/remote/tests/camera_jpeg_tests.rs` crate-level `#![allow(..)]` header | no `clippy::manual_div_ceil` | add `clippy::manual_div_ceil` (the `avg2` reference keeps `(a + b + 1) / 2`) | Lint-only (candid review MAJOR 1; assertions unchanged) |
| M9 | `crates/remote/tests/camera_ipc_tests.rs` crate-level `#![allow(..)]` header | no `clippy::enum_variant_names` | add `clippy::enum_variant_names` (fake-daemon `Reply` enum) | Lint-only (candid review MAJOR 1; assertions unchanged) |
| M10 | `crates/daemon/tests/preview_remote_view_tests.rs` crate-level `#![allow(..)]` header | no `clippy::useless_format`, no `clippy::print_stderr` | add both (fixture `format!` at l.791, skip notice `eprintln!` in `test_rlc_default_peer_source_reads_proc_cgroup`) | Lint-only (CI clippy `--all-features`; assertions unchanged) |
| M11 | `tests/invariants/src/remote_camera_contract.rs` test 58 `test_rlc_s9_installer_template`, final loop | on every non-comment, non-`echo` line: `assert!(!line.contains("/etc"))` (false positive on the read-only `target="$(readlink -f /etc/resolv.conf 2>/dev/null \|\| true)"` of `warn_resolver_stub`, already on `main`) | every non-comment line goes through `installer_line_touches_etc`: a `>`/`>>` redirection or a `tee` stage towards `/etc` is rejected on **every** line, `echo` included (stricter than before); any non-`echo` line naming `/etc` is rejected (covers `cp`, `install`, `mv`, `ln`, `sed -i`, `mkdir`, `rm`, `chmod`, `chown`), except the exact constant `INSTALLER_RESOLVER_PROBE`; a self-check asserts the detector rejects 16 synthetic write lines (including `readlink` on another `/etc` file and the probe with a redirection) and accepts only the probe and a print-only `echo`. The `sudo` assertion is unchanged; the script is unchanged (no path obfuscation) | Assertion change, owner approval relayed by the workflow orchestrator (candid review round 2 MAJOR); not recorded elsewhere in the repository, the owner must confirm before merge |

The fifth `RemoteConfig` literal, `crates/remote/tests/common/camera.rs`, is new tester code and already carries
`camera:`. No other existing test or assertion was touched beyond M1-M11. Any other existing test that fails after the
implementation must be reported, not edited (spec §12).

## 6. Flakiness check

Nothing of the new suites runs before the implementation exists (compile red), except the 10 invariant tests, which
are static and deterministic. The IPC suite was run 10 times in a row against a reference implementation in a
scratch crate (green 10/10, ≈ 6 s per run); the pure JPEG/slot suites against their reference were deterministic.
The developer must run every timing-sensitive test of `camera_ipc_tests.rs` and `camera_server_tests.rs` 10 times in a
row once green:
`for i in $(seq 10); do cargo test --locked -p soos-remote --all-features --test camera_server_tests -q || break; done`.

## 7. Round 2 — auditor defects B1–B11 (`AI/auditor_constraints_remote_live_camera.md`, Clearance BLOCKED)

Only new test files of this branch were changed. No existing test was touched, the orchestrator's migrations M1–M7 were
left as they are, and no assertion was weakened (every change adds a check or removes a timing hazard).

| Defect | Change | File | Evidence |
|---|---|---|---|
| B1 | `test_rlc_s1` runs `cargo tree -e normal --target all --format "{p}\|{f}"` for `-p soos-remote` and for `--workspace`: every resolved `jpeg-encoder` node must have exactly the features `{default, std}`, so a `simd` feature enabled through unification fails. A tree without `jpeg-encoder` fails ("must resolve jpeg-encoder"); the test is skipped only when cargo is unavailable. Any `simd`/`features` on a `jpeg-encoder` line, or a `[target.` table naming it, in either manifest also fails. | `tests/invariants/src/remote_camera_contract.rs` | red (assertion) |
| B2 | 4 new tests. `test_rlc_default_peer_source_reads_proc_cgroup`: no injected source, `enforce_active_session` false and true; the verdict must match `classify_preview_peer_cgroup(/proc/self/cgroup)`, which fails a trait-default `Ok(None)` source or one built only under enforcement. `test_rlc_default_peer_source_is_system_procfs`: static pin; the proc root cannot be set through `DispatcherConfig`, so `dispatcher.rs` must contain `SystemLogind::with_paths(` and `DEFAULT_PROC_ROOT`. `test_rlc_procfs_peer_source_classifies_companion`: `SystemLogind::with_paths(tmp_sessions, tmp_proc)` with a fake `<own pid>/cgroup`; the companion is refused with `remote_view = false` and served with `true`; a cgroup file over `MAX_CGROUP_FILE_SIZE` is refused. `test_rlc_peer_cgroup_read_once_per_preview_connection`: a counting source sees 0 reads for `Status`/`Auth`, 1 per preview connection over two frames, 2 after a second connection. | `crates/daemon/tests/preview_remote_view_tests.rs` | compile red (27 errors, all spec API) |
| B3 | Test 41 gains 17 cases. RemoteCompanion: any slice depth (`app.slice/app-soos.slice/…`, `a.slice/b.slice/c.slice/…`) and sub-cgroups (`…/soos-remote.service/child`, `…/a/b`). Local: only the first non-`.slice` component counts (`other.service/soos-remote.service`, `app-x.scope/…`). Malformed: two disagreeing `0::` lines (both orders); `..` (user-manager path, session path, v1 `name=systemd` line); an empty component `//` at two positions (C29). | same | compile red |
| B4 | `test_rlc_s3` counts `event!`/`tracing::event!` as logging macros. It asserts that the camera functions of `server.rs` (`camera_options/start/stream/stop` + every fn naming `camera`), `http.rs` (`encode_camera_stream_head/part_head`) and `push.rs` (`send_camera`, `queue_camera_view`, `camera_payload`) exist. Their log calls may have no `{` arguments and no field containing `token`, `width`, `height`, `bytes`, `sequence` or `len`. | invariants | red (assertion) |
| B5 | `test_rlc_s4` also bans `fs::`, `{fs`, ` fs,`, `, fs}`, `, fs `, `process::Command` in `camera*.rs` and in those camera functions. New self-test `test_rlc_s4_recording_scanner_catches_aliased_fs` (passes) proves `use std::{fs, io}; fs::write(..)` and an `event!` with a `token` field are caught, and clean code is not. | invariants | red; self-test green |
| B6 | The fake-side close-wait check gets a 10 ms tolerance (`CLOSE_WAIT_TOLERANCE_MS`). Two strict client-side checks are added: accept time − call start ≥ `CAMERA_DAEMON_CLOSE_WAIT_MS`, and call duration ≥ `CAMERA_DAEMON_CLOSE_WAIT_MS`. | `crates/remote/tests/camera_ipc_tests.rs` | scratch reference: 11/11 green, 10 runs in a row |
| B7 | Test 30(a) uses `wait_write_blocked`: a step counts as quiet only after a full real `ENCODE_WAIT` (1.5 s) with no `next_frame` start. Before the stop it asserts the view is still streaming (no EOF) and `spy.frames_returned() >= parts + 2`; the existing post-stop checks are unchanged. | `camera_server_tests.rs`, `common/camera.rs` | stubs: red |
| B8 | Test 61(i) ends at 12 700 ms, off the 1 500 ms exchange boundary. `cancelled == 0` is kept; the count check becomes `completed + 1 >= started && completed <= started`. | `camera_server_tests.rs` | stubs: red |
| B9 | Readers wait `ENCODE_WAIT` only for a frame with a new sequence (`SourceSpy::fresh_frames()`). Tests 31 and 61(ii) carry a `RealBudget`, which fails with a message beyond 17 s of real time, before the 20 s `FrozenClock` fallback. | both | stubs: red |
| B10 | The three `open_view(.., 0)` calls use `VIEW_STEP_MS`. | `camera_server_tests.rs` | stubs: red |
| B11 | `open_raw` is bounded: 64 scheduler rounds, then 20 ms virtual steps, a 12 s real-time cap and a failure at `max_ms + 1 s` ("the response body never completed"), so a wrong implementation fails instead of hanging. | `common/camera.rs` | stubs: 14/14 fail in 0.02 s, no hang |

Round-2 power-check limits: against the stubs the camera routes return `404`, so the new B7–B11 code paths have not
run yet. They are first exercised by the implementation; the 10× flakiness run of §6 applies to them. Note that the
already-applied migrations (M1 in invariants, M2 in `common/harness.rs`, M6 in `preview_authorization_tests.rs`) are red
or do not compile until the API and manifest exist, as expected.

## Owner-approved amendment M12 (2026-10-07): stream `Content-Type`

Test 27 `test_rlc_stream_is_multipart_jpeg` expected `multipart/x-mixed-replace; boundary=soosframe`. On the owner's
iPhone the iOS network stack split such a response and the page's `fetch` failed ("Load failed"). With the owner's
explicit approval ("corrige ca", 2026-10-07) the expected head `Content-Type` is now `application/octet-stream`; the
part framing (`--soosframe`, `Content-Type: image/jpeg`, `Content-Length`), the mandatory headers, the CSP, the absence
of `Content-Length`/`Transfer-Encoding` and every JPEG assertion are unchanged.

## Owner-approved amendment M13 (2026-10-07): no cooldown between views

The owner asked (2026-10-07, after the live view worked on the iPhone) to "remove the countdown when we activate the
camera and deactivate and reactivate it", and explicitly approved turning the cooldown tests into assertions that
there is **no** cooldown. Face ID per view, the single global view slot, the token TTL and the shared failure lockout
are unchanged; no other test was weakened.

| Id | Test / location | Old rule | New rule |
|---|---|---|---|
| M13a | `camera_slot_tests.rs` test 10 `test_rlc_slot_reserve_begin_stream_end_cycle` | `end(shown)` → `(Cooldown, Some(10_000))`; second reserve only after `CAMERA_VIEW_COOLDOWN_MS` | `end` → `Idle`, `check` is `Ok` and a second reserve/begin succeeds at the same instant with a new view id |
| M13b | `camera_slot_tests.rs` test 14 `test_rlc_slot_cooldown_only_after_shown_view` → `test_rlc_slot_no_cooldown_after_any_view` | cooldown only after a shown view, `SlotError::Cooldown { remaining_ms }` counting down to 0 | after a shown view, a stopped view and an unshown view the slot is `Idle` at once and reserves immediately; stale/second `end` stay no-ops; exhaustive matches pin `SlotPhase` = {Idle, Pending, Starting, Streaming} and `SlotError` = {Busy, TokenRejected} (stricter: a cooldown variant no longer compiles) |
| M13c | `camera_slot_tests.rs` test 15 `test_rlc_slot_stop` | the Streaming sub-case started at `t0 + CAMERA_VIEW_COOLDOWN_MS` | it starts at `t0`, right after the previous view (proves the immediate restart) |
| M13d | `camera_slot_tests.rs` (whole file) | `ViewSlot::phase` returned `(SlotPhase, Option<u64>)`, `end(now, view, shown)` | `phase` returns `SlotPhase`, `end(view)` (API shape only, assertions otherwise identical) |
| M13e | `camera_config_tests.rs` `test_rlc_camera_constants_match_the_spec` | `CAMERA_VIEW_COOLDOWN_MS == 10_000` | constant removed; its absence is pinned by invariant test 60 |
| M13f | `camera_server_tests.rs` helper `assert_ended_shown` | `state == "cooldown"`, `cooldown_ms > 0` | `state == "idle"`, no `cooldown_ms` field |
| M13g | `camera_server_tests.rs` test 23 `assert_camera_disabled` key list | keys include `cooldown_ms` | `cooldown_ms` removed from the `GET /api/camera` shape (every other key unchanged) |
| M13h | `camera_server_tests.rs` test 28 `test_rlc_stream_token_single_use_and_owner_bound` | `advance_ms(CAMERA_VIEW_COOLDOWN_MS)` before `idle`; `cooldown_ms == 0` after the TTL | `idle` 100 ms after the view closed without waiting; no `cooldown_ms` field |
| M13i | `camera_server_tests.rs` test 29 `test_rlc_one_view_and_cooldown` → `test_rlc_one_view_and_no_cooldown` | `429 camera_cooldown` with `retry_after_ms` after a stopped view, `state == "cooldown"` | after a stopped view: `state == "idle"`, no `cooldown_ms`, an immediate new start (fresh assertion) streams pixels again, then a third start is accepted; `409 view_in_progress` during a view unchanged |
| M13j | `camera_server_tests.rs` test 30 `test_rlc_view_end_conditions` | stop / `max_view_s` sub-cases checked the cooldown state | they now also assert an immediate restart (`assert_immediate_restart`) |
| M13k | `common/camera.rs` comment of `HARNESS_REAL_BUDGET` | mentions cooldown timers | comment only |
| M13l | `AI/VERIFICATION_MATRIX.md` RLC9 / RLC16 | 10 s cooldown, "second start within 10 s is refused" | no cooldown, immediate restart; full screen and rotation in the owner check |

New static test (not a migration): `tests/invariants/src/remote_camera_contract.rs` test 60
`test_rlc_page_fullscreen_rotate_and_no_cooldown` pins the full-screen toggle (`requestFullscreen` /
`webkitRequestFullscreen` with the overlay fallback), the exit control (`"Exit full screen"`, Escape,
`fullscreenchange`), the two-mode orientation class `camera-portrait` (owner correction 2026-10-07: two modes, not four rotations; `camera-rot-*`, `rotate(180deg)` and `rotate(270deg)` are now forbidden), the full-screen exit in `endCameraView`, the
classList-only styling, and the absence of `cooldown`, `retry_after_ms` and `CAMERA_VIEW_COOLDOWN_MS` in the page and
the camera/server sources. Red evidence: the slot, config and server test files did not compile against the old API
(E0308/E0061), and test 60 failed on its first assertion before the page change.

## Owner-approved amendment M14 (2026-10-07): no push notification at camera start

The owner asked (2026-10-07) to "remove the notification that is sent when we start the camera" and explicitly
approved removing the camera-start Web Push and amending the tests that pin it. The failed-password alert pushes, the
test notification, the audit lines (`camera view started` / `ended` / `refused`) and every other camera test are
unchanged; no other test was weakened.

| Id | Test / location | Old rule | New rule |
|---|---|---|---|
| M14a | `camera_server_tests.rs` test 35 `test_rlc_push_on_view_start_is_best_effort` → `test_rlc_no_push_on_camera_view` | one delivery with topic `sooscamera` and the fixed payload per started view; a hanging or failing transport never delays the frames; no push when the first frame failed | with push active and one subscription, a view that starts, runs (frames flowing), stops and ends causes **zero** transport calls whatever the topic, and so does a view whose first frame failed; positive control: the same spy then receives exactly one test notification (`PUSH_TEST_TOPIC`), so the silence is not an unwired transport |
| M14b | `camera_push_tests.rs` test 39 `test_rlc_camera_payload` | fixed camera payload text, `kind: "camera"`, topic `sooscamera` | file deleted (nothing meaningful remains); the absence is pinned by invariant test 63 |
| M14c | `camera_config_tests.rs` `test_rlc_camera_constants_match_the_spec` | `PUSH_CAMERA_TOPIC == "sooscamera"` | constant removed; its absence is pinned by invariant test 63 |
| M14d | `remote_camera_contract.rs` `camera_functions_outside_modules` (used by tests 52 and 53) | `push.rs` must define `send_camera`, `queue_camera_view`, `camera_payload` (scanned for log hygiene and recording paths) | `push.rs` entry removed; the `server.rs` and `http.rs` camera functions are still required and scanned |
| M14e | `AI/VERIFICATION_MATRIX.md` RLC13 / RLC15 / RLC16 | best-effort push `sooscamera` at view start; owner check "the push camera view started arrives" | no Web Push for a camera view; owner check "no push notification arrives for the view" |

New static test (not a migration): `tests/invariants/src/remote_camera_contract.rs` test 63
`test_remote_camera_sends_no_push` asserts that no file of `crates/remote/src` (comments included) contains
`sooscamera`, `queue_camera_view`, `camera_payload`, `send_camera` or `PUSH_CAMERA_TOPIC`, that `push.rs` renders no
`kind: "camera"`, that `sw.js` has no camera notification (`"camera"`, `soos-camera`) and that this contract records
M14. Red evidence: test 35 (new form) failed with one `sooscamera` call on the old code path, and test 63 failed on
`crates/remote/src/lib.rs must not contain sooscamera` before the removal.
