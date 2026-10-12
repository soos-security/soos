# Walkthrough 195 — soos-remote: Direct Biometric Prompt on Android

- **Date**: 2026-10-12
- **Issue**: GitHub #354, sub-issue 1 (GitHub-only; no `AI/BACKLOG.md` entry, not registered in
  `scripts/sync_issue.py`; commits say `Refs #354`). **Branch**: `feat/remote-direct-biometric`.
  Nothing is pushed, installed, enabled or restarted by the agents; no agent runs `tailscale`, `sudo`
  or `curl` against a live endpoint. Merge only after explicit owner approval.
- **ADR**: "[2026-10-12] Named Credentials on Authenticated Passkey Ceremonies of `soos-remote`"
  (`AI/DECISIONS.md`); it amends, without superseding, the 2026-10-06 passkey entry ("`allowCredentials`
  never sent" is narrowed; "no credential id ever reaches an anonymous client" is unchanged).
- **Documents**: spec `AI/architect_spec_remote_direct_biometric.md` (APPROVED, PE-1 to PE-6 folded in);
  plan evaluation `AI/plan_evaluator_report.md`; tester contract
  `AI/tester_contract_remote_direct_biometric.md`; auditor constraints
  `AI/auditor_constraints_remote_direct_biometric.md` (C-1 to C-16, CLEARED); candid review
  `AI/candid_review_report.md`.
- **Matrix criteria**: RDB1–RDB10 (component `remote-direct-biometric`; RDB10 is the owner hardware
  check, pending).

## 1. Context & Objectives

On Android, the unlock and camera-start passkey prompts of the phone page opened a credential-provider
picker (Google Password Manager, Proton Pass, ...) before the fingerprint prompt, because every
ceremony was discoverable (no `allowCredentials`). The owner decided (issue #354, owner decisions
parts 1 and 2):

- OD-1: device-bound and password-manager passkeys stay accepted (no `BE=0` policy);
- OD-2: remote unlock keeps working over Funnel;
- OD-3: sign-in stays discoverable: no credential id in the unauthenticated login options;
- OD-4: unlock and camera start, which already require a session (allowed tailnet identity or Funnel
  web session), name the registered credentials so Android goes straight to the biometric prompt;
- OD-5: registration prefers the phone's own authenticator; the docs explain the Android preferred
  passkey service and how to move a soos passkey out of Proton Pass.

## 2. Architect Design

| Area | Change |
|---|---|
| `credentials.rs` | `AuthenticatorTransport` (the six WebAuthn L3 names, canonical order), `TransportSet` (private `u8` bitset, `DEFAULT_ALLOW` = `["internal","hybrid"]`), `PasskeyRecord.transports`, optional `transports` key in the store record (omitted when empty, strictly validated on read), pure `allow_credentials(&PasskeyFile)`. |
| `auth.rs` | `RegisterVerifyBody.transports` (optional): at most 8 entries of at most 16 UTF-8 bytes, else `400 bad_request` (counted by the limiter, before `check_code`); unknown names ignored, duplicates collapsed, exact case-sensitive names. |
| `server.rs` | `OptionsCredentials { Discoverable, Registered }` argument of `issue_options`; `login_options` passes `Discoverable`, `unlock_options` and `camera_options` pass `Registered`; the list is built from the same store snapshot that passed the non-empty check under the same mutex hold and only on success; `register_verify` stores the transports; `persist_counter` keeps them. |
| `lib.rs` | `MAX_TRANSPORT_FIELD_ENTRIES` (8), `MAX_TRANSPORT_NAME_BYTES` (16), one more compile-time bound against `MAX_AUTH_BODY_BYTES`. |
| `app.js` | `getAssertion(options, nameCredentials)` sets the allow list only for unlock and camera (`nameCredentials === true`, array, non-empty, ≤ 4 entries, transports filtered to `PASSKEY_TRANSPORTS`); sign-in calls `getAssertion(options.body, false)`; registration adds `authenticatorAttachment: "platform"`, `hints: ["client-device"]` and the filtered `getTransports()` list (`credentialTransports`, `try`/`catch`). |

Invariants touched: the anonymous non-disclosure of credential ids (kept: the two named routes are not
in `is_funnel_public`, and the Funnel gate answers `403 login_required` before routing); fail closed on
a malformed stored `transports` key (`StoreError::Malformed`). Unchanged: WebAuthn verification (UV,
`userHandle` required, BE/BS, counter, origin, challenge purpose/class/binding), CSP, CSRF, Funnel
gates, limiter keys, RC-1, `MAX_AUTH_BODY_BYTES`, `MAX_CREDENTIAL_STORE_BYTES`. The allow list is a UX
hint and never reaches the verifier.

Assumptions (spec §13): A-1 default transports `["internal","hybrid"]`; A-2 unknown names ignored;
A-3 `soos-remote passkeys list` unchanged; A-4 platform attachment for every new enrollment (hardware
keys deferred to the device-bound sub-issue of #354); A-5 no migration of existing passkeys; A-6
downgrade refused fail-closed (see §9).

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: **VALIDATION_VERDICT: APPROVED**, no CRITICAL or MAJOR finding. Items
folded into the spec in `7aa93dc`:

- PE-1: `allowCredentials` appears only in the single `publicKey.allowCredentials` assignment of
  `app.js` (comments say "allow list"), so the exact-count pins hold.
- PE-2: the RDB1/RDB2 tests store `{Internal, Usb}` (different from the default) next to a legacy
  record that must be named with `["internal","hybrid"]`.
- PE-3: login options are also exercised by a Funnel caller presenting a valid session cookie.
- PE-4: a fifth malformed `transports` body, then `429 rate_limited`; the enrollment code file is still
  present after the malformed bodies.
- PE-5: §2h says to enroll on the phone itself, to delete the Proton Pass passkey first, and what
  "already exists" means.
- PE-6: amendments AM-1 to AM-5 approved as owner-required exact replacements or additive setup.

### Test amendments

| # | Test | Amendment |
|---|---|---|
| AM-1 | `crates/remote/tests/auth_server_tests.rs::test_rmc_unlock_requires_a_fresh_passkey_on_every_path` | key set `["challenge","rp_id","timeout_ms"]` → `["allow_credentials","challenge","rp_id","timeout_ms"]`; "body does not contain the owner credential id" → `allow_credentials` equals exactly `[{"id": <owner id>, "transports": ["internal","hybrid"]}]` (OD-4; stricter; the anonymous direction stays pinned by unchanged `test_rmc_funnel_login_issues_a_host_cookie_session` and the new RDB3/RDB4 tests) |
| AM-2 | `crates/remote/tests/camera_server_tests.rs::test_rlc_start_requires_fresh_uv_camera_assertion` | exact key set gains `allow_credentials` (OD-4) |
| AM-3 | `tests/invariants/src/remote_passkey_contract.rs::test_rmc_s18_page_uses_modal_webauthn_without_storage` | the `allowCredentials` forbidden needle is replaced by an exact count of 1 plus the `getAssertion(options.body, false)` sign-in pin; every other forbidden needle unchanged (OD-3/OD-4) |
| AM-4 | `crates/remote/tests/credentials_tests.rs` helper `record` | setup only: `transports: Default::default()` |
| AM-5 | `crates/remote/tests/auth_store_tests.rs` Debug-redaction literal | setup only: `transports: Default::default()` |

## 4. Tester Contract

- Commit `2d8c8f0` (plus the auditor addition of `7f2f395`):
  `crates/remote/tests/direct_biometric_tests.rs` 12 end-to-end tests (paused clock, raw HTTP over the
  Unix socket), 3 new tests in `crates/remote/tests/credentials_tests.rs`, and
  `tests/invariants/src/remote_direct_biometric_contract.rs` S-1 to S-6 plus one helper self-test.
- Mapping: RDB1 `test_rdb_unlock_options_name_every_registered_credential` + AM-1 + S-2; RDB2
  `test_rdb_camera_options_name_every_registered_credential` + AM-2 + S-2; RDB3
  `test_rdb_login_options_never_name_a_credential`, test 36 (unchanged), S-1, AM-3; RDB4
  `test_rdb_named_options_refused_without_funnel_session`, `test_rdb_refusals_never_name_a_credential`,
  S-5; RDB5 S-3; RDB6 `test_rdb_register_stores_allowlisted_transports`,
  `test_rdb_register_without_transports_is_accepted`, `test_rdb_register_transports_bounds`,
  `test_rdb_register_transports_byte_bound_and_exact_names`, S-4; RDB7 the three `credentials_tests`
  and `test_rdb_counter_update_keeps_transports`; RDB8 `test_rdb_assertion_rules_unchanged_with_allow_list`,
  `test_rdb_options_never_log_a_credential_id`; RDB9 S-6.
- Red evidence: the behaviour tests failed on their assertions (registration with a `transports` key
  was `400 bad_request` under `deny_unknown_fields`; the unlock and camera positive controls saw the
  old key set); the store tests were compile-red only on the specified API (`AuthenticatorTransport`,
  `TransportSet`, `allow_credentials`, `PasskeyRecord::transports`); the static tests failed on their
  assertions. A throwaway prototype (deleted, never committed) proved the contract satisfiable;
  `direct_biometric_tests` ran 10/10 green against it.
- Tester assumptions T-1 to T-7 (tester contract): notably T-1, a tailnet caller of the Funnel-only
  login options gets `403 forbidden` that names no credential; T-5, an empty transport set encodes byte
  for byte as the legacy fixture.

## 5. Auditor Constraints

CLEARED with C-1 to C-16 and one added test (`test_rdb_register_transports_byte_bound_and_exact_names`:
UTF-8 byte bound and exact, case-sensitive names). The task requirement "unlock and camera options are
never reachable unauthenticated over Funnel" was verified in code: `handle_connection` answers
`403 login_required` for every route outside `is_funnel_public` before routing when no session
validates, and `is_funnel_public` lists neither `UnlockOptions` nor `CameraOptions`. How the
constraints are met:

- C-1, C-14: `is_funnel_public`, the Funnel gate, CSP, CSRF, limiter keys, RC-1 and
  `MAX_AUTH_BODY_BYTES` unchanged (`routes.rs` has no diff to `is_funnel_public`).
- C-2, C-3: typed `OptionsCredentials`; `allow_credentials` only on success, from the same snapshot,
  under the same mutex hold; no refusal carries an id.
- C-4 to C-7: bounds checked in `decode_registration` before any per-entry work and before
  `check_code`; `TransportSet` keeps a private `u8`; the store keeps `deny_unknown_fields`, the key is
  `default` + `skip_serializing_if = "Vec::is_empty"`, and > 6 entries, unknown or wrongly cased names
  and duplicates are `StoreError::Malformed`.
- C-8, C-9: `register_verify` copies the transports before dropping the registration; `webauthn.rs` has
  no diff and `check_assertion` never reads the allow list.
- C-10, C-11: no new log line; no `unwrap`/`expect`/indexing/unchecked arithmetic (clippy
  `-D warnings`).
- C-12, C-13: `app.js` as in §2; no storage API, `innerHTML`, conditional mediation or external resource.
- C-15: the developer changed no test file (`git diff 2d8c8f0 -- crates/remote/tests tests/invariants`
  shows only the auditor's additive test).
- C-16: the docs never ask the owner to weaken a gate; the Proton Pass procedure deletes the old passkey
  first and enrolls over the tailnet only; the downgrade note is in §9 below.

## 6. Implementation

- Commit `c1ec02b` (`feat(remote): name registered passkeys on unlock and camera prompts`): the
  production changes of §2 in `crates/remote/src/{credentials,auth,server,lib}.rs` and
  `crates/remote/assets/app.js`, plus `Docs/REMOTE_COMPANION.md` (§2b sentence on named credentials,
  §2h item 5 with "Preferred passkey service" and "Move a soos passkey out of Proton Pass", §2h item 11
  step 8 = RDB10, §6 rows for unlock/camera options and register verify, §8 residual) and the
  `AI/ARCHITECTURE.md` §13 passkey row.
- Notable decisions: an empty `TransportSet` is not serialised, so a store without transports encodes
  byte-identically to the previous format; legacy records are named with `TransportSet::DEFAULT_ALLOW`.
  The allow list on the page is at most 4 entries (`MAX_PASSKEYS`), entries without a string `id` are
  skipped.
- Traceability (this phase): `AI/VERIFICATION_MATRIX.md` component `remote-direct-biometric`
  (RDB1–RDB10, brace lists without spaces so every cited test is resolved by the citation invariant);
  this walkthrough rewritten to the standard structure. ADR, `AI/ARCHITECTURE.md` and `Docs/` were
  re-checked against the code and needed no further change.

## 7. Candid Review

`AI/candid_review_report.md`: date 2026-10-12, base `fb2977c9`, fingerprint
`64cd78f15b0658a0cdac1133d43192afd6b093bcf2624e88f50b3823e7224dfa`, **VERDICT: APPROVED**. No CRITICAL
or MAJOR finding. Findings and resolution:

- **MINOR** `credentials.rs` `RecordJson` with `deny_unknown_fields`: a rollback to an older
  `soos-remote` refuses a store holding a `transports` key. Accepted risk A-6, documented in §9.
- **SUGGESTION** `lib.rs` compile-time body bound counts `name + 3` per entry and relies on the shared
  256-byte slack for the `"transports":[]` key; it holds with a wide margin. Informative only.

The report predates this traceability commit; the commit adds documentation only.

## 8. Verification Results

All on 2026-10-12 at `d2c581d` (plus this documentation-only commit):

- `cargo test --locked --all-features -p soos-remote --no-fail-fast`: 386 passed, 0 failed
  (`direct_biometric_tests` 12, `credentials_tests` 7, `auth_server_tests` 17, `auth_store_tests` 6,
  `camera_server_tests` 15).
- `cargo test --locked --all-features -p soos-invariants`: 571 passed, 0 failed (the 7
  `remote_direct_biometric_contract` tests, `test_rmc_s18_page_uses_modal_webauthn_without_storage`,
  and `test_matrix_claimed_rows_cite_only_existing_evidence` /
  `test_matrix_claimed_rows_cite_at_least_one_resolvable_evidence` over the RDB rows included).
- `cargo fmt --all -- --check`: clean; `cargo clippy --locked -p soos-remote -p soos-invariants
  --all-targets --all-features -- -D warnings`: clean.
- `python3 scripts/sync_issue.py --check`: OK (GitHub-only issue, no backlog sub-issue to tick).

## 9. Known Limitations / Follow-ups

- **RDB10 owner hardware check (pending)**: on the owner's Android phone, after re-enrolling a passkey
  stored on the phone (`Docs/REMOTE_COMPANION.md` §2h item 5), **unlock** and **camera start** show the
  fingerprint/face prompt directly with no provider picker, over the tailnet and over Funnel; sign-in
  may still show the picker; the iPhone flow still works.
- **Downgrade (A-6)**: an older `soos-remote` refuses a store file holding a `transports` key
  (`deny_unknown_fields`) and fails closed (`503 store_unavailable`). Rollback: re-enroll the passkeys,
  or delete the `transports` keys from `remote-passkeys.json` by hand.
- Residual risks (spec §14): R-1 an authenticated caller learns the registered credential ids (opaque,
  cannot sign; accepted by OD-4); R-2 Android may still show a picker when the passkey sits in a
  third-party provider and several providers are enabled, and sign-in keeps the picker by design; R-3
  `userHandle` stays required, so an authenticator omitting it under `allowCredentials` would be refused
  (fail-closed; all soos passkeys are discoverable and return it).
- Follow-ups: enrolling a cross-platform FIDO2 hardware key and any device-bound policy belong to the
  later sub-issues of #354; existing passkeys keep the default transports until re-enrolled.
