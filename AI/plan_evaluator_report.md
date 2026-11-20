# Plan Evaluation Report
- **Date**: 2026-11-20
- **Issue**: GitHub #479 — security review 2026-11-10 batch 2 (findings #601, #584, #605, #616, #583; no backlog id)
- **Branch**: `fix/remote-tailnet-identity`
- **Base commit**: `0cce947`
- **Spec**: `AI/architect_spec_remote_tailnet_identity.md`

## 1. Coverage Matrix

| Acceptance line / regression test named by the review | Spec element | Status |
|---|---|---|
| #584: "with two allowed logins, five failures from login A must not make login B's unlock options answer 429" | `LimitKey::Tailnet(TailnetCaller)`, `unlock_key()` | Mapped (RTI2) |
| #584: the tailnet unlock / camera pools hold 4 entries and never evict (pool-full denial of service) | `ChallengeBinding::Tailnet`, per-caller pool, `MAX_PENDING_TAILNET_CHALLENGES` | Mapped (RTI1, RTI3) |
| #584: register routes hard-code the shared key | register routes use `unlock_key()` | Mapped (RTI2) |
| #584: register body errors in a separate bucket; per-device keys | — | Deferred: an existing test pins the shared accounting (`auth_server_tests`, "four malformed bodies leave the tailnet key open, the fifth locks it"), and a per-device key needs `X-Forwarded-For` (RMC-S21). Owner question. |
| #616: Sign out unsubscribes this phone and posts `/api/push/unsubscribe` before `/api/auth/logout` | `requestLogout` → `dropPushSubscription` | Mapped (RTI4, RTI5) |
| #583: section 2a revocation paragraph names `push remove` / `push reset`; section 8 names every signed-in caller | Docs | Mapped (RTI6) |
| #601: refuse self-originated tailnet requests / require a session on the tailnet path | — | Deferred, owner question (RMC-S21 and ADR 2026-10-06 F-2; tailnet one-tap tests). Section 2 documents the residual (RTI7). |
| #605: `IPAddressDeny=` in the sender units; correct the isolation sentences | Docs only | Unit deferred, owner question (PSS1 pins the exact unit lines); documentation corrected (RTI8). |
| ADR, walkthrough, matrix | §8 of the spec | Mapped (RTI9) |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `MAX_PENDING_CHALLENGES` | `crates/remote/src/lib.rs:224` | 4 | yes |
| `MAX_ALLOWED_LOGINS` | `crates/remote/src/lib.rs:79` | 8 | yes |
| `MAX_AUTH_FAILURES` | `crates/remote/src/lib.rs:246` | 5 | yes |
| `MAX_ENROLL_CODE_ATTEMPTS` (global, keyed by code hash) | `crates/remote/src/lib.rs:260`, `auth.rs::wrong_code` | 3 | yes |
| Tailnet key shared by every caller | `server.rs::RequestContext::unlock_key`, register routes `let key = LimitKey::Tailnet;` | yes | yes |
| Tailnet challenge binding `None` | `server.rs::RequestContext::unlock_binding` | yes | yes |
| `X-Forwarded-For` read only in `client_hint` | `tests/invariants/src/remote_passkey_contract.rs` RMC-S21 | enforced | yes (blocks the XFF fix) |
| Sender unit lines exact | `tests/invariants/src/push_sender_system_contract.rs` PSS1 | 47 exact lines | yes (blocks `IPAddressDeny=`) |
| Logout keeps push | `crates/remote/assets/app.js::requestLogout` | only `postJson(LOGOUT_PATH, …)` | yes |
| The spec file list of the bench is exact | `remote_playwright_contract::test_rpw_s3_specs_cover_the_rxb_behaviours` | 5 files | yes (the new test goes into `signin.spec.mjs`) |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: per-login keys weaken brute-force resistance (N logins give N×5
  failures). The failures guard passkey assertions (not guessable) and register bodies; the
  enrollment-code counter is global per code file (`wrong_code`), so code guessing stays at 3
  attempts in total. A caller cannot create a key (logins come from the 1..=8 validated
  configuration).
- Failure scenario: a challenge issued to login A taken by login B. The binding mismatches →
  `passkey_rejected`, entry removed (stricter than before).
- Residual: devices that share one login still share one key and one pool (Tailscale gives no
  per-node header other than `X-Forwarded-For`). Recorded.
- Result: PASS.

### Pillar 2 — PAM deadline & concurrency
- Not touched (no PAM, daemon or IPC change). Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario: `TailnetCaller::of` returns `None` (login authorized but not found, or a
  position beyond `u8`). The plan answers `403 forbidden` before routing; no default key. PASS.
- Failure scenario: the sign-out clean-up throws, or the push service never answers the browser
  unsubscribe. Every rejection is caught, the browser unsubscribe is not awaited, the only awaited
  step is one same-origin request (as bounded as the logout request itself), and the logout is
  always sent. PASS. (Revision 1 of the spec: a first design raced the clean-up against a new
  3 s timer; RMO-S9 pins the `app.js` timer set, so the design was changed instead of the test.)

### Pillar 4 — Dependencies
- No new dependency. Result: PASS.

### Pillar 5 — Data confidentiality
- `TailnetCaller` is a position, `Debug` redacted like every limiter type; no login reaches a log.
- Sign out sends this browser's own endpoint only (the body of *Disable notifications*).
- Result: PASS.

### Pillar 6 — Test integrity
- No existing test assertion changes. The harness gains an `Options` field (default empty) and a
  `Via` variant; store-level tests keep the legacy `None` pools, which stay valid.
- Test power: a single shared key fails RTI2 (login B gets `429`); a per-caller pool without the
  binding check fails RTI1's cross-login `take`; a logout without clean-up fails RTI4 and RTI5.
- Result: PASS.

## 4. Findings

- **[MINOR]** Two of the five findings (#601 code fix, #605 unit filter) cannot be delivered without
  migrating pinned invariants; the plan defers them with explicit owner questions instead of
  editing the tests. Accepted.
- **[MINOR]** The Playwright test cannot run in this environment (no `node_modules`, Docker image
  not pulled); RTI5 is also covered by the static invariant RTI4. Accepted; the browser CI job runs it.

## 5. Verdict
VALIDATION_VERDICT: APPROVED
