# Auditor Constraints — soos-remote Security Hardening (security review of 2026-10-12)

- **Date**: 2026-10-12
- **Phase**: 3 (auditor-agent). GitHub-only security-review fix, no backlog id, not registered in
  `scripts/sync_issue.py`.
- **Branch**: `fix/remote-security-hardening` (from `origin/main` `f9234ea`); audited at `450ef73`.
- **Audited**: `AI/architect_spec_remote_security_hardening.md` (round 2, APPROVED `8883c1c`, PE-SR8 to PE-SR11
  folded in `bccaa28`), `AI/plan_evaluator_report.md`, `AI/tester_contract_remote_security_hardening.md` and every
  test file of `450ef73`; the code about to change: `crates/daemon/src/{limits,dispatcher,session_policy,
  preview_image,preview}.rs`, `crates/remote/src/{lib,http,server,camera,camera_slot,camera_ipc,push}.rs`,
  `crates/remote/assets/sw.js`, `crates/push-sender/src/{lib,main}.rs`, `crates/push-protocol` (exit codes only),
  `crates/vision/src/color.rs` (`convert_to_rgb`, MJPEG bounds), `packaging/soos-push-sender.service`,
  `scripts/install_remote.sh`.
- **Agent limits** (unchanged for Phase 4): never push, open a PR, run `./save.sh` or merge; never install, enable,
  start, stop or restart a unit; never run `tailscale`, `sudo` or `curl` against a live endpoint.

## Audit checklist results

| # | Check | Command / evidence | Result |
|---|---|---|---|
| 1 | Panic paths | `grep -rnE '\.unwrap\(\|\.expect\(\|panic!\|todo!\|unimplemented!\|unreachable!' crates/{remote,push-sender,push-protocol,daemon}/src` | 0 hits in production code. Workspace lints deny `unwrap_used`/`expect_used`/`panic`/`indexing_slicing`/`arithmetic_side_effects` under `-D warnings`. |
| 2 | Unsafe | `grep -rn unsafe crates/{remote,push-sender,push-protocol,daemon}/src` | `#![forbid(unsafe_code)]` in `remote/lib.rs`, `push-sender/{lib,main}.rs`; daemon only `mlock.rs` (documented, allowed). Nothing in this change needs `unsafe`. |
| 3 | Output isolation | `grep -rnE 'println!\|eprintln!\|dbg!'` over the push sender and the touched daemon files | 0 hits. PAM crate untouched. |
| 4 | Bounded I/O & deadlines | Read `dispatcher.rs` L416–590, `session_policy.rs` `read_bounded` (`MAX_CGROUP_FILE_SIZE` 16 KiB), `color.rs` MJPEG bounds | Frame length validated before allocation (unchanged, moves into `read_client_frame`); the reserve window is nested inside `connection_timeout`; mount table capped at 1 MiB; MJPEG decode bounded by `MAX_MJPEG_COMPRESSED_BYTES`/`MAX_MJPEG_DIMENSION` and header = declared size. |
| 5 | Arithmetic | Spec §4.2, §9.4 | Counters saturating (as today); decimation step via `div_ceil`/`max(1)`; output ≤ 640x480x3. |
| 6 | Filesystem safety | `install_remote.sh` L69–133 | S-8a confirmed: `[[ ! -e ]]` follows a dangling symlink. Fix (`-L` refusal + `set -C`) is sound; `umask 077` already precedes creation, so the file is 0600 at creation. |
| 7 | Secrets & privacy | Spec §4.3, §6.2, §8.3, §9.3 | New log lines carry `peer_uid` / constant text only; no endpoint, key, path or pixel data. |
| 8 | Fail closed | Spec §4.3, §6.1, §8 | Every new refusal closes without answer (S-3), returns `ProtocolError`/`UidMismatch` (S-5), or exits 78 (S-7). No path to `PAM_SUCCESS`; `pam_soos.so` untouched. |
| 9 | Supply chain | `cargo deny --locked check bans licenses sources` → `bans ok, licenses ok, sources ok` | Only change: daemon dev-dependency `jpeg-encoder` (workspace pin `=0.7.1`, already in `Cargo.lock`; one dependency-edge line added). No runtime dependency. |
| 10 | CI / workflow | — | `.github/`, `.githooks/` untouched; `scripts/install_remote.sh` is user-side only. |

## Test contract review

- **Amendments are additive only** (`git show 450ef73 --numstat`): every existing test file has 0 deleted lines except
  `tests/invariants/src/remote_push_contract.rs` (1 line: `[&str; 37]` → `[&str; 38]`), plus exactly one inserted
  element `"InaccessiblePaths=-/var/log -/var/lib/systemd/coredump"` after `"TemporaryFileSystem=/run:ro"`: this is
  CM-SR1 exactly (owner-approved); the forbidden list and the `soos-remote.service` assertion are unchanged. AM-SR1
  (`preview_remote_view_tests.rs`, one `write_session(.., "4", ..)` line) and AM-SR2
  (`preview_authorization_tests.rs`, own-session record when `/proc/self/cgroup` names a scope) are setup-only and
  touch no assertion. Verified.
- **No tautologies found.** Each static invariant fails today on a concrete missing item (re-run: the S-3 tests
  fail on `AUTH_RESERVED_CONNECTIONS_PER_UID ... not found`); behaviour tests are compile-red on exactly the
  specified API, and the tester showed assertion-level red under a wrong-value stub. Guards green today are
  declared as such and justified (RMC-S34 companion, "unchanged" contracts, the S-8b behaviour test with no
  deterministic seam backed by the static `test_rsh_s8b_*`).
- **Gaps found and closed by this audit (additive, new tests only):**
  - **A-1** `test_rsh_reserve_partial_frame_closed_within_window` passes even if the stalled connection is refused
    at admission (the tester noted it passes under the stub). New
    `crates/daemon/tests/peer_limits_tests.rs::auth_reserve::test_rsh_reserve_stalled_frame_holds_the_reserve_for_the_window`:
    the stalled connection must hold a permit (6 → 5 free) and be closed no earlier than
    `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS - 50` ms after `connect`, then release it. Verified: compile-red today
    on the missing API only; under the wrong-value stub it fails on an assertion (`permits must return to 5, got 8`);
    `cargo clippy --all-features --test peer_limits_tests -D warnings` clean under that stub (stub removed, `src/`
    unchanged).
  - **A-2** `test_rsh_s3_reserve_constants_and_dispatch` pins the `Auth`-only check before `handle_request(` but not
    before `handle_event(` (spec §4.3 step 3: `PasswordFailed` events are not admitted on the reserve; the socket test
    cannot observe a dropped event). New static
    `tests/invariants/src/remote_security_hardening_contract.rs::test_rsh_s3_reserve_refuses_events_before_processing`:
    after `decode_client_message(`, an `AdmissionClass::AuthReserve` check and `RequestKind::Auth` precede both
    `handle_request(` and `handle_event(`. Red today (`AdmissionClass::AuthReserve not found`).
  - These two additions are new test functions, not amendments; the tester contract's file list grows by them.
- **Accepted untestable items** (covered statically or by owner check): the `MAX_MOUNTINFO_BYTES` overflow path of
  `probe_sandbox` (no injection seam; static `.take(` needle + C-17), the S-8b post-head check (static), the real
  unit's mount table (RSH18-1/2).

## Audit Constraints

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-1 | `packaging/soos-push-sender.service` gains exactly `InaccessiblePaths=-/var/log -/var/lib/systemd/coredump` directly after `TemporaryFileSystem=/run:ro`, plus the header comment naming `/var/log`. No other directive is added, removed or reordered (no `TemporaryFileSystem=/var`, no `Environment=`, no `ProtectProc=`/`ProcSubset=`/`PrivatePIDs=`). | `packaging/soos-push-sender.service` | `test_rsh_s1_*`, test 45 (CM-SR1) |
| C-2 | No existing test is modified beyond CM-SR1, AM-SR1, AM-SR2 and nothing in the tests of `450ef73` or of this audit commit is edited. A failing test is fixed in production code. If an existing test must change, stop and report. | all `tests/`, `crates/*/tests/` | `git diff 450ef73.. --numstat -- '*/tests/*' tests/` shows no deletions; candid review |
| C-3 | Limiter: `try_acquire` is byte-for-byte unchanged in behaviour; `try_acquire_auth_reserve` checks `GlobalCapacity`, then the unprivileged capacity (`ReservedForPrivileged`), then the reserved count (`PerUidCap` at `AUTH_RESERVED_CONNECTIONS_PER_UID`), refuses root with `PerUidCap`, and mutates nothing on refusal. `Drop` releases the matching map entry (removed at zero) and the counters with saturating arithmetic; poisoned-lock recovery as today. `tracked_uids()` counts the union of both maps. | `crates/daemon/src/limits.rs` | `test_rsh_limiter_*`, `test_rsh_permit_reports_admission_class`, `test_limiter_*` |
| C-4 | The reserve constants are `pub const` literals (`1`, `200`), never a `daemon.toml` key or `PeerLimitsConfig` field. | `limits.rs`, `config.rs` | `test_rsh_s3_reserve_constants_and_dispatch` |
| C-5 | Dispatcher: the reserve is tried only after `Err(PerUidCap)` from `try_acquire`; every other rejection keeps today's warn + `ConcurrencyLimitReached` path. On a reserve permit, the first frame is read as `timeout(Duration::from_millis(AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS), read_client_frame(stream))` (the whole frame, prefix and body; no `.readable()`); the decoded message is refused with `DaemonError::Protocol(..)` before `handle_request`/`handle_event` unless it is a `Request` with `kind == RequestKind::Auth`. Nothing is written on any refusal; the connection is closed and the permit released. `read_client_frame` keeps the exact checks of today's Steps 2–3 (prefix, `> MAX_MESSAGE_SIZE \|\| == 0` before allocation, `read_exact` body). | `crates/daemon/src/dispatcher.rs::{handle_connection, read_and_process, read_client_frame}` | `test_rsh_s3_*` (both), `test_rsh_reserve_*`, `test_rsh_auth_admitted_on_reserve_*` |
| C-6 | A reserve connection serves at most one request: the `Auth` response sets `one_shot`, and no code path loops a second time on an `AuthReserve` connection (a completion error or any non-`Auth` ends it). The window is nested inside `connection_timeout` (`request_started` unchanged); a stalled reserve holds its permit at most the window. | `dispatcher.rs::handle_connection` | `test_rsh_reserve_auth_is_one_shot`, `test_rsh_reserve_stalled_frame_holds_the_reserve_for_the_window` (A-1), `test_rsh_reserve_idle_*`, `test_rsh_reserve_partial_*` |
| C-7 | New dispatcher log lines: reserve admission `debug!` and window elapse `warn!` carry `peer_uid` only and constant text; the text avoids the words `frame`, `password`, `image`, `embedding` (reword, never weaken an invariant). No request body, nonce or service string is logged. | `dispatcher.rs` | grep; invariant log audits; candid review |
| C-8 | Companion retry path: `drop(conn)` is replaced by `graceful_close(conn.stream).await` before `self.connect()`; no constant changes; no `drop(conn)` remains in `exchange`. | `crates/remote/src/camera_ipc.rs::DaemonPreviewClient::exchange` | `test_rsh_s3_reserve_constants_and_dispatch` |
| C-9 | Placement: `classify_peer_placement` tries `parse_session_id_from_cgroup` first, then `parse_user_manager_uid_from_cgroup`; a malformed manager path or disagreeing lines is an error (refusal). `authorize_preview_placement` follows spec §6.1 exactly; logind read error → `LogindUnavailable`; absent own record → `CallerSessionUnresolved` (fail closed). The record checks of `authorize_user_manager_caller` are factored into one private helper used by both paths; the root `Auth` path keeps the same denials in the same order. | `crates/daemon/src/session_policy.rs` | `preview_placement_tests.rs` (11 tests, incl. `test_rsh_root_auth_user_manager_rule_unchanged`), `session_policy_tests` |
| C-10 | Dispatcher placement wiring: the cgroup is read **once per connection** with the existing bounded reader (`MAX_CGROUP_FILE_SIZE`) and yields both origin and placement; any classification error refuses as today. `has_local_seat_session(peer_uid)` (6c-2) is kept; `authorize_preview_placement` (6c-2a) is evaluated on **every** preview request, after 6c-2 and before the preview rate limiter; refusal = `ProtocolError`/`UidMismatch`, one `warn!` with `peer_uid` and `denial.as_str()` only. Root peers skip 6c-2/6c-2a. The policy is the dispatcher's `session_policy` (disabled validator disables it). | `dispatcher.rs::{handle_preview_request, classify_preview_peer}`, `PreviewPeerState` | `test_rsh_s5_*`, `test_rsh_placement_rechecked_per_request`, `test_rsh_session_peer_dispatcher_wiring`, AM-SR1/AM-SR2 tests, `test_rlc_*` |
| C-11 | Hardening headers: `HARDENING_HEADERS` is the exact spec §7 constant, placed after an untouched `MANDATORY_HEADERS`; the three head encoders interpolate `{MANDATORY_HEADERS}{HARDENING_HEADERS}`; one CSP definition; no `includeSubDomains`/`preload`. | `crates/remote/src/http.rs` | `test_rsh_s6_*`, `test_rsh_hardening_headers_on_every_head`, RLC-S6 |
| C-12 | Sender self-check is pure and environment-free: `home_from_exe` uses component-wise `Path::ends_with(SENDER_EXE_HOME_RELATIVE)` and three `parent()` steps (never `str::ends_with`), rejects relative, empty, `/` and any `..` component. The sender's only environment read stays `std::env::var_os("XDG_RUNTIME_DIR")` in `main.rs`; no `home_dir`, `"HOME"`, `getenv`, `User::from_uid`, `getpwuid`, NSS lookup, no new crate. | `crates/push-sender/src/lib.rs::home_from_exe`, `main.rs` | `test_rsh_sandbox_home_anchor_unresolved_refused`, `test_rsh_s7_sender_reads_no_other_environment_variable`, RMC-S34 |
| C-13 | `evaluate_sandbox` rules in the specified order (anchor, `/run`, home, logs). Mount lines are split on single spaces; mount ID and parent ID parse as decimal `u64` or the line is skipped; mount points are octal-unescaped (`\040`, `\011`, `\012`, `\134`) before every comparison; `ro` is matched as an exact item of the **per-mount** options (field 6), never of the super options; fstype is exactly `tmpfs`; prefixes are component-wise (`Path::starts_with`), never textual. Order-independent over lines. | `push-sender/src/lib.rs::evaluate_sandbox` | `sandbox_tests.rs` (14 tests) |
| C-14 | Re-exposure rule (PE-SR9): a qualifying home tmpfs T is accepted only if none of its **direct children** (parent ID = T's mount ID) has a mount point equal to the home, a component-wise prefix of it, or below it, except exactly `<home>/.local/bin/soos-push-sender`. A child of another parent, or outside the home, does not change the verdict. | `evaluate_sandbox` | `test_rsh_sandbox_refuses_mount_over_hidden_home`, `test_rsh_sandbox_binary_bind_is_the_only_allowed_child` |
| C-15 | `SandboxError` variants have fixed texts and echo no path; `main` logs exactly `tracing::error!("push sender sandbox not detected; refusing to start")` (no reason, no format argument) and returns `exit_code(EXIT_CONFIG)`. `install_logging();` stays the first statement; the probe runs after `check_not_root` and before reading `XDG_RUNTIME_DIR` and `bind_socket` (no directory or socket is created before it). No override switch. | `push-sender/src/main.rs::main` | `test_rsh_s7_sender_checks_its_sandbox_first`, `test_rsh_sender_binary_exits_78_outside_its_sandbox`, RMC-S36/S42 |
| C-16 | `probe_sandbox` reads `SANDBOX_MOUNTINFO_PATH` with `take(MAX_MOUNTINFO_BYTES + 1)` (checked add) and fails closed (`MountInfoUnreadable`) when more than `MAX_MOUNTINFO_BYTES` were read or the content is not UTF-8; `current_exe()` error → `HomeUnresolved`; `/var/log` is only opened with `read_dir(..).is_ok()` (entries never iterated, read or logged; `NotFound` and `PermissionDenied` both count as hidden). | `push-sender/src/lib.rs::probe_sandbox` | `test_rsh_s7_*` (static needles), `test_rsh_probe_sandbox_refuses_an_unsandboxed_process`; candid review |
| C-17 | Installer: `[[ -L "${CONFIG_PATH}" ]]` branch prints to stderr and `exit 1` before any write; the template branch is `elif [[ ! -e "${CONFIG_PATH}" ]]`, keeps `umask 077` before `mkdir -p` and writes only with `(set -C; cat > "${CONFIG_PATH}") <<'TEMPLATE'`; template text unchanged; `chmod 0600` kept; the S-2 lines are `echo` only. The script never runs `tailscale`, `sudo`, `systemctl` other than the existing user-manager calls. | `scripts/install_remote.sh` | `test_rsh_s8a_*`, `test_rsh_s2_*`, RMC-S6, `bash -n` |
| C-18 | Tailscale minimum: `pub const TAILSCALE_MIN_VERIFIED_VERSION: &str = "1.102.4";` in `crates/remote/src/lib.rs`, read by no runtime code path; `1.38.3` removed from `Docs/REMOTE_COMPANION.md`. | `crates/remote/src/lib.rs`, docs | `test_rsh_s2_*` |
| C-19 | Stop re-checks: `runtime.stop_requested(ticket.view)` after `mark_streaming` and before the head (refusal through `camera_first_frame_refusal(.., ViewEnd::Stopped)`, `log_unshown_end`), and again after a successful head write and before `set_shown()`/`audit::camera_view_started()`/`write_part` (`audit_refused`, `finish_view(.., Stopped)`, `return None`). The JPEG and view buffers are dropped on both paths; no new log line with a format argument or a field named `token`/`width`/`height`/`bytes`/`sequence`/`len`. | `crates/remote/src/server.rs::camera_stream` | `test_rsh_s8b_*`, `test_rsh_stop_after_first_frame_sends_no_part`, RLC-S3 hygiene |
| C-20 | Renewal: `SubscribeBody` gains `#[serde(default)] replaces_endpoint: Option<String>` (still `deny_unknown_fields`, body limit 2048 unchanged); an unparsable old endpoint is treated as absent (no `400`, no oracle); a non-string value is `400`. `replace_or_upsert` performs removal and upsert inside **one** `PushStore::update` closure (one lock, one atomic write); capacity is counted after the removal; `TooMany` writes nothing; an old endpoint equal to the new one is a plain upsert. Answer `200 subscribed` on every success; audit `removed` then `added` for `Renewed`; no endpoint or key in logs. | `crates/remote/src/push.rs::{SubscribeBody, PushStore::replace_or_upsert, subscribe}` | `push_server_tests::renewal::*`, existing push tests |
| C-21 | Worker: `const SUBSCRIBE_BODY_LIMIT = 2048;`, `renewalPayload` as specified (size measured with `TextEncoder` on the exact JSON sent); `postSubscription` unchanged; exactly one `fetch(`; `replacesEndpoint` only in `renewalPayload`; `app.js` unchanged. | `crates/remote/assets/sw.js` | `test_rsh_s8c_*`, RAN-S2 |
| C-22 | Remote conversion: `remote_preview_image_for_frame` never calls the GUI `preview_image_for_frame`; its IR branch recurses into itself; output is Grey/YUYV(even width)/RGB24 within 640x480 or the explicit empty image; MJPEG decoded only through the bounded `soos_vision::convert_to_rgb`, the decoded buffer held in `Zeroizing`, every intermediate pixel buffer zeroized; all arithmetic checked/saturating, no indexing outside `get`. `preview_image_for_frame` stays bit-for-bit identical (`downscale_within(.., None)`). | `crates/daemon/src/preview_image.rs` | `preview_downscale_tests::remote_preview::*` (incl. proptest), existing downscale tests |
| C-23 | The remote conversion is called only in `handle_preview_request`, for `PreviewPeerOrigin::RemoteCompanion`, after the `preview_limiter` guard's block has closed, with no dispatcher/camera/pipeline lock held, and never on the `Auth` path. | `dispatcher.rs::handle_preview_request` | `test_rsh_s8d_*`, `test_rsh_remote_origin_*`, `test_rsh_local_origin_*` |
| C-24 | Docs, ADR and walkthrough per spec §2.3, §3.2, §4.5, §5, §6.4, §8.4, §9.4, §14 (incl. the PE-SR2 conversation dependency, PE-SR11 `tmux`/`loginctl` row, PE-SR10 `null` `oldSubscription`, §13.6 `/run` residual), English only. No `scripts/sync_issue.py` registration, no `AI/BACKLOG.md` entry. | `Docs/`, `AI/` | `test_rsh_docs_adr_and_walkthrough`, `test_rsh_s1_*`, `test_rsh_s4_*`, traceability phase |
| C-25 | Gate before hand-off: `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`, `cargo test --locked --workspace --all-features`, `cargo deny --locked check`; the `auth_reserve` socket tests run 10 times in a row green. Local commits only. | workspace | developer report, candid review |

### Observations (non-blocking, no spec change)

- **O-1** A mount table containing a non-UTF-8 mount point anywhere makes `probe_sandbox` fail closed (exit 78,
  Web Push disabled, never weakened). Accepted as specified (`MountInfoUnreadable`); mention it in the §2d *Limits*
  text if the developer touches it.
- **O-2** A same-UID process reconnecting in a loop can produce one reserve-window `warn!` per 200 ms; it can already
  produce one over-cap `warn!` per connection today. No new amplification class.
- **O-3** The preview placement cgroup is read through the peer PID of `SO_PEERCRED` once per connection, as the
  existing origin classification already does (PID-reuse race unchanged, same UID only).

## Pre-existing violations found (not introduced by this change)

None in the audited production code (0 panic macros, 0 undocumented `unsafe`, 0 stdout/stderr prints). The findings
S-1 … S-8d themselves are the pre-existing defects this change fixes.

## Clearance: CLEARED

The spec is consistent with the invariants, the test contract encodes every finding (with A-1 and A-2 closing the
two gaps found), and the only change to an existing test is the owner-approved additive CM-SR1 plus the
setup-only AM-SR1/AM-SR2. The developer may start, bound by C-1 … C-25.
