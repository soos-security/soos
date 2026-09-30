# Walkthrough 94 — PAM Cumulative Deadline, Neutral Feedback and PAM_SERVICE Flags

- **Date**: 2026-09-30
- **Issues**: Review findings PAM-02 (GitHub #173), PAM-03 (GitHub #174), PAM-05 (GitHub #176)
- **Branch**: `fix/pam-deadline-and-messages`
- **Matrix criteria**: PDM1–PDM3 (new); LSF3 annotated as superseded by PDM3

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, area PAM Module & IPC
Protocol) confirmed three MAJOR findings in `crates/pam`:

1. **PAM-02**: `read_exact_counted` looped on `read()` with `SO_RCVTIMEO` set only twice (before the
   header, before the body). The socket timeout bounds one syscall, so a daemon writing one byte
   every 70 ms kept `pam_sm_authenticate` waiting ~2.6 s with `timeout_ms=100`, and the late
   `Allow` was still honored.
2. **PAM-03**: the lock-screen text distinguished `(Deny, PadFailed)` ("Biometric spoof
   detected.") from other denials and `(Unavailable, CameraUnavailable)` from other failures, while
   `ReasonClass` is documented as never exposed to unprivileged users. That gives a presentation
   attacker an oracle and discloses camera state.
3. **PAM-05**: `is_disabled()` checked `/etc/soos/gdm.disable` only when `service` contained `gdm`,
   but `service` came solely from a `service=` argument that no installed line passes. The module
   never read `PAM_SERVICE`, so `soos-admin gdm disable` had no effect (and the daemon's
   GDM-keyed `notify_activity()` never fired).

Objectives: one cumulative deadline per IPC exchange, verdict-class-only user messages, and a
service name taken from `PAM_SERVICE` so the disable flags work — all without touching
`crates/admin-cli/src/gdm.rs` (parallel install batch) or the daemon.

## 2. Architect Design

- `crates/pam/src/ipc.rs`:
  - private `Deadline { start, total }` with `remaining()` (wraps `remaining_budget`).
  - `read_exact_before_deadline(stream, buf, expected_total, already_received, deadline)` and
    `write_all_before_deadline(stream, bytes, deadline)`: re-arm `set_read_timeout` /
    `set_write_timeout` with `deadline.remaining()?` before every syscall, retry `EINTR` only
    while budget remains, `Ok(0)` → `TruncatedResponse` / `WriteZero`.
  - `authenticate` and `notify_event` use them; `authenticate` calls `deadline.remaining()?`
    after decoding so a verdict finished after the deadline is `IpcError::Timeout`.
- `crates/pam/src/config.rs`:
  - `DEFAULT_FLAG_DIR = "/etc/soos"`; `PamConfig::flag_dir` (not settable from PAM arguments) and
    `PamConfig::service_from_args` (set when a non-empty `service=` is parsed).
  - `PamConfig::apply_pam_service(&[u8])`: no-op if `service_from_args`; UTF-8, trimmed, bounded
    to `MAX_SERVICE_LEN` on a char boundary; blank/invalid keeps the current value.
  - `is_disabled()`: `flag_dir/disabled`, `disable_file`, `flag_dir/gdm.disable` when the service
    contains `gdm`, and `flag_dir/<service>.disable` for names made of `[A-Za-z0-9._-]` not
    starting with `.`.
- `crates/pam/src/lib.rs`:
  - `usable_handle` (shared low-address guard `>= 0x10000`, also used by `send_pam_info`).
  - `with_pam_service`: clones the config and applies `pamh.get_item::<items::Service>()` —
    resolved before `is_disabled()`, inside `catch_unwind`.
  - `feedback_message(&Result<(Verdict, ReasonClass), IpcError>)`: Allow → "Face recognized.
    Unlocking...", Deny → "Face not recognized.", everything else → "Face verification
    unavailable.". `PAM_SUCCESS` still only on `Ok((Verdict::Allow, _))`.

## 3. Plan Evaluation

Checked against `AGENTS.md`, `AI/ARCHITECTURE.md` and project facts §2: no Tokio, blocking
`UnixStream` only, every blocking call now bounded by the clamped `timeout_ms` cumulatively (the
invariant as enforced), no new `unsafe`, no password or biometric data involved. No ADR is needed:
PAM-03 restores the documented `ReasonClass` invariant instead of revoking it. The per-service
flag is an additive, fail-safe control (it can only make the module return `PAM_IGNORE`).
Precedence: explicit `service=` over `PAM_SERVICE`, so existing custom lines keep their meaning.

## 4. Tester Contract (Red Phase)

API skeleton first (`flag_dir`, `service_from_args`, no-op `apply_pam_service`) so every test
failed on an assertion, not on compilation.

| Test | Red evidence |
|---|---|
| `ipc_tests::test_ipc_drip_fed_body_respects_cumulative_deadline` | FAILED (returned `PAM_SUCCESS`, suite took 2.78 s) |
| `ipc_tests::test_ipc_direct_drip_fed_body_returns_timeout` | FAILED (`Ok(Allow)` after ~2.6 s) |
| `ipc_tests::test_ipc_fragmented_body_within_budget_is_accepted` | passes (regression guard) |
| `config_tests::test_service_argument_marks_service_as_explicit`, `test_pam_service_item_used_when_no_service_argument` (`left: "pam_soos"`, `right: "gdm-password"`), `test_pam_service_item_is_bounded_and_validated`, `test_gdm_disable_flag_in_flag_dir_disables_every_gdm_service`, `test_global_disabled_flag_in_flag_dir_disables_everything`, `test_per_service_disable_flag_in_flag_dir` | FAILED |
| `config_tests::test_default_flag_dir_is_etc_soos`, `test_explicit_service_argument_wins_over_pam_service_item`, `test_per_service_flag_rejects_path_traversal_service_names` | pass (negative / precedence guards) |
| `pam_handle_tests::test_pam_service_item_forwarded_to_daemon_when_no_service_argument`, `test_gdm_disable_flag_honored_through_pam_service_item`, `test_per_service_disable_flag_honored_through_pam_service_item` | FAILED (daemon saw `pam_soos`; flag ignored, `PAM_SUCCESS`) |
| `pam_handle_tests::test_deny_feedback_is_identical_for_every_reason_class` | FAILED (`PadFailed` text differed) |
| `pam_handle_tests::test_unavailable_feedback_is_a_single_generic_text` | FAILED (three distinct texts incl. "Camera unavailable.") |
| `pam_handle_tests::test_explicit_service_argument_overrides_pam_service_item` | passes (precedence guard) |

`crates/pam/tests/pam_handle_tests.rs` is new: it creates a real Linux-PAM handle with
`pam_start` (non-existent service name, so no real stack is involved) and a capturing
conversation function, which is the conversation mock the review asked for (PAM-15). Every
failure pathway asserts `PAM_IGNORE`.

Docker matrix **T11** (`tests/docker/test_suite.sh`): PAM service `gdm-password` with the
`soos-admin` arguments (no `service=`), mock daemon in `allow` mode. Control run without the flag
must authenticate with 0 prompts; with `/etc/soos/gdm.disable` a no-password run must fail (prompt
reached), a valid password must pass and an invalid one must fail.

## 5. Auditor Constraints

1. No `unwrap`/`expect`/indexing in production PAM code; all new code uses `?`, `get`, saturating ops.
2. Every `read()`/`write()` re-arms its timeout from the single `Deadline`; zero remaining → `Timeout`.
3. A verdict decoded after the deadline is never honored.
4. `EINTR` retries are bounded by the deadline check at the top of the loop.
5. `pam_get_item` is only called on handles passing the `>= 0x10000` guard, inside `catch_unwind`.
6. The `PAM_SERVICE` value is bounded to 64 bytes, UTF-8 validated, never trusted as a path unless
   it matches `[A-Za-z0-9._-]+` without a leading `.`.
7. Service resolution happens before `is_disabled()` and before any socket activity.
8. User-facing texts depend on the verdict class only; no new log line contains `ReasonClass`,
   frames, embeddings or credentials.
9. No existing test modified (all additions); existing `PamConfig { .., ..Default::default() }`
   literals compile unchanged.

## 6. Implementation

See §2. `send_pam_info` now uses `usable_handle`; the `match` on the IPC outcome is split into
`feedback_message` (text) and a two-arm `match` (`Allow` → `PAM_SUCCESS`, everything else →
`PAM_IGNORE`). `notify_event` gained the same per-write re-arming within its 20 ms budget.

## 7. Documentation

- `Docs/PAM_MODULE.md`: cumulative deadline (§2, §6), new §7 service resolution and disable
  flags, new §8 user-facing messages table.
- `Docs/IPC_PROTOCOL.md`: `reason_class` never shown to users; cumulative client deadline.
- `Docs/PAM_DOCKER_TEST_MATRIX.md`, `Docs/CI_CD_AND_SECURITY.md`, `run_tests.sh`,
  `.agents/skills/dev-workflow/references/project-facts.md`: T1–T11, flag files row.
- `AI/BACKLOG.md` (#48 feedback spec) and matrix LSF3 annotated as superseded.

## 8. Verification Results

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked --workspace --all-targets --all-features --no-fail-fast` | all green except `soos-vision::bench_tests::test_pipeline_latency_budget_under_150ms_p95` (p95 217 ms at host load average ~40 from parallel agents; untouched crate, passes when re-run alone in 1.95 s) |
| `./scripts/candid_review.sh` | PASSED (7 audits) |
| `./run_tests.sh` (Ubuntu Docker matrix, scratch clone of the branch) | T1–T11 all passed |
| Docker red check: `origin/main` sources + new `test_suite.sh` | T11 FAILED as expected: "gdm.disable present but gdm-password still authenticated facially!" |

## 9. Known Limitations / Follow-ups

- The `PAM_SILENT` flag is still ignored for `PAM_TEXT_INFO` messages (separate review finding).
- Fedora / Arch Docker variants (`./run_tests.sh --matrix`) were not run locally; T11 is
  distribution-agnostic and runs there through `tests/docker/test_suite.sh` in CI.
- `soos-admin gdm status` still reports the flag purely from file existence; with this change the
  report is now accurate because the module honors the flag.
