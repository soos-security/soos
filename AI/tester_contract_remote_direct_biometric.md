# Tester Contract — soos-remote: direct biometric prompt on Android (GitHub #354, sub-issue 1)

- **Date**: 2026-10-12
- **Branch**: `feat/remote-direct-biometric` (GitHub-only issue, no backlog id, not registered in
  `scripts/sync_issue.py`; commits say `Refs #354`)
- **Spec**: `AI/architect_spec_remote_direct_biometric.md` (plan evaluator APPROVED, PE-1 … PE-6 folded in `7aa93dc`)
- **ADR**: "[2026-10-12] Named Credentials on Authenticated Passkey Ceremonies of `soos-remote`" (already registered)
- **Matrix rows**: RDB1–RDB9 (RDB10 is the owner hardware check, no automated test)
- **Production code**: none.

## Files

| File | Kind | Tests |
|---|---|---|
| `crates/remote/tests/direct_biometric_tests.rs` (new) | end-to-end, paused clock, raw HTTP over the Unix socket (`tests/common` harness) | 11 |
| `crates/remote/tests/credentials_tests.rs` (appended; **AM-4** setup line) | pure store / allow-list | 3 |
| `tests/invariants/src/remote_direct_biometric_contract.rs` (new) + `mod` line in `tests/invariants/src/lib.rs` | static | 6 + 1 helper self-test |
| `crates/remote/tests/auth_server_tests.rs` | **AM-1** (test 39) | — |
| `crates/remote/tests/camera_server_tests.rs` | **AM-2** (test 24) | — |
| `tests/invariants/src/remote_passkey_contract.rs` | **AM-3** (test 51) | — |
| `crates/remote/tests/auth_store_tests.rs` | **AM-5** setup line | — |

## How red was established

1. **Behaviour tests** (`direct_biometric_tests`) use only the HTTP surface, compile today and fail on their
   assertions: every registration that sends a `transports` key (including `null`) is `400 bad_request` under the
   current `deny_unknown_fields` body, and every positive control on the unlock/camera options sees the old key set
   `["challenge", "rp_id", "timeout_ms"]`.
2. **Store unit tests** (`credentials_tests`) are compile-red on exactly the specified API of spec §3.1 and nothing
   else: `soos_remote::credentials::{AuthenticatorTransport, TransportSet, allow_credentials}` and the field
   `PasskeyRecord::transports` (15 errors, all E0425/E0432/E0560/E0609 on these items). The same missing field makes
   `auth_store_tests` compile-red (1 error, AM-5 line).
3. **Static invariants** compile today and fail on their assertions.
4. **Satisfiability check** (not committed): a throwaway prototype of spec §3 (credentials, auth, server, app.js) in a
   temporary worktree under the scratchpad made all 14 new behaviour/unit tests, the amended tests 24/39/51, S-1 … S-5,
   the whole `soos-remote` suite (`--no-fail-fast`) and the rest of `soos-invariants` green, with `cargo clippy
   --all-targets -- -D warnings` clean; only `test_rdb_s6_docs_adr_and_walkthrough` stayed red (docs and walkthrough
   are later phases). The prototype was deleted; nothing of it is on the branch.

Commands: `cargo test --locked -p soos-remote --all-features --test <file>` and
`cargo test --locked -p soos-invariants --all-features -- remote_direct_biometric test_rmc_s18`.

## Tester Contract — GitHub #354 sub-issue 1

| Test (path::name) | Acceptance line / matrix ID | Red evidence (failure message) |
|---|---|---|
| `direct_biometric_tests::test_rdb_unlock_options_name_every_registered_credential` | RDB1 (D-1, D-3, PE-2: stored `["internal","usb"]` next to a legacy default) | `expected 200 registered` — left `(400, "bad_request")` (registration with `transports` refused) |
| `direct_biometric_tests::test_rdb_camera_options_name_every_registered_credential` | RDB2 (D-1, D-3, PE-2) | `expected 200 registered` — left `(400, "bad_request")` |
| `direct_biometric_tests::test_rdb_login_options_never_name_a_credential` | RDB3 (D-2, OD-3, PE-3: anonymous Funnel, Funnel with a valid session cookie, tailnet) | `expected 200 registered` — left `(400, "bad_request")` (the stored transports it needs cannot be written) |
| `direct_biometric_tests::test_rdb_named_options_refused_without_funnel_session` | RDB4 (D-7, V-4: no cookie, unknown cookie, revoked session; no challenge issued) | `unlock` — left `["challenge", "rp_id", "timeout_ms"]`, right `["allow_credentials", …]` (valid-session control) |
| `direct_biometric_tests::test_rdb_refusals_never_name_a_credential` | RDB4 (§3.3, §5: 409, 429, 403 unlock_disabled, 403 camera_disabled) | `unlock` — left `["challenge", "rp_id", "timeout_ms"]` (named control before the lockout) |
| `direct_biometric_tests::test_rdb_register_stores_allowlisted_transports` | RDB6 (D-5, A-2: unknown ignored, duplicates collapsed, canonical) | `expected 200 registered` — left `(400, "bad_request")` |
| `direct_biometric_tests::test_rdb_register_without_transports_is_accepted` | RDB6 (D-5, V-8: absent, `null`, `[]`) | `expected 200 registered` — left `(400, "bad_request")` (the `null` body) |
| `direct_biometric_tests::test_rdb_register_transports_bounds` | RDB6 (§3.2 bounds, PE-4: 9 entries, 17 bytes, number, object, string; then `429`; code file kept; at-bound accepted) | `expected 200 registered` — left `(400, "bad_request")` (8 entries of ≤ 16 bytes at the bound) |
| `direct_biometric_tests::test_rdb_counter_update_keeps_transports` | RDB7 (§2 `persist_counter`) | `expected 200 registered` — left `(400, "bad_request")` |
| `direct_biometric_tests::test_rdb_assertion_rules_unchanged_with_allow_list` | RDB8 (D-6, R-3: no `userHandle`, UV clear, unregistered id ⇒ `403 passkey_rejected`) | `named options` — left `["challenge", "rp_id", "timeout_ms"]` |
| `direct_biometric_tests::test_rdb_options_never_log_a_credential_id` | RDB8 (§7 privacy row) | `expected 200 registered` — left `(400, "bad_request")` |
| `credentials_tests::test_rdb_transport_set_canonical_and_bounded` | RDB7 (§3.1, §4) | compile: `unresolved imports soos_remote::credentials::AuthenticatorTransport, TransportSet` |
| `credentials_tests::test_rdb_store_transports_roundtrip_and_strict_parse` | RDB7 (§3.1, D-5, A-6: legacy byte-identical, strict read, ≤ 16 KiB) | compile: `no field transports on type PasskeyRecord` |
| `credentials_tests::test_rdb_allow_credentials_shape` | RDB7 (D-1, D-3) | compile: `unresolved import soos_remote::credentials::allow_credentials` |
| `remote_direct_biometric_contract::test_rdb_s1_login_ceremony_stays_discoverable` | RDB3 (D-2, PE-1) | `login_options passes OptionsCredentials::Discoverable` |
| `remote_direct_biometric_contract::test_rdb_s2_unlock_and_camera_name_the_registered_credentials` | RDB1, RDB2 (D-1, OD-4) | `only the unlock and camera options name the credentials` — left `[]` |
| `remote_direct_biometric_contract::test_rdb_s3_registration_prefers_the_platform_authenticator` | RDB5 (D-4, §3.5) | `registration must contain authenticatorAttachment: "platform"` |
| `remote_direct_biometric_contract::test_rdb_s4_transport_names_single_sourced` | RDB6 (§3.5, §4) | `const PASSKEY_TRANSPORTS = [...] in app.js` |
| `remote_direct_biometric_contract::test_rdb_s5_options_routes_are_not_funnel_public` | RDB4 (D-7, V-4) | `only the two non-public options handlers name credentials` — left `0`, right `2` |
| `remote_direct_biometric_contract::test_rdb_s6_docs_adr_and_walkthrough` | RDB9 (§10, PE-5) | `§2h: preferred passkey service` |
| `remote_direct_biometric_contract::test_rdb_top_level_item_and_section_helpers` | helper self-test of the static parser (no matrix row) | green by design (pins the extractors used by S-1 … S-6) |
| `auth_server_tests::test_rmc_unlock_requires_a_fresh_passkey_on_every_path` (AM-1) | RDB1 | left `["challenge", "rp_id", "timeout_ms"]`, right `["allow_credentials", "challenge", "rp_id", "timeout_ms"]` |
| `camera_server_tests::test_rlc_start_requires_fresh_uv_camera_assertion` (AM-2) | RDB2 | same key-set mismatch |
| `remote_passkey_contract::test_rmc_s18_page_uses_modal_webauthn_without_storage` (AM-3) | RDB3 | `allowCredentials only in getAssertion` — left `0`, right `1` |

Unchanged guard: `auth_server_tests::test_rmc_funnel_login_issues_a_host_cookie_session` (test 36: login keys, no
credential id, no `allowCredentials`/`allow_credentials` text) keeps guarding OD-3 and is untouched.

### Migrated existing tests (spec §9, approved by the plan evaluator as PE-6)

| Test | Old assertion | New assertion | Mandating decision |
|---|---|---|---|
| AM-1 `auth_server_tests` test 39 | keys `["challenge","rp_id","timeout_ms"]`; body does not contain the owner credential id | keys `["allow_credentials","challenge","rp_id","timeout_ms"]`; `allow_credentials` equals exactly `[{"id": <owner id>, "transports": ["internal","hybrid"]}]` | OD-4 (owner decision part 2); exact replacement, stricter than a removal. The anonymous non-disclosure property stays pinned by test 36 and the new RDB3/RDB4 tests. |
| AM-2 `camera_server_tests` test 24 | keys `["challenge","rp_id","timeout_ms"]` | keys `["allow_credentials","challenge","rp_id","timeout_ms"]` | OD-4 (camera start) |
| AM-3 `remote_passkey_contract` test 51 | `allowCredentials` in the `forbidden` list; doc "never `allowCredentials`" | entry removed; `app.matches("allowCredentials").count() == 1` and `app.contains("getAssertion(options.body, false)")`; doc "`allowCredentials` only for the authenticated unlock and camera ceremonies, never at sign-in" | OD-3/OD-4; every other `forbidden` needle unchanged (PE-6 condition) |
| AM-4 `credentials_tests` helper `record` | — | `transports: Default::default(),` added | setup only (new public field) |
| AM-5 `auth_store_tests` Debug-redaction literal | — | `transports: Default::default(),` added | setup only (new public field) |

No other existing test was modified.

### Assumptions recorded by the tester

- T-1 (PE-3): the login options are Funnel-only, so a tailnet caller gets `403 forbidden`; the test asserts that this
  refusal names no credential (no id, no `allow`, no `transports`) instead of the 200 key set, and exercises the 200
  key set for two anonymous Funnel callers and a Funnel caller presenting a valid session cookie.
- T-2: `"transports": []` in register verify is accepted and stored as empty (no key in the file), like absent and
  `null` (spec §3.2 table).
- T-3: the at-bound input (8 entries, each ≤ 16 bytes, one known name and seven unknown 16-byte names) registers and
  stores `["internal"]`; this is the positive control that makes `test_rdb_register_transports_bounds` red today.
- T-4: on read, `transports` values that are a JSON string, a number, an object, nested arrays, an unknown or
  wrongly cased name, a duplicate or 7 entries are `StoreError::Malformed`. `null` and `[]` on disk are not pinned
  (`encode_store` never writes them).
- T-5: an empty transport set encodes byte for byte as the legacy `store_json` fixture text (backward compatible both
  ways for such files, spec §3.1).
- T-6: S-6 also pins, from spec §10 and PE-5: "already exists", "on the phone", `soos-remote passkeys remove`,
  "fingerprint" in §2h, `transports` in §6 and `client-device` in `AI/ARCHITECTURE.md` (§13 passkey row).
- T-7: S-1/S-2 pin the exact signatures of spec §3.3 and §3.5 (`issue_options(…, credentials: OptionsCredentials)`,
  `function getAssertion(options, nameCredentials)`, `nameCredentials === true`,
  `Array.isArray(options.allow_credentials)`); S-3 pins `transports: credentialTransports(response)` and
  `typeof response.getTransports === "function"` inside a `try`/`catch` helper.

### Flakiness check

`direct_biometric_tests` (paused virtual clock, no wall-clock assertion) ran 10 times in a row against the throwaway
prototype: 10/10 green, 11/11 tests each run.
