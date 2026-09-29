# Walkthrough 81 — Preview Frame Authorization (CAM-01 / DMN-02)

- **Date**: 2026-09-30 (work started 2026-09-29, resumed after a quota interruption)
- **Issue**: Review finding CAM-01 / DMN-02 (GitHub #143) — **Branch**: `fix/preview-frame-authorization`
- **Matrix criteria**: PFA1–PFA6 (new component `preview-frame-authorization`); touches PRX1, PRX2, CLP3, CLP4

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, base `fbb99c4`) rated
CAM-01 / DMN-02 **CRITICAL** and the verifier confirmed it. Re-verified in code before any change:

- `crates/daemon/src/dispatcher.rs:347-413`: the `RequestKind::PreviewFrame` branch ran right after
  `req.validate()`, **before** Step 6 `verify_peer_credentials` and Step 6b (session validation,
  which was in any case restricted to `Auth`). It called `pipe.camera.notify_activity()`
  unconditionally, cloned the full `frame.data` into a `PreviewResponse`, and returned.
- No rate limit applied (`check_allowed` only lives in the `Auth` path), and `daemon.toml` had no
  gating key. The socket is `0660 root:soos`, so every `soos` group member (including a remote SSH
  session) could stream the webcam at ~30 fps and keep the privacy LED on indefinitely.
- `crates/gui/src/ipc_camera.rs:118-125`: the GUI sent a constant nonce (`[0x5A; 32]`) and treated
  every non-`PreviewResponse` reply as a transport error, reconnecting every 100 ms.

Objectives (task scope: daemon, also resolves DMN-02): authorize `PreviewFrame` (root peer or an
explicit, default-off opt-in), require an active session, rate limit it, never serve frame bytes to
other peers, zeroize preview buffers, and adapt the GUI without breaking its existing tests.

## 2. Architect Design

- New module `crates/daemon/src/preview.rs`:
  - `PreviewConfig { enabled: bool, allowed_uids: Vec<u32>, max_requests_per_sec: u32 }`,
    `Default` = `{ false, [], 40 }`; `validate()` rejects more than `MAX_PREVIEW_ALLOWED_UIDS` (64)
    entries; `rate_limit_config()` builds a `soos_policy::RateLimitConfig` (window
    `PREVIEW_RATE_WINDOW_NS` = 1 s, `PREVIEW_RATE_MAX_TRACKED_UIDS` = 64).
  - `PreviewDenied { UidMismatch, Disabled, NotAllowed }` (`thiserror`).
  - `authorize_preview(cfg, peer_uid, requested_uid)`: root always allowed; otherwise
    `peer_uid == requested_uid` **and** `enabled` **and** allow-listed.
- `DaemonConfig.preview` + `[preview]` TOML section (`enabled`, `allowed_uids`, `max_requests_per_sec`);
  `from_toml_str` validates the bound. Sentinels: `max_requests_per_sec = 0` refuses every request
  (existing `RateLimiter` semantics); missing section keeps the fail-closed default.
- `ConnectionDispatcher::with_preview_config(PreviewConfig)` builder (a dispatcher built without it
  keeps the fail-closed default) and `preview_config()` accessor; `main.rs` wires `config.preview`.
- Dispatcher: preview branch removed from Step 5c and re-created as Step 6c
  (`handle_preview_request`) **after** Step 6 peer verification: authorization → session check for
  unprivileged peers (same `SessionValidator` as `Auth`) → per-peer-UID `RateLimiter::check_and_record`
  → only then `notify_activity()`, readiness wait and frame copy. Refusals are standard `Response`
  messages (`ProtocolError` / `UidMismatch` or `RateLimited`; `Unavailable` / `InternalError` on
  clock failure) with zero pixel bytes.
- Memory hygiene: `PreviewResponse` implements `Zeroize` + `Drop`; `ResponseOutput` /
  `ProcessedOutput` buffers are `Zeroizing<Vec<u8>>`.
- GUI: `IpcPreviewError { Unauthorized, RateLimited, Unavailable, Protocol, Io }`, per-request
  random nonce (`getrandom`, already a workspace dependency), nonce-bound refusal detection,
  `IpcCameraManager::last_error()`, `IpcCameraManager::probe_preview()`, worker stops on
  `Unauthorized`; `main.rs` probes once and falls back to direct V4L2 access with a clear warning.
- ADR recorded in `AI/DECISIONS.md` (2026-09-29) and `AI/ARCHITECTURE.md` §12 pitfall reworded.

## 3. Plan Evaluation

Condensed for a review-issue branch (no plan-evaluator report). Checked against
`AI/ARCHITECTURE.md` invariant 3 (kernel-verified UID, active logind session), the fail-closed
rule, the bounded-allocation rule and `Docs/IPC_PROTOCOL.md`: the design reuses the existing
`verify_peer_credentials`, `SessionValidator` and `soos_policy::RateLimiter` rather than adding a
second authorization mechanism, and keeps the GUI's persistent-connection contract (CLP3).

## 4. Tester Contract

| Test (path::name) | Criterion | Red evidence |
|---|---|---|
| `preview_authorization_tests::test_preview_config_defaults_deny_unprivileged_peers` | PFA1 | `unresolved import soos_daemon::preview` |
| `preview_authorization_tests::test_authorize_preview_root_peer_always_allowed` | PFA1 | same (specified API missing) |
| `preview_authorization_tests::test_authorize_preview_default_denies_unprivileged_peer` | PFA1 | same |
| `preview_authorization_tests::test_authorize_preview_listed_but_disabled_is_denied` | PFA1 | same |
| `preview_authorization_tests::test_authorize_preview_enabled_but_not_listed_is_denied` | PFA2 | same |
| `preview_authorization_tests::test_authorize_preview_uid_mismatch_is_denied_even_when_listed` | PFA2 | same |
| `preview_authorization_tests::test_preview_config_rejects_oversized_allow_list` | PFA1 | same |
| `preview_authorization_tests::test_daemon_config_default_preview_is_disabled` | PFA1 | `no field preview on type DaemonConfig` |
| `preview_authorization_tests::test_daemon_config_parses_preview_section` | PFA1 | same |
| `preview_authorization_tests::test_daemon_config_rejects_oversized_preview_allow_list` | PFA1 | same |
| `preview_authorization_tests::test_preview_frame_rejected_when_disabled` | PFA2 | after API scaffolding: panic at the `assert_denied` size bound (`Denial reply must be bounded by MAX_MESSAGE_SIZE`, a full 320x240 RGB frame was served) |
| `preview_authorization_tests::test_preview_frame_uid_mismatch_rejected` | PFA2 | same assertion (frame served to foreign `uid_hint`) |
| `preview_authorization_tests::test_preview_frame_rejected_when_enabled_but_uid_not_listed` | PFA2 | `Denied preview must decode as a standard Response` |
| `preview_authorization_tests::test_preview_frame_rejected_without_active_session` | PFA3 | same |
| `preview_authorization_tests::test_preview_frame_allowed_for_configured_uid_serves_frames` | PFA3 | `no method with_preview_config` |
| `preview_authorization_tests::test_preview_frame_rate_limited_per_peer_uid` | PFA4 | `Denied preview must decode as a standard Response` (third request served) |
| `preview_authorization_tests::test_preview_frame_denied_peer_does_not_wake_camera` | PFA5 | same (spy camera received `notify_activity`) |
| `preview_tests::test_preview_response_zeroize_erases_pixel_data` | PFA5 | `the method zeroize exists for struct PreviewResponse, but its trait bounds were not satisfied` |
| `ipc_camera_tests::test_ipc_camera_manager_stops_on_unauthorized_response` | PFA6 | `unresolved import soos_gui::IpcPreviewError`, `no method last_error` |
| `ipc_camera_tests::test_ipc_camera_manager_keeps_polling_after_rate_limit` | PFA6 | same |
| `ipc_camera_tests::test_probe_preview_reports_unauthorized` | PFA6 | `no associated function probe_preview` |
| `ipc_camera_tests::test_probe_preview_accepts_authorized_reply` | PFA6 | same |
| `ipc_camera_tests::test_probe_preview_reports_io_error_when_socket_absent` | PFA6 | same |
| `ipc_camera_tests::test_ipc_preview_error_display_is_english_and_specific` | PFA6 | `unresolved import soos_gui::IpcPreviewError` |

Red phase was recorded twice: first as compile errors on exactly the specified API, then, after
scaffolding the API but before moving the dispatcher branch, as assertion failures
(`11 passed; 6 failed` in `preview_authorization_tests`), proving the tests detect the defect itself.

Red re-proof on resumption (2026-09-30): the defect was temporarily re-introduced on top of the
finished implementation (`authorize_preview` forced to `Ok(())`, session check and rate-limit check
short-circuited in `handle_preview_request`), then
`cargo test --locked -p soos-daemon --all-features --test preview_authorization_tests` gave
`8 passed; 9 failed` (the four pure `authorize_preview` denial tests, `rejected_when_disabled`,
`rejected_when_enabled_but_uid_not_listed`, `rejected_without_active_session`,
`rate_limited_per_peer_uid` and `denied_peer_does_not_wake_camera`). Only
`test_preview_frame_uid_mismatch_rejected` stayed green because Step 6 `verify_peer_credentials`
now runs before the preview branch. Production files were restored byte-for-byte afterwards.

### Migrated existing tests (Contract Migration, mandated by GitHub #143)

| Test | Old assumption | New assumption |
|---|---|---|
| `dispatcher_tests::test_dispatcher_preview_frame_roundtrip` | any peer receives a `PreviewResponse` | dispatcher built with `with_preview_config(preview_allow(current_uid))`; assertions unchanged |
| `dispatcher_tests::test_dispatcher_persistent_stream_multiple_requests` | same | same; assertions unchanged |

One tester-owned assertion was corrected during the Red/Green loop: the initial "zero frame bytes"
check tried to decode the refusal as a `PreviewResponse` and expected an error, but postcard happily
reinterprets a `Response` as a tiny garbage `PreviewResponse`. The contract now asserts that the
refusal is a `Response` bound to the nonce and at most 59 bytes long (`MAX_DENIAL_REPLY_LEN`), which
leaves no room for pixel data.

### Flakiness check

`preview_authorization_tests` (17 tests) and `ipc_camera_tests` (6 tests) run 10 times in a row:
10/10 green each.

## 5. Auditor Constraints

| # | Constraint | How it was met |
|---|---|---|
| 1 | Preview served only after peer verification, authorization, session and rate-limit checks; no `notify_activity()` before | Step 6c `handle_preview_request` placed after Step 6/6b; `test_preview_frame_denied_peer_does_not_wake_camera` (spy camera) |
| 2 | Refusals carry zero frame bytes and stay within `MAX_MESSAGE_SIZE` | Refusals use `build_response` (`Response`); `assert_denied` bounds the reply to 59 bytes |
| 3 | No `unwrap`/`expect`/`panic`/indexing in production code | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` green; new code uses `?`, `get_mut`, `map_err` |
| 4 | Log statements free of sensitive keywords | `logging_audit_test::test_daemon_source_code_has_zero_sensitive_data_in_logs` green ("Generated preview response", "Preview request refused ...") |
| 5 | Bounded allow-list and limiter, saturating arithmetic | `MAX_PREVIEW_ALLOWED_UIDS` = 64 validated at parse time; limiter capacity 64; `saturating_add` in GUI buffer sizing |
| 6 | Preview buffers zeroized | `PreviewResponse: Zeroize + Drop`; `Zeroizing<Vec<u8>>` for every encoded daemon response |
| 7 | GUI stops on `Unauthorized`, binds replies to a random nonce | `poll_once` returns `PollStep::Stop`; `fresh_request_id` via `getrandom::fill`; `Response::matches_request` |
| 8 | Default configuration fail-closed, root always allowed | `PreviewConfig::default()`, `authorize_preview` tests |
| 9 | No new dependency version | `getrandom 0.3.4` already in `Cargo.lock`; only the `soos-gui` edge was added |

Pre-existing issues found (not introduced here): `test_daemon_startup_initializes_all_pipeline_components`
(daemon) and `test_gui_worker_fallback_renders_raw_rgb_on_pipeline_error` (GUI) are timing-sensitive
and occasionally fail under full-workspace parallel load; the first one also failed 1/6 runs on
`origin/main` in a scratch worktree. Both pass 6/6 in isolation. Matrix row PRX1 cites a test name
(`dispatcher_tests::test_dispatcher_handles_preview_frame_request`) that does not exist.

Clearance: CLEARED.

## 6. Implementation

Files changed:

- `crates/daemon/src/preview.rs` (new), `crates/daemon/src/lib.rs`, `crates/daemon/src/config.rs`,
  `crates/daemon/src/dispatcher.rs`, `crates/daemon/src/main.rs`
- `crates/protocol/src/types.rs` (`PreviewResponse` zeroization)
- `crates/gui/src/ipc_camera.rs` (rewritten around `request_preview` / `poll_once`),
  `crates/gui/src/lib.rs`, `crates/gui/src/main.rs`, `crates/gui/Cargo.toml`, `Cargo.lock`
- Tests: `crates/daemon/tests/preview_authorization_tests.rs` (new),
  `crates/daemon/tests/dispatcher_tests.rs` (migration), `crates/gui/tests/ipc_camera_tests.rs` (new),
  `crates/protocol/tests/preview_tests.rs`
- Docs: `AI/DECISIONS.md`, `AI/ARCHITECTURE.md`, `Docs/IPC_PROTOCOL.md` (§3, §4, §8, new §9),
  `AI/VERIFICATION_MATRIX.md`, this walkthrough

Notable decisions:

- The rate limiter applies to root as well: a root process bug must not spin the daemon; the quota
  (40/s) leaves headroom above the GUI's ~30 fps poll.
- Session validation reuses the dispatcher's `SessionValidator`, so `enforce_active_session = false`
  in tests keeps the previous behaviour while production requires a seated session for the GUI user.
- The GUI keeps `probe()` (socket liveness, PRX2) and adds `probe_preview()` (authorization); on
  refusal it logs the `[preview]` opt-in hint and falls back to direct V4L2 access.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`): PASSED on the final working tree (unsafe, PAM panic,
Tokio-in-PAM, OpenCV/nokhwa, shell syntax, PAM output isolation and English-only audits).
Layer 2 (`./scripts/candid_subagent.sh --prepare` + fingerprint-bound report): deliberately not
run in this session; it is executed by the release step (`save.sh` / `pr_loop.sh`) before push.

## 8. Verification Results

```bash
cargo fmt --all -- --check                                                        # ok
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings      # ok
cargo test --locked --workspace --all-targets --all-features --no-fail-fast       # 2026-09-30 final run: 105 test binaries, 503 passed, 0 failed
cargo test --locked -p soos-daemon --all-features --test preview_authorization_tests  # 17 passed (10/10 runs)
cargo test --locked -p soos-gui --all-features --test ipc_camera_tests             # 6 passed (10/10 runs)
cargo test --locked -p soos-protocol --all-features --test preview_tests           # 3 passed
./scripts/candid_review.sh                                                        # ok
```

## 9. Known Limitations / Follow-ups

- The installer does not ship a `daemon.toml`; administrators enabling the GUI preview must add the
  `[preview]` section by hand (documented in `Docs/IPC_PROTOCOL.md` §9). A packaging follow-up could
  add `allowed_uids` for the enrolling user.
- The review recommended "one preview client at a time"; the per-UID rate limit bounds the load but
  does not serialize clients. Left for a follow-up if needed.
- Two pre-existing timing-sensitive tests (§5) deserve a deterministic fixture in a separate change.
