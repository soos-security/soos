# Walkthrough 194 — soos-remote Security Hardening (security review of 2026-10-12)

- **Date**: 2026-10-12
- **Issue**: none (GitHub-only security-review fix; no backlog id, not registered in `scripts/sync_issue.py`, no
  `AI/BACKLOG.md` entry). **Branch**: `fix/remote-security-hardening`, created from `origin/main` at `f9234ea`.
  Nothing is pushed, installed, enabled or restarted by the agents; no agent runs `tailscale`, `sudo` or `curl`
  against a live endpoint.
- **ADR**: "[2026-10-12] soos-remote Security Hardening" (`AI/DECISIONS.md`); it corrects the 2026-10-06 Web Push
  ADR (sender sandbox) and amends LC-3a of the 2026-10-07 live camera ADR (preview placement).
- **Documents**: spec `AI/architect_spec_remote_security_hardening.md` (round 2, APPROVED); plan evaluation
  `AI/plan_evaluator_report.md`; tester contract `AI/tester_contract_remote_security_hardening.md`; auditor
  constraints `AI/auditor_constraints_remote_security_hardening.md` (C-1 to C-25, CLEARED).
- **Matrix criteria**: RSH1–RSH18 (RSH2 is the agent host check of spec §1.1; RSH18 is the owner hardware check,
  pending).

## 1. Context & Objectives

A security review of `soos-remote`, the Web Push sender, the live camera view and the daemon preview channel found
one MAJOR, four MINOR and two SUGGESTION items plus four small defects (S-1 to S-8d). Every finding was re-verified
in the code (S-1 also on the owner host) before the fix was designed. Nothing touches the root PAM path,
`pam_soos.so`, the IPC wire format or the policy crate.

## 2. Changes by finding

| Finding | Fix | Where |
|---|---|---|
| S-1 sender can read the system journal | `InaccessiblePaths=-/var/log -/var/lib/systemd/coredump` directly after `TemporaryFileSystem=/run:ro`; header comment; docs and ADR corrected | `packaging/soos-push-sender.service`, `Docs/REMOTE_COMPANION.md` §2d, `AI/ARCHITECTURE.md` §13 |
| S-2 Tailscale minimum version | `TAILSCALE_MIN_VERIFIED_VERSION = "1.102.4"`; docs require it for Serve and Funnel; the installer prints it in `echo` lines only (it still never runs `tailscale`) | `crates/remote/src/lib.rs`, `scripts/install_remote.sh`, `Docs/REMOTE_COMPANION.md` §2, §2b, §4, §8 |
| S-3 preview starves face auth | `AUTH_RESERVED_CONNECTIONS_PER_UID = 1`, `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS = 200`, `AdmissionClass`, `PeerConnectionLimiter::try_acquire_auth_reserve`, `PeerConnectionPermit::class`; the dispatcher tries the reserve only after `PerUidCap`, reads the whole first frame with `timeout(.., read_client_frame(..))` and refuses anything but an `Auth` request before `handle_request`/`handle_event`; the companion's retry path closes gracefully | `crates/daemon/src/{limits,dispatcher}.rs`, `crates/remote/src/camera_ipc.rs` |
| S-4 Android lock-screen privacy | §2h item 4 (Android and Chrome settings, `push_previews = "generic"`) | `Docs/REMOTE_COMPANION.md` |
| S-5 preview served to SSH processes | `PeerPlacement`, `classify_peer_placement`, `LocalSessionPolicy::authorize_preview_placement` (the user-manager record rule is a shared private helper of the root `Auth` path and the preview path); the dispatcher reads the cgroup once per connection for origin and placement and evaluates step 6c-2a on every preview request | `crates/daemon/src/{session_policy,dispatcher}.rs`, `Docs/IPC_PROTOCOL.md`, `Docs/DAEMON.md` |
| S-6 missing response headers | `HARDENING_HEADERS` (HSTS, COOP, CORP, Permissions-Policy) after the untouched `MANDATORY_HEADERS` in the three head encoders | `crates/remote/src/http.rs` |
| S-7 sender never checks its sandbox | `home_from_exe`, `evaluate_sandbox` (pure), `probe_sandbox` (bounded mount table, `current_exe()`, `read_dir("/var/log").is_ok()`); `main` exits 78 right after the root check, before reading `XDG_RUNTIME_DIR` or binding | `crates/push-sender/src/{lib,main}.rs` |
| S-8a installer follows a dangling symlink | `[[ -L ]]` refusal, then `(set -C; cat > ...)` (noclobber, `O_EXCL`) | `scripts/install_remote.sh` |
| S-8b one JPEG after a stop | `stop_requested` re-checked after `mark_streaming` (before the head) and after the head (before `set_shown`/the first part) | `crates/remote/src/server.rs` |
| S-8c stale push subscription slot | the worker sends `replacesEndpoint` (`renewalPayload`, within `SUBSCRIBE_BODY_LIMIT` = 2048); the server removes it and upserts the new subscription in one `PushStore::replace_or_upsert` update; an unparsable old endpoint is ignored | `crates/remote/assets/sw.js`, `crates/remote/src/push.rs` |
| S-8d MJPEG / NV12 / >480-line frames | `remote_preview_image_for_frame` (Grey/YUYV/RGB24 within 640x480; MJPEG through the bounded decoder; IR branch recursing into itself) used for the remote-companion origin only, after the preview limiter guard; the GUI path is bit-for-bit unchanged | `crates/daemon/src/{preview_image,preview,dispatcher}.rs` |

## 3. Notes on the implementation

- `read_and_process` no longer holds a `read_exact`: steps 2–3 moved byte for byte into the free function
  `read_client_frame`. A reserve connection never loops a second time (`one_shot || AuthReserve` ends the loop).
- `tracked_uids()` counts the union of the regular and reserved maps; `Drop` releases the map of the permit's class.
- The sandbox rules compare paths component-wise (`Path::starts_with`, `Path::ends_with`), unescape octal mount
  points before every comparison, and match `ro` as an exact item of the per-mount options; malformed lines (ID
  fields that do not parse, too few fields, no `-` separator) are skipped. Every read-only tmpfs covering the home
  is checked for children that re-expose it.
- `replace_or_upsert` returns a fourth variant `RenewResult::RenewedExisting` when the old endpoint was removed
  and the new one was already stored (its keys replaced): it logs only the `removed` audit line, so the audit
  lines always match the store change.
- The remote YUYV path accepts an odd width: the last pixel whose macropixel is incomplete at the end of the buffer
  uses neutral chroma (remote path only; the GUI path still fails such a frame as before).

## 4. Test integrity

The only changes to pre-existing tests are the owner-approved CM-SR1 (test 45, one exact unit line added) and the
setup-only AM-SR1/AM-SR2, all made in Phase 2. Phase 4 changed no test file.

## 5. Gate results

Phase 4 (developer), exact CI commands:

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --locked --all-features -- -D warnings`: clean.
- `cargo test --workspace --locked --all-features`: 3403 passed, 0 failed, 6 ignored (pre-existing ignores);
  `soos-invariants` 564 passed.
- `cargo deny --locked check`: advisories, bans, licenses and sources ok.
- `cargo test --locked -p soos-daemon --all-features --test peer_limits_tests rsh`: 10 of 10 runs green.
- `./scripts/candid_review.sh` (layer 1): passed.

## 6. Owner hardware check (RSH18, pending)

1. After reinstall, `systemctl --user show soos-push-sender -p InaccessiblePaths` lists both paths and a test
   notification still arrives.
2. Running `~/.local/bin/soos-push-sender` by hand exits 78 ("sandbox not detected").
3. With a live camera view **and** `soos-gui` open, the lock screen face unlock works.
4. The `soos-gui` preview and the phone view still work; with an SSH session of the owner open, the phone view is
   refused (`camera_refused`) and works again after logout.
5. A 640x480 MJPEG or 640x512 IR camera shows a live view.
6. The Android notification icon and the iPhone home-screen icon still load (CORP).
7. `tailscale version` reports 1.102.4 or later.

## 7. Residual risks

As listed in the spec §13: same-UID `/proc` and world-readable files remain visible to the sender; the proxy version
cannot be detected; a same-UID process can hold the reserve for 200 ms per connection and a slow PAM conversation
misses it; placements outside session scopes and the user manager keep the UID-level rule; a bind added under `/run`
is not detected by the sender self-check; renewals without an old subscription keep the stale slot; the remote MJPEG
decode runs inline (bounded, off the `Auth` path, no shared lock held).
