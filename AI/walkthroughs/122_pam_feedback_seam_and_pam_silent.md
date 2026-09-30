# Walkthrough 122 — PAM Feedback Seam, `PAM_SILENT` and Preview Fuzzing

- **Date**: 2026-09-30
- **Issues**: GitHub #220 (PAM-07, handle heuristic and missing tests), #221 (PAM-08,
  `PAM_SILENT` ignored, messages with no daemon), #227 (PAM-15, conversation / preview /
  fuzzing test gaps)
- **Branch**: `fix/p2-pam-handle-silent-tests`
- **Matrix criteria**: PHS1–PHS10 (new, component `pam-feedback-silent`)
- **ADR**: 2026-09-30 "PAM Feedback Seam, `PAM_SILENT` and Post-Connect Announcement"

---

## 1. Context

- `send_pam_info` and the `PAM_SERVICE` lookup skipped handles below `0x10000` only so that
  tests could pass a dummy `0x1000` pointer; `get_user` had no such guard and was reached only
  because every test passed `uid=`. Walkthrough 73 cited two tests that never existed.
- Both authentication entry points discarded `flags`, so `PAM_SILENT` callers (cron, some
  `su` / `sshd` paths) still received `PAM_TEXT_INFO`, and every `sudo` on a machine without a
  running daemon printed "Looking for face..." followed by "Face verification unavailable.".
- The conversation, the user lookup, `PreviewFrame` / `PreviewResponse` and the libFuzzer
  targets had no automated coverage; the fuzz harness could not even be built.

## 2. Design (Phase 1)

- `pub trait PamFeedback { fn info(&mut self, &str); fn user(&mut self) -> Option<String>;
  fn service(&mut self) -> Option<Vec<u8>>; }` in `crates/pam/src/lib.rs`, implemented for
  `PamHandle` (the only place that calls `get_item` / `get_user`) and for a private `Detached`
  value (null handle).
- `SoosPam::authenticate_with_feedback(&mut dyn PamFeedback, &PamConfig, PamFlag)` is the
  single flow; `authenticate_with_config(Option<&mut PamHandle>, &PamConfig)` is kept as the
  flag-less wrapper so every existing caller and test compiles unchanged.
- `PAM_SILENT` is a bit test (`flags & PAM_SILENT != 0`). The C ABI `int flags` is
  reinterpreted bit-for-bit as `PamFlag` (`from_ne_bytes`), never cast.
- `ipc::authenticate_with_progress(config, uid, on_connected)`: `on_connected` runs after a
  successful connect and before the request, inside the cumulative deadline.
  `ipc::authenticate` delegates with a no-op callback.
- The fuzz harness gets its own `[workspace]` table and a committed `Cargo.lock`; a new
  `decode_preview` target; `.github/workflows/fuzz.yml` runs every target nightly for
  `FUZZ_MAX_TOTAL_TIME` = 300 s and uploads `fuzz/artifacts/` on failure.

## 3. Tests first (Phase 2) and red evidence

| New test file | Red on `origin/main` (e2f602c) |
|---|---|
| `crates/pam/tests/pam_silent_tests.rs` (real `pam_start` handle, C ABI and `PamHooks`) | 7 of 8 failed: every `PAM_SILENT` case received 2 messages, both daemon-absent cases received "Looking for face..."; only the non-silent regression guard passed |
| `crates/pam/tests/pam_feedback_tests.rs` (recorder, 12 tests) | did not compile: `error[E0432]: unresolved import pam_soos::PamFeedback` |
| `tests/invariants/src/pam_feedback_contract.rs` (5 tests) | fail: no `impl PamFeedback for PamHandle`, `_flags` discarded, fuzz harness without `[workspace]`, no `fuzz.yml`, no `decode_preview` target |
| `crates/protocol/tests/preview_property_tests.rs` (4 tests) | passed immediately: test-gap only (no production change needed) |

`crates/pam/tests/common/mod.rs` holds the shared real-handle harness (capturing conversation,
one-shot mock daemon recording `uid_hint` and `service`).

## 4. Audit (Phase 3)

- No `unwrap` / `expect` / `panic` in production code; the adapter discards conversation
  errors; `catch_unwind` still wraps every entry point and the flow; `PAM_SUCCESS` only on
  `Verdict::Allow` (unchanged match).
- The `on_connected` callback runs inside the existing deadline: the conversation time is
  deducted from the budget, so the exchange stays bounded by the clamped `timeout_ms`.
- No new allocation beyond one copy of the `PAM_SERVICE` string (already bounded to
  `MAX_SERVICE_LEN` by `apply_pam_service`); no secret, frame or embedding logged.
- No Tokio, no OpenCV; `fuzz.yml` uses a read-only token, SHA-pinned actions
  (`upload-artifact` v7.0.1 SHA resolved with `gh api`), no persisted credentials, never the
  `fault-injection` feature.
- `pam_get_user` may still prompt when `PAM_USER` is unset, even under `PAM_SILENT`; this is
  the documented Linux-PAM behaviour of modules that need a user (as `pam_unix`).

## 5. Implementation (Phase 4) and gate

- `crates/pam/src/lib.rs`: trait + adapters, flag threading, silent check, post-connect
  announcement; `crates/pam/src/ipc.rs`: `authenticate_with_progress`.
- Green: all 8 + 12 new PAM tests, 4 protocol tests, 5 invariants; every pre-existing test
  unchanged and passing.
- Gate: `cargo fmt --all -- --check` OK; `cargo clippy --locked --workspace --all-targets
  --all-features -- -D warnings` OK; `cargo test --locked --workspace --all-targets
  --all-features --no-fail-fast` OK (exit 0); `./scripts/candid_review.sh` OK.
- Fuzzing (local, nightly `cargo-fuzz` 0.13.2): `decode_preview` 10.9 M runs in 21 s,
  `decode_request` / `decode_response` / `decode_event` 5 s each, no crash. Before the
  `[workspace]` table cargo refused to build the harness ("current package believes it's in
  a workspace when it's not").

## 6. Not done — test change proposals and decisions

The binding test-integrity rule forbids editing pre-existing tests, so two parts stay open:

1. **Remove the address heuristic (PHS4, #220).**
   `crates/pam/tests/pam_bindings_tests.rs::test_pam_hooks_authenticate_offline_daemon_returns_ignore`
   and `::test_pam_hooks_authenticate_password_failed_event_returns_ignore` build
   `let dummy_ptr = 0x1000 as *mut PamHandle;` and call `SoosPam::sm_authenticate(pamh, args, 0)`.
   Proposal: `mod common;` and
   `let (res, _) = common::with_pam_handle("soos-contract", |pamh| SoosPam::sm_authenticate(pamh, args, 0));`
   with the assertion unchanged. `test_pam_hooks_unhandled_hooks_return_ignore` never reaches
   libpam and can keep its dummy reference. Then delete `is_libpam_handle` and its three
   call sites, and flip PHS3/PHS4.
2. **Silence the "unavailable" line when the daemon is absent (PHS7, #221).**
   `crates/pam/tests/pam_handle_tests.rs::test_unavailable_feedback_is_a_single_generic_text`
   inserts the final message of the offline run (`unavailable.insert(final_message(&messages));`)
   into the set that must have exactly one element, so an empty conversation fails it.
   Proposal: replace that line with
   `assert!(messages.is_empty(), "an unreachable daemon stays silent, got {messages:?}");`
   and map `Err(IpcError::Connect(_))` to no message. This is also a UX decision: on a machine
   where soos is installed but the daemon crashed, the user would then get no hint at all.

## 7. Follow-ups

- Docker matrix case for `PAM_SILENT` and the absent socket (PHS10): `pam_test_runner` needs a
  flags option and a T-number coordinated with the parallel matrix work.
- `arb_request_kind` in `crates/protocol/tests/property_tests.rs` still omits `PreviewFrame`
  (pre-existing test); `preview_property_tests.rs` covers it with its own strategy.

## Correction (2026-09-30, candid review finding 2)

Proposal 1 above was approved and applied in commit `4f15282`: both `pam_bindings_tests` cases use
`common::with_pam_handle`, and `is_libpam_handle` is gone, so the guard is no longer "confined to
the adapter": it does not exist. Matrix PHS3 is superseded and PHS4 is `✅ Verified`, backed by the
new invariant `pam_handle_guard_removal_contract::test_pam_handle_address_heuristic_is_absent_from_pam_sources`.
The pre-existing `fault_injection_tests::test_fault_inject_via_pam_hooks_returns_pam_ignore` still
passes a synthetic `0x1000` handle (its SAFETY comment is now outdated, and the test is not edited
under the test-integrity rule). It is safe because `fault_injection::trigger` panics before any
libpam call. `crates/pam/src/lib.rs` documents this ordering next to the trigger, and the new
invariant `pam_handle_guard_removal_contract::test_fault_injection_trigger_runs_before_any_libpam_call`
(matrix PHS11) pins it. Proposal 2 (PHS7) is still pending.
