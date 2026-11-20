# Candid Review Report

- **Date**: 2026-11-20
- **Target Branch**: `fix/remote-tailnet-identity`
- **Base (merge-base)**: `0cce947`
- **Reviewed-Diff-Fingerprint**: `221ea96925042723da2c09ae53d8915de2f46a558d8e1d81b7a1dfa02ddd4667`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/architect_spec_remote_tailnet_identity.md`,
  `AI/auditor_constraints_remote_tailnet_identity.md`,
  `AI/tester_contract_remote_tailnet_identity.md`,
  `AI/walkthroughs/258_remote_tailnet_identity.md`, `Docs/REMOTE_COMPANION.md`,
  `crates/remote/assets/app.js`, `crates/remote/src/auth.rs`, `crates/remote/src/challenge.rs`,
  `crates/remote/src/identity.rs`, `crates/remote/src/lib.rs`, `crates/remote/src/server.rs`,
  `crates/remote/tests/common/harness.rs`, `crates/remote/tests/tailnet_caller_tests.rs`,
  `tests/browser/specs/signin.spec.mjs`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/remote_tailnet_identity_contract.rs`
  (`AI/plan_evaluator_report.md` is excluded from the fingerprint by the gate script.)

## 1. Executive Summary

Round 1 of the independent review of security review batch 2 (GitHub #479; findings #584,
#616, #583, #601, #605). Scope: `crates/remote` (no PAM, daemon or FFI code is touched).

- **#584 PTSN-03** is fixed: one limiter key and one bounded challenge sub-pool per allowed
  tailnet login. The server tests fail when every caller is mapped back to one key (checked by
  mutation in this review: 3 of 7 tests fail).
- **#616 RWEB-02** is fixed in principle, but the new sign-out chain makes the logout request
  wait on `registerServiceWorker()`, a promise the existing code documents as possibly never
  settling. One MAJOR finding (section 4).
- **#583 PTSN-02** is closed as an accepted residual with a documentation fix and a written
  rationale. Acceptable.
- **#601 RIDN-01** and **#605 RPSHB-01** are documented only. Their code fixes are deferred
  to the owner because each needs an existing pinned test to change (RMC-S21 or the tailnet
  one-tap route tests for #601, PSS1 for #605). The workflow rules require exactly that
  (stop and ask the owner), so this is not a blocking finding. It does mean this PR does not
  break chains CH-REMOTE-03 and CH-LOCAL-07. #479, #601 and #605 must stay open.

Tests run by the reviewer with the per-worktree build env: `cargo test -p soos-remote
--all-targets --all-features` all green; `cargo test -p soos-invariants --all-targets
--all-features` 928 passed; `cargo clippy -p soos-remote -p soos-invariants --all-targets
--all-features -- -D warnings` clean. The browser bench (RTI5) was not run locally.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `crates/remote/tests/common/harness.rs`,
  `crates/remote/tests/tailnet_caller_tests.rs` (new), `tests/browser/specs/signin.spec.mjs`
  (one test appended), `tests/invariants/src/lib.rs` (one `mod` line added),
  `tests/invariants/src/remote_tailnet_identity_contract.rs` (new).
- Removed or changed assertion, `#[test]`, `proptest!` or `#[should_panic]` lines: **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: none.
- `harness.rs`: fixture-only additions. `Options.extra_logins` defaults to empty, and the
  allowlist built for the default case is identical to before (`[LOGIN]`). `Via::TailnetAs` is
  handled in `with_cookie` and `headers`. No existing expectation changed. The owner's batch-3
  default-migration exception is not used and not needed.

## 3. Deep Reasoning Audit

### Logic & Architecture

- `TailnetCaller::of` uses the same equality as `identity::authorize`, and `allowed_logins`
  is deduplicated after lowercasing (`config.rs:74`), so positions are unique. It is capped at
  `MAX_ALLOWED_LOGINS = 8` and converted with `u8::try_from` (no cast). PASS.
- Every consumer of the old `LimitKey::Tailnet` / `ChallengeBinding::None` was updated:
  `unlock_key()` and `unlock_binding()` are used by unlock options and verify
  (`server.rs:1607-1610`, `1755`, `1784`), camera options and start (`2370-2373`, `2405`,
  `2441`), and both register routes (`1826`, `1930`). No literal tailnet key remains. PASS.
- Scenario: login B presents a challenge issued to login A. `ChallengeBinding::matches` gives
  `Tailnet(a) != Tailnet(b)`, so the result is `Mismatch` and the entry is removed. RTI3
  answers `403 passkey_rejected` with no unlock. PASS.
- Scenario: a Funnel request carries a tailnet binding. `pool_of` has no
  `(Funnel, _, Tailnet(_))` arm, so the result is `Mismatch` (RTI1). PASS.
- Scenario: an authorized login has no position (allowlist inconsistency). The request is
  refused `403` before routing (fail closed). This path cannot be reached with today's
  `classify_request` and is not tested. Acceptable.
- Scenario: sign-out while the service-worker registration is pending or stalled. **FINDING
  M1** (section 4).
- Scope: limited to the batch. Nothing in batches 8, 9 or 16 is touched.

### PAM Concurrency & Deadlines

- No file under `crates/pam` changed. No Tokio is added outside `crates/remote`. N/A, PASS.

### Panic Safety & Fail-Closed

- No `unwrap`, `expect`, `panic!` or indexing in the production changes. `TailnetCaller::of`
  returns `Option`. The `MAX_PENDING_TAILNET_CHALLENGES` product is a compile-time constant
  (clippy is clean).
- No error becomes an `Allow` or a success. The unresolved caller is refused, and the challenge
  binding mismatch rejects the ceremony. PASS.

### Test Integrity & Anti-Weakening

- No existing assertion was removed or changed (section 2).
- Mutation run by the reviewer: every tailnet caller was forced to position 0 in
  `handle_connection`. `test_rti2_one_login_cannot_lock_another_out`,
  `test_rti2_register_failures_count_on_the_callers_key` and
  `test_rti3_challenge_pool_of_one_login_is_not_the_owners` then fail. The store-level RTI1 and
  RTI2 tests pin the per-caller pool and the key separation. PASS.
- RTI4 is a static shape test. RTI5 (browser) asserts the request order and the endpoint, but
  only for a registration that resolves at once. Nothing covers a pending or failing
  registration (see M1).

### Memory, Bounds & Secrets

- Pools stay bounded and never evict: at most 4 entries per login and 32 per tailnet
  unlock or camera pool. Limiter maps have at most 8 tailnet keys.
- `Debug` of `TailnetCaller`, `LimitKey::Tailnet` and `ChallengeBinding::Tailnet` is redacted
  (RTI1).
- The 403 path logs only `"tailnet caller not resolved"`, never the login. PASS.

### Supply Chain & Automation

- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/`, `.githooks/` or packaging change. N/A.

### English-Only Policy

- All added code, comments, documentation, the ADR, the walkthrough and the commit message are
  in English.
- No owner-host material: no login name, host path, tailnet name or address of a real machine.
  The `100.x` and `fd7a:` values are the public Tailscale ranges. Findings are cited only by
  public issue numbers and IDs. PASS.

## 4. Detailed Findings & Action Items

- **[MAJOR] M1** `crates/remote/assets/app.js:1209` (`dropPushSubscription`), together with
  `requestLogout` at `app.js:1228-1235`.
  - **Defect.** The logout request now runs only after `registerServiceWorker()` settles.
    That function issues a fresh `navigator.serviceWorker.register()` whenever
    `pushRegistration` is still `null`, for example when the start-up registration at
    `app.js:2650` has not finished or has failed. It has no bound, and the existing comment
    at `app.js:1383-1385` says so: "a register() that never settles". If it stalls (for example
    a slow `sw.js` fetch over Funnel on a mobile network), `POST /api/auth/logout` is never
    sent, the `finally` never runs, and the phone keeps a valid web session after the user
    tapped Sign out.
  - **Why it blocks.** Before this diff, logout was sent at once in every case. The ADR item
    (2), section 2b ("the session ends in every case") and the RTI4 docstring ("bounded") all
    claim the opposite of the new behaviour.
  - **Required correction (production code only; the RTI4 needles stay satisfied).**
    1. Do not register a worker during sign-out. Use the cached registration, or look up an
       existing one without waiting for installation. For example:
       `(pushRegistration !== null ? registerServiceWorker() : navigator.serviceWorker.getRegistration().then(function (r) { return r || null; }))`.
       Alternatively, reuse the existing timer-bounded `readyRegistration()`, which adds no new
       timer (RMO-S9) but changes an RTI4 needle.
    2. Keep `.catch` so that a rejection still reaches the logout.
    3. Add a browser case (RTI5 file) where `register()` never settles and no registration
       exists, and assert that `/api/auth/logout` is still posted and "Signed out" is shown.
- **[MINOR] m1** `crates/remote/tests/tailnet_caller_tests.rs:719-720`: the comment says "The
  owner's failures do not lift or extend login A's lockout", but the test records no owner
  failure before advancing the clock. Either record owner failures, or reword the comment.
  This is a new test, so adding a check is allowed; do not remove the existing assertion.
- **[MINOR] m2 (owner question, not blocking)** #601 RIDN-01 and #605 RPSHB-01 are documented
  but not fixed. The deferral follows the workflow rule, because each fix changes a pinned
  existing test (RMC-S21 or the tailnet route tests; PSS1). The PR body must therefore:
  - say plainly that the chain-breaking fixes of #479 (refuse self-originated tailnet
    requests, `IPAddressDeny=`) are not delivered;
  - keep #479, #601 and #605 open;
  - put the two owner questions explicitly.
  The G201-01 caveat must be carried into whichever fix the owner picks: interface addresses
  read at request time with a short cache and fail closed, never a setup-written list.
- **[MINOR] m3 (residual, documented)** Register-body decode errors still count on the
  caller's unlock lockout, which the review asked to separate. An existing test pins this.
  It is recorded as a residual in the ADR and walkthrough §9. Acceptable for this round.
- **[SUGGESTION] s1** The `403 forbidden` branch for an unresolved tailnet caller
  (`server.rs:1275-1283`) has no test. It cannot be reached through `classify_request`
  today, so this is optional.

## 5. Final Verdict

One MAJOR finding (M1): the sign-out logout can be blocked indefinitely by an unbounded
service-worker registration.

**VERDICT: CHANGES_REQUESTED**
