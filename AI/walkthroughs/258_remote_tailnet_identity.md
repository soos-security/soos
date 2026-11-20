# Walkthrough 258 — Per-Login Tailnet Limiters, Sign-Out Push Clean-Up, Tailnet Identity Residuals

- **Date**: 2026-11-20
- **Issue**: GitHub #479 (security review 2026-11-10, batch 2; tracking #477), no backlog id —
  **Branch**: `fix/remote-tailnet-identity`
- **Findings**: #584 PTSN-03, #616 RWEB-02, #583 PTSN-02, #601 RIDN-01, #605 RPSHB-01
- **Matrix criteria**: RTI1–RTI9 (RTI5 pending on the browser CI job)
- **ADR**: "[2026-11-20] Per-Login Tailnet Limiters, Sign-Out Push Clean-Up and the Tailnet Local
  Listener Residual"

## 1. Context & Objectives

Batch 2 of the 2026-11-10 review groups the findings about the tailnet identity of
`soos-remote`: other local accounts reaching the companion as the owner through `tailscaled`'s
local listener (#601), a compromised push sender doing the same over TCP (#605), one shared
limiter and challenge pool for every tailnet caller (#584), a push subscription that survives
*Sign out* (#616) and a revocation checklist that forgets the notification devices (#583).
The batch plan asked to refuse self-originated tailnet requests, require a session on the tailnet
path, separate the limiter keys, add `IPAddressDeny=` to the sender units and drop the push
subscription at sign-out.

## 2. Architect Design

Spec: `AI/architect_spec_remote_tailnet_identity.md`.

- `identity::TailnetCaller` (position of the login in `allowed_logins`, `u8`, `Debug` redacted)
  keys the tailnet limiter (`LimitKey::Tailnet(TailnetCaller)`) and binds the tailnet unlock and
  camera-view challenges (`ChallengeBinding::Tailnet`). Per-caller pools hold
  `MAX_PENDING_CHALLENGES` entries each, the pool at most `MAX_PENDING_TAILNET_CHALLENGES`
  (= 4 × 8 = 32), never evicting. The register routes use the caller's key instead of the literal
  shared one. An authorized login without a position is `403 forbidden` before routing.
- `app.js`: `requestLogout` runs `dropPushSubscription` first: the browser `unsubscribe()` is
  started and not awaited, `POST /api/push/unsubscribe` removes this browser's endpoint on the PC,
  then the logout, then the sign-in screen in every case.
- #601 and #605: documentation only, code deferred to the owner (section 9).
- **Revision 1**: the first draft raced the clean-up against a new 3 s timer. The full invariant
  run showed that `remote_motion_contract::test_rmo_s9_security_truth_first` (RMO-S9) pins the
  exact set of `app.js` timers, so the design changed: no timer, the push service is never
  awaited, and the only awaited step is one same-origin request, bounded like the logout request.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: `VALIDATION_VERDICT: APPROVED`, two MINOR findings: two findings
are deferred instead of migrating pinned tests, and the browser test cannot run locally. Facts
verified against code: `MAX_PENDING_CHALLENGES` 4, `MAX_ALLOWED_LOGINS` 8, `MAX_AUTH_FAILURES` 5,
the global enrollment-code counter, RMC-S21 and PSS1.

## 4. Tester Contract

`AI/tester_contract_remote_tailnet_identity.md`.

- `crates/remote/tests/tailnet_caller_tests.rs` (7 tests, RTI1–RTI3): caller identity and
  redaction, per-caller pools and their total bound, cross-login `take` refused, per-caller limiter
  keys at store level, and through the server the review's regression test (five failures of a
  second login leave the owner's unlock options, register options and unlock working), register
  failures on the caller's key, and a second login's full pool not reaching the owner.
  Red: every test failed on its assertion (`left: 429, right: 200`, `Err(Mismatch)` vs `Ok(0)`,
  equal keys) against stubs that compiled.
- `tests/invariants/src/remote_tailnet_identity_contract.rs` (5 tests, RTI4, RTI6–RTI9): the
  sign-out clean-up shape and order, the revocation steps, the local-listener gap paragraph, the
  sender TCP sentence, the limiter wording, ADR and walkthrough. Red: each panicked on its first
  missing needle.
- `tests/browser/specs/signin.spec.mjs`, "sign out drops this phone's notifications before ending
  the session" (RTI5): synthetic push registration installed before any page script; asserts the
  request order `push-unsubscribe` then `logout`, the endpoint body and one browser unsubscribe.
  The spec file list of the bench is pinned (RPW-S3), so the test was added to the existing file.
- Migrated existing tests: none. The shared harness gained `Options.extra_logins` (default empty)
  and `Via::TailnetAs(String)`; no assertion changed.

## 5. Auditor Constraints

`AI/auditor_constraints_remote_tailnet_identity.md`, A1–A10, `Clearance: CLEARED`:

- A1 `u8::try_from`, no cast or index (clippy `-D warnings` green).
- A2 fail closed on an unresolved caller (`403` before routing).
- A3 redaction of the caller, the limiter key and the binding (RTI1).
- A4 bounded pools and limiter maps (RTI1).
- A5 binding equality only within `Tailnet`, refused on Funnel and register (RTI1, RTI3).
- A6 enrollment-code counter and fresh assertions unchanged (existing suites green).
- A7 `X-Forwarded-For` untouched (RMC-S21 green).
- A8 sign-out clean-up never awaits the push service, adds no timer, always logs out (RTI4).
- A9 no dependency, unit, installer or wire change.
- A10 public issue numbers only, no host-specific content.

## 6. Implementation

- `crates/remote/src/lib.rs`: `MAX_PENDING_TAILNET_CHALLENGES`.
- `crates/remote/src/identity.rs`: `TailnetCaller` and `TailnetCaller::of`.
- `crates/remote/src/auth.rs`: `LimitKey::Tailnet(TailnetCaller)`.
- `crates/remote/src/challenge.rs`: `ChallengeBinding::Tailnet`, `Pool::Caller`, per-caller and
  total bounds, binding match.
- `crates/remote/src/server.rs`: `RequestContext.tailnet`, `unlock_key()`, `unlock_binding()`, the
  register routes on the caller's key, `403` on an unresolved caller.
- `crates/remote/assets/app.js`: `dropPushSubscription`, `requestLogout`.
- `Docs/REMOTE_COMPANION.md`: section 2 known gap (#601), section 2a revocation (#583), section 2b
  limiter keys and *Sign out* (#584, #616), section 2d sender TCP path (#605), section 8 wording
  (#583).
- `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md` (RTI1–RTI9), this walkthrough.

## 7. Candid Review

Not run in this session: the fingerprint-bound review (`./scripts/candid_subagent.sh --prepare`,
`AI/candid_review_report.md`) is done next by an independent reviewer with no conversation
context, before any push.

## 8. Verification Results

Environment: `CARGO_TARGET_DIR` per worktree, `CARGO_INCREMENTAL=0`, no debug info,
`CARGO_BUILD_JOBS=3`, `--test-threads=4`, `LIBONNXRUNTIME_NO_PKG_CONFIG=1`.

```text
cargo fmt --all -- --check                                                  ok
cargo clippy --locked -p soos-remote -p soos-invariants --all-targets --all-features -- -D warnings   ok
cargo test --locked -p soos-remote --all-targets --all-features            all green (tailnet_caller_tests 7/7)
cargo test --locked -p soos-invariants --all-targets --all-features        all green (remote_tailnet_identity_contract 5/5)
./scripts/candid_review.sh                                                 ok
```

The seven `tailnet_caller_tests` ran 10 times in a row, all green. The browser bench (RTI5) was not
run locally: `tests/browser/node_modules` is absent and the pinned Playwright image was not
pulled; the browser CI job runs it on Chromium, Firefox and WebKit.

## 9. Known Limitations / Follow-ups (owner questions)

1. **#601 RIDN-01, code fix.** Refusing tailnet requests whose `X-Forwarded-For` is one of the
   PC's own addresses (read at request time, fail closed, review caveat G201-01) makes that header
   an authorization input. ADR 2026-10-06 (F-2) and RMC-S21
   (`remote_passkey_contract::test_rmc_s21_forwarded_for_is_read_only_by_client_hint`) forbid it.
   Requiring a passkey session on the tailnet path instead replaces the one-tap tailnet routes that
   the existing lock, alerts and push tests pin. Either needs an owner-approved migration of those
   tests; the residual is documented in section 2 with interim advice.
2. **#605 RPSHB-01, egress filter.** `IPAddressDeny=` / `IPAddressAllow=` in
   `packaging/soos-push-sender@.service` changes the exact line set pinned by PSS1
   (`push_sender_system_contract::test_pss1_system_units_are_exact`). A proposed set: deny
   `localhost link-local multicast 0.0.0.0/8 100.64.0.0/10 fd7a:115c:a1e0::/48`, allow the
   systemd-resolved stubs `127.0.0.53/32 127.0.0.54/32` and MagicDNS `100.100.100.100/32
   fd7a:115c:a1e0::53/128` (LAN resolvers keep working; a resolver on another loopback address
   would need a drop-in). The legacy user unit may not apply it.
3. **#584 PTSN-03, rest.** Devices sharing one login share its key and pools (no per-node header
   besides `X-Forwarded-For`, see 1). Register-body decode errors still count on the unlock lockout
   of the same login: an existing test pins that shared accounting ("four malformed bodies leave
   the tailnet key open, the fifth locks it"). Three wrong enrollment codes still delete the code
   file (anti-guessing bound, kept).
4. **#616 RWEB-02, deeper fix.** Binding each push subscription to the passkey or session that
   created it (dropped at logout, session expiry or `passkeys remove`) changes the accepted Web
   Push risk and is left to the owner.
5. **#583 PTSN-02** is closed with the documentation fix as an accepted residual (ADR 2026-10-06
   Web Push, section 2d).
