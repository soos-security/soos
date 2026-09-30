# Walkthrough 146 — P2 Review Follow-ups (Docs Accuracy, Quality-Gate Reports, EINTR, Lexer)

- **Date**: 2026-10-01
- **Issue**: GitHub #285 ("[P2-FU] Non-blocking follow-ups from the P2 batch reviews")
- **Branch**: `fix/p3-review-followups`
- **Matrix criteria**: RFX1–RFX13 (new component `p2-review-followups`); in-place updates of
  FIL3, VAY6, PCZ6 (now `⏹ Superseded`) and DRW5, because #285 names those rows explicitly
- **ADR**: 2026-10-01 "P2 Review Follow-ups (GitHub #285)"

---

## 1. Checklist Triage

| #285 item | Outcome | Row |
|---|---|---|
| `Docs/IPC_PROTOCOL.md` dropped facts (2500 ms, `StatusResponse` data, `PreviewResponse` zeroize) | Restored | RFX1 |
| Fixture `#[path]` prose (MOCK_STRATEGY, DECISIONS, FIL3, `fixtures_contract.rs` doc), fixture embeddings in AGENTS / ARCHITECTURE, `test_suite.sh` `stable` comment | Fixed (new ADR marks the old fixtures statement as historical instead of rewriting it) | RFX2 |
| `message.rs` module doc implied v1 readers accept tagged frames | Fixed; DRW5 wording clarified the same way | RFX3 |
| `soos-daemon.service` "Listen before the greeter" over-claim | Fixed (unit comment and `Docs/PACKAGING_AND_PROVISIONING.md`) | RFX4 |
| VAY6 "never copied" vs `.to_vec()` into zeroizing containers | Row reworded; new invariant: every `.to_vec()` in a run site is the direct argument of `Zeroizing::new(` / `BiometricEmbedding::new(` | RFX5 |
| PCZ6 still Pending | `⏹ Superseded (DRW3, DRW4, DRW6, BBX1–BBX4)` | RFX6 |
| `v4l_impl.rs` "same hints as the resolver" | Comment now describes the narrower supervisor hints | RFX7 |
| `candid_review.sh` filter misses block comments / raw strings; CI doc over-claims | Awk filter rewritten as a multi-line character lexer; doc updated | RFX8 |
| `soos-enroll` `FaceTooSmall` / `FaceBlurred` abort or generic error | Invalid candidate in `enroll`, `Deny` report in `verify` | RFX9 |
| `laplacian_variance` luma not zeroized; GUI spoof dropped without pose/embedding | Luma in `Zeroizing`; spoof counted via `record_presentation_attack` | RFX10 |
| `capture.rs` `Interrupted` counts as a full stall | EINTR never spends the stall budget | RFX11 |
| Mock daemon unknown tag parity | Unknown tag: no handler, no response, recorded `rejected unknown-tag` | RFX12 |
| `fault_injection_tests.rs` stale SAFETY comment / `with_pam_handle` migration | **Not done**: edits a pre-existing test (needs approval) | RFX13 |
| `import_from_reader` → `pub(crate)` | **Not done**: contract tests call it (needs approval) | RFX13 |
| admin-cli `test_pam_tests` 250 ms client timeout | **Not done**: re-timing a pre-existing test (needs approval) | RFX13 |
| Remaining partial issues (#195 ... #281) | Out of scope: tracked on their own issues / other clusters | — |

## 2. Specification

- `EnrollmentService::enroll`: `Err(VisionError::FaceTooSmall { .. } | FaceBlurred { .. })`
  pushes an empty `CandidateEvaluation` and `None` output (same as `NoFaceDetected`). When no
  candidate is valid, `select_best_frame` returns `LowQualityFrames`; nothing is stored.
- `EnrollmentService::verify`: `Deny` report, `face_count = 1`, `match_score = 0.0`,
  `pad_score = None`, `pad_threshold = None`, `pad_result`
  `FACE_TOO_SMALL(width=<w>px, min=<m>px)` or `FACE_BLURRED(sharpness=<s>, min=<m>)`.
  The values are geometry and sharpness only: no frame, embedding or credential is printed.
- `GuidedEnrollmentSession::record_presentation_attack(&mut self) -> EnrollmentStepFeedback`
  (public): same guards as `process_sample` (aborted → `SessionAborted`, completed →
  `AllStepsCompleted`), then the private `record_spoof`.
- `worker::feed_guided_enrollment`: the PAD liveness decision is computed first; a frame without
  pose or embedding but with a non-live verdict calls `record_presentation_attack`. All other
  paths are unchanged (existing `guided_liveness_feed_tests` untouched and green).
- `run_capture_loop`: the `next_buffer` error arm is split; `TimedOut` resyncs and spends the
  stall budget as before, `Interrupted` only sets `resync_pending` (parity with `wait_ready` and
  `resync`). The loop head re-checks `running` on every iteration, so shutdown stays bounded.
- `laplacian_variance`: luma plane held in `zeroize::Zeroizing<Vec<i32>>`.
- `mock_daemon.py`: a trailer `>= 0x80` other than `0xA0` / `0xA1` records
  `rejected unknown-tag` and closes without a response.
- `RUST_PRODUCTION_FILTER`: character lexer with state across lines (`blk` nesting depth,
  `in_str`, `in_raw` + hash count); line comments cut, block comments removed (one space kept),
  string / raw-string / char contents blanked, lifetimes kept; one output line per input line;
  the `#[cfg(test)]` item rule is unchanged.

## 3. Tests Written First (Red Evidence)

All new tests were run on the unmodified code first:

| Test | Red failure |
|---|---|
| `capture::tests::test_rfx_dequeue_interrupted_does_not_count_as_stall` | `Err(Starved)` instead of `Shutdown` |
| `guided_spoof_without_embedding_tests::*` (3) | `None` instead of `SpoofDetected` / `SessionAborted` |
| `quality_gate_report_tests::*` (5) | `Vision(FaceTooSmall { width_px: 10.0, min_width_px: 48.0 })` / `Vision(FaceBlurred { .. })` returned as errors |
| `candid_review_contract::test_rfx_candid_*` (4) | gate passed with a hidden production `unwrap`/`expect`, or failed on a commented-out panic |
| `review_followups_contract::test_rfx_*` (10) | stale prose present, PCZ6 Pending, VAY6 wording, luma not zeroized, mock answered an unknown-tag frame (41 bytes) |

The first draft of the raw-string case (`r#"say "hi" {"#`) passed by accident on the old filter
(the greedy `"[^"]*"` blanked the brace); it was replaced by `r#"a"{"#`, which fails on the old
filter. The RFX5 invariant first matched per line; a mutation (`let _leak = logits.to_vec();`
next to a `Zeroizing::new(`) slipped through, so the check now requires `.to_vec()` to be the
direct argument of the wrapper, and the same mutation fails it.

## 4. Audit

- No `unwrap`/`expect` added to production code; no PAM or FFI code touched; no new `unsafe`.
- No new allocation without bound; the luma buffer is now wiped.
- Diagnostic strings contain face size and sharpness only.
- The mock daemon change is test infrastructure; its socket mode rules are unchanged.
- `#![forbid(unsafe_code)]` crates untouched in that respect.

## 5. Gate

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
`cargo test --locked --workspace --all-targets --all-features --no-fail-fast` (1712 passed, 0 failed),
`./scripts/candid_review.sh`, `bash -n scripts/candid_review.sh tests/docker/test_suite.sh`.

## 6. Pending Approval (RFX13)

1. `crates/pam/tests/fault_injection_tests.rs::test_fault_inject_via_pam_hooks_returns_pam_ignore`:
   the `// SAFETY:` comment says the module never dereferences a handle below `0x10000`; the real
   guarantee is the trigger ordering pinned by PHS11. Proposed: reword the comment, or migrate the
   test to `common::with_pam_handle`.
2. `EnrollmentService::import_from_reader` stays `pub`; `pub(crate)` would break
   `import_stdin_tests`, `import_stdin_overwrite_tests` and `cli_hygiene_tests`.
3. `crates/admin-cli/tests/test_pam_tests.rs`: the 250 ms client timeout in
   `test_simulate_pam_auth_allow` and `test_simulate_pam_auth_deny_yields_pam_ignore` is
   load-sensitive; proposed `MAX_TIMEOUT_MS` (5000 ms).

## Follow-up after user approval (2026-10-01)

The user approved both pre-existing test edits: the SAFETY comment of `fault_injection_tests::test_fault_inject_via_pam_hooks_returns_pam_ignore` now cites the PHS11 trigger ordering, and the two verdict tests of `crates/admin-cli/tests/test_pam_tests.rs` use a 5000 ms client timeout (the offline-socket test keeps 250 ms). `EnrollmentService::import_from_reader` stays public; its documentation already names `import_with_overwrite_from_reader` as the `--yes`-gated CLI path. Matrix row RFX13 is ✅ Verified.
