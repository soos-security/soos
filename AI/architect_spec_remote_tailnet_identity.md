# Architect Spec — Security Review Batch 2 (GitHub #479): tailnet identity, per-caller limiters, sign-out push clean-up

- **Date**: 2026-11-20
- **Issue**: GitHub #479 (review 2026-11-10, batch 2, tracking #477); findings #601 RIDN-01, #584
  PTSN-03, #605 RPSHB-01, #616 RWEB-02, #583 PTSN-02. No backlog id.
- **Branch**: `fix/remote-tailnet-identity` (the batch issue suggests `fix/remote-self-origin`; the
  branch name is the orchestrator's).
- **Matrix rows**: RTI1–RTI9 (new component `remote-tailnet-identity`).

## 1. Scope & Blast Radius

| Finding | Decision in this PR | Why |
|---|---|---|
| #584 PTSN-03 | **Fixed in part**: limiter keys and the tailnet unlock / camera challenge pools are per allowed login. | The batch item "separate limiter keys so one tailnet caller cannot lock the owner out", and the finding's own regression test (two allowed logins). |
| #616 RWEB-02 | **Fixed**: *Sign out* drops this phone's push subscription (browser and PC) before ending the session, best effort and bounded. | Client-side fix named by the finding and its verifier. |
| #583 PTSN-02 | **Closed with rationale + documentation fix**: accepted residual (ADR 2026-10-06 Web Push, Docs section 2d); the section 2a revocation list now names `push list` / `push remove N` / `push reset`, and the section 8 sentence names every signed-in caller. | Verifier: INFO documentation mismatch over an accepted residual. |
| #601 RIDN-01 | **Documented, code fix deferred (owner question)**. | Both fixes the review names change a pinned contract: the `X-Forwarded-For` self-origin refusal makes that header an authorization input, which invariant RMC-S21 (`remote_passkey_contract::test_rmc_s21_forwarded_for_is_read_only_by_client_hint`) and ADR 2026-10-06 F-2 forbid; the session requirement on the tailnet path breaks every existing tailnet lock / alerts / push test (one-tap tailnet routes of ADR 2026-10-06). Section 2 of the companion documentation now states the residual. |
| #605 RPSHB-01 | **Documented, unit change deferred (owner question)**. | `IPAddressDeny=` in `packaging/soos-push-sender@.service` changes the exact unit line set pinned by PSS1 (`push_sender_system_contract::test_pss1_system_units_are_exact`). The documentation isolation sentences now state the TCP path. |

Crates and files: `crates/remote/src/{identity.rs, auth.rs, challenge.rs, server.rs, lib.rs}`,
`crates/remote/assets/app.js`, `crates/remote/tests/{common/harness.rs, tailnet_caller_tests.rs}`,
`tests/invariants/src/{lib.rs, remote_tailnet_identity_contract.rs}`,
`tests/browser/specs/signin.spec.mjs`, `Docs/REMOTE_COMPANION.md`, `AI/DECISIONS.md`,
`AI/VERIFICATION_MATRIX.md`, walkthrough 258. No other crate depends on `soos-remote`; no wire,
config key or installed file changes.

## 2. Types & Signatures

```rust
// crates/remote/src/identity.rs
/// Limiter and challenge identity of one tailnet caller: the position of its login in
/// `allowed_logins` (0..MAX_ALLOWED_LOGINS). `Debug` redacted.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TailnetCaller(u8);
impl TailnetCaller {
    /// `None` when `login` is not in `allowed` or its position does not fit (never in a valid config).
    #[must_use]
    pub fn of(login: &TailscaleLogin, allowed: &[TailscaleLogin]) -> Option<Self>;
}

// crates/remote/src/auth.rs
pub enum LimitKey {
    Tailnet(TailnetCaller),          // was the unit variant `Tailnet`
    FunnelSession,
    FunnelAnonymous(ClientHint),
}
// Debug: "LimitKey::Tailnet(<redacted>)".

// crates/remote/src/challenge.rs
pub enum ChallengeBinding {
    None, Login(ClientHint), WebSession([u8; 32]), EnrollCode([u8; 32]),
    /// One allowed tailnet login (tailnet unlock and camera view, review PTSN-03).
    Tailnet(TailnetCaller),
}
```

Pools: `(Tailnet, Unlock | CameraView, Tailnet(c))` is the per-caller pool of `c`: at most
`MAX_PENDING_CHALLENGES` entries bound to `c`, and at most
`MAX_PENDING_TAILNET_CHALLENGES` = `MAX_PENDING_CHALLENGES * MAX_ALLOWED_LOGINS` (32) entries in
the `(class, purpose)` pool; full → `PoolFull`, never evicts. `matches`: `Tailnet(a)` matches only
`Tailnet(b)` with `a == b`. The legacy `(Tailnet, Unlock | CameraView, None)` pool is unchanged
(store-level tests use it; the server no longer issues it).

`server.rs`: `RequestContext` gains `tailnet: Option<TailnetCaller>` (`Some` exactly for the tailnet
class); `unlock_key()` → `LimitKey::Tailnet(c)` / `FunnelSession`; `unlock_binding()` →
`Tailnet(c)` / `WebSession(hash)` / `None`; the register routes use `unlock_key()` instead of the
literal shared key. A tailnet caller whose `TailnetCaller` cannot be derived is `403 forbidden`
(fail closed; unreachable with a validated configuration).

Bounds: `TailnetCaller` < `MAX_ALLOWED_LOGINS` (8, `u8`); limiter maps hold at most
`MAX_ALLOWED_LOGINS + 1` non-anonymous keys per limiter; challenge store at most
`3 × MAX_PENDING_CHALLENGES + 2 × MAX_PENDING_TAILNET_CHALLENGES + MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES`
entries (+ the unused legacy tailnet pools).

## 3. Constants & Config

- `MAX_PENDING_TAILNET_CHALLENGES: usize = MAX_PENDING_CHALLENGES * MAX_ALLOWED_LOGINS` in
  `crates/remote/src/lib.rs` (32). No config key.
- `app.js`: no new constant and no new timer (Revision 1: RMO-S9,
  `remote_motion_contract::test_rmo_s9_security_truth_first`, pins the `app.js` timer set; the
  first draft's 3 s race timer was dropped). The browser `unsubscribe()` runs in the background;
  the only awaited step is the same-origin `POST /api/push/unsubscribe`.

## 4. Error Taxonomy

No new HTTP result. Per-caller lockout and pool exhaustion keep `429 rate_limited` /
`429 too_many_challenges`; a challenge taken by another login is `403 passkey_rejected`
(`ChallengeError::Mismatch`). Sign-out push clean-up failures are silent; the logout is always sent.

## 5. Latency Budget

Auth path (PAM) untouched. Sign out waits for one same-origin request (the PC unsubscribe) before
the logout request; it never waits for the push service.

## 6. Invariants Touched

RMC-S21 (unchanged: `X-Forwarded-For` stays a Funnel rate-limit hint only), RMC32 / RLC challenge
pools (legacy `None` pools unchanged), F-2 limiter keys (now per login on the tailnet), RWEB push
lifecycle. Unchanged: every unlock and camera view needs a fresh UV assertion; Funnel keys.

## 7. Test Hooks for Tester

- Harness: `Options.extra_logins: Vec<String>` (appended to `allowed_logins`, default empty) and
  `Via::TailnetAs(String)` (tailnet request with that login). Additions only.
- Store-level: `ChallengeStore::issue/take` with `ChallengeBinding::Tailnet`,
  `AuthState::{record_failure, locked_out, record_issuance, options_exhausted}` with
  `LimitKey::Tailnet(TailnetCaller::of(..))`.
- App: `requestLogout` → `dropPushSubscription` (static invariant) and the Playwright sign-out test.

## 9. Revision 1 (after the first full invariant run)

The first draft bounded the sign-out clean-up with a new `setTimeout` race
(`LOGOUT_PUSH_TIMEOUT_MS`). `remote_motion_contract::test_rmo_s9_security_truth_first` pins the
exact set of `app.js` timers, so the design changed instead of that test: the browser
`unsubscribe()` (the only step that talks to the push service) is started and not awaited, and
the PC record is removed with one same-origin request before the logout.

## 8. Documentation Drift / ADR

- New ADR "[2026-11-20] Per-Login Tailnet Limiters, Sign-Out Push Clean-Up and the Tailnet Local
  Listener Residual" recording the decisions above, the two deferred owner questions and the
  documentation corrections (section 2 reason 2, section 2a revocation list, section 2d sender
  isolation, section 8).
- ADR 2026-10-06 Web Push item (3) isolation sentence is amended by reference (the TCP path), not
  rewritten.
