# Candid Review Report

- **Date**: 2026-11-20
- **Target Branch**: `fix/remote-tailnet-identity`
- **Base (merge-base)**: `0cce947`
- **Reviewed head**: `6a7dd3f` (round 2)
- **Reviewed-Diff-Fingerprint**: `0d58cb446583bfbf5e7100b770d3a264ac09a59db36cb6e71cd7d1b6ad63cf86`
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
  (`AI/plan_evaluator_report.md` and this report are excluded from the fingerprint by the gate
  script.)

## 1. Executive Summary

Round 2 of the independent review of security review batch 2 (GitHub #479; findings #584,
#616, #583, #601, #605). The review was done from the raw diff, the batch issue and its five
finding issues, `AI/ARCHITECTURE.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` and the matrix
rows RTI1-RTI9. Scope: `crates/remote`, its web app, documentation and tests. No PAM, daemon,
FFI, packaging, dependency or CI file is touched.

- **#584 PTSN-03 is fixed.** There is now one limiter key and one bounded challenge sub-pool per
  allowed tailnet login. The camera and register routes use the caller's key too, and a
  challenge issued to one login is refused for another. The server regression test of the
  review (`test_rti2_one_login_cannot_lock_another_out`) fails against the old shared key
  (`429` instead of `200` for the owner).
- **#616 RWEB-02 is fixed**, and round 1 finding M1 is resolved. Sign out no longer calls
  `navigator.serviceWorker.register()`. It reuses the cached registration, which the start-up
  call at `app.js:2664` sets, or calls `getRegistration()`, which does not wait for an
  installation. Every failure falls through to the logout. A new browser case covers a
  `register()` that never settles.
- **#583 PTSN-02 is closed with a documentation fix.** This is an accepted residual (ADR
  2026-10-06 Web Push), with a written rationale in the ADR and the walkthrough.
- **#601 RIDN-01 and #605 RPSHB-01 are documented but not fixed in code.** They are deferred to
  the owner. Both fixes would change an existing pinned test:
  - for #601, RMC-S21 forbids reading `X-Forwarded-For` outside `client_hint`, and the existing
    tailnet route tests pin one-tap routes with no session;
  - for #605, PSS1 pins the exact line set of the system unit.

  I checked both pins in the code. The workflow rule requires stopping here for an owner
  question, so the deferral is correct. However, the chain-breaking part of #479 (CH-REMOTE-03,
  CH-LOCAL-07) is **not** delivered by this branch (see m1).

The full `soos-remote` and `soos-invariants` suites pass, as do `fmt` and `clippy -D warnings`.
I found no CRITICAL or MAJOR finding.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched:
  - `crates/remote/tests/common/harness.rs` (shared harness);
  - `crates/remote/tests/tailnet_caller_tests.rs` (new);
  - `tests/browser/specs/signin.spec.mjs` (two cases appended);
  - `tests/invariants/src/lib.rs` (one `mod` line added);
  - `tests/invariants/src/remote_tailnet_identity_contract.rs` (new).
- Removed or changed assertions, `#[test]`, `proptest!` or `#[should_panic]` lines: **none**
  (the grep is empty).
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.
- The only removed line under tests is a setup line:
  `harness.rs`: `allowed_logins: vec![TailscaleLogin::parse(LOGIN).unwrap()]`. It becomes
  `once(LOGIN).chain(options.extra_logins...)`, and `extra_logins` defaults to empty, so every
  existing harness user builds the identical configuration. `Via::TailnetAs` is additive. No
  assertion changed.
- `test_rti2_one_login_cannot_lock_another_out` changed between round 1 and round 2. The test
  is new on this branch, and the change only adds checks (round 1 m1). This is allowed.
- The batch-3 default migration exception was not used, and is not needed here.

## 3. Deep Reasoning Audit

### Logic & Architecture

Scenarios attempted:

- **Shared key through another path.** I listed every `LimitKey::` and `ChallengeBinding`
  construction in `server.rs` (lines 1145-1156, 1578, 1607-1610, 1647, 1755-1784, 1826, 1930,
  2370-2441). Each tailnet route goes through `ctx.unlock_key()` / `ctx.unlock_binding()`, and
  `RequestContext.tailnet` is `Some` exactly for `Caller::Tailnet`. No literal shared tailnet
  key remains (the compiler enforces this too, since the variant now carries data). **PASS**
- **Unresolved caller.** `TailnetCaller::of` uses the same equality as `authorize`, so it
  cannot be `None` for an authorized login. If it ever is, the request answers `403` before
  routing, so it fails closed. `u8::try_from` is used and `MAX_ALLOWED_LOGINS = 8` is enforced
  by `config.rs:904`. **PASS**
- **Pool bound.** A caller-bound entry counts against both its caller (4) and the whole
  `(Tailnet, purpose)` pool (`MAX_PENDING_TAILNET_CHALLENGES = 32`), and is never evicted. The
  `Funnel` and `Register` paths with a `Tailnet` binding give `Mismatch` (RTI1). **PASS**
- **Cross-login take.** The take removes the entry whatever the outcome, so login B could remove
  login A's challenge only by presenting its 32 random bytes, which B cannot know. **PASS**
- **Sign out.** I traced `requestLogout`:
  1. `dropPushSubscription` runs. If push is unsupported, it resolves `null`. If it is
     supported, it takes the cached registration, or `getRegistration()` (rejection caught),
     then `getSubscription()`, starts the browser `unsubscribe()` without awaiting it, and
     posts `POST /api/push/unsubscribe`, which needs the still-valid session on Funnel.
  2. The logout request is posted.
  3. `finally` shows "Signed out".

  The only awaited network step is one same-origin request, like the logout itself. The server
  route answers `429` at once when its 1 s gate is closed; it never waits. **PASS**, with one
  edge case noted in m3.
- **Scope.** Every change maps to a finding of #479. There is no scope creep. **PASS**

### PAM Concurrency & Deadlines

There is no change in `crates/pam`, so this pillar does not apply. **PASS**

### Panic Safety & Fail-Closed

- The production diff adds no `unwrap`, `expect`, `panic!` or indexing. `position()?`,
  `try_from().ok()` and `checked_add().unwrap_or` (pre-existing) are used. **PASS**
- No path turns an error into success. An unresolved caller gives `403`. A mismatched binding
  gives `passkey_rejected` and no unlock (RTI3 checks `unlock_ids().is_empty()`). Nothing
  reaches PAM. **PASS**

### Test Integrity & Anti-Weakening

- Section 2 shows that no existing assertion was removed or relaxed. **PASS**
- I checked by reasoning that the new tests fail on a plausible wrong implementation:
  - with a shared tailnet key, `test_rti2_one_login_cannot_lock_another_out` and
    `test_rti2_register_failures_count_on_the_callers_key` fail (`429` vs `200`);
  - with an unbound pool, `test_rti3...` fails (`too_many_challenges`) and so do the RTI1 store
    tests.

  Round 1 confirmed this by mutation (3 of 7 tests failed).
- The RTI4 invariant is a static source check; the behaviour is covered by the two RTI5 browser
  cases. **PASS**
- Not re-run here: I could not run the browser bench (RTI5). The pinned Playwright browsers
  (revision 1248) are not installed on this machine, and downloading them was out of scope. I
  checked statically that the first RTI5 case relies on the start-up `registerServiceWorker()`
  (`app.js:2664`) caching `pushRegistration`, because its stub container has no
  `getRegistration`. That holds. The walkthrough reports 9/9 green on three engines. CI is the
  authority (m2).

### Memory, Bounds & Secrets

- The new state is bounded:
  - the limiter maps gain at most `MAX_ALLOWED_LOGINS` tailnet keys;
  - the challenge store gains at most `2 x 32` caller-bound entries, and its doc comment is
    updated.
- `TailnetCaller`, `LimitKey::Tailnet` and `ChallengeBinding::Tailnet` have redacted `Debug`
  output (RTI1). The new `debug!` line carries no login. No frame, embedding or credential is
  touched. **PASS**

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. **PASS**

### English-Only Policy

All added code, comments, docs and test strings are in English. A grep for non-ASCII Latin
letters in added lines is empty.

Privacy:

- A grep of added lines for host paths, account names, tailnet names and `100.x.y.z` addresses
  finds only the generic CGNAT and MagicDNS ranges, which are public Tailscale constants.
- Nothing from private review material is referenced. Findings are named only by their public
  issue numbers and IDs. **PASS**

### Commands run by this reviewer

The build environment of the workflow rules was used for every command (per-worktree target
directory, `CARGO_INCREMENTAL=0`, no debug info, 3 jobs, `--test-threads=4`).

```text
cargo fmt --all -- --check                                                   ok
cargo clippy --locked -p soos-remote -p soos-invariants --all-targets --all-features -- -D warnings   ok
cargo test --locked -p soos-remote --all-targets --all-features              all green (tailnet_caller_tests 7/7)
cargo test --locked -p soos-invariants --all-targets --all-features          928/928 green (RTI4, RTI6-RTI9 included)
npx playwright test specs/signin.spec.mjs                                    not run: pinned browsers not installed locally
```

## 4. Detailed Findings & Action Items

- **[MINOR] m1 (carried from round 1, owner question, not blocking)** #601 RIDN-01 and #605
  RPSHB-01 are documented only (`Docs/REMOTE_COMPANION.md` section 2 "Known gap", section 2d).
  - The PR body must state plainly that the chain-breaking fixes of #479 (refuse
    self-originated tailnet requests, require a session on tailnet state-changing routes,
    `IPAddressDeny=`) are **not** delivered.
  - The PR must not use closing keywords for #479, #601 or #605.
  - The PR must ask the two owner questions explicitly (migrate RMC-S21 or the tailnet route
    tests; migrate PSS1).
  - Whichever #601 fix is chosen must respect caveat G201-01: interface addresses read at
    request time, with a short cache, failing closed; never a setup-written list.
- **[MINOR] m2** `tests/browser/specs/signin.spec.mjs` (RTI5, RTI5b) was not re-run by this
  reviewer (browsers missing locally). Confirm it is green in CI, or in the bench script, before
  merge.
- **[MINOR] m3** `crates/remote/assets/app.js:1209-1236`: the browser subscription is dropped
  even when `POST /api/push/unsubscribe` is refused, for example `429` from the 1 s push route
  gate when the user signs out right after another push action. In that case the PC record
  stays until the push service answers 404/410, or until `push remove N`. The phone receives
  nothing either way, because its browser subscription is gone, so this is not a leak. The
  record does use one of the 4 slots until it expires. Optional: document it, or ignore it.
- **[MINOR] m4 (residual, documented)** Register-body decode errors still count on the caller's
  unlock lockout, and three wrong enrollment codes still delete the code file. An existing test
  pins the first. The second is an anti-guessing bound. Both are recorded in walkthrough §9.3.
  Acceptable.
- **[SUGGESTION] s1** The `403 forbidden` branch for an unresolved tailnet caller
  (`server.rs:1275-1286`) has no test. `classify_request` cannot reach it today.

## 5. Final Verdict

No CRITICAL or MAJOR finding. The round 1 MAJOR (M1) is resolved. #584 and #616 are fixed and
reproduced by tests. #583 is closed with a rationale. #601 and #605 are deferred to the owner
under the workflow rule, and the PR text must say so (m1).

**VERDICT: APPROVED**
