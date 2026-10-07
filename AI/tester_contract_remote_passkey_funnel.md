# Tester Contract — GitHub #339 follow-up: Tailscale Funnel Access and In-House Passkey Authentication for `soos-remote`

- **Date**: 2026-10-06
- **Branch**: `feat/remote-funnel-passkey` (nothing committed, pushed or deployed, D-I)
- **Spec**: `AI/architect_spec_remote_passkey_funnel.md` round 2 (APPROVED by `AI/plan_evaluator_report.md`)
- **ADR**: "[2026-10-06] Tailscale Funnel Access and In-House Passkey Authentication for `soos-remote`"
- **Matrix**: RMC26–RMC41 (new), RMC23–RMC25 (amended)

## How the Red phase was obtained

The specified API does not exist yet, so the tester added **red-phase stubs** with the exact spec
signatures and deliberately wrong bodies. The contract tests therefore compile and fail on their
**assertions**, not on missing items. The developer replaces every stub body. Signatures may change
only where the spec says something different.

| File | Stub content |
|---|---|
| `crates/remote/src/{webauthn,challenge,websession,credentials,enroll,auth}.rs` | Spec §4.1–§4.6 types and signatures. Every function returns an error, `None`, an empty value or `usize::MAX`. |
| `crates/remote/src/lib.rs` | `pub mod` lines and every §3.1 constant with its specified value. The compile-time asserts of §3.1 are left to the developer. |
| `crates/remote/src/{identity,config,http,routes}.rs` | `PathClass`, `Caller`, `classify_request`, `ClientHint` (with an unredacted `Debug`, which is wrong on purpose), `client_hint`, `AuthError::{Ambiguous, FunnelDisabled}`, `AuthConfig`, `RemoteConfig.auth`, the new `ConfigError` variants, `resolve_credentials_path`, `BodyFraming`, `ParsedRequest`, `parse_request`, `read_body`, `BodyError`, `HttpError::BodyTooLarge` (mapped to 400 on purpose), the new `Route` variants, `accepts_body`, `is_funnel_public`, `check_auth_csrf`. In this phase the new TOML keys are parsed and then ignored. |
| `crates/remote/src/server.rs` | Harness hooks defined by the tester: `ServerState::with_random(RandomSource)`, `with_credentials_path(PathBuf)` and `with_file_owner_uid(u32)`. The last one sets the uid that must own the store and code files; its default is the state uid. The new routes answer 404. |
| `Cargo.toml` | `[workspace.dependencies]` gains `p256 = { version = "0.13.2", default-features = false, features = ["ecdsa"] }`, `base64ct`, `subtle` (spec §3.4). |
| `crates/remote/Cargo.toml` | dev-dependencies `p256`, `sha2`, `ciborium` (test fixtures); `zeroize` normal (stub signature `Zeroizing`). The developer adds `p256`, `sha2`, `ciborium`, `getrandom`, `base64ct`, `subtle` to `[dependencies]` (test 46). |

Shared fixtures (no product code):

- `crates/remote/tests/common/passkey.rs` provides:
  - P-256 keys derived from fixed seeds (deterministic RFC 6979 signatures) and the `n − s` signature complement.
  - CBOR attestation objects, COSE keys, client data, authenticator data, and the assertion and registration JSON bodies.
  - A local base64url codec.
  - The verbatim WebAuthn L3 §16.2 vector.
  - Store JSON (spec §4.4) and the code-file line (spec §4.5), both written by hand.
- `crates/remote/tests/common/harness.rs` is the `server_tests.rs` harness (frozen paused clock, scripted logind, raw HTTP over the Unix socket in a `TempDir`) with these additions:
  - the passkey configuration and a credential store fixture;
  - Funnel requests built as `tailscaled` sends them (`Host: localhost`, `X-Forwarded-Host`, `X-Forwarded-Proto: https`, `Tailscale-Funnel-Request: ?1`, `X-Forwarded-For`, optional cookie);
  - every ceremony;
  - held connections.

All test code passes `cargo fmt --check` and `cargo clippy --all-targets -D warnings`.

## Tester Contract — GitHub #339 follow-up (Funnel + passkeys)

Red evidence was observed with `cargo test --offline -p soos-remote --no-fail-fast` and `cargo test --offline -p soos-invariants`. The pass/fail set was identical over 3 runs.

| Test (path::name) | Acceptance line / matrix ID | Red evidence (failure message) |
|---|---|---|
| `crates/remote/tests/config_tests.rs::test_rmc_passkey_constants_match_the_adr` | Test 1, §3.1, RMC28 | **passes at red** (constants are spec values; guard) |
| `config_tests.rs::test_rmc_parse_config_auth_defaults_are_off` | Test 2, RMC28 | **passes at red** (stub returns the default; guard) |
| `config_tests.rs::test_rmc_parse_config_rp_id_rules` | Test 3 + G-6, RMC28 | `left: None` (rp_id not applied) |
| `config_tests.rs::test_rmc_parse_config_allow_funnel_requires_rp_id` | Test 4, S-2, RMC28 | `left: Ok(AuthConfig { rp_id: None, .. })` |
| `config_tests.rs::test_rmc_parse_config_credentials_path_rules` | Test 5, RMC28 | `left: None` |
| `config_tests.rs::test_rmc_resolve_credentials_path` | Test 6, S-5, RMC28 | `left: Err(InvalidCredentialsPath { max: 4096 })` |
| `config_tests.rs::test_rmc_auth_config_error_messages_are_fixed_english_text` | Test 7, RMC28 | `unwrap_err()` on `Ok` (refused `rp_id` accepted) |
| `identity_tests.rs::funnel_classification::test_rmc_classify_request_table` | Test 8, §2.1, RMC26 | `left: Err(Missing)` |
| `identity_tests.rs::funnel_classification::test_rmc_classify_request_ignores_spelling_variants` | Test 9, §2.2, RMC26 | `left: Err(Missing)` (no Tailnet caller) |
| `identity_tests.rs::funnel_classification::test_rmc_classify_request_never_reads_forwarding_headers` | Test 10, RMC26 | **passes at red** (constant stub; invariance guard paired with test 8) |
| `identity_tests.rs::funnel_classification::test_rmc_client_hint_parsing` | Test 58 + G-4, S-12, RMC37 | `left: ClientHint([255; 16])` (UNKNOWN) |
| `http_tests.rs::body_framing::test_rmc_parse_request_matches_parse_request_head_for_non_body_routes` (proptest) | Test 11, RMC29 | `Ok(RequestHead{..}) vs Err(Malformed)` |
| `http_tests.rs::body_framing::prop_rmc_parse_request_never_panics` | Test 11, RC-4, RMC29 | `Err(HeadTooLarge) vs Err(Malformed)` |
| `http_tests.rs::body_framing::test_rmc_parse_request_body_framing` | Test 12, S-13/F-9, RMC29 | `left: Err(Malformed)` |
| `http_tests.rs::body_framing::test_rmc_read_body_is_bounded` | Test 13, F-9, RMC29 | `left: Err(Closed)` |
| `routes_tests.rs::passkey_routes::test_rmc_auth_route_table` | Test 14 + G-2b, RMC27/RMC29 | `left: NotFound` |
| `routes_tests.rs::passkey_routes::test_rmc_check_auth_csrf_requires_origin` | Test 15, RMC33 | `left: Err(MissingActionHeader)` |
| `webauthn_tests.rs::test_rmc_parse_client_data_rules` | Test 16, RMC30/31 | `origin is exactly https:// + rp_id`, `left: ""` |
| `webauthn_tests.rs::test_rmc_verify_registration_accepts_a_valid_none_attestation` | Test 17, RMC30 | `valid: AttestationMalformed` |
| `webauthn_tests.rs::test_rmc_verify_registration_rejections` | Test 18, S-7, RMC30 | `baseline is valid` |
| `webauthn_tests.rs::test_rmc_l3_vector_16_2_is_rejected_for_missing_uv` | Test 19, RMC31 | `left: Some(AttestationMalformed)` (expected `UserVerificationMissing`) |
| `webauthn_tests.rs::test_rmc_verify_assertion_accepts_a_valid_uv_assertion` | Test 20, RMC31 | `left: Err(SignatureInvalid)` |
| `webauthn_tests.rs::test_rmc_verify_assertion_rejections` | Test 21, RMC31 | baseline `is_ok()` false |
| `webauthn_tests.rs::test_rmc_webauthn_errors_never_echo_values` | Test 22, D-H, RMC38 | **passes at red** (error enum texts are part of the declared type; guard) |
| `webauthn_tests.rs::test_rmc_b64url_helpers_are_strict` | §4.1 helpers | `left: Err(())` |
| `webauthn_tests.rs::test_rmc_passkey_fixtures_are_self_consistent` | fixture guard (checks the fixtures with `p256`/`ciborium` directly) | **passes at red by design** |
| `auth_store_tests.rs::test_rmc_challenge_store_is_single_use_and_scoped` | Test 23, F-1, RMC32 | `left: Err(PoolFull)` |
| `auth_store_tests.rs::test_rmc_challenge_pools_are_bounded_without_eviction` | Test 24, RMC32 | `left: Err(PoolFull)` |
| `auth_store_tests.rs::test_rmc_anonymous_login_pool_evicts_oldest` | Test 24a, F-2, RMC32 | `198.51.0.2 is a valid hint` (client_hint stub) |
| `auth_store_tests.rs::test_rmc_web_sessions_ttl_cap_and_logout` | Test 25, F-4, RMC33 | `unwrap()` on `Err(TooMany)` |
| `auth_store_tests.rs::test_rmc_session_cookie_parsing` | Test 26, RMC33 | `left: None` |
| `credentials_tests.rs::test_rmc_credential_store_roundtrip_and_validation` | Test 27, RMC35 | `round trip` |
| `credentials_tests.rs::test_rmc_credential_store_file_rules` | Test 28, RMC35 | `missing = zero passkeys` |
| `credentials_tests.rs::test_rmc_credential_store_atomic_update` | Test 29, RMC35 | `left: Err(Io)` |
| `enroll_tests.rs::test_rmc_enroll_code_generation_and_normalisation` | Test 30, RMC34 | `scripted code: Failed` |
| `enroll_tests.rs::test_rmc_enroll_code_file_rules` | Test 31, S-6, RMC34 | encoded line `left: ""` |
| `auth_server_tests.rs::test_rmc_funnel_disabled_by_default_is_forbidden` | Test 32, S-2, RMC26 | **passes at red** (today every Funnel request is already `403`; regression guard) |
| `auth_server_tests.rs::test_rmc_funnel_public_routes_and_login_required` | Test 33 + G-2a/b, D-D, RMC27 | `left: 403` on `GET /` (Funnel assets not public yet) |
| `auth_server_tests.rs::test_rmc_funnel_host_must_equal_rp_id` | Test 34, S-1, RMC27 | `expected 421 misdirected_request, left: (403, "forbidden")` |
| `auth_server_tests.rs::test_rmc_forged_identity_on_funnel_is_refused` | Test 35, §2.2, RMC26 | `expected 403 forbidden, left: (200, <index.html>)` (Funnel marker ignored today) |
| `auth_server_tests.rs::test_rmc_funnel_login_issues_a_host_cookie_session` | Test 36, S-3, RMC33 | `left: 403` on login options |
| `auth_server_tests.rs::test_rmc_funnel_login_rejections_consume_the_challenge` | Test 37, RMC32 | `options refused: forbidden` |
| `auth_server_tests.rs::test_rmc_funnel_sessions_expire_and_logout` | Test 38 + F-4/F-6/G-3, RMC33 | `login: forbidden` |
| `auth_server_tests.rs::test_rmc_unlock_requires_a_fresh_passkey_on_every_path` | Test 39, D-C, RMC36 | `expected 403 passkey_required, left: (202, "unlock_requested")` (an identity-only unlock still succeeds today) |
| `auth_server_tests.rs::test_rmc_unlock_refuses_assertions_without_user_verification_end_to_end` | Test 39a, F-5, RMC36 | `options refused: not_found` |
| `auth_server_tests.rs::test_rmc_unlock_order_disabled_before_body_and_logind` | Test 40, RMC36 | `expected 403 unlock_disabled, left: (413, "body_not_allowed")` |
| `auth_server_tests.rs::test_rmc_registration_is_tailnet_only_with_a_local_code` | Test 41, D-E, RMC34 | `login: forbidden` |
| `auth_server_tests.rs::test_rmc_first_registration_then_login_and_unlock_use_the_same_user_handle` | Test 41a, F-1, RMC34 | `left: 413` (`body_not_allowed` on register options) |
| `auth_server_tests.rs::test_rmc_interleaved_registrations_on_an_empty_store` | Test 41b, F-1, RMC34 | `left: (413, 413)` |
| `auth_server_tests.rs::test_rmc_auth_limiters_are_per_limit_key` | Test 42, F-2, RMC37 | `expected 409 already_unlocked, left: (404, "not_found")` |
| `auth_server_tests.rs::test_rmc_auth_fails_closed_on_store_and_random_errors` | Test 43, F-7, RMC38 | `login: forbidden` |
| `auth_server_tests.rs::test_rmc_passkey_removal_revokes_sessions` | Test 44, F-6, RMC33/35 | `login: forbidden` |
| `auth_server_tests.rs::test_rmc_auth_never_logs_secrets` | Test 45, D-H, RMC38 | `options refused: forbidden` |
| `auth_capacity_tests.rs::test_rmc_tailnet_is_served_while_funnel_slots_are_saturated` | Test 54, S-11, RMC41 | `login: forbidden` |
| `auth_capacity_tests.rs::test_rmc_funnel_refusals_never_starve_the_tailnet_of_connections` | **Test 54a (plan-evaluator G-1)**, RMC41 | `login: forbidden` |
| `auth_capacity_tests.rs::test_rmc_anonymous_funnel_capacity_and_body_reads` | Test 55 (server part), RMC41 | `login: forbidden` |
| `auth_capacity_tests.rs::test_rmc_capacity_permits_are_bounded` | Test 55 (unit part, `Capacity`), RMC41 | `anonymous permit: Busy` |
| `auth_capacity_tests.rs::test_rmc_owner_login_survives_a_single_hint_filling_the_login_pool` | Test 56, F-2, RMC37 | `left: 403` on login options |
| `auth_capacity_tests.rs::test_rmc_anonymous_failures_never_lock_session_holders_or_other_hints` | Test 57, F-2, RMC37 | `login: forbidden` |
| `auth_capacity_tests.rs::test_rmc_client_hint_table_is_bounded` | Test 59 + G-5, RMC37 | `expected 400 bad_request, left: (413, "body_not_allowed")` |
| `tests/invariants/src/remote_passkey_contract.rs::test_rmc_s13_passkey_dependencies_are_pure_rust` | Test 46, RMC-S13/F-3, RMC39 | `crates/remote [dependencies] takes p256 from the workspace` (also verified green with the deps added: `cargo tree` inside the test works, `rand` absent) |
| `remote_passkey_contract.rs::test_rmc_s14_deny_skip_for_der_is_documented` | Test 47, RMC-S14 | `exactly one der@0.7.10 skip: []` |
| `remote_passkey_contract.rs::test_rmc_s15_auth_logging_hygiene` | Test 48, RMC-S15 | **passes at red** (stubs log nothing; static guard) |
| `remote_passkey_contract.rs::test_rmc_s16_csprng_is_getrandom_only` | Test 49, RMC-S16 | `getrandom::fill is called from auth.rs only, left: []` |
| `remote_passkey_contract.rs::test_rmc_s17_cookie_attributes` | Test 50, RMC-S17 | **passes at red** (constants are spec values; static guard) |
| `remote_passkey_contract.rs::test_rmc_s18_page_uses_modal_webauthn_without_storage` | Test 51, RMC-S18 | `app.js must contain navigator.credentials.get` |
| `remote_passkey_contract.rs::test_rmc_s19_funnel_and_passkeys_are_documented` | Test 52, RMC-S19 | `Docs/REMOTE_COMPANION.md must mention tailscale funnel --bg unix:` |
| `remote_passkey_contract.rs::test_rmc_s20_subcommands_refuse_root_and_never_print` | Test 53, RMC-S20 | `main.rs names the enroll-code subcommand` |
| `remote_passkey_contract.rs::test_rmc_s21_forwarded_for_is_read_only_by_client_hint` | Test 53a, RMC-S21/F-2 | `client_hint reads FORWARDED_FOR_HEADER` |

Summary: 69 new tests (60 in `soos-remote`, 9 in `soos-invariants`). 8 of them pass at red as guards (listed above). Every other test fails on an assertion that pins the missing behaviour. Every existing test still passes except the 5 superseded ones listed below.

### Migrated existing tests

These migrations are mandated by ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey Authentication for `soos-remote`" item (10) and by spec §10.7. No assertion was weakened.

All changes are in `crates/remote/tests/server_tests.rs`:

| Test | Old assertion | New assertion | Mandating line |
|---|---|---|---|
| `Harness::start_with` (setup) | `RemoteConfig` without `auth`; no store | `auth: AuthConfig { rp_id: Some(HOST), allow_funnel: false, credentials_path }`; owner synced passkey stored `0600`; `with_credentials_path`, `with_file_owner_uid(getuid)` | §10.7 "setup only" |
| new helpers `unlock_request_with_passkey` / `unlock_with_passkey` | — | tailnet unlock options (must be `200`) then `POST /api/unlock` with a valid owner assertion | §10.7 "new helper `unlock_with_passkey`" |
| `test_rmc_unlock_flow_and_rate_limit` | `h.unlock(&[])` (identity only) | `h.unlock_with_passkey(&[])`. Every status, result, logind-call and read assertion is unchanged. One added `advance_ms(OPTIONS_WINDOW_MS)` before the `no_session` block, because the 13 unlocks of this test would otherwise exceed the new 10-per-minute options limiter. This only moves time, and no assertion depends on the elapsed time. | ADR item (10); §5.2 options limiter |
| `test_rmc_unlock_and_lock_rate_limits_are_independent` | `h.unlock(&[])` | `h.unlock_with_passkey(&[])` | ADR item (10) |
| `test_rmc_unlock_requires_identity_host_and_csrf` | "a 2-byte body `{}` ⇒ `413`"; final `h.unlock(..)` ⇒ `202` | "`{}` ⇒ `400 bad_request`" **and** "a body of `MAX_AUTH_BODY_BYTES + 1` ⇒ `413`", both followed by the unchanged "no logind read" assertion; the final `202` carries an assertion | ADR item (10), §10.7 |
| `test_rmc_unlock_flow_deadline` | raw identity-only unlock bytes | `unlock_request_with_passkey(&[])` bytes; timing and result assertions unchanged | ADR item (10) |
| `test_rmc_unlock_is_audited_without_identity` | identity-only unlocks; needles {login, session id, host, uid, path, header} | passkey unlocks. The old needles are unchanged. The test adds the owner credential id, the public key, the user handle and every issued challenge (≥ 2) as forbidden needles. | ADR item (10), §10.7 |

`tests/invariants/src/remote_companion_contract.rs`: nine helper functions and `TRACING_MACROS` changed from private to `pub(crate)`, so that `remote_passkey_contract.rs` can reuse them. No assertion changed.

### Plan-evaluator findings encoded although the spec has not folded them yet

The plan evaluator asked the architect to fold G-1 to G-4 into the spec before these tests were written. The spec file still reads round 2, so the tester encoded the evaluator's required behaviour. The architect must confirm it.

- **G-1 (test 54a)**: refused or capped Funnel connections must release their global permit within `FUNNEL_REFUSAL_LINGER_BOUND_MS = 100` ms of virtual time. This bound is **proposed by the tester** from the evaluator's example. It needs a spec constant and an ADR residual restatement.
- **G-2a (test 33)**: Funnel register routes answer `403 forbidden` before `login_required`.
- **G-2b (tests 14 and 33)**: `Logout` is in `is_funnel_public`. Logout without a session returns `200 logged_out` and a clearing `Set-Cookie`.
- **G-3 (test 38)**: an asset or `404` fetch with the cookie at 14 min does not refresh the idle time.
- **G-4 (test 58)**: distinct IPv4 addresses get distinct hints, and IPv4 is never masked to /64.
- **G-5 (test 59)**: the eviction of a locked hint bucket is asserted as the documented residual.
- **G-6 (test 3)**: `rp_id` must be a full `*.ts.net` node host also when `allowed_hosts` is set.

### Deviations from the spec test list

- Test 59 is a server-level test in `auth_capacity_tests.rs` instead of a pure test in `auth_store_tests.rs`. The `AuthState` limiter maps are private, and the eviction is observable through the HTTP surface. The test uses 1 ms between issuances so that "oldest window" is strictly ordered.
- Test 31 has no "future-dated" case: `read_code_file` takes no clock. Expired and future-dated codes are covered end to end in test 41 instead.
- Test 55 is split. The server part is `test_rmc_anonymous_funnel_capacity_and_body_reads` and the unit part is `test_rmc_capacity_permits_are_bounded`, which assumes `Capacity::new()`. The spec gives no constructor.
- Test 38 checks session rotation through behaviour, not through the record count. After 6 rotating logins, 3 other clients can still log in and the 5th client is refused. A refused 5th client proves the rotating browser holds exactly 1 record.
- The harness hooks `with_random`, `with_credentials_path` and `with_file_owner_uid` were defined by the tester. The spec only says "`ServerState` gets the store path / RNG hooks". The owner-uid hook exists because the harness logind uid (1000) differs from the CI runner uid.

### Flakiness check

The full `soos-remote` and `soos-invariants` suites ran 3 times at red. The pass/fail set was identical each time (119 ok / 59 failed in `soos-remote`, 471 / 7 in `soos-invariants`). Every server test uses the frozen paused clock: time moves only through `advance_ms`, and the I/O takes no virtual time. The only wall-clock assertions are lower bounds: the store lock wait is at least `STORE_LOCK_TIMEOUT_MS − 25` ms and at most 5 s (test 29), and the FIFO open must not block for 5 s. The 10× loop on the timing-sensitive tests (38, 43, 54, 54a, 55, 59, 13, 29) must be repeated by the developer once they are green, because a red test fails deterministically before reaching its timing.
