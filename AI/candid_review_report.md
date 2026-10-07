# Candid Review Report

- **Date**: 2026-10-07
- **Target Branch**: `feat/remote-live-camera` (GitHub #345), round 5
- **Base (merge-base)**: `05001c9`
- **Reviewed-Diff-Fingerprint**: `3b685280322d4c459bd163b9c7bfb86be280d23f79cb017062ff0f846a0c7857`
- **Audited Files**: .agents/skills/dev-workflow/references/project-facts.md, .github/workflows/ci.yml, AI/ARCHITECTURE.md, AI/DECISIONS.md, AI/VERIFICATION_MATRIX.md, AI/architect_spec_remote_live_camera.md, AI/auditor_constraints_remote_live_camera.md, AI/research_live_camera.md, AI/tester_contract_remote_live_camera.md, AI/walkthroughs/191_remote_live_camera.md, Cargo.lock, Cargo.toml, Docs/DAEMON.md, Docs/GUI_APPLICATION.md, Docs/IPC_PROTOCOL.md, Docs/REMOTE_COMPANION.md, Docs/SECURITY_AND_QUALITY_GUIDELINES.md, crates/daemon/src/config.rs, crates/daemon/src/dispatcher.rs, crates/daemon/src/lib.rs, crates/daemon/src/main.rs, crates/daemon/src/preview.rs, crates/daemon/src/preview_image.rs, crates/daemon/src/preview_peer.rs, crates/daemon/src/session.rs, crates/daemon/src/session_policy.rs, crates/daemon/tests/preview_authorization_tests.rs, crates/daemon/tests/preview_remote_view_tests.rs, crates/gui/src/ipc_camera.rs, crates/protocol/src/types.rs, crates/remote/Cargo.toml, crates/remote/assets/app.js, crates/remote/assets/style.css, crates/remote/assets/sw.js, crates/remote/src/audit.rs, crates/remote/src/camera.rs, crates/remote/src/camera_ipc.rs, crates/remote/src/camera_jpeg.rs, crates/remote/src/camera_slot.rs, crates/remote/src/challenge.rs, crates/remote/src/config.rs, crates/remote/src/http.rs, crates/remote/src/lib.rs, crates/remote/src/main.rs, crates/remote/src/push.rs, crates/remote/src/routes.rs, crates/remote/src/server.rs, crates/remote/tests/alerts_server_tests.rs, crates/remote/tests/camera_challenge_tests.rs, crates/remote/tests/camera_config_tests.rs, crates/remote/tests/camera_ipc_tests.rs, crates/remote/tests/camera_jpeg_tests.rs, crates/remote/tests/camera_push_tests.rs, crates/remote/tests/camera_routes_tests.rs, crates/remote/tests/camera_server_tests.rs, crates/remote/tests/camera_slot_tests.rs, crates/remote/tests/common/camera.rs, crates/remote/tests/common/harness.rs, crates/remote/tests/push_server_tests.rs, crates/remote/tests/server_tests.rs, deny.toml, scripts/install_remote.sh, tests/invariants/src/lib.rs, tests/invariants/src/remote_camera_contract.rs, tests/invariants/src/remote_companion_contract.rs

## 1. Executive Summary

### Round 5 (re-review after a CI fix)

CI run 37639513976 failed one test on the round-4 commit: `test_rlc_s1_remote_camera_dependencies` panicked because
`cargo tree --offline --locked --workspace --target all` could not resolve `android-activity v0.6.1` (a crate of a
non-host target that the host-only build never downloads; the test's own message says to run `cargo fetch`). The fix
is CI environment setup, not test or production code: `.github/workflows/ci.yml` gains a `cargo fetch --locked` step
(all targets, lockfile-pinned) before "Build test binaries" in the test job. The test itself is unchanged and keeps
`--target all`, so target-specific `jpeg-encoder` feature activation is still checked.

Pillar 6 on the new hunk: no new action, no `${{ github.event.* }}` interpolation, `--locked` keeps the lockfile
authoritative, `permissions:` unchanged. The new frozen patch differs from round 4 only by these 6 added lines.
Re-run: `cargo test --locked -p soos-invariants` 530 passed, 0 failed; `./scripts/candid_review.sh` PASSED. No
finding. Verdict: APPROVED.

### Round 4 (re-review after traceability)

Phase 6 traceability changed the diff after round 3 (fingerprint `43826ff3…`), so the review was re-run on the new
frozen patch (`./scripts/candid_subagent.sh --prepare`, fingerprint `714ade40…`). Only two files differ from the
round-3 target, and both are documentation:
- `AI/VERIFICATION_MATRIX.md`: new component section `remote-live-camera` with rows RLC1–RLC16. Every one of the 72
  cited test names (the 71 `test_rlc_*` tests and `test_rmc_s4_remote_is_a_leaf_crate`) was checked to exist as a
  `fn` in the tree, and all 71 `test_rlc_*` tests in the tree are cited. RLC16 (owner hardware check) is honestly
  marked pending, not claimed.
- `AI/walkthroughs/191_remote_live_camera.md` (new, next number after 190): English only; it describes the M11
  owner-approved amendment accurately (stricter detector, installer unchanged) and lists the RLC16 steps that match
  `Docs/REMOTE_COMPANION.md` §2f.

No code, test, script or dependency changed between rounds 3 and 4; the mechanical test-change listing of §2 gives
the same hits on the new patch. Commands re-run on the round-4 tree: `cargo fmt --all -- --check` exit 0;
`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` exit 0;
`./scripts/candid_review.sh` PASSED (7 audits); `cargo test --locked --all-features -p soos-invariants -p soos-remote
-p soos-daemon -p soos-protocol --no-fail-fast`: 1635 passed, 0 failed (the matrix-evidence invariants included).
The round-3 analysis below still applies unchanged. Verdict: APPROVED.

### Round 3

Round 3 review of the live camera view in `soos-remote`: the daemon preview channel, the single view slot, the
single-use stream token, the multipart JPEG stream, the daemon `remote_view` cgroup gate and the local seat predicate.

The only open round-2 finding was MAJOR: invariant test 58 (`test_rlc_s9_installer_template`) was red. It is closed
by migration M11, which is recorded in tester contract §5 as an owner-approved assertion change. The amended detector
is **stricter** than the original:
- `echo` lines are no longer exempt from redirection or `tee` checks;
- any non-`echo` line that names `/etc` is still rejected;
- only the exact read-only `INSTALLER_RESOLVER_PROBE` line is exempt;
- a 16-sample self-check keeps the scan from passing vacuously.

The installer script is unchanged (no path obfuscation).

The production code was re-read in full for this round. It shows no broken RLC-S1..S13 invariant, no fail-open
path, no panic path, no unbounded I/O and no pixel or token logging. No CRITICAL or MAJOR finding remains.
Verdict: APPROVED.

Commands run by the reviewer, against the frozen tree:
- `cargo fmt --all -- --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test -p soos-invariants -p soos-remote -p soos-daemon --no-fail-fast`: 115 test binaries, 1541 passed,
  0 failed. soos-invariants alone: 530 passed, 0 failed, test 58 included.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- **Removed or changed checks** (`^-.*assert|#[test]...`): one hit, patch line 2720. It is a row of the Markdown table
  in `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` that contains the word "assertions"; it is not a test.
- **New escape hatches**:
  - The `tolerance` hits are the documented B6 `CLOSE_WAIT_TOLERANCE_MS` (10 ms) in the new `camera_ipc_tests.rs`.
    Strict client-side bounds compensate for it. Accepted.
  - No `#[ignore]`, `cfg(any())` or `should_panic`.
- **Inline `mod tests`**: no hit.
- **Crate-level `#![allow(..)]` headers**: only in new test files, plus M8-M10, which are lint-only and recorded in
  the contract. Legitimate.
- **Existing test files**, verified with `git diff origin/main` on each one:
  - M1: `test_rmc_s4_remote_is_a_leaf_crate`. Owner-approved assertion migration LC-1. It drops `soos-protocol` from
    the forbidden list and adds two assertions: the exact workspace dependency line, and every `soos-*` key is in
    {`soos-protocol`, `soos-push-protocol`}. Stricter than before. Legitimate.
  - M2-M7: setup-only. They add a `camera:` field, `remote_view: false`, and the seat fixture keys
    `REMOTE=0/SEAT/CLASS=user`. No assertion changes.
  - M11: test 58, described in §1. Owner-approved amendment named in the review mandate and recorded in tester
    contract §5. Stricter than before. Legitimate.
  - `tests/invariants/src/lib.rs`: registers the new module only.
- No other existing test was modified, and no assertion was weakened.

## 3. Deep Reasoning Audit

### Logic & Architecture
Scenarios tried:
- **Slot state machine.**
  - A wrong token or a wrong owner gives `TokenRejected` and keeps the reservation. The token is 256-bit and compared
    in constant time, so brute force is not feasible.
  - An expired Pending slot returns to Idle without cooldown. `end` with a stale view id does nothing.
  - `stop` is durable: a flag on Starting and Streaming, and Pending is cleared.
  - Running out of view ids gives `Busy`.
  - `stop_requested` reads true for any other view or for Idle and Pending.
  - PASS.
- **`camera_start` ordering.** CSRF with Origin required, then enabled, Funnel flag, `rp_id`, body present, host,
  lockout and slot pre-check. No body byte is read before these. Then the bounded body, the `CameraView` assertion
  with UV (failures counted on the shared unlock key), the persisted counter, the CSPRNG token and the reservation.
  An unlock challenge replayed as `CameraView` is rejected by the purpose-scoped pools. PASS.
- **`camera_stream`.**
  - The Funnel needs a web session before routing, because `CameraStream` is not Funnel-public. The owner is bound
    to class and session hash.
  - `ViewGuard` is created right after `begin`. A pre-head failure is a JSON error with zero pixel bytes.
  - `set_shown()` runs before the first part write.
  - A query string in the path fails the token parse and returns 404.
  - PASS.
- **Server shutdown.** `closing` is sent, then every connection task is aborted. `ViewGuard::drop` frees the slot on
  abort. PASS.
- **Wire format codes.** The new `soos-protocol` constants (0-4, 255) match the daemon's `wire_format_code`, and the
  GUI uses the shared `PREVIEW_FORMAT_EMPTY`. PASS.

### PAM Concurrency & Deadlines
- `crates/pam` is untouched.
- On the daemon side, the cgroup read is `read_bounded` (16 KiB, reaching the bound is an error). It is synchronous
  and runs inside the per-request `connection_timeout`. PASS.

### Panic Safety & Fail-Closed
- **Daemon `handle_preview_request`.** Scenarios tried, each giving `ProtocolError/UidMismatch` with zero pixel bytes:
  - a missing or non-positive PID (`MissingPid`);
  - an unreadable file, or `Ok(None)` (`Unreadable`);
  - a `.`, `..` or empty component (`Malformed`);
  - disagreeing `0::` and `name=systemd` lines (`Malformed`);
  - a slice, service or peer UID mismatch (`Malformed`);
  - `RemoteCompanion` without `remote_view`.

  The origin is cached per connection only on success, and root bypasses classification. An unprivileged peer then
  needs `has_local_seat_session`: active, `REMOTE=0`, a non-empty `SEAT` and `CLASS=user`, reusing the existing
  `check_local_seat_session_of`. An unreadable session file counts as no match. PASS.
- **Remote client.**
  - The daemon peer must be uid 0.
  - `Verdict::Allow` on an error-shaped reply maps to `Protocol`.
  - A poisoned slot mutex reads as Busy, stopped or idle, never as a start.
  - The grep finds no `unwrap`, `expect`, `panic!` or direct indexing in the production camera modules or in
    `preview_peer.rs`. Slice access uses `get`/`get_mut`, and arithmetic uses `checked_*` or `saturating_*`.
  - `#![forbid(unsafe_code)]` is still in `soos-remote`.
  - PASS.

### Cancellation Correctness of the View Loop
- The frame step is pinned and polled by `&mut` inside a biased `select!`.
- Non-terminal arms do not drop the in-flight exchange or encode. These arms are stray read bytes, a stop epoch for
  another view, and a passing Funnel session check (awaited in the arm body, a bounded mutex lock with no I/O).
- The stop condition is checked at the top of every iteration, through the durable flag and the `watch` epoch.
- Each part is written whole with `write_bounded`, outside the `select!`.
- Dropping a step on a terminal arm drops the daemon connection. The `PreviewSource` contract is "a dropped future
  drops the connection", so no stale reply is ever read. Only the blocking encode is detached.
- Every wait has an upper bound:
  - the first-frame phase is bounded by `CAMERA_FIRST_FRAME_TIMEOUT_MS`;
  - the stream phase is bounded by `ends_at` (max duration) and by `stall_at`;
  - the daemon exchange has one 3 s IO deadline for the attempt and its single retry, a 3 s connect and a 200 ms
    close-wait.
- PASS.

### Test Integrity & Anti-Weakening
- See §2. The new suites cover:
  - wrong token, wrong owner, stale sequence and stop durability;
  - stall, first frame and an oversize reply;
  - a non-root daemon and the cgroup edge cases.

  They can fail against plausible wrong implementations. The M11 self-check rejects 16 write forms, including
  `readlink` on another `/etc` file and the probe followed by a redirection. PASS.

### Memory, Bounds & Secrets
- **IPC.** The length prefix is checked against `MAX_PREVIEW_MESSAGE_SIZE` before allocation, and zero is refused.
  The buffer is exactly sized and `Zeroizing`. The pixels are moved out into `Zeroizing`. An Io or Protocol error
  drops the connection. PASS (RLC-S5/S6).
- **JPEG.**
  - Geometry is validated with `checked_mul`, and the exact data length is required.
  - Scratch buffers are capped by `CAMERA_MAX_SCRATCH_BYTES` and zeroized.
  - The sink is preallocated, bounded and never grows; on overflow it gives `TooLarge`.
  - There is no decoder.
  - PASS (RLC-S7/S8).
- **Token.**
  - 32 CSPRNG bytes in `Zeroizing`.
  - The `stream_path` string is `Zeroizing` and appears only in the authenticated JSON body.
  - `Route::CameraStream` carries no token, so `debug!(?resolved)` is safe.
  - No log line prints the request path or the token, and the token's `Debug` output is redacted.
  - PASS (RLC-S4).
- **Logging.**
  - `log_end` uses fixed texts, and the audit lines have no fields.
  - The daemon debug line lost its `bytes` field and logs one fixed-text line on the first remote frame.
  - The push payload is a flag only.
  - PASS (RLC-S7/S11).
- **Page.** `app.js` has no `<img>`, object URL, `innerHTML`, `console.*` or web storage. PASS (RLC-S12).

### Supply Chain & Automation
- `jpeg-encoder = "=0.7.1"` is pinned with default features (no `simd`).
- The `IJG` licence exception in `deny.toml` is scoped to that crate. `nix` gains the `time` feature only.
- A dev-profile `opt-level` override changes test build speed only.
- The installer adds commented template keys and `echo` lines only: no `sudo`, no write under `/etc`.
- PASS.

### English-Only Policy
- A diacritic scan and a common-French-word scan of the added lines found no issue. The only hits are the `[est]`
  "estimate" tag in the research note and an `'é'` test vector in a token-parse test.
- Code, comments, docs, UI strings, audit lines and the push text are English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/remote/src/camera_ipc.rs` `decode_reply`: a `PreviewResponse` is not matched to the
  request id. This is safe today: exchanges are serial on one connection, and an Io or Protocol error drops it.
  Match it if the preview wire format ever carries the id.
- **[SUGGESTION]** `CAMERA_DAEMON_SOCKET_PATH` repeats `/run/soos/daemon.sock` (auditor D-3). The daemon's
  `wire_format_code` and the GUI's `pixel_format_from_wire` also still hard-code the 0-4 codes next to the new
  `soos-protocol` constants. Use the shared constants in a follow-up, so that there is one source of truth.
- **[NOTE]** Tester contract §5 records that the M11 owner approval was relayed by the workflow orchestrator. The
  owner should confirm it before merge.

No CRITICAL, MAJOR or MINOR findings.

## 5. Final Verdict

**VERDICT: APPROVED**
