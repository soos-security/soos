# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/remote-auth-alerts`
- **Base (merge-base)**: `222665f`
- **Reviewed-Diff-Fingerprint**: `4e9001617229845f6b4f208cd9c4a7705b4c021a5a1e267503df50466f4a1b70`
- **Audited Files**: full branch diff vs `origin/main` (`target/candid_diff.patch`, 108 files). The
  branch up to HEAD `86ad4df` was approved under fingerprint `39c34d96…`; the push-topic delta was
  reviewed under `9bd0bbdd…` (CHANGES_REQUESTED: F-1 MAJOR, F-2 MINOR). This review re-checks the
  whole uncommitted delta against HEAD: `crates/remote/src/lib.rs`,
  `crates/remote/tests/push_tests.rs`, `crates/remote/tests/push_server_tests.rs`,
  `AI/architect_spec_remote_web_push.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/tester_contract_push.md`, `AI/research_push.md`, plus every remaining occurrence of the
  topic strings (`crates/remote/assets/sw.js`, `tests/invariants/src/remote_push_contract.rs`,
  `AI/auditor_constraints_push.md`, `crates/push-protocol/tests/protocol_tests.rs`,
  `crates/push-sender/tests/sender_tests.rs`).

## 1. Executive Summary

Both findings of the previous review are resolved and nothing else changed. The code delta
(`git diff HEAD -- crates`), re-read in full, matches the one described under `9bd0bbdd…` apart
from the two corrected `lib.rs` doc comments: `PUSH_TOPIC = "soosalerts"`, `PUSH_TEST_TOPIC = "soostest"`, const-checked
non-empty ASCII alphanumeric, distinct and `<= MAX_TOPIC_LEN`, with a new runtime regression test.
The topic migration is now recorded consistently in the architect spec (W-11, §3.1 example and
constant table with the alphanumeric rule, §6.4, tests 28/31 descriptions, RMC68, F-5), the Web
Push ADR, matrix row RMC68, the tester contract (Contract Migration 3, with hardware evidence and
exact line numbers that match the diff) and the research note. The `sw.js` display tags
`soos-alerts` / `soos-test` (spec §5.6 lines 912-913, auditor C-26, invariant test 48) correctly
remain unchanged, and the spec now says so explicitly. Targeted tests pass. **APPROVED.**

## 2. Test Changes

Mechanical listing over the frozen patch (`target/candid_diff.patch`):

- Removed assertion lines: one hit, `tests/invariants/src/presence_unlock_contract.rs`
  (`test_pau_zbus_is_used_only_by_the_daemon`), part of the branch already approved under
  `39c34d96…`; it is a recorded contract migration (ADR 2026-10-05 item (7), PAU17) that keeps
  and widens the check. Unchanged by this delta.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): none
  (the only hit is prose in a walkthrough stating none were added).
- Delta-specific test edits vs HEAD:
  - `push_tests.rs:102-103` and `push_server_tests.rs:577-578, 1047, 1383` (and doc comment
    1361): pinned topic values change from `soos-alerts`/`soos-test` to `soosalerts`/`soostest`.
    Exact-value strength kept. Justified by Contract Migration 3 in
    `AI/tester_contract_push.md` (Apple `400 BadWebPushTopic` vs `201`, hardware evidence) and by
    the updated RMC68 acceptance line in both the matrix and the spec. Line numbers in the
    contract entry match the diff.
  - New `test_rwp_topics_are_ascii_alphanumeric_for_apple` (`push_tests.rs:1002-1014`): fails
    against `soos-test` or an empty topic; strengthens the contract.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Tried: does any producer still emit a hyphenated topic? The only consumer of both constants is
  the dispatcher in `push.rs`; grep finds no other literal. `push-protocol` still accepts the RFC
  8030 alphabet, which is correct (protocol-level validation, not Apple policy) and its tests
  still use `soos-alerts` for that check, as documented. PASS.
- Tried: spec vs code drift. Spec §3.1 table, example JSON (243), §6.4 (713-714), tests 28/31
  (1090, 1103-1104), RMC68 (1243), F-5 (1338) all name `soosalerts`/`soostest`; ADR and matrix
  RMC68 agree and give the rationale. F-1 resolved. PASS.
- Tried: were the `sw.js` notification tags wrongly renamed? No: `sw.js:16,29` keep
  `soos-alerts`/`soos-test`, matching spec §5.6 (912-913), auditor C-26 and invariant test 48
  (`remote_push_contract.rs:680-681`). These are local `showNotification` tags, never sent to a
  push service. PASS.
- `const fn` recursion over a slice pattern is valid in const context; the empty-slice base case
  is guarded by `!bytes.is_empty()` in the caller. The F-2 comment now says the helper returns
  `true` for an empty slice and that `_` is excluded as a precaution, which is accurate. PASS.

### PAM Concurrency & Deadlines
- No change under `crates/pam`. N/A, PASS.

### Panic Safety & Fail-Closed
- The new checks are compile-time `const` asserts (a violation fails the build, never runtime).
  No new `unwrap`/`expect`/indexing in production. No path to `Allow`/`PAM_SUCCESS`. PASS.

### Test Integrity & Anti-Weakening
- Every changed assertion keeps exact-value comparison and is justified by a recorded contract
  migration with external evidence; a stronger property test is added. Verified:
  `cargo test -p soos-remote --test push_tests --test push_server_tests` green;
  `cargo test -p soos-invariants test_rmc_s38` green. PASS.

### Memory, Bounds & Secrets
- Topics are shorter, still `<= MAX_TOPIC_LEN` (const-checked). No secret, endpoint or key added
  to logs or docs. PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change in this delta. PASS.

### English-Only Policy
- All new text (comments, spec, ADR, matrix, contract, research) is English. PASS.

## 4. Detailed Findings & Action Items

- **F-1 (previous, MAJOR)** — resolved: spec, ADR, matrix RMC68, tester contract and research
  note record the `soosalerts`/`soostest` migration with evidence.
- **F-2 (previous, MINOR)** — resolved: `lib.rs` comments are accurate.
- **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md:2115` — the RMC68 evidence column could also cite
  `push_tests::test_rwp_topics_are_ascii_alphanumeric_for_apple` (it is named in the tester
  contract). Optional; traceability phase may add it.

## 5. Final Verdict

**VERDICT: APPROVED**
