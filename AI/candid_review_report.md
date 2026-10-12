# Candid Review Report

- **Date**: 2026-10-12
- **Target Branch**: `fix/remote-security-hardening`
- **Base (merge-base)**: `f9234ea3c`
- **Reviewed-Diff-Fingerprint**: `a7fc77f519a1713bf485039034c733eb2c7b04eed6d2d429f739b50db69692e7`
- **Audited Files**: `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/architect_spec_remote_security_hardening.md`,
  `AI/auditor_constraints_remote_security_hardening.md`, `AI/tester_contract_remote_security_hardening.md`,
  `AI/walkthroughs/194_remote_security_hardening.md`, `Cargo.lock`, `Docs/DAEMON.md`, `Docs/IPC_PROTOCOL.md`,
  `Docs/REMOTE_COMPANION.md`, `crates/daemon/Cargo.toml`, `crates/daemon/src/dispatcher.rs`,
  `crates/daemon/src/limits.rs`, `crates/daemon/src/preview.rs`, `crates/daemon/src/preview_image.rs`,
  `crates/daemon/src/session_policy.rs`, `crates/daemon/tests/peer_limits_tests.rs`,
  `crates/daemon/tests/preview_authorization_tests.rs`, `crates/daemon/tests/preview_downscale_tests.rs`,
  `crates/daemon/tests/preview_placement_tests.rs`, `crates/daemon/tests/preview_remote_view_tests.rs`,
  `crates/push-sender/src/lib.rs`, `crates/push-sender/src/main.rs`, `crates/push-sender/tests/sandbox_tests.rs`,
  `crates/remote/assets/sw.js`, `crates/remote/src/camera_ipc.rs`, `crates/remote/src/http.rs`,
  `crates/remote/src/lib.rs`, `crates/remote/src/push.rs`, `crates/remote/src/server.rs`,
  `crates/remote/tests/camera_server_tests.rs`, `crates/remote/tests/http_tests.rs`,
  `crates/remote/tests/push_server_tests.rs`, `packaging/soos-push-sender.service`, `scripts/install_remote.sh`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/remote_push_contract.rs`,
  `tests/invariants/src/remote_security_hardening_contract.rs`

## 1. Executive Summary

The diff implements the eight remediations of the 2026-10-12 soos-remote security review: S-1 (push sender
unit hides `/var/log` and the coredump directory), S-2 (single-sourced minimum verified Tailscale version
in installer/docs), S-3 (one auth-reserved daemon connection per unprivileged UID, `Auth`-only, 200 ms
first-frame window), S-5 (preview peer placement rule: own local seat session, or own user manager without
a remote session), S-6 (browser hardening headers on every response head), S-7 (push sender start-up
sandbox self-check, exit 78), S-8a-d (symlink-safe installer config write, stop re-checked before the
camera head and first part, in-place push subscription renewal, daemon-side conversion of MJPEG/NV12/oversized
frames to Grey/RGB24 within 640x480 for the remote companion origin only).

The PAM crate is untouched; no path from an error to `Allow`/`PAM_SUCCESS` was introduced; every new
daemon branch fails closed. Test changes are additive except one owner-approved contract migration
(CM-SR1) that strengthens an exact line set. `cargo test` (daemon, remote, push-sender, invariants, all
features), `cargo clippy --all-targets --all-features -D warnings` and `cargo fmt --check` are green
locally. No CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing (step 3 of the skill) over `target/candid_diff.patch`:

- Removed lines matching an assertion pattern: one hit, line 1916 `// Step 3: Bounded memory safety
  assertion` in `crates/daemon/src/dispatcher.rs`. This is a production comment; the bounded size check it
  labelled moved verbatim into `read_client_frame` (`0 < size <= MAX_MESSAGE_SIZE` before allocation). Not a
  test change.
- New escape hatches: one hit, a comment "within JPEG tolerance" in the new `preview_downscale_tests.rs`
  (a bounded per-channel pixel comparison on a lossy MJPEG fixture). Justified; not an `#[ignore]`.
- Inline `mod tests` changes: none.
- Modified existing tests (`git diff origin/main...HEAD` lines starting with `-` in test files):
  - `tests/invariants/src/remote_push_contract.rs` `SENDER_UNIT_LINES: [&str; 37]` -> `[&str; 38]` with one
    inserted line `InaccessiblePaths=-/var/log -/var/lib/systemd/coredump`: tester contract **CM-SR1**
    (owner-approved), spec §2.2/§11.3. Exact-set assertion kept, nothing removed or reordered:
    strengthening.
  - `crates/daemon/tests/preview_remote_view_tests.rs::test_rlc_local_peer_unaffected_by_remote_view`: one
    added setup line writing session record `"4"` (**AM-SR1**). Assertions untouched.
  - `crates/daemon/tests/preview_authorization_tests.rs::test_preview_frame_allowed_for_configured_uid_serves_frames`:
    additive setup writing the test process's own session record when `/proc/self/cgroup` names a session
    scope (**AM-SR2**). Assertions untouched.
- All other test files (`peer_limits_tests.rs`, `preview_downscale_tests.rs`, `preview_placement_tests.rs`,
  `sandbox_tests.rs`, `camera_server_tests.rs`, `http_tests.rs`, `push_server_tests.rs`,
  `remote_security_hardening_contract.rs`, `tests/invariants/src/lib.rs`) contain additions only.

## 3. Deep Reasoning Audit

### Logic & Architecture
- S-3 admission: `try_acquire` -> on `PerUidCap` only, `try_acquire_auth_reserve`. Tried: root peer (never
  reserved, returns `PerUidCap`), global/unprivileged capacity full (checked before the reserve, state
  untouched on refusal), second reserve of the same UID (refused), permit drop accounting (class-specific
  map decremented, totals restored). `tracked_uids` counts reserve-only UIDs; the reserve map can only
  contain UIDs that hit the regular cap, so memory stays bounded. PASS.
- S-3 dispatch: a reserve connection must deliver its whole frame within 200 ms (nested in the outer
  `connection_timeout`), the decoded message must be `Request` with `RequestKind::Auth`, and the loop
  breaks after one request. Tried: preview/event on the reserve (refused before processing, covered by
  `test_rsh_reserve_closes_non_auth_without_payload` and the invariant `refuses_events_before_processing`),
  slow-loris partial frame (closed in window). PASS.
- S-5 placement: `classify_peer_placement` reads the same cgroup content as the origin, session scope
  first. Tried: a user creating `session-4.scope` inside the delegated `user@UID.service` subtree to pose
  as a session (classified `UserManager` because only the first unit component is considered; covered by
  test), SSH session peer (own record `REMOTE=1` -> refused), session ending between two requests on one
  connection (re-checked per request), disabled policy (permits, as specified). `Other` is permitted by
  design (spec §6.3); an unprivileged process cannot move itself out of its session scope or user manager
  into `Other`. PASS.
- S-8d conversion: applied only to `PreviewPeerOrigin::RemoteCompanion`; the IR recursion terminates after
  one level (output is Grey); output bound 640x480x3 holds via `height_step`/`width` step and the byte
  budget loop. Local origins keep `preview_image_for_frame` (test `test_rsh_local_preview_unchanged_for_mjpeg`). PASS.
- S-8c renewal: `replace_or_upsert` removes the old endpoint and upserts in one locked update; capacity is
  evaluated after the removal; `TooMany` writes nothing; an unparsable old endpoint is treated as absent.
  The route already requires an authenticated owner session, who could unsubscribe any endpoint anyway. PASS.
- S-8b: stop is re-checked after `mark_streaming` and after the head; both paths audit as refused and never
  count the view as shown. PASS.

### PAM Concurrency & Deadlines
- `crates/pam` is not in the diff. The new reserve window is daemon-side and only shortens how long an
  over-cap connection may stay idle; a PAM client that misses the window gets a closed socket and falls back
  to `PAM_IGNORE` through the unchanged client path. PASS.

### Panic Safety & Fail-Closed
- No `unwrap/expect/panic!/todo!/unreachable!` or slice indexing added in production (grep over the
  production hunks; workspace lints pass under `-D warnings`). Every new error path refuses: placement errors
  map to `refuse_preview`, cgroup malformation to `PreviewPeerError::Malformed`, logind read failure to
  `LogindUnavailable`, sandbox probe failure to exit 78, unconvertible frames to the empty image. No path
  reaches `Allow`/`PAM_SUCCESS`. PASS.

### Test Integrity & Anti-Weakening
- See §2. New tests can fail against plausible wrong implementations: the reserve tests distinguish
  "admit any request" from "Auth only", the placement tests distinguish "any local session of the UID" from
  "the peer's own session", the sandbox tests reject non-`ro` options, `/` tmpfs, a mount stacked on the home
  and a second child bind. Locally all green. PASS.

### Memory, Bounds & Secrets
- Frame reads bounded before allocation (`read_client_frame`); mountinfo read bounded to 1 MiB + 1 and
  refused above; decoded MJPEG buffers stay in `Zeroizing`; the IR intermediate frame is zeroized by
  `Drop for Frame`. Sandbox errors carry fixed texts (no path echoed); the probe never reads `/var/log`
  entries. Installer refuses a symlinked config and writes with `set -C` under `umask 077`. PASS.

### Supply Chain & Automation
- `jpeg-encoder` added to `soos-daemon` dev-dependencies only, already a workspace dependency (no new crate
  in `Cargo.lock` beyond the dependency edge). No workflow changes. `scripts/install_remote.sh` change is
  defensive. PASS.

### English-Only Policy
- No non-English text in added lines, commit subjects or docs. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/daemon/src/preview_image.rs` (`sample_rgb`, lenient chroma) — for an odd-width YUYV
  frame the macropixel index `(y*width + (x & !1))*2` does not follow row boundaries, so interior pixels can
  take chroma bytes from the neighbouring row; only the final incomplete macropixel falls back to neutral
  chroma. Colour fidelity only (odd-width YUYV is not a real V4L2 layout); no bound or safety impact. No
  change required for merge.
- **[SUGGESTION]** `crates/daemon/src/limits.rs` / `dispatcher.rs` — a same-UID process can repeatedly
  occupy the single auth-reserved permit (connect, send `Auth`, reconnect). Same-UID processes are inside the
  trust boundary of the user-level lock screen, so this does not regress the threat model; a short per-UID
  cool-down on reserve admissions could be considered later.
- **[SUGGESTION]** `crates/push-sender/src/lib.rs` `probe_sandbox` — `std::env::current_exe` returns a
  path suffixed with ` (deleted)` after the binary is replaced in place, which makes the self-check refuse
  with exit 78 until the unit restarts on the new binary. Fail-closed and acceptable; worth one line in the
  troubleshooting table.

## 5. Final Verdict

**VERDICT: APPROVED**
