# Tester Contract — GitHub #479 (security review 2026-11-10 batch 2)

| Test (path::name) | Finding / matrix ID | Red evidence (failure message before the implementation) |
|---|---|---|
| `crates/remote/tests/tailnet_caller_tests.rs::test_rti1_tailnet_caller_identifies_each_allowed_login` | #584 / RTI1 | `left: true, right: false` (every login mapped to one caller) |
| `tailnet_caller_tests.rs::test_rti1_tailnet_challenge_pools_are_per_caller` | #584 / RTI1 | `left: Err(Mismatch), right: Ok(0)` (no caller-bound pool) |
| `tailnet_caller_tests.rs::test_rti1_tailnet_pool_total_is_bounded` | #584 / RTI1 | `left: Err(Mismatch), right: Ok(0)` |
| `tailnet_caller_tests.rs::test_rti2_auth_limiter_keys_are_per_tailnet_caller` | #584 / RTI2 | `assert_ne!` failed: both keys equal |
| `tailnet_caller_tests.rs::test_rti2_one_login_cannot_lock_another_out` | #584 / RTI2 (the review's regression test) | `left: 429, right: 200` (owner locked out by the second login) |
| `tailnet_caller_tests.rs::test_rti2_register_failures_count_on_the_callers_key` | #584 / RTI2 | `left: 429, right: 200` |
| `tailnet_caller_tests.rs::test_rti3_challenge_pool_of_one_login_is_not_the_owners` | #584 / RTI3 | `left: 429, right: 200` (`too_many_challenges` for the owner) |
| `tests/invariants/src/remote_tailnet_identity_contract.rs::test_rti4_sign_out_drops_the_push_subscription_first` | #616 / RTI4 | panicked: `app.js defines function dropPushSubscription` (first draft: no `LOGOUT_PUSH_TIMEOUT_MS` constant; the needles were revised with the spec, Revision 1, before any implementation was accepted) |
| `tests/browser/specs/signin.spec.mjs` "sign out drops this phone's notifications before ending the session" | #616 / RTI5 | not run locally (no `node_modules`, Playwright image not pulled); before the change `order` is `["logout"]` |
| `remote_tailnet_identity_contract::test_rti6_revocation_names_the_notification_devices` | #583 / RTI6 | panicked: section 2a revocation lacks `soos-remote push list` |
| `remote_tailnet_identity_contract::test_rti7_trust_model_states_the_local_listener_gap` | #601 / RTI7 | panicked: no "Known gap: other local accounts" paragraph |
| `remote_tailnet_identity_contract::test_rti8_sender_isolation_states_the_tcp_path` | #605 / RTI8 | panicked: no "These are file-system limits" sentence |
| `remote_tailnet_identity_contract::test_rti9_documentation_adr_and_walkthrough` | RTI9 | panicked: section 2b still says "per limiter key (tailnet," |

### Migrated existing tests

None. Fixture additions only, in `crates/remote/tests/common/harness.rs`: the `Options` field
`extra_logins` (default empty, appended after `LOGIN`) and the `Via::TailnetAs(String)` variant
(handled in `with_cookie` and `headers`). No existing assertion, expected value or test body
changed.

### Flakiness check

The server tests run on the frozen paused clock of the harness (no wall-clock assertion); the
seven `tailnet_caller_tests` were run 10 times in a row, all green after the implementation.
