# Plan Evaluation Report

- **Date**: 2026-10-12
- **Issue**: GitHub-only security review of 2026-10-12 (S-1 … S-8d), no backlog id — soos-remote Security Hardening
- **Spec**: `AI/architect_spec_remote_security_hardening.md` (round 1, commit `1702291`)
- **Branch**: `fix/remote-security-hardening`
- **Base commit**: `f9234ea` (`origin/main`)
- **Round**: 1

## 1. Coverage Matrix

| Finding / acceptance line | Spec element | Status |
|---|---|---|
| S-1 sender reads the system journal; add `InaccessiblePaths=`; correct docs; CM-SR1 on test 45 | §2.1–§2.4, §11.3 CM-SR1, RSH1/RSH2/RSH18-1 | Covered |
| S-2 raise documented Tailscale minimum; installer warns/refuses (best effort, never breaks without tailscale) | §3 (`TAILSCALE_MIN_VERIFIED_VERSION` = 1.102.4, docs, installer **prints** only) | Covered; "refuse" credibly refuted (RMC-S6 forbids any non-`echo`/`printf` line naming `tailscale`, verified in `tests/invariants/src/remote_companion_contract.rs:735`) |
| S-3 face auth not starved by preview connections, per-UID DoS bound kept | §4 auth-reserved permit + companion graceful close on retry | Covered, see PE-SR2, PE-SR3 |
| S-4 Android lock-screen privacy + `push_previews = "generic"` | §5 | Covered |
| S-5 preview only for processes inside a local seat session, soos-gui / soos-remote keep working | §6 `PeerPlacement`, `authorize_preview_placement`, 6c-2a | Covered |
| S-6 CORP, COOP, Permissions-Policy, HSTS; CSP byte-identical | §7 `HARDENING_HEADERS` | Covered |
| S-7 sender fails closed (78) unless sandbox detected; robust, testable probes | §8 `evaluate_sandbox` / `probe_sandbox` | **Covered but conflicts with an existing contract — PE-SR1** |
| S-8a dangling symlink followed by the installer | §9.1 `-L` refusal + `set -C` | Covered |
| S-8b JPEG sent after a stop | §9.2 re-check after `mark_streaming` | Covered |
| S-8c stale subscription slot on renewal | §9.3 `replacesEndpoint` + `replace_or_upsert` | Covered, see PE-SR4 |
| S-8d MJPEG / NV12 / >480 px refused by the companion | §9.4 daemon-side `remote_preview_image_for_frame` | Covered |
| Zero test weakening; only CM-SR1 owner-approved; other changes AM-SRn | §11.3 CM-SR1, AM-SR1, AM-SR2 (setup only, additive) | Covered, but the exhaustive list missed RMC-S34 (PE-SR1) |
| Daemon changes keep fail-closed PAM behaviour | §4.3, §10 | Covered (reserve refusals close unanswered → PAM `PAM_IGNORE`; root path untouched) |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `DEFAULT_MAX_CONNECTIONS_PER_UID` = 2 | `crates/daemon/src/limits.rs:23` | 2 | Yes |
| Admission before any read | `crates/daemon/src/dispatcher.rs:424` (`try_acquire`, then `ConcurrencyLimitReached`, no byte written) | as cited | Yes |
| `has_local_seat_session(peer_uid)` only for preview | `dispatcher.rs:1192` | as cited | Yes |
| `with_session_validator` also rebuilds `session_policy` | `dispatcher.rs:323-324` | `LocalSessionPolicy::from_validator` | Yes (so `test_rlc_default_peer_source_reads_proc_cgroup`, which disables the validator for both `enforce` values, stays green) |
| `authorize_user_manager_caller` rule (remote/UID-less record refuses, local seat needed) | `session_policy.rs:589-621` | as cited | Yes |
| `parse_session_id_from_cgroup` first-unit rule (nested `session-*.scope` under the manager is not a session) | `session_policy.rs:279` | as cited | Yes |
| Owner host records: manager session `REMOTE=0`, seat session `REMOTE=0` | `/run/systemd/sessions/{1,4}` (read-only check) | `CLASS=manager REMOTE=0`, `CLASS=user REMOTE=0 SEAT=seat0` | User-manager rule passes on the owner host |
| Agent test process placement | `/proc/self/cgroup` | `user@1000.service/session.slice/tmux-spawn-….scope` → `UserManager` | AM-SR2 harmless here; needed under a TTY session scope |
| `local_cgroup(uid)` = `session-4.scope`; first scenario writes only record `"3"` | `crates/daemon/tests/preview_remote_view_tests.rs:142,1088` | as cited | Yes (AM-SR1 justified) |
| `test_rlc_preview_requires_local_seat_session` writes record `"4"` | same file `:884` | `"4"` | Yes (unchanged test stays valid) |
| `CLOSE_DEADLINE` of the over-cap tests | `peer_limits_tests.rs:58`, `preview_remote_view_tests.rs:74` | 500 ms | 200 ms window fits (300 ms margin) |
| Permit dropped before the stream on a reserve refusal | `handle_connection(&self, mut stream)`: locals (`_permit`) drop before the by-value parameter | as cited | Yes, keeps `available_permits()` assertions of `test_same_uid_connection_over_cap_is_closed_without_blocking_others` exact |
| "PAM writes its request right after `connect`" (§4.1, §4.3) | `crates/pam/src/ipc.rs:514-534`, `crates/pam/src/lib.rs:302-307` | `on_connected()` runs between connect and write and sends `PAM_TEXT_INFO` through the PAM conversation | **No — PE-SR2** |
| Camera retry path `drop(conn)` | `crates/remote/src/camera_ipc.rs:178-181` | `drop(conn); conn = self.connect().await?` | Yes |
| `SENDER_UNIT_LINES: [&str; 37]`, forbidden prefixes | `tests/invariants/src/remote_push_contract.rs:452-525` | 37; no forbidden prefix matches `InaccessiblePaths=` | Yes |
| Sender env reads | `remote_push_contract.rs:429-439` (RMC-S34) | every `env::var(`/`env::var_os(`/`std::env::vars` must read `"XDG_RUNTIME_DIR"` | **Spec §8.1 reads `$HOME` — PE-SR1** |
| Sender tracing hygiene (one string literal) | `remote_push_contract.rs:580-588` (RMC-S36) | as cited | Spec complies |
| Docs `Tailscale 1.38.3` | `Docs/REMOTE_COMPANION.md:177` | as cited; no test pins `1.38.3` | Yes |
| RMC-S6 installer rule | `remote_companion_contract.rs:735-790` | `tailscale` only in `echo`/`printf` lines | Yes |
| `MANDATORY_HEADERS` pinned byte-for-byte; one CSP | `remote_camera_contract.rs:740-762` | as cited | Yes; `assert_mandatory_headers` checks only named headers, no header-count assertion exists |
| `--http` refused (HSTS rationale) | `crates/remote/src/identity.rs:111-160` (`X-Forwarded-Proto` must be `https`) | as cited | Yes |
| `mark_streaming` keeps the stop flag; `first_frame` biased `select!` | `camera_slot.rs:286`, `camera.rs:487-560` | stop checked at loop top and before `step`; a stop raised inside the producing poll is missed | Yes (S-8b window real; the planned test seam can fail without the fix) |
| `SubscribeBody` `deny_unknown_fields`, camelCase | `crates/remote/src/push.rs:1124-1132` | as cited | Yes |
| RAN-S2 worker assertions | `remote_android_contract.rs:559-693` | listener must contain `renewedSubscription(event)` and `postSubscription(`, end with terminal `.catch(() => null)`; one `fetch(`; `postSubscription` unchanged | Spec §9.3 complies |
| `MAX_PUSH_SUBSCRIPTIONS` 4, `MAX_PUSH_SUBSCRIBE_BODY_BYTES` 2048 | `crates/remote/src/lib.rs:377,383` | 4 / 2048 | Yes |
| `push_previews` default `Detailed` | `crates/remote/src/config.rs:152,539` | as cited | Yes |
| Daemon MJPEG forwarded unchanged at any size; existing bounded `downscale` handles NV12 and MJPEG | `crates/daemon/src/preview_image.rs:84-110,173-262`; `crates/vision/src/color.rs:228` (size/dimension caps, header must match frame) | as cited | Yes |
| `jpeg-encoder =0.7.1` workspace dep; RLC-S1 feature check uses `-e normal` | `Cargo.toml:84`, `remote_camera_contract.rs:100-140` | a daemon **dev**-dependency is outside the checked edges | Yes |
| Highest walkthrough 193 | `AI/walkthroughs/` | (spec claim; not contradicted) | Yes |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Scenario: an SSH-only process of the owner asks for preview frames while the owner also has a local session. Under the plan it sits in `session-N.scope` with `REMOTE=1` (→ `Session`, refused), or reaches `user@uid.service` through `systemd-run --user` (→ `UserManager`, refused while the SSH record exists). Only a root-placed process (`Other`) keeps the UID-level rule; that requires root help. Residual stated in §13.4.
- Scenario: soos-gui under the owner's TTY-launched compositor (`session-4.scope`, local) → served; soos-remote (`user@1000.service`, records all `REMOTE=0`) → served. Verified against the host records.
- Scenario: compromised sender reads `/var/log/journal` via `adm`/`wheel` ACL. Blocked by `InaccessiblePaths=` (host-verified per §1.1); `/run/log/journal` already hidden by `TemporaryFileSystem=/run:ro`.
- Result: PASS.

### Pillar 2 — PAM deadline & concurrency
- No change in `crates/pam`; no Tokio in PAM. The reserve adds a daemon-side readiness wait only on the reserve path.
- Scenario: user-level lock screen (swaylock-plugin, unprivileged) while soos-gui and the phone view hold both regular permits. PAM connects (reserve admitted), then calls `on_connected` → `PAM_TEXT_INFO` through the conversation, then writes. If the conversation takes ≥ 200 ms, the reserve closes the connection unanswered → PAM read fails → `PAM_IGNORE` → password. Fail-safe, but the plan's latency claim is wrong (PE-SR2).
- Scenario: a same-UID process sends 1 byte on the reserve and stalls: readiness is satisfied, then `read_exact` waits up to `connection_timeout` (default 2500 ms, max 10 000 ms), holding the only reserve (PE-SR3).
- Result: FINDING (MINOR ×2).

### Pillar 3 — Panic safety & fail-closed
- Every new refusal (reserve non-`Auth`, reserve timeout, placement denial, sandbox not detected, invalid `replacesEndpoint`) ends in a closed connection, a `ProtocolError`/`UidMismatch` preview refusal, exit 78, or `400` — none produces an allow or `PAM_SUCCESS`. Logind read errors map to `LogindUnavailable` (deny). No `unwrap`/`expect` introduced by the spec.
- Scenario: placement re-check per request when the session turns remote mid-connection → second request refused (`test_rsh_placement_rechecked_per_request`).
- Result: PASS.

### Pillar 4 — Dependencies
- No new runtime crate; daemon gains a dev-dependency on the already-pinned `jpeg-encoder =0.7.1` (normal-edge feature check RLC-S1/B1 unaffected; `deny.toml` IJG exception is crate-scoped and already present). `soos-remote` stays decoder-free (RLC-S10).
- Result: PASS.

### Pillar 5 — Data confidentiality
- New log lines carry `peer_uid` and constant strings only; sender keeps one-literal tracing (RMC-S36). Decoded MJPEG buffers stay in the existing `Zeroizing` path. `replacesEndpoint` is never logged (push modules have no tracing, RMC-S37).
- Scenario: the S-7 probe reads the environment to find `$HOME` — this is a confidentiality-hardening contract of the sender (RMC-S34: the sender reads no environment variable except `XDG_RUNTIME_DIR`), which the plan would break (PE-SR1).
- Result: FINDING (MAJOR, via Pillar 6).

### Pillar 6 — Test integrity
- CM-SR1 is exactly the owner-approved additive change (37 → 38, one line inserted, forbidden list unchanged).
- AM-SR1 / AM-SR2 are setup-only additions; assertions untouched; justified by the new placement rule. Verified the other preview tests either use record `"4"` with `session-4.scope`, a `remote_cgroup` (user manager, no remote record) or a disabled validator.
- `test_same_uid_connection_over_cap_is_closed_without_blocking_others`, `test_idle_polling_peer_cannot_exhaust_global_capacity`, `test_rlc_view_connection_leaves_room_for_auth`: still valid (non-`Auth` closed after decode with zero bytes; idle third connection closed after 200 ms < 500 ms; permit released before the stream closes).
- **Missed contract**: RMC-S34 (`test_rmc_s34_sender_policy_literals`) fails against §8.1 as written; it is not in §11.3 and cannot be amended (it would relax an assertion). PE-SR1.
- Test power: the S-8b seam (stop raised inside the producing poll) fails without the fix because `first_frame` polls `stop_rx` before `step` in the same `select!`; the sandbox tests include exact option matching and component-wise prefixes (`/home2`); the hardening-header test pins exact values. Missing power: no test for a partial first frame on the reserve (PE-SR3) and none for a delayed-but-valid `Auth` on the reserve (PE-SR2).
- Result: FINDING (MAJOR).

## 4. Findings

- **PE-SR1 [MAJOR] S-7 probe reads `$HOME` from the environment, which breaks RMC-S34.**
  `tests/invariants/src/remote_push_contract.rs:429-439` requires every `env::var(`, `env::var_os(` and `std::env::vars` in `crates/push-sender/src` to read exactly `"XDG_RUNTIME_DIR"`. §8.1 `probe_sandbox` takes "$HOME from the environment", so RMC-S34 would fail, and §11.3 does not list it (and must not: relaxing it is forbidden).
  *Required plan change*: obtain the home anchor without the environment and without textual circumvention (never `std::env::home_dir()`), e.g. the passwd entry of `getuid()` (`nix::unistd::User::from_uid`, adding the `user` feature to the sender's `nix` if missing, re-checked against `deny.toml`), or the parent chain of `std::env::current_exe()` (the unit's `BindReadOnlyPaths=%h/.local/bin/soos-push-sender`); state which, and what happens when the anchor is unavailable (fail closed, exit 78). Rename/redefine `SandboxError::HomeUnset` and `test_rsh_sandbox_relative_or_empty_home_refused` accordingly, add RMC-S34 to the "checked compatible without change" list, and add a static RSH assertion that the sender still reads no environment variable other than `XDG_RUNTIME_DIR`. Also state the behaviour for a home outside `/home` (e.g. `/var/home` on image-based distributions where `ProtectHome=tmpfs` covers `/home`): either covered by rule 2 or a documented refusal.

- **PE-SR2 [MINOR] The 200 ms reserve window rests on a false fact.**
  §4.1/§4.3 say "PAM writes its request right after `connect`". In `crates/pam/src/ipc.rs:514-534` the `on_connected` callback runs between connect and write, and `crates/pam/src/lib.rs:302-307` sends `PAM_TEXT_INFO` ("[soos] Looking for face...") through the application's conversation there. A slow conversation (≥ 200 ms) makes the reserve close an honest `Auth` (fail-safe password fallback, i.e. S-3 not fixed for that lock screen).
  *Required plan change*: correct the wording; record the dependency on the conversation latency in §13 and in `Docs/IPC_PROTOCOL.md`/`Docs/REMOTE_COMPANION.md` §2f; add a behaviour test where the reserve client writes a valid `Auth` after ~100 ms and is answered. (Moving `on_connected` after the write would touch the PAM path; if chosen, it needs its own justification and PAM tests — not required here.)

- **PE-SR3 [MINOR] The reserve window bounds only readiness, not the first frame.**
  `stream.readable()` returns after one byte; `read_and_process` then waits for the rest under `connection_timeout` (default 2500 ms, up to `MAX_CONNECTION_TIMEOUT_MS` 10 000 ms). A same-UID process can hold the only reserve for seconds per attempt, so §13.3 ("≤ 200 ms at a time") is inaccurate.
  *Required plan change*: on the reserve, bound reading the whole first frame (4-byte length + body) by the 200 ms window (e.g. a `timeout` around a reserve-specific first read, then hand the decoded message to the normal path), or restate the residual with the real bound. Add `test_rsh_reserve_partial_frame_closed_within_window` (1 byte then stall → closed within 500 ms, zero bytes, permit released).

- **PE-SR4 [MINOR] An invalid `replacesEndpoint` discards the renewal.**
  §9.3 answers `400` and writes nothing when `PushEndpoint::parse(replacesEndpoint)` fails, so a renewal whose *old* endpoint no longer parses (allowlist change, browser format change) loses the *new* subscription, which is worse than today's behaviour.
  *Required plan change*: either treat an unparsable `replacesEndpoint` as absent (plain upsert, no oracle) or justify the `400`; adjust `test_rsh_replaces_endpoint_invalid_is_400_nothing_written` to the chosen rule before Phase 2.

- **PE-SR5 [MINOR, observation] S-8b leaves a residual window during the head write.**
  The re-check happens before `write_bounded(head)`; a stop processed while the head write is pending (socket buffer full) still lets `write_part` send the first JPEG. Bounded and rare, but the review asked for "no JPEG after a stop". *Suggested*: re-check `stop_requested` again after the head and before `set_shown`/`write_part` (end with the normal `finish_view(Stopped)` path, view not shown), or list it in §13.

- **PE-SR6 [MINOR, observation] S-8d decodes MJPEG inline on a Tokio worker.**
  Up to 10 decodes/s of a 640x480 frame per remote view inside the dispatcher task. Bounded by `MAX_MJPEG_COMPRESSED_BYTES`/`MAX_MJPEG_DIMENSION` and the preview rate limit, and stated in §13.8; acceptable, but the developer should keep it out of the `Auth` critical path (no shared lock held during the decode). No plan change required.

- **PE-SR7 [MINOR, observation] Test timing margin.**
  Two existing tests now depend on the 200 ms reserve timer against a 500 ms `CLOSE_DEADLINE` (300 ms margin under CI load). Keep `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS` a non-configurable constant (as specified) and make the static RSH test pin it at ≤ 250 ms so a later increase cannot make those contracts flaky.

## 5. Verdict

One MAJOR finding (PE-SR1): the S-7 design breaks the existing RMC-S34 contract, and that contract may not be relaxed. PE-SR2 to PE-SR4 must be resolved in the same revision; PE-SR5 to PE-SR7 are recommended.

VALIDATION_VERDICT: REVISION_REQUIRED
