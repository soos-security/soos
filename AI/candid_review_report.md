# Candid Review Report

- **Date**: 2026-10-01
- **Target Branch**: `fix/p3fu5-batch`
- **Base (merge-base)**: `9347449`
- **Reviewed-Diff-Fingerprint**: `4b3821766631c44bb830e7bbe9de738da0a9cb108964d3ed4c669544d7427744`
- **Fingerprint cross-check**: `./scripts/candid_subagent.sh --prepare` on the clean working tree
  and the same pinned `review_diff` command over `HEAD^{tree}` (commit `3a1dfb3`, the input of the
  `--rev` gate) both yield the fingerprint above.
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/walkthroughs/163_daemon_template_check_before_wake.md`,
  `AI/walkthroughs/164_sface_gui_spec_doc_followups.md`, `Docs/BIOMETRIC_STORE_CRATE.md`,
  `Docs/DAEMON.md`, `Docs/GUI_APPLICATION.md`, `Docs/INFERENCE_ORT_CRATE.md`, `Docs/POLICY_CRATE.md`,
  `crates/daemon/src/dispatcher.rs`, `crates/daemon/tests/template_check_order_tests.rs`,
  `crates/gui/src/app.rs`, `crates/gui/src/worker.rs`,
  `crates/gui/tests/match_reference_selection_tests.rs`,
  `crates/gui/tests/match_reference_zeroize_tests.rs`, `crates/inference-ort/src/embedding.rs`,
  `crates/inference-ort/tests/embedding_spec_layout_tests.rs`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/sface_followups_contract.rs` (19 files)

## 1. Executive Summary

This is the second review of GitHub #298. It covers the same diff as the review bound to
`e76646e6…`, plus commit `3a1dfb3`, which addresses that review's two MINOR findings.
`git diff 683d5e4 3a1dfb3 --stat` shows that the commit touches only four files:

- `AI/DECISIONS.md`: matrix range TCO1–TCO9, plus a precedence sentence.
- `AI/VERIFICATION_MATRIX.md`: new row TCO9.
- Walkthrough 163: the taxonomy now states the precedence, and the store-error row cites TCO9.
- `crates/daemon/tests/template_check_order_tests.rs`: new `Enrolled::CorruptFile` fixture case
  and the TCO9 test.

No production code changed after the first review. Both previous MINOR findings are resolved, and
no new defect was found.

**Part A (daemon).** Step 8 of `handle_request` now reads the enrolled template once and runs
`classify_template` before the atomic rate-limit reservation and the camera wake. A `Foreign`
template is answered `Unavailable`/`ModelUnavailable` without consuming an attempt or waking the
camera. Every other outcome still reserves exactly one attempt under the policy write lock before
any camera or vision work.

**Part B (GUI and embedding).**
- The live-match reference is held in a `Zeroizing` container.
- One helper swaps the reference and clears the score, with a consistent lock order.
- A failed lookup clears stale state.
- `OrtEmbeddingExtractor::with_spec` refuses non-NCHW specs.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched (all new on this branch relative to the merge-base):
  - `crates/daemon/tests/template_check_order_tests.rs`
  - `crates/gui/tests/match_reference_selection_tests.rs`
  - `crates/gui/tests/match_reference_zeroize_tests.rs`
  - `crates/inference-ort/tests/embedding_spec_layout_tests.rs`
  - `tests/invariants/src/sface_followups_contract.rs`
  - `tests/invariants/src/lib.rs` is not new: 5 added lines that register the module, nothing removed.
- Removed or changed `assert` / `#[test]` / `#[tokio::test]` / `proptest!` / `#[should_panic]`
  lines in the frozen patch: **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, `tolerance`, `epsilon`): **none**.
- Inline `mod tests` added or removed: **none**.
- Commit `3a1dfb3` only adds lines to a test file that is new on this branch:
  - one enum variant;
  - a match arm widened to `CurrentLiveIdentity | CorruptFile`, which builds the same template;
  - a post-enroll file overwrite gated on `CorruptFile`;
  - one new test.

  No pre-existing assertion of any test was touched.
- Rerun locally:
  - `template_check_order_tests`: 11/11 pass.
  - From the first review: `rate_limit_reservation_tests`, `sface_template_binding_tests`,
    `template_model_binding_tests`, the GUI selection and zeroize tests, and
    `embedding_spec_layout_tests` all pass.

## 3. Deep Reasoning Audit

### Logic & Architecture

- **Reservation bypass.** Only the `TemplateModelBinding::Foreign` branch returns before
  `record_attempt`, and it returns before any wake, capture or inference.
  - `Ok(None)`, `Err` and a current template all reach the unchanged write-locked reservation.
  - A rejected reservation returns `RateLimited`.
  - The consensus loop is reachable only with `Ok(Some(tmpl))` after the reservation.
  - → PASS.
- **TOCTOU with a concurrent root re-enrollment.** There is a single `biometric_store.get`, and its
  result is moved into `enrolled_template` and matched. No second read exists, so a re-enrollment
  cannot turn an uncharged request into an evaluated one. There is one template per UID. → PASS.
- **`expected_embedding_model == None`.** Classification is skipped as before. → PASS.
- **Earlier gates.** All of the following still return before step 8:
  - 5a wire validation;
  - 6 `SO_PEERCRED` versus `uid_hint`;
  - 6b local-session policy;
  - 6c preview routing;
  - 7 clock and deadline check.

  → PASS.
- **Oracle and attempt budget.** An unprivileged peer can only target its own UID. "Enrolled or not"
  enumeration is still charged (TCO4, TCO9). The single new observable is documented in the ADR: a
  rate-limited foreign-template requester gets `ModelUnavailable` instead of `RateLimited`, which
  concerns its own UID and repeats its first answer. Nothing that can produce a score bypasses the
  budget. → PASS.
- **Verdict precedence.** The ADR #298 bullet and the walkthrough 163 taxonomy now state that the
  template outcome wins over camera availability:
  - missing template or store error with a camera that cannot wake → `InternalError`;
  - foreign template with a cold camera → `ModelUnavailable`;
  - both were formerly `CameraUnavailable`, and all map to `PAM_IGNORE`.

  This matches the code. → PASS (previous MINOR-1 resolved).
- **NCHW refusal.** `with_spec` returns `InvalidInput` for a non-NCHW spec. The infallible `new`
  is guarded by a `const` assertion on `SHIPPED_EMBEDDING_MODEL.input_layout`. → PASS.

### PAM Concurrency & Deadlines

- `crates/pam` is untouched, and the PAM client deadline from the clamped `timeout_ms` still
  bounds the caller.
- On the daemon side the template read is the same size-bounded, authenticated `get`, moved
  earlier; `RequestDeadline` is threaded unchanged.
- The foreign, missing and store-error paths now skip the wake wait, so their latency only
  decreases.
- → PASS.

### Panic Safety & Fail-Closed

- No `unwrap`, `expect`, `panic!` or indexing was added in production code.
- The GUI recovers poisoned locks in `select_match_reference`. `update_live_match_score` skips
  publishing on a poisoned lock.
- Every refusal branch returns `Unavailable` or `ProtocolError`, never `Allow`.
- → PASS.

### Test Integrity & Anti-Weakening

- **TCO9 discriminates ordering.** It would fail if:
  - the store-error branch answered before the reservation (remaining attempts 5, not 4);
  - the branch were placed after the wake (wakes or captures 1);
  - inference ran (inferences 0).

  The corrupt file is written with the same length as the real file, so the envelope size bound
  passes. Envelope parsing or authentication then fails, which exercises the `Err` branch rather
  than `Ok(None)`: the file exists, so `Ok(None)` cannot occur. Both answer `InternalError`, but the attempt-count and wake assertions pin the
  ordering either way. The fixture resets the inference counter after enrollment, so
  `inferences == 0` is meaningful.
- → PASS (previous MINOR-2 resolved).

### Memory, Bounds & Secrets

- The reference is `Mutex<Option<Zeroizing<Vec<f32>>>>`, cloned from the
  `Zeroizing<Vec<f32>>` template embedding with no plain intermediate. The old `to_vec` copy is
  gone.
- Lock order is reference then score in both helpers; `app.rs:815` takes the score lock alone. No
  inversion is possible.
- Daemon logs carry the UID, model id and dimension only. No embedding, frame or credential is
  logged.
- → PASS.

### Supply Chain & Automation

- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. → PASS.

### English-Only Policy

- Non-ASCII characters on added lines are only dashes, arrows and status markers. All content is
  in English. → PASS.

## 4. Detailed Findings & Action Items

No CRITICAL, MAJOR or MINOR findings.

Previous findings (review bound to `e76646e6…`):
- **MINOR-1** (verdict precedence undocumented): resolved in `AI/DECISIONS.md` (#298 bullet) and
  walkthrough 163.
- **MINOR-2** (no store-error ordering test): resolved by
  `template_check_order_tests::test_store_error_is_refused_before_the_camera_wake_and_still_counts`
  (matrix TCO9).

Suggestions (optional):
- **[SUGGESTION]** `crates/daemon/src/dispatcher.rs` step 8a: if the template read ever becomes
  expensive, a read-locked pre-check of the remaining attempts could spare rate-limited requests
  that read. It must not replace the atomic reservation.
- **[SUGGESTION]** `crates/inference-ort/src/embedding.rs:294`: `is_nhwc()` can no longer return
  `true` now that non-NCHW specs are refused. A later cleanup could deprecate it; that would need a
  test migration, so it is out of scope here.
- **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md`: row TCO9 cites a `path.rs::test_name` form, while
  sibling rows use `module::test_name`. The citation resolves either way; the form is cosmetic.

## 5. Final Verdict

**VERDICT: APPROVED**
