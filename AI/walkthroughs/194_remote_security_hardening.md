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
  constraints `AI/auditor_constraints_remote_security_hardening.md` (C-1 to C-25, CLEARED); candid review
  `AI/candid_review_report.md` (APPROVED).
- **Matrix criteria**: RSH1–RSH18 (component `remote-security-hardening`; RSH2 is the agent host check of spec §1.1;
  RSH18 is the owner hardware check, pending).

## 1. Context & Objectives

A security review of `soos-remote`, the Web Push sender, the live camera view and the daemon preview channel found
one MAJOR, four MINOR and two SUGGESTION items plus four small defects (S-1 to S-8d). Every finding was re-verified
in the code (S-1 also on the owner host) before the fix was designed:

- **S-1 (MAJOR)** the sandboxed push sender could read the whole system journal through the owner's
  `wheel`/`adm`/`systemd-journal` membership, contrary to the 2026-10-06 ADR claim.
- **S-2** the documented Tailscale minimum (1.38.3) predates the Serve/Funnel behaviour the companion relies on.
- **S-3** two preview connections (`soos-gui` and a phone view) exhausted `max_connections_per_uid` and starved the
  lock screen's face `Auth`.
- **S-4** Android shows push notification text on the lock screen by default.
- **S-5** the preview channel served any process of a UID with a local seat session, including SSH processes.
- **S-6** responses lacked HSTS, COOP, CORP and a Permissions-Policy.
- **S-7** the sender never checked that its sandbox was in force.
- **S-8a–d** the installer followed a dangling `remote.toml` symlink; one JPEG could be sent after a stop; a renewed
  push subscription left a stale slot; MJPEG, NV12 and frames taller than 480 lines could not be viewed remotely.

Out of scope: the root PAM path, `pam_soos.so`, the IPC wire format and the policy crate are untouched.

## 2. Architect Design

| Finding | Design | Where |
|---|---|---|
| S-1 | `InaccessiblePaths=-/var/log -/var/lib/systemd/coredump` directly after `TemporaryFileSystem=/run:ro`; header comment; docs and ADR corrected | `packaging/soos-push-sender.service`, `Docs/REMOTE_COMPANION.md` §2d, `AI/ARCHITECTURE.md` §13 |
| S-2 | `TAILSCALE_MIN_VERIFIED_VERSION = "1.102.4"`, read by no runtime path; the installer prints it in `echo` lines only (it still never runs `tailscale`) | `crates/remote/src/lib.rs`, `scripts/install_remote.sh`, `Docs/REMOTE_COMPANION.md` §2, §2b, §4, §8 |
| S-3 | `AUTH_RESERVED_CONNECTIONS_PER_UID = 1`, `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS = 200` (non-configurable, ≤ 250), `AdmissionClass`, `PeerConnectionLimiter::try_acquire_auth_reserve`, `PeerConnectionPermit::class`; the reserve is tried only after `PerUidCap`, reads the whole first frame within the window and accepts only an `Auth` request; the companion's retry path closes gracefully | `crates/daemon/src/{limits,dispatcher}.rs`, `crates/remote/src/camera_ipc.rs` |
| S-4 | §2h item 4: Android and Chrome "Hide sensitive" settings, `push_previews = "generic"` | `Docs/REMOTE_COMPANION.md` |
| S-5 | `PeerPlacement` (`SessionScope`, `UserManager`, `Other`), `classify_peer_placement`, `LocalSessionPolicy::authorize_preview_placement` (step 6c-2a, every preview request); the user-manager record rule is one private helper shared with the root `Auth` path | `crates/daemon/src/{session_policy,dispatcher}.rs`, `Docs/IPC_PROTOCOL.md`, `Docs/DAEMON.md` |
| S-6 | `HARDENING_HEADERS` (HSTS 1 year, COOP/CORP `same-origin`, deny-all Permissions-Policy) after the untouched `MANDATORY_HEADERS` in the three head encoders; CSP byte-identical | `crates/remote/src/http.rs` |
| S-7 | `home_from_exe`, `evaluate_sandbox` (pure), `probe_sandbox` (mount table bounded by `MAX_MOUNTINFO_BYTES`, `current_exe()`, `read_dir("/var/log").is_ok()`); `main` exits 78 right after the root check, before reading `XDG_RUNTIME_DIR` or binding | `crates/push-sender/src/{lib,main}.rs` |
| S-8a | `[[ -L ]]` refusal, then `(set -C; cat > ...)` (noclobber, `O_EXCL`) | `scripts/install_remote.sh` |
| S-8b | `stop_requested` re-checked after `mark_streaming` (before the head) and after the head (before `set_shown`/the first part) | `crates/remote/src/server.rs` |
| S-8c | the worker sends `replacesEndpoint` (`renewalPayload`, within `SUBSCRIBE_BODY_LIMIT` = 2048); `PushStore::replace_or_upsert` removes it and upserts in one store update; an unparsable old endpoint is ignored | `crates/remote/assets/sw.js`, `crates/remote/src/push.rs` |
| S-8d | `remote_preview_image_for_frame` (Grey/YUYV/RGB24 within `REMOTE_PREVIEW_MAX_WIDTH` x `REMOTE_PREVIEW_MAX_HEIGHT` = 640x480, integer decimation; MJPEG through the bounded `soos_vision` decoder; IR branch recursing into itself) for the remote-companion origin only, after the preview limiter guard | `crates/daemon/src/{preview_image,preview,dispatcher}.rs` |

Invariants touched: fail closed on every new branch (no path to `Allow`/`PAM_SUCCESS`), bounded reads before
allocation, per-UID DoS bound becomes `max_connections_per_uid + 1` (unprivileged only, within the global and
unprivileged capacity), `soos-remote` stays network-free and decoder-free, the sender reads no environment variable
other than `XDG_RUNTIME_DIR`.

## 3. Plan Evaluation

Round 1 (`f5058b6`): `REVISION_REQUIRED` with one MAJOR (PE-SR1: a `$HOME`-based home anchor would break RMC-S34;
resolved by deriving the home from `current_exe()` and a stronger static test) and PE-SR2 to PE-SR7 (reserve window
wording and the 100 ms PAM conversation case, whole-frame read instead of `.readable()`, unparsable
`replacesEndpoint` = absent, second stop check after the head, static placement of the remote decode, constant not
configurable). Round 2 (`8883c1c`): `APPROVED`; PE-SR8 to PE-SR11 folded in `bccaa28` (IR recursion bounded to
640x480, re-exposure rule for mounts over the hidden home, `null` `oldSubscription`, SSH/`tmux` troubleshooting
row).

### Test amendments

| # | Test | Amendment |
|---|---|---|
| CM-SR1 (owner-approved) | `tests/invariants/src/remote_push_contract.rs::test_rmc_s35_sender_unit_is_sandboxed` | `SENDER_UNIT_LINES` 37 → 38 exact lines, `InaccessiblePaths=-/var/log -/var/lib/systemd/coredump` inserted after `TemporaryFileSystem=/run:ro`; nothing removed or reordered, forbidden list unchanged |
| AM-SR1 | `crates/daemon/tests/preview_remote_view_tests.rs::test_rlc_local_peer_unaffected_by_remote_view` | setup only: also writes the local seat record of the peer's own session `4`; assertions untouched |
| AM-SR2 | `crates/daemon/tests/preview_authorization_tests.rs::test_preview_frame_allowed_for_configured_uid_serves_frames` | setup only: writes the test process's own session record when `/proc/self/cgroup` names a session scope; assertions untouched |

## 4. Tester Contract

- Commit `450ef73` (plus the auditor additions of `6f43708`): 14 static tests in
  `tests/invariants/src/remote_security_hardening_contract.rs` and 56 behaviour tests: `peer_limits_tests` 12
  (`mod auth_reserve`), `preview_placement_tests` 10 (new file), `preview_downscale_tests` 10 (`mod remote_preview`,
  one proptest), `preview_remote_view_tests` 2, `http_tests` 1, `camera_server_tests` 1, `push_server_tests` 6
  (`mod renewal`), `sandbox_tests` 14 (new file).
- Mapping: RSH1 `test_rsh_s1_*` + CM-SR1; RSH3 `test_rsh_s2_*`; RSH4 `test_rsh_limiter_*`,
  `test_rsh_permit_reports_admission_class`; RSH5 `test_rsh_reserve_*`, `test_rsh_auth_admitted_on_reserve_*`,
  `test_rsh_s3_*`; RSH6 `test_rsh_s3_reserve_constants_and_dispatch`; RSH7 `test_rsh_s4_*`; RSH8/RSH9
  `preview_placement_tests.rs`, `test_rsh_s5_*`; RSH10 `test_rsh_hardening_headers_on_every_head`, `test_rsh_s6_*`;
  RSH11/RSH12 `sandbox_tests.rs`, `test_rsh_s7_*`; RSH13 `test_rsh_s8a_*`; RSH14
  `test_rsh_stop_after_first_frame_sends_no_part`, `test_rsh_s8b_*`; RSH15 `push_server_tests::renewal::*`,
  `test_rsh_s8c_*`; RSH16 `remote_preview::*`, `remote_conversion::*`, `test_rsh_s8d_*`; RSH17
  `test_rsh_docs_adr_and_walkthrough`.
- Red evidence: `soos-invariants` 550 passed, 13 failed (the 12 new static tests and CM-SR1 test 45); the
  `peer_limits`, `preview_placement`, `preview_downscale` and `sandbox` files were compile-red on the specified API
  only and failed on assertions under a temporary wrong-value stub (removed, no `src/` change committed);
  `http_tests` 1, `push_server_tests` 5 and `preview_remote_view_tests` 1 failed on assertions.
- Declared guards (green before the implementation): `test_rsh_s7_sender_reads_no_other_environment_variable`
  (stronger companion of RMC-S34), `test_rsh_stop_after_first_frame_sends_no_part` (no deterministic seam; S-8b is
  pinned statically), and the "unchanged" contracts (GUI path, plain subscribe, `Other` placement, disabled
  validator, root `Auth`).

## 5. Auditor Constraints

CLEARED with C-1 to C-25 and two added tests (A-1 `test_rsh_reserve_stalled_frame_holds_the_reserve_for_the_window`,
A-2 `test_rsh_s3_reserve_refuses_events_before_processing`). How the main ones are met:

- C-1, C-18: the unit gains exactly one line; the Tailscale constant is read by no runtime path; `1.38.3` is gone
  from the docs.
- C-2, C-25: no test changed beyond CM-SR1, AM-SR1, AM-SR2; Phase 4 changed no test file; the full gate is green.
- C-3 to C-6: the limiter refuses without mutating; `try_acquire` unchanged; the reserve window wraps the whole
  frame read (`timeout(.., read_client_frame(..))`, no `.readable()`), nested inside `connection_timeout`; anything
  but `Auth` is refused before `handle_request`/`handle_event`; a reserve connection is one-shot
  (`one_shot || AuthReserve` ends the loop).
- C-7, C-15: new log lines carry `peer_uid` and constant text only; the sender refusal is the constant
  "push sender sandbox not detected; refusing to start".
- C-9, C-10: the cgroup is read once per connection for origin and placement; 6c-2a runs after 6c-2 and before the
  rate limiter on every preview request; root peers skip it; the root `Auth` rule keeps its denials and order.
- C-12 to C-16: component-wise path comparisons (`Path::starts_with`/`ends_with`), octal-unescaped mount points,
  exact per-mount `ro`, malformed lines skipped, only direct children of the home tmpfs considered, the binary bind
  the only allowed child; mount table read with `take(MAX_MOUNTINFO_BYTES + 1)`; `/var/log` never iterated.
- C-17: symlink refusal before any write, `set -C` write under the existing `umask 077`.
- C-19 to C-21: both stop checks end the view as refused and never count it as shown; renewal is one locked store
  update, `TooMany` writes nothing; the worker keeps exactly one `fetch(`.
- C-22, C-23: the remote conversion never calls the GUI function, holds decoded pixels in `Zeroizing`, uses checked
  arithmetic, and runs only in `handle_preview_request` for the remote origin, after the limiter guard and off the
  `Auth` path; the GUI path is bit-for-bit unchanged.
- C-24: docs, ADR and this walkthrough (see §6).

## 6. Implementation

- Commit `b3d8ca5` (`fix(remote): harden soos-remote after the 2026-10-12 security review`): the production changes
  of §2. Notes:
  - `read_and_process` no longer holds a `read_exact`: steps 2–3 moved byte for byte into the free function
    `read_client_frame`. `tracked_uids()` counts the union of the regular and reserved maps; `Drop` releases the map
    of the permit's class.
  - `replace_or_upsert` returns a fourth variant `RenewResult::RenewedExisting` when the old endpoint was removed
    and the new one was already stored (its keys replaced): it logs only the `removed` audit line, so the audit
    lines always match the store change.
  - The remote YUYV path accepts an odd width: the last pixel whose macropixel is incomplete at the end of the
    buffer uses neutral chroma (remote path only; the GUI path still fails such a frame as before).
- Commit `dd0b1ea` (`docs(ai): document the soos-remote security hardening`): the ADR, `AI/ARCHITECTURE.md` §13,
  `Docs/REMOTE_COMPANION.md` (Tailscale 1.102.4 minimum, sender sandbox and self-check, auth-reserved connection,
  placement rule and SSH troubleshooting row, Android lock-screen privacy, renewal note, supported camera formats,
  residuals), `Docs/IPC_PROTOCOL.md` (row 6c-2a, admission reserve, remote frame size) and `Docs/DAEMON.md`.
- Traceability (this phase): `AI/VERIFICATION_MATRIX.md` component `remote-security-hardening` (RSH1–RSH18, brace
  lists written without spaces so that every cited test is resolved by the citation invariant);
  `Docs/REMOTE_COMPANION.md` §7 sender row gains the ` (deleted)` binary case (candid review suggestion); this
  walkthrough rewritten to the standard structure.

## 7. Candid Review

`AI/candid_review_report.md`: date 2026-10-12, base `f9234ea3c`, fingerprint
`a7fc77f519a1713bf485039034c733eb2c7b04eed6d2d429f739b50db69692e7`, **VERDICT: APPROVED**. No CRITICAL or MAJOR
finding. Findings and resolution:

- **MINOR** `preview_image.rs` lenient chroma: for an odd-width YUYV frame interior pixels can take chroma from the
  neighbouring row. Colour fidelity only (odd-width YUYV is not a real V4L2 layout); no bound or safety impact. Left
  as a follow-up (§9).
- **SUGGESTION** a same-UID process can repeatedly occupy the single reserve. Inside the trust boundary of the
  user-level lock screen; listed as a residual (spec §13.3) and a possible later per-UID cool-down.
- **SUGGESTION** a ` (deleted)` suffix from `current_exe()` makes the self-check refuse with exit 78. Fail-closed;
  documented in the `Docs/REMOTE_COMPANION.md` §7 sender row by this phase.

The report predates this traceability commit; the commit adds documentation only.

## 8. Verification Results

All on 2026-10-12:

- Phase 4 gate at `b3d8ca5` (exact CI commands): `cargo fmt --all -- --check` clean;
  `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` clean;
  `cargo test --locked --workspace --all-features`: 3403 passed, 0 failed, 6 ignored (pre-existing ignores);
  `cargo deny --locked check`: advisories, bans, licenses and sources ok;
  `cargo test --locked -p soos-daemon --all-features --test peer_limits_tests rsh`: 10 of 10 runs green;
  `./scripts/candid_review.sh` (layer 1): passed.
- This phase, `cargo test --locked --all-features` filtered on `rsh`: `soos-daemon` `peer_limits_tests` 12,
  `preview_placement_tests` 10, `preview_downscale_tests` 10, `preview_remote_view_tests` 2; `soos-remote`
  `http_tests` 1, `camera_server_tests` 1, `push_server_tests` 6; `soos-push-sender` `sandbox_tests` 14;
  `soos-invariants` 14 `remote_security_hardening_contract` tests and `test_rmc_s35_sender_unit_is_sandboxed`;
  all passed, 0 failed.
- `cargo test -p soos-invariants --locked --all-features`: 564 passed, 0 failed, including
  `test_matrix_claimed_rows_cite_only_existing_evidence` and
  `test_matrix_claimed_rows_cite_at_least_one_resolvable_evidence` over the RSH rows.
- `python3 scripts/sync_issue.py --check`: OK (GitHub-only review fix, no backlog sub-issue to tick).

## 9. Known Limitations / Follow-ups

- **RSH18 owner hardware check (pending)**:
  1. After reinstall, `systemctl --user show soos-push-sender -p InaccessiblePaths` lists both paths and a test
     notification still arrives.
  2. Running `~/.local/bin/soos-push-sender` by hand exits 78 ("sandbox not detected").
  3. With a live camera view **and** `soos-gui` open, the lock screen face unlock works.
  4. The `soos-gui` preview and the phone view still work; with an SSH session of the owner open, the phone view is
     refused (`camera_refused`) and works again after logout.
  5. A 640x480 MJPEG or 640x512 IR camera shows a live view.
  6. The Android notification icon and the iPhone home-screen icon still load (CORP).
  7. `tailscale version` reports 1.102.4 or later.
- Residual risks (spec §13): same-UID `/proc` and world-readable files remain visible to the sender; the proxy
  version cannot be detected; a same-UID process can hold the reserve for 200 ms per connection and a slow PAM
  conversation misses it; placements outside session scopes and the user manager keep the UID-level rule; a bind
  added under `/run` is not detected by the sender self-check; renewals without an old subscription keep the stale
  slot; the remote MJPEG decode runs inline (bounded, off the `Auth` path, no shared lock held).
- Follow-ups: row-aware chroma for odd-width YUYV on the remote path (candid MINOR); an optional per-UID cool-down on
  reserve admissions (candid SUGGESTION). Brace lists with spaces after the commas (`::{a, b}`) in older matrix rows
  are not split by the citation parser and are therefore not checked; a later traceability pass could normalise
  them.
