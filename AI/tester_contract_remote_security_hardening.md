# Tester Contract — soos-remote Security Hardening (security review of 2026-10-12)

- **Date**: 2026-10-12
- **Branch**: `fix/remote-security-hardening` (GitHub-only security-review fix, no backlog id, not registered in
  `scripts/sync_issue.py`)
- **Spec**: `AI/architect_spec_remote_security_hardening.md` round 2 (plan evaluator APPROVED `8883c1c`, PE-SR8 to
  PE-SR11 folded in `bccaa28`)
- **Matrix rows**: RSH1–RSH17 (RSH2 is the agent host check recorded in spec §1.1; RSH18 is the owner hardware check,
  no automated test)
- **Production code**: none. The only non-test changes are the daemon dev-dependency `jpeg-encoder = { workspace =
  true }` (spec §11.2 and §15: valid MJPEG fixtures) and its one-line `Cargo.lock` entry.

## Files

| File | Kind | Tests |
|---|---|---|
| `tests/invariants/src/remote_security_hardening_contract.rs` (new) + `mod` line in `tests/invariants/src/lib.rs` | static | 13 `test_rsh_s*` / `test_rsh_docs_*` |
| `tests/invariants/src/remote_push_contract.rs` | static, **CM-SR1** | test 45 amended (additive) |
| `crates/daemon/tests/peer_limits_tests.rs` (appended `mod auth_reserve`) | pure limiter + Unix socket, unprivileged test UID | 11 |
| `crates/daemon/tests/preview_placement_tests.rs` (new) | pure policy + dispatcher over a temporary sessions directory | 11 |
| `crates/daemon/tests/preview_downscale_tests.rs` (appended `mod remote_preview`) | pure conversion + proptest | 10 |
| `crates/daemon/tests/preview_remote_view_tests.rs` (appended `mod remote_conversion`; **AM-SR1** setup line) | dispatcher with a fixed MJPEG camera | 2 |
| `crates/daemon/tests/preview_authorization_tests.rs` | **AM-SR2** setup lines | — |
| `crates/daemon/Cargo.toml`, `Cargo.lock` | dev-dependency `jpeg-encoder` (workspace pin `=0.7.1`, default features) | — |
| `crates/remote/tests/http_tests.rs` (appended `mod hardening_headers`) | pure | 1 |
| `crates/remote/tests/camera_server_tests.rs` (appended `mod stop_before_first_part`) | end-to-end, paused clock | 1 |
| `crates/remote/tests/push_server_tests.rs` (appended `mod renewal`) | end-to-end, paused clock, `FakeTransport` | 6 |
| `crates/push-sender/tests/sandbox_tests.rs` (new) | pure mount-table fixtures + the built binary | 14 |

## How red was established

1. **Static invariants** compile today and fail on their assertions (messages below).
2. **Behaviour tests whose API exists** (`http_tests`, `push_server_tests`, `preview_remote_view_tests`) compile today
   and fail on their assertions.
3. **Behaviour tests needing the new API** (`peer_limits_tests`, `preview_placement_tests`, `preview_downscale_tests`,
   `sandbox_tests`) are compile-red on exactly the specified items, nothing else:
   - `soos_daemon::limits::{AdmissionClass, AUTH_RESERVED_CONNECTIONS_PER_UID, AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS}`,
     `PeerConnectionLimiter::try_acquire_auth_reserve` (12 sites), `PeerConnectionPermit::class` (3 sites);
   - `soos_daemon::session_policy::{classify_peer_placement, PeerPlacement}`,
     `LocalSessionPolicy::authorize_preview_placement` (16 sites);
   - `soos_daemon::preview::{remote_preview_image_for_frame, REMOTE_PREVIEW_MAX_WIDTH, REMOTE_PREVIEW_MAX_HEIGHT}`;
   - `soos_push_sender::{evaluate_sandbox, home_from_exe, probe_sandbox, SandboxError, MAX_MOUNTINFO_BYTES,
     SANDBOX_HIDDEN_LOG_DIR, SANDBOX_MOUNTINFO_PATH, SENDER_EXE_HOME_RELATIVE}`.
4. **Assertion-level red** of the compile-red files was checked under a **temporary signature stub** (wrong values:
   reserve = `try_acquire`, `class()` = `Regular`, placement always `Other`/`Ok`, remote conversion = GUI function,
   sandbox functions = `MountInfoUnreadable`/`Ok`). With the stub, `cargo clippy --locked --all-targets --all-features
   -- -D warnings` on the four packages was clean (the test code type-checks against the specified signatures) and
   every test listed as "red" below failed on an assertion. The stub was removed with `git checkout` of the five
   production files; `git diff --numstat` shows no `src/` change.

Commands (all `--locked --all-features`):

```
cargo test --locked -p soos-invariants --all-features                       # 550 passed, 13 failed (the 12 new + CM-SR1 test 45)
cargo test --locked -p soos-daemon --all-features --test peer_limits_tests --test preview_placement_tests \
  --test preview_downscale_tests                                            # compile-red, missing API only
cargo test --locked -p soos-daemon --all-features --test preview_remote_view_tests   # 14 passed, 1 failed
cargo test --locked -p soos-daemon --all-features --test preview_authorization_tests --test session_policy_tests  # all green (AM-SR2)
cargo test --locked -p soos-remote --all-features --test http_tests        # 21 passed, 1 failed
cargo test --locked -p soos-remote --all-features --test camera_server_tests  # 15 passed (guard, see below)
cargo test --locked -p soos-remote --all-features --test push_server_tests # 17 passed, 5 failed
cargo test --locked -p soos-push-sender --all-features --test sandbox_tests # compile-red, missing API only
```

## Tester Contract

### Static invariants (`tests/invariants/src/remote_security_hardening_contract.rs`)

| Test | Row | Red evidence (failure message today) |
|---|---|---|
| `test_rsh_s1_sender_unit_hides_system_logs` | RSH1 | `packaging/soos-push-sender.service must carry InaccessiblePaths=-/var/log -/var/lib/systemd/coredump exactly once` (left 0, right 1) |
| `test_rsh_s2_tailscale_min_version_is_single_sourced` | RSH3 | `crates/remote/src/lib.rs must define TAILSCALE_MIN_VERIFIED_VERSION = "1.102.4"` |
| `test_rsh_s3_reserve_constants_and_dispatch` | RSH5, RSH6 | `AUTH_RESERVED_CONNECTIONS_PER_UID: pub const AUTH_RESERVED_CONNECTIONS_PER_UID: usize =  not found` |
| `test_rsh_s4_android_lock_screen_privacy_documented` | RSH7 | `Docs/REMOTE_COMPANION.md §2h must contain push_previews (spec §5)` |
| `test_rsh_s5_preview_placement_is_checked` | RSH9 | `handle_preview_request: authorize_preview_placement( not found` |
| `test_rsh_s6_hardening_headers_exact_and_csp_unchanged` | RSH10 | `HARDENING_HEADERS is exactly the spec §7 constant` |
| `test_rsh_s7_sender_checks_its_sandbox_first` | RSH12 | `main: probe_sandbox() not found` |
| `test_rsh_s7_sender_reads_no_other_environment_variable` | RSH12 (PE-SR1) | **guard, green today** (see "Guards") |
| `test_rsh_s8a_installer_refuses_symlinked_config` | RSH13 | `the installer tests [[ -L "${CONFIG_PATH}" ]] first (spec §9.1)` |
| `test_rsh_s8b_stop_rechecked_before_head_and_before_first_part` | RSH14 | `camera_stream: stop_requested( not found` |
| `test_rsh_s8c_worker_names_the_replaced_endpoint` | RSH15 | `crates/remote/assets/sw.js: SUBSCRIBE_BODY_LIMIT = 2048 (MAX_PUSH_SUBSCRIBE_BODY_BYTES)` |
| `test_rsh_s8d_remote_conversion_off_the_auth_path` | RSH16 (PE-SR6, PE-SR8) | `crates/daemon/src/dispatcher.rs calls remote_preview_image_for_frame(` |
| `test_rsh_docs_adr_and_walkthrough` | RSH17 | `AI/DECISIONS.md must contain the ADR [2026-10-12] soos-remote Security Hardening` |

What they pin beyond the spec table (all taken from the spec text): S-1 the line is **directly** after
`TemporaryFileSystem=/run:ro` (§2.1) and a unit comment names `/var/log`; S-3 the exact bounded read
`timeout(Duration::from_millis(AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS), read_client_frame(` (whitespace-insensitive,
§4.3), `read_and_process` holds no `read_exact(` any more, `read_client_frame` keeps `read_exact(` and
`MAX_MESSAGE_SIZE`, `AdmissionClass` exists; S-7 the four §8.2 constants, the exact refusal log line and
`exit_code(EXIT_CONFIG)`, `probe_sandbox` uses `current_exe(`, `.take(`, both path constants and `evaluate_sandbox(`;
S-8a the refusal branch exits non-zero and writes nothing, exactly one redirection into `${CONFIG_PATH}`, `bash -n`;
S-8c `renewalPayload` checks `SUBSCRIBE_BODY_LIMIT`; S-8d the two 640/480 constants and no call of the GUI
`preview_image_for_frame(` inside `remote_preview_image_for_frame` (PE-SR8).

### Behaviour tests

| Test (path::name) | Row | Red evidence |
|---|---|---|
| `peer_limits_tests::auth_reserve::test_rsh_limiter_auth_reserve_admits_one_extra_per_uid` | RSH4 | compile-red (`try_acquire_auth_reserve`, `class`); FAILED on an assertion under the stub |
| `peer_limits_tests::auth_reserve::test_rsh_limiter_auth_reserve_respects_unprivileged_and_global_capacity` | RSH4 | compile-red; FAILED on an assertion under the stub |
| `peer_limits_tests::auth_reserve::test_rsh_limiter_auth_reserve_released_on_drop_and_tracked` | RSH4 | compile-red; FAILED on an assertion under the stub |
| `peer_limits_tests::auth_reserve::test_rsh_limiter_regular_cap_unchanged` | RSH4 | compile-red; FAILED on an assertion under the stub |
| `peer_limits_tests::auth_reserve::test_rsh_permit_reports_admission_class` | RSH4 | compile-red; FAILED on an assertion under the stub |
| `peer_limits_tests::auth_reserve::test_rsh_auth_admitted_on_reserve_while_two_connections_held` | RSH5 | compile-red; FAILED on an assertion under the stub |
| `peer_limits_tests::auth_reserve::test_rsh_reserve_closes_non_auth_without_payload` | RSH5 | compile-red; FAILED on an assertion under the stub |
| `peer_limits_tests::auth_reserve::test_rsh_reserve_idle_connection_closed_within_window` | RSH5 | compile-red; FAILED on an assertion under the stub |
| `peer_limits_tests::auth_reserve::test_rsh_reserve_partial_frame_closed_within_window` | RSH5 (PE-SR3) | compile-red; passes under the stub (refused at admission is also "closed within 500 ms, zero bytes"): it pins the bound once the reserve exists |
| `peer_limits_tests::auth_reserve::test_rsh_reserve_auth_after_100ms_is_answered` | RSH5 (PE-SR2) | compile-red; FAILED on an assertion under the stub |
| `peer_limits_tests::auth_reserve::test_rsh_reserve_auth_is_one_shot` | RSH5 | compile-red; FAILED on an assertion under the stub |
| `preview_placement_tests::test_rsh_classify_peer_placement` | RSH8 | compile-red; FAILED on an assertion under the stub |
| `preview_placement_tests::test_rsh_session_peer_needs_its_own_local_seat_session` | RSH8 | compile-red; FAILED on an assertion under the stub |
| `preview_placement_tests::test_rsh_user_manager_peer_refused_while_remote_session_exists` | RSH8 | compile-red; FAILED on an assertion under the stub |
| `preview_placement_tests::test_rsh_other_placement_keeps_seat_rule` | RSH8 | compile-red; regression guard (the `Other` path keeps today's rule), green under the stub |
| `preview_placement_tests::test_rsh_other_placement_keeps_seat_rule_policy` (extra) | RSH8 | compile-red; guard, green under the stub |
| `preview_placement_tests::test_rsh_placement_rule_disabled_with_validator` | RSH8 | compile-red; guard, green under the stub |
| `preview_placement_tests::test_rsh_placement_rule_disabled_policy` (extra) | RSH8 | compile-red; guard, green under the stub |
| `preview_placement_tests::test_rsh_placement_rechecked_per_request` | RSH8, RSH9 | compile-red; FAILED on an assertion under the stub |
| `preview_placement_tests::test_rsh_session_peer_dispatcher_wiring` (extra) | RSH8, RSH9 | compile-red; FAILED on an assertion under the stub |
| `preview_placement_tests::test_rsh_root_auth_user_manager_rule_unchanged` | RSH8 | compile-red; regression guard of the factored helper, green under the stub |
| `preview_downscale_tests::remote_preview::test_rsh_remote_preview_decodes_mjpeg_640x480` | RSH16 | compile-red; FAILED on an assertion under the stub |
| `preview_downscale_tests::remote_preview::test_rsh_remote_preview_bounds_tall_grey_ir_frame` | RSH16 | compile-red; FAILED on an assertion under the stub |
| `preview_downscale_tests::remote_preview::test_rsh_remote_preview_bounds_wide_yuyv` | RSH16 | compile-red; FAILED on an assertion under the stub |
| `preview_downscale_tests::remote_preview::test_rsh_remote_preview_converts_nv12` | RSH16 | compile-red; FAILED on an assertion under the stub |
| `preview_downscale_tests::remote_preview::test_rsh_remote_preview_odd_width_yuyv_becomes_rgb` | RSH16 | compile-red; FAILED on an assertion under the stub |
| `preview_downscale_tests::remote_preview::test_rsh_remote_preview_malformed_mjpeg_is_empty` | RSH16 | compile-red; FAILED on an assertion under the stub |
| `preview_downscale_tests::remote_preview::test_rsh_remote_preview_ir_stamp_is_grey` | RSH16 (PE-SR8) | compile-red; FAILED on an assertion under the stub |
| `preview_downscale_tests::remote_preview::test_rsh_remote_preview_output_always_within_640x480` (proptest: format, size, `sensor_type`, truncation) | RSH16 (PE-SR8) | compile-red; FAILED on an assertion under the stub |
| `preview_downscale_tests::remote_preview::test_rsh_remote_preview_bounds_are_640x480` (extra) | RSH16 | compile-red (constants; green under a stub carrying the spec values) |
| `preview_downscale_tests::remote_preview::test_rsh_local_preview_unchanged_for_mjpeg` | RSH16 | compile-red; GUI-path guard, green under the stub |
| `preview_remote_view_tests::remote_conversion::test_rsh_remote_origin_receives_converted_mjpeg` | RSH16 | `MJPEG is decoded to RGB24 for soos-remote` (left 4, right 0) |
| `preview_remote_view_tests::remote_conversion::test_rsh_local_origin_receives_mjpeg_unchanged` | RSH16 | guard, green today |
| `http_tests::hardening_headers::test_rsh_hardening_headers_on_every_head` | RSH10 | `response: strict-transport-security exactly once with the exact value` (left [], right ["max-age=31536000"]) |
| `camera_server_tests::stop_before_first_part::test_rsh_stop_after_first_frame_sends_no_part` | RSH14 | **guard, green today** (see "Guards") |
| `push_server_tests::renewal::test_rsh_subscribe_with_replaces_endpoint_renews_in_place` | RSH15 | `expected 200 subscribed` (left (400, "bad_request")) |
| `push_server_tests::renewal::test_rsh_replaces_endpoint_absent_from_store_is_plain_upsert` | RSH15 | `expected 409 too_many_subscriptions` (left (400, "bad_request")) |
| `push_server_tests::renewal::test_rsh_replaces_endpoint_equal_is_plain_upsert` | RSH15 | `expected 200 subscribed` (left (400, "bad_request")) |
| `push_server_tests::renewal::test_rsh_replaces_endpoint_invalid_is_plain_upsert` | RSH15 (PE-SR4) | `expected 200 subscribed` (left (400, "bad_request")) |
| `push_server_tests::renewal::test_rsh_subscribe_without_replaces_endpoint_unchanged` | RSH15 | guard, green today |
| `push_server_tests::renewal::test_rsh_renewal_audit_lines` | RSH15 | `expected 200 subscribed` (left (400, "bad_request")) |
| `sandbox_tests::test_rsh_sandbox_constants` (extra) | RSH11 | compile-red; FAILED on an assertion under the stub |
| `sandbox_tests::test_rsh_sandbox_accepts_the_unit_mount_table` | RSH11 (PE-SR9 fixture with the binary bind child) | compile-red; FAILED on an assertion under the stub |
| `sandbox_tests::test_rsh_sandbox_refuses_host_mount_table` | RSH11 | compile-red; FAILED on an assertion under the stub |
| `sandbox_tests::test_rsh_sandbox_requires_ro_tmpfs_run` | RSH11 | compile-red; FAILED on an assertion under the stub |
| `sandbox_tests::test_rsh_sandbox_requires_tmpfs_over_home` | RSH11 | compile-red; FAILED on an assertion under the stub |
| `sandbox_tests::test_rsh_sandbox_refuses_listable_logs` | RSH11 | compile-red; FAILED on an assertion under the stub |
| `sandbox_tests::test_rsh_sandbox_unescapes_mount_points` | RSH11 | compile-red; FAILED on an assertion under the stub |
| `sandbox_tests::test_rsh_sandbox_option_match_is_exact` | RSH11 | compile-red; FAILED on an assertion under the stub |
| `sandbox_tests::test_rsh_sandbox_home_anchor_unresolved_refused` | RSH11 (PE-SR1) | compile-red; FAILED on an assertion under the stub |
| `sandbox_tests::test_rsh_sandbox_home_outside_home_dir` | RSH11 (PE-SR1) | compile-red; FAILED on an assertion under the stub |
| `sandbox_tests::test_rsh_sandbox_refuses_mount_over_hidden_home` | RSH11 (PE-SR9) | compile-red; FAILED on an assertion under the stub |
| `sandbox_tests::test_rsh_sandbox_binary_bind_is_the_only_allowed_child` | RSH11 (PE-SR9) | compile-red; FAILED on an assertion under the stub |
| `sandbox_tests::test_rsh_probe_sandbox_refuses_an_unsandboxed_process` (extra) | RSH12 | compile-red; stub (`Ok`): `assertion failed: probe_sandbox().is_err()` |
| `sandbox_tests::test_rsh_sender_binary_exits_78_outside_its_sandbox` (extra) | RSH12 | compile-red; FAILED on an assertion under the stub |

### Guards (green before the implementation, kept on purpose)

- `test_rsh_s7_sender_reads_no_other_environment_variable` (PE-SR1): today's sender already reads only
  `XDG_RUNTIME_DIR`; the test is the stronger companion of RMC-S34 that the implementation must keep green (it fails
  on any `$HOME`, `home_dir`, passwd lookup or second `env::var*`).
- `test_rsh_stop_after_first_frame_sends_no_part` (RSH14): a stop processed while the scripted source produces the
  first frame (20 offsets inside a 400 ms first-frame window, 10/10 runs green) already ends the view through the
  first-frame loop's stop arm, because `soos-remote` runs on a current-thread runtime and no await separates
  `FirstFrame::Ready` from the head write; there is no deterministic seam for the window S-8b closes (spec §11.2 says
  the same for the second check). The red evidence for S-8b is the static
  `test_rsh_s8b_stop_rechecked_before_head_and_before_first_part`; the behaviour test keeps the end-to-end contract
  (no `200` head, no JPEG part, `refused` audit, never `started`, slot idle).
- `test_rsh_local_origin_receives_mjpeg_unchanged`, `test_rsh_local_preview_unchanged_for_mjpeg`,
  `test_rsh_subscribe_without_replaces_endpoint_unchanged`, `test_rsh_other_placement_keeps_seat_rule*`,
  `test_rsh_placement_rule_disabled_*`, `test_rsh_root_auth_user_manager_rule_unchanged`: "unchanged" contracts
  required by the spec (GUI path, plain subscribe, `Other` placement, disabled validator, root `Auth`).

## Migrated existing tests (amendments, exhaustive)

| Id | Test | Old | New | Mandate |
|---|---|---|---|---|
| **CM-SR1** (owner-approved) | `tests/invariants/src/remote_push_contract.rs::test_rmc_s35_sender_unit_is_sandboxed` | `SENDER_UNIT_LINES: [&str; 37]` | `[&str; 38]` with exactly `"InaccessiblePaths=-/var/log -/var/lib/systemd/coredump"` inserted after `"TemporaryFileSystem=/run:ro"`; nothing removed or reordered; forbidden list and the `soos-remote.service` assertion unchanged | spec §2.2, §11.3; red today (`exact sandbox of spec §8.2`, left 37 lines, right 38) |
| **AM-SR1** | `crates/daemon/tests/preview_remote_view_tests.rs::test_rlc_local_peer_unaffected_by_remote_view` | first scenario writes session record `"3"` only | additionally writes `write_session(&sessions, "4", &seat_session(uid))` (the peer cgroup fixture is `session-4.scope`); setup only, every assertion untouched | spec §11.3 (S-5: the peer's own session must be a local seat session) |
| **AM-SR2** | `crates/daemon/tests/preview_authorization_tests.rs::test_preview_frame_allowed_for_configured_uid_serves_frames` | record `"3"` only | additionally, when `/proc/self/cgroup` names a session scope (`soos_daemon::session_policy::parse_session_id_from_cgroup`), the same local-seat record is written under that id; setup only, every assertion untouched | spec §11.3 (the test runs from a developer TTY session scope) |

`git diff --numstat` of the commit: every existing test file has 0 deleted lines except `remote_push_contract.rs`
(1 line: the array length `37` → `38`). No other existing test changed. The spec's "checked compatible" list was
re-run: `preview_remote_view_tests` (all 13 existing green), `preview_authorization_tests` (17/17),
`session_policy_tests` (47/47), every other invariant (550 green) including RMC-S6, RMC-S34, RMC-S41, RMC-S42,
RLC-S1 (`cargo tree -e normal` ignores the new dev-dependency), RLC-S6 and RAN-S2.

## Notes for the auditor and the developer

1. **Dispatcher permit check**: the dispatcher exposes `available_permits()` only (no `tracked_uids()`); the socket
   tests assert that `available_permits()` returns to the pre-connection value, and `tracked_uids()` (union of regular
   and reserved maps) is pinned by `test_rsh_limiter_auth_reserve_released_on_drop_and_tracked`. No new dispatcher
   accessor is required.
2. **`PushStore::replace_or_upsert` / `RenewResult`** (spec §9.3) are exercised through `/api/push/subscribe` only, so
   `push_server_tests` stays compilable and shows assertion-level red; the developer still implements the specified
   API.
3. **Odd-width YUYV** (`test_rsh_remote_preview_odd_width_yuyv_becomes_rgb`): the 321x240 fixture has exactly
   `321 * 240 * 2` bytes and expects a 321x240 RGB24 image; the last pixel of the last row has no complete
   macropixel in the buffer, so the remote path must not require bytes past the buffer for it. The GUI path
   (`preview_image_for_frame`) must stay bit-for-bit identical (spec §9.4).
4. **IR proptest**: an IR-stamped frame that is not empty must come out Grey (format 1), whatever its pixel format
   or size (PE-SR8).
5. **Reserve window tests** use the existing 500 ms `CLOSE_DEADLINE`; a compile-time assertion keeps
   `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS + 100 < 500`.
6. **Sender binary test** spawns `CARGO_BIN_EXE_soos-push-sender` with a cleared environment and a `TempDir` as
   `XDG_RUNTIME_DIR`, never the real runtime directory, and kills it after 5 s; it requires the exit code 78, the
   constant stderr line, and no `soos-push` directory created (the probe runs before any bind).

## Flakiness check

- `camera_server_tests::stop_before_first_part::test_rsh_stop_after_first_frame_sends_no_part`: 10/10 green in a row.
- The `auth_reserve` socket tests are compile-red until the API exists; the developer runs them 10 times in a row
  (`for i in $(seq 10); do cargo test --locked -p soos-daemon --all-features --test peer_limits_tests rsh -q || break;
  done`) before hand-off.

## Auditor additions (Phase 3, new tests only)

Recorded in `AI/auditor_constraints_remote_security_hardening.md` (A-1, A-2). Both are new test functions; no
existing test was changed.

| Test (path::name) | Row | Red evidence |
|---|---|---|
| `peer_limits_tests::auth_reserve::test_rsh_reserve_stalled_frame_holds_the_reserve_for_the_window` (A-1) | RSH5 | compile-red (missing API only); under the wrong-value stub: `permits must return to 5, got 8` |
| `remote_security_hardening_contract::test_rsh_s3_reserve_refuses_events_before_processing` (A-2) | RSH5 | `read_and_process after decode_client_message: AdmissionClass::AuthReserve not found` |
