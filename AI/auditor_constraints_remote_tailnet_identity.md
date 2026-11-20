# Audit Constraints — GitHub #479 (security review 2026-11-10 batch 2)

Spec: `AI/architect_spec_remote_tailnet_identity.md`. Tester contract:
`AI/tester_contract_remote_tailnet_identity.md`. Greps run on the affected files at the base
commit `0cce947`: no `unwrap`/`expect`/`panic!`/`unreachable!` in `identity.rs`, `challenge.rs`,
`auth.rs`; `#![forbid(unsafe_code)]` at the top of `crates/remote/src/lib.rs`; no `debug!`/`info!`
line of `server.rs` carries a login.

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| A1 | No panic path in new production code: the caller position is converted with `u8::try_from`, never cast or indexed. | `identity.rs::TailnetCaller::of` | `cargo clippy --all-targets --all-features -D warnings` (workspace lints deny `unwrap_used`, `indexing_slicing`, `cast_possible_truncation`) |
| A2 | Fail closed: a tailnet caller whose `TailnetCaller` cannot be derived is `403 forbidden` before routing; there is no default or shared fallback key. | `server.rs::handle_connection` | code review (candid), RTI2/RTI3 (distinct keys reach the routes) |
| A3 | Redaction: `TailnetCaller`, `LimitKey::Tailnet(_)` and `ChallengeBinding::Tailnet(_)` print `<redacted>`; no log line names a login or a caller position. | `identity.rs`, `auth.rs`, `challenge.rs`, `server.rs` | RTI1 `test_rti1_tailnet_caller_identifies_each_allowed_login`; RMC log-hygiene tests stay green |
| A4 | Bounded memory: a caller-bound tailnet pool holds at most `MAX_PENDING_CHALLENGES` per caller and `MAX_PENDING_TAILNET_CHALLENGES` in total; refuses (`PoolFull`) and never evicts; the limiter maps hold at most `MAX_ALLOWED_LOGINS + 1` non-anonymous keys (positions come from the validated configuration only). | `challenge.rs::ChallengeStore::issue`, `auth.rs::AuthState::window` | RTI1 `test_rti1_tailnet_challenge_pools_are_per_caller`, `test_rti1_tailnet_pool_total_is_bounded` |
| A5 | Binding: `Tailnet(a)` matches only `Tailnet(b)` with `a == b`; never `None`; the Funnel and register pools refuse it (`Mismatch`). The mismatched entry is removed (single use). | `challenge.rs::ChallengeBinding::matches`, `pool_of` | RTI1, RTI3 |
| A6 | Brute-force bounds unchanged: the enrollment-code counter stays global per code file; every unlock and camera view still needs a fresh UV assertion; Funnel keys unchanged. | `auth.rs::wrong_code`, `server.rs` | existing `auth_server_tests`, `camera_server_tests`, `direct_biometric_tests` stay green |
| A7 | `X-Forwarded-For` stays a Funnel rate-limit hint only (no new reader). | `identity.rs`, `server.rs` | `remote_passkey_contract::test_rmc_s21_forwarded_for_is_read_only_by_client_hint` |
| A8 | Sign-out clean-up: never waits on the push service (the browser `unsubscribe()` is not awaited), adds no timer (RMO-S9 pins the `app.js` timer set), its only request is one same-origin `postJson` like the logout itself; every rejection caught; the logout request and `showLogin("Signed out")` always follow; only this browser's own endpoint is sent, to the same-origin unsubscribe route; no new origin, API permission or CSP change. | `app.js::dropPushSubscription`, `requestLogout` | RTI4 (static), RTI5 (browser bench), `remote_cross_browser_contract` stays green |
| A9 | No dependency, unit, installer or wire change; `#![forbid(unsafe_code)]` kept. | workspace | `cargo deny --locked check`, `test_business_crates_forbid_unsafe_code` |
| A10 | Documentation states each deferred finding as an open owner decision with its public issue number; nothing host-specific enters the repository. | `Docs/REMOTE_COMPANION.md`, `AI/DECISIONS.md`, walkthrough 258 | RTI6–RTI9; `grep -rn "/home/"` on the touched docs is empty |

### Pre-existing violations found (not introduced by this change)

- The tailnet trust model trusts tailscaled's local listener (RIDN-01, #601) and the system sender
  unit has no IP egress filter (RPSHB-01, #605). Both fixes change pinned invariants
  (RMC-S21, PSS1) and are deferred to the owner; documented in section 2 and section 2d.

### Clearance: CLEARED
