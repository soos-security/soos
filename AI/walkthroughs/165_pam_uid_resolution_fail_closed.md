# Walkthrough 165 — Fail-Closed PAM UID Resolution, `uid=` Match and PAM Review Follow-ups

- **Date**: 2026-10-02
- **Issue**: GitHub #300 (PAM-NEW-1, CRITICAL), #302 (PAM-NEW-2, MAJOR), #311 (PAM-NEW-3..7,
  minor group); review `AI/reviews/FULL_PROJECT_REVIEW_2026-10-02.md`, base `095eae9`; no
  backlog issue (GitHub-only findings, branch not registered in `scripts/sync_issue.py`) —
  **Branch**: `fix/pam-uid-resolution`
- **Matrix criteria**: PUR1–PUR20 (component `pam-uid-resolution`); PDR4 and PHS2 reworded

## 1. Context & Objectives

1. **#300**: `default_uid_resolver` (`crates/pam/src/lib.rs`) returned `libc::getuid()` whenever
   `pam_get_user` failed (including non-UTF-8 names), `CString::new` failed or `getpwnam_r`
   reported not-found, `ERANGE` past the cap or an NSS error. In a setuid-root host (`su B` run by
   A, `polkit-agent-helper-1 B`) the real UID is A's, so the daemon received `uid_hint = A`. For a
   root peer the daemon takes `uid_hint` as the target, the session binding passed for A, A's face
   matched, and the module returned `PAM_SUCCESS` for `PAM_USER = B`. The `password-failed` event
   was attributed to the wrong UID as well. The protocol docs wrongly called `uid_hint` a mere
   consistency assertion.
2. **#302**: `uid=` replaced `PAM_USER` without any comparison. In a shared stack, `uid=1000` let
   user 1000's face answer `su root`. `Docs/PAM_MODULE.md` and a log line recommended `uid=`.
3. **#311**: PAM-NEW-3 (tagged encoders allocate through `to_allocvec`, push and copy, so nonce
   copies were left behind and the size was checked only after allocation). PAM-NEW-4 (the
   docs and the ADR still described a generic daemon-absent message). PAM-NEW-5 (`.with`
   followed by `.borrow_mut()` in `take_panic_location`, which runs outside `catch_unwind`).
   PAM-NEW-6 (`getrandom::fill` can block before the CRNG is initialized). PAM-NEW-7 (security
   rejections were never logged, and the deadline started after the service and disable
   checks).

Owner decisions (2026-10-02) were relayed by the coordinator:
- The resolver returns `Option<u32>`. Any failure gives `PAM_IGNORE` without contacting the
  daemon, and no event on the event path.
- `uid=` is honoured only when it matches the UID `PAM_USER` resolves to.
- The `getuid()` stand-in is kept only on the null-handle path.
- Three assertion changes and the setup migrations were approved (§4).

## 2. Architect Design

- `SoosPam::authenticate_with_uid_resolver<R>` / `authenticate_flow<R>`:
  `R: FnOnce(&mut dyn PamFeedback) -> Option<u32>`. The resolver now runs exactly once per call,
  also when `uid=` is set.
- `fn target_uid(configured: Option<u32>, resolved: Option<u32>) -> Result<u32, TargetUidError>`
  with `TargetUidError::{Unresolved, UidArgumentMismatch}` and a value-free `log_message()`. Both
  errors give `PAM_IGNORE` before any socket activity, on both the authentication path and the
  event path.
- `default_uid_resolver` returns `feedback.user()?` and then `resolve_username_to_uid` (`None` on
  every failure). `detached_uid_resolver` returns `Some(getuid())`. It is used only by
  `SoosPam::authenticate_detached` (`pam_sm_authenticate(NULL, ..)` and
  `authenticate_with_config(None, ..)`). libpam never passes a null handle.
- Flow order: fault-injection hook, `ExchangeDeadline::start`, `with_pam_service`,
  `is_disabled()`, `resolve_uid`, `target_uid`, then the event path or the auth path.
- `soos_protocol::codec::encode_with_limit_and_trailer<T>(msg, max, trailer: Option<u8>)`. The
  payload is sized with `ser_flavors::Size`, and an oversize payload plus tag is refused before
  allocation. The frame is one exact-size `vec!`, serialized with `to_slice`, with the tag
  written in place and the buffer zeroized on both error arms. `encode_with_limit` delegates
  with `None`, and `message::encode_tagged` with `Some(tag)`.
- `pam_soos::ipc`:
  - `MAX_RANDOM_ATTEMPTS = 8`.
  - `fill_random_with(buf, source)`: `EINTR` and short reads are retried, bounded. `EAGAIN`, any
    other error and a zero-byte read give `IpcError::Random`.
  - `fill_random_nonblocking` calls `libc::getrandom(.., GRND_NONBLOCK)`.
  - `IpcError::Random` now carries `std::io::Error`. The `getrandom` crate dependency of
    `soos-pam` was dropped.
  - `IpcError::security_log_message() -> Option<String>`, value-free, for `RequestIdMismatch`,
    `StaleResponse` and `UnsupportedVersion`. `lib.rs` logs it at `LOG_WARNING`.
- `syslog::take_panic_location`: `try_with` + `try_borrow_mut`, `None` on failure.
- Invariants touched:
  - ARCHITECTURE invariant 5 (no error path to `PAM_SUCCESS`).
  - ADR "Local Session Binding" (`uid_hint` is authoritative for root peers).
  - ADR "PAM Deadline Derived From Clamped `timeout_ms`".
  - #225 single-buffer encoding.
  - PHS11 (the fault-injection hook stays first).
- ADR added: "[2026-10-02] Fail-Closed PAM UID Resolution; `uid=` Must Match PAM_USER". Item (5)
  of the 2026-09-30 ADR on PAM-09/16/17 was rewritten to the fail-quiet decision.

## 3. Plan Evaluation

The plan was condensed into the architect design above under the coordinator's instructions
(no separate plan-evaluator report). The owner decisions resolve the only open design choices:
the resolver type, the `uid=` semantics, and where the `getuid` stand-in survives.

## 4. Tester Contract

| Test (path::name) | Criterion | Red evidence (stub keeping the pre-fix flow) |
|---|---|---|
| `uid_resolution_fail_closed_tests::test_pur_real_handle_nonexistent_user_returns_ignore_without_daemon_contact` | PUR1 | `real handle: the daemon must not be contacted` |
| `uid_resolution_fail_closed_tests::test_pur_real_handle_nonexistent_user_rust_entry_and_event_path` | PUR1, PUR3 | panicked at `uid_resolution_fail_closed_tests.rs:107` (no-contact check) |
| `uid_resolution_fail_closed_tests::test_pur_real_handle_non_utf8_user_returns_ignore_without_daemon_contact` | PUR2 | daemon contacted |
| `uid_resolution_fail_closed_tests::test_pur_real_handle_nonexistent_user_sends_no_password_failed_event` | PUR3 | daemon contacted |
| `uid_resolution_fail_closed_tests::test_pur_real_handle_uid_argument_mismatching_pam_user_returns_ignore` | PUR4 | daemon contacted |
| `uid_resolution_fail_closed_tests::test_pur_real_handle_uid_argument_does_not_rescue_unknown_user` | PUR4 | daemon contacted |
| `uid_resolution_fail_closed_tests::test_pur_real_handle_uid_argument_matching_pam_user_authenticates` | PUR5 | passed (control) |
| `uid_resolution_fail_closed_tests::test_pur_resolver_failure_returns_ignore_without_daemon_contact` | PUR6 | daemon contacted |
| `uid_resolution_fail_closed_tests::test_pur_detached_path_matches_uid_argument_against_process_uid` | PUR7 | daemon contacted on the mismatch case |
| `pam_feedback_tests::test_feedback_unknown_user_returns_ignore_without_daemon_contact` | PUR2, PHS2 | daemon contacted |
| `pam_feedback_tests::test_feedback_uid_argument_mismatching_pam_user_returns_ignore` | PUR4, PHS2 | daemon contacted |
| `pam_feedback_tests::test_feedback_uid_argument_matching_pam_user_sends_request` | PUR5, PHS2 | `user_calls` 0 ≠ 1 (lookup skipped) |
| `pam_feedback_tests::test_feedback_unresolved_user_sends_no_password_failed_event` | PUR3 | daemon contacted |
| `deadline_uid_tests::test_uid_argument_requires_matching_resolved_uid` | PUR3, PUR6, PDR4 | resolver call count 0 ≠ 1 (`deadline_uid_tests.rs:324`) |
| `ipc_hardening_tests::test_pur_random_eagain_is_random_error_without_retry`, `ipc_hardening_tests::test_pur_random_eintr_and_short_reads_are_bounded` | PUR9 | stub returned `Ok(())` |
| `ipc_hardening_tests::test_pur_random_nonblocking_fills_the_nonce` | PUR11 | passed (stub used `getrandom::fill`) |
| `ipc_hardening_tests::test_pur_security_rejections_have_value_free_log_lines` | PUR10 | `RequestIdMismatch must be logged` |
| `ipc_hardening_tests::test_pur_ordinary_errors_are_not_security_log_lines` | PUR10 | passed (stub returned `None`) |
| `tagged_encoder_tests::*` (5 tests) | PUR12 | passed: the pre-fix encoder was already behaviourally correct (exact capacity, boundary). The defect (intermediate `to_allocvec` buffer) is only observable statically, so PUR17 carries the red evidence. |
| `pam_uid_resolution_contract::*` (8 invariants) | PUR13–PUR20 | all 8 failed (e.g. ADR item (5) `still produces the single generic ...`, `getuid` count 2 ≠ 1, `GRND_NONBLOCK` missing) |

Proof of the reviewer's PoC on `main` (`095eae9`):
`cargo test -p soos-pam --test pam_feedback_tests unknown_user` passed. The test
`test_feedback_unknown_user_falls_back_to_process_uid` asserted that a missing or nonexistent
PAM user produced a request with `uid_hint = <process uid>`. Against an `Allow` daemon that
request returns `PAM_SUCCESS` for a user the caller is not.

### Migrated existing tests (owner approval 2026-10-02)

Assertion changes:

| Test | Old assertion | New assertion |
|---|---|---|
| `pam_feedback_tests::test_feedback_unknown_user_falls_back_to_process_uid` → renamed `test_feedback_unknown_user_returns_ignore_without_daemon_contact` | for `None` and an unknown user: `PAM_IGNORE`, `user_calls == 1`, `seen.unwrap().uid_hint == process_uid` | for `None`, an unknown user and `"ro\0ot"`: `PAM_IGNORE`, `user_calls == 1`, no conversation, the daemon is never contacted (non-blocking listener) |
| `pam_feedback_tests::test_feedback_user_lookup_skipped_when_uid_argument_present` → renamed `test_feedback_uid_argument_mismatching_pam_user_returns_ignore` | PAM user `root`, `uid=4242`, Deny daemon: `user_calls == 0`, `uid_hint == 4242` | PAM user `root`, `uid=4242`: `PAM_IGNORE`, `user_calls == 1`, no conversation, no daemon contact; plus the new `test_feedback_uid_argument_matching_pam_user_sends_request` (`root` + `uid=0` → request with `uid_hint == 0`, `PAM_SUCCESS`) |
| `deadline_uid_tests::test_uid_argument_bypasses_the_resolver` → renamed `test_uid_argument_requires_matching_resolved_uid` | `uid=1000` on the event path: resolver never called, event `uid == Some(1000)` | resolver called exactly once; `Some(1000)` → event with `uid == Some(1000)`; `Some(1001)` and `None` → no event, `PAM_IGNORE` |

Setup-only migrations (no assertion touched):
- `deadline_uid_tests`: the resolver closures `|_| 4242` now return `Some(4242)`.
- `common/mod.rs` and the own harness of `pam_handle_tests`: the `pam_start` user changed from
  `soos-contract-user` (unresolvable) to `root`.
- `uid=1000` / `Some(1000)` changed to `uid=0` / `Some(0)` in:
  - `pam_handle_tests` (`base_config`, explicit-service test)
  - `pam_silent_tests`
  - `pam_fail_quiet_tests`
  - `pam_bindings_tests` (two hook tests)
- `pam_feedback_tests`: the recorders get PAM user `root` (`root_recorder()`) and `uid`
  `Some(0)`.
- `response_expiry_tests`: on the null-handle path, `uid=` is the current process UID.

Also added in `common/mod.rs`: `with_pam_handle_as`, `silent_listener`, `assert_never_contacted`
and the `CONTRACT_USER` / `CONTRACT_UID` constants.

Matrix rows PDR4 and PHS2 were reworded to the new behaviour, citing the renamed tests with
`*(formerly ...)*` annotations.

### Flakiness check

The four touched `soos-pam` test binaries were run 10 times in a row:
`uid_resolution_fail_closed_tests`, `pam_feedback_tests`, `deadline_uid_tests` and
`ipc_hardening_tests`. All 10 runs passed.

## 5. Auditor Constraints

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| 1 | No path from an unresolved user or a `uid=` mismatch to a socket connect, a request or an event | `lib.rs::authenticate_flow`, `target_uid` | PUR1–PUR7 (non-blocking listener, `accept` → `WouldBlock`) |
| 2 | `getuid` only in `detached_uid_resolver` | `lib.rs` | PUR13 |
| 3 | No user name and no UID in the new log lines | `TargetUidError::log_message`, event-budget line | code review (static strings; the event line prints only durations) |
| 4 | No unwrap/expect/panic or indexing in PAM production code; `unsafe` only with `// SAFETY:` | `ipc.rs::fill_random_nonblocking`, `syslog.rs` | clippy `-D warnings`, candid layer 1 |
| 5 | Nonce filling is bounded (≤ 8 syscalls) and never blocks | `ipc.rs::fill_random_with` | PUR9, PUR18 |
| 6 | Encoder: size before allocation, single buffer, zeroize on both error arms | `codec.rs::encode_with_limit_and_trailer` | PUR12, PUR17 |
| 7 | Security log lines never render a nonce, stamp or version number | `IpcError::security_log_message` | PUR10 |
| 8 | Fault-injection hook still runs before any libpam access; deadline starts before service, flags and resolution | `authenticate_flow` | PHS11, PUR20 |
| 9 | No threads or async in `crates/pam` | crate | unchanged (grep) |

Pre-existing violations found: none. Clearance: CLEARED.

## 6. Implementation

- `crates/pam/src/lib.rs`:
  - resolver type `Option<u32>`, `target_uid` / `TargetUidError`, `detached_uid_resolver` and
    `authenticate_detached`;
  - deadline moved before `with_pam_service`;
  - "configure uid=" removed from the event-budget log line;
  - security rejections logged at `LOG_WARNING`;
  - module docs updated.
- `crates/pam/src/ipc.rs`: `MAX_RANDOM_ATTEMPTS`, `fill_random_with`, `fill_random_nonblocking`,
  `IpcError::Random(std::io::Error)`, `IpcError::security_log_message`.
- `crates/pam/src/syslog.rs`: panic-free `take_panic_location`.
- `crates/pam/src/config.rs`: doc of `PamConfig::uid`.
- `crates/pam/Cargo.toml` and `Cargo.lock`: `getrandom` dependency of `soos-pam` removed.
- `crates/protocol/src/codec.rs`: `encode_with_limit_and_trailer`; `encode_with_limit`
  delegates to it.
- `crates/protocol/src/message.rs`: `encode_tagged` delegates to the codec helper (the
  `Zeroizing` import is gone).
- `crates/protocol/src/types.rs`: `Request` / `uid_hint` docs.
- Docs:
  - `Docs/PAM_MODULE.md`: UID resolution, the new fail-closed rule, fail-quiet daemon-absent
    text, non-blocking nonce, logged rejections, detached stand-in.
  - `Docs/IPC_PROTOCOL.md`: `uid_hint` description, tagged encoder sentence.
  - `AI/DECISIONS.md`: item (5) rewritten, new ADR.
  - `AI/VERIFICATION_MATRIX.md`: PUR section after TCO; PDR4 and PHS2 reworded.
- Tests:
  - new `crates/pam/tests/uid_resolution_fail_closed_tests.rs`,
    `crates/pam/tests/ipc_hardening_tests.rs`, `crates/protocol/tests/tagged_encoder_tests.rs`
    and `tests/invariants/src/pam_uid_resolution_contract.rs`;
  - migrations as listed in §4.

Notable decisions:
- The resolver runs even when `uid=` is set. That is the only way to compare the argument with
  `PAM_USER`, so `uid=` no longer avoids a slow NSS lookup.
- `EAGAIN` is not retried: spinning until the CRNG is ready would only burn the PAM budget.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) passed. Per the coordinator's instructions, the layer 2
sub-agent review, `save.sh`, the push and the PR are left to the coordinator, so no fingerprint
is recorded here.

## 8. Verification Results

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --all-targets --all-features -p soos-pam -p soos-protocol -p soos-daemon
  -p soos-admin-cli -p soos-invariants -- -D warnings`: clean.
- `cargo test --locked --all-features -p soos-pam -p soos-protocol -p soos-daemon -p soos-admin-cli
  -p soos-invariants`: 1126 passed, 0 failed, 2 ignored (117 test binaries).
- `ORT_SKIP_DOWNLOAD=1 cargo check --locked -p soos-pam -p soos-protocol --all-targets
  --all-features --target i686-unknown-linux-gnu`: clean.
- `./scripts/candid_review.sh` (layer 1): PASSED.
- `python3 scripts/sync_issue.py --check`: OK (no branch registration: GitHub-only findings).
- PAM Docker matrix: `tests/docker/test_suite.sh` in the `soos-sandbox` image, run on a tar copy
  of the source (to avoid root-owned `target/` files, #308): "ALL IN-CONTAINER PAM MATRIX TESTS
  PASSED SUCCESSFULLY!". In particular, T1 and T6 still authenticate `testuser` by face (PAM
  user resolved through NSS, no `uid=`), T11 delivers exactly one `PasswordFailed` event, and
  T10, T12, T13, T14 and T15 keep the password fallback.

## 9. Known Limitations / Follow-ups

- A `PAM_USER` that NSS cannot resolve (including a transient LDAP/SSSD failure) now always
  falls back to the password. This is accepted in the ADR.
- `uid=` no longer avoids the NSS lookup on the `event=password-failed` line. A slow lookup is
  still bounded by the documented budget rule and logged.
- `IpcError::Random` changed its payload type from `getrandom::Error` to `std::io::Error`. No
  consumer outside `soos-pam` matches on it.
