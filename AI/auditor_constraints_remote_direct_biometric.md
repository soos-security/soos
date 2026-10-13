# Auditor Constraints — soos-remote: direct biometric prompt on Android (GitHub #354, sub-issue 1)

- **Date**: 2026-10-12
- **Branch**: `feat/remote-direct-biometric` (GitHub-only issue; `Refs #354`)
- **Inputs audited**: `AI/architect_spec_remote_direct_biometric.md` (plan evaluator APPROVED, PE-1 … PE-6),
  `AI/tester_contract_remote_direct_biometric.md` (commit `2d8c8f0`), ADR "[2026-10-12] Named Credentials on
  Authenticated Passkey Ceremonies of `soos-remote`", and the code about to change:
  `crates/remote/src/{credentials.rs, auth.rs, server.rs, lib.rs, routes.rs}`, `crates/remote/assets/app.js`.
- **Crates touched**: `soos-remote` only (business crate, `#![forbid(unsafe_code)]` in `lib.rs` and `main.rs`).
  No PAM, daemon, FFI, packaging, CI, `scripts/` or `.githooks/` change.

## Audit results (checklist)

| # | Item | Result |
|---|---|---|
| 1 | Panic paths (`unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!`/`unreachable!`) in `credentials.rs`, `auth.rs`, `server.rs`, `lib.rs` | 0 hits. Workspace lints deny them; `indexing_slicing`/`arithmetic_side_effects` are errors under `-D warnings`. |
| 2 | `unsafe` | 0 hits in `crates/remote/src`; `#![forbid(unsafe_code)]` present (`lib.rs:36`, `main.rs:34`). |
| 3 | Output isolation | Not a PAM crate; no new print. |
| 4 | Bounded I/O | Request body already bounded by `MAX_AUTH_BODY_BYTES` (8192) before `serde_json`; store file bounded by `MAX_CREDENTIAL_STORE_BYTES` (16 KiB) before parse; the allow list is bounded by `MAX_PASSKEYS` (4). Worst-case register body 5882 B < 8192 B (spec V-9). |
| 5 | Arithmetic | The only new arithmetic is a `const` assertion (compile time) and a `u8` bitset with bit indices < 6. |
| 6 | Filesystem | Store write path unchanged (`0600`, owner check, `O_NOFOLLOW`, `flock`, `fsync`, atomic rename); one optional JSON key. |
| 7 | Secrets and privacy | Credential ids are identifiers (already redacted from `Debug`); no new log line; the response logger logs only the status (`server.rs:987`). Pinned by `test_rdb_options_never_log_a_credential_id`. |
| 8 | Fail-closed | The allow list never reaches `check_assertion`/`verify_assertion`; a malformed `transports` key on disk is `StoreError::Malformed` (existing fail-closed path); no new path to a success response. |
| 9 | Supply chain | No new dependency (`serde`, `serde_json` already used). No `Cargo.toml`/`Cargo.lock`/`deny.toml` change allowed. |
| 10 | CI/workflows | Not touched. |

Exposure check (task requirement "never reachable unauthenticated over Funnel"): `handle_connection`
(`server.rs` ≈1192–1210) refuses every route outside `is_funnel_public` with `403 login_required` when
`validate_session` returns `None`, **before** routing; `is_funnel_public` (`routes.rs:379`) lists neither
`Route::UnlockOptions` nor `Route::CameraOptions`. Tailnet callers reach routing only after
`classify_request` against a non-empty `allowed_logins` (`config.rs:233`, empty list is a config error).
`login_options` refuses tailnet callers (`403 forbidden`) and is the only Funnel-public options route. Verified
in code and pinned by `test_rdb_s5_options_routes_are_not_funnel_public`,
`test_rdb_named_options_refused_without_funnel_session` and `test_rdb_login_options_never_name_a_credential`.

## Audit Constraints — GitHub #354 sub-issue 1

| # | Constraint | Applies to (file::fn) | Verified by (test / lint / invariant / grep) |
|---|---|---|---|
| C-1 | `is_funnel_public` and the pre-routing Funnel gate in `handle_connection` stay byte-identical; `UnlockOptions`/`CameraOptions` never become Funnel-public; `RegisterOptions`/`RegisterVerify` stay `403 forbidden` over Funnel. | `routes.rs::is_funnel_public`, `server.rs::handle_connection` | `test_rdb_s5_options_routes_are_not_funnel_public`, `test_rdb_named_options_refused_without_funnel_session`, existing `auth_server_tests`; `git diff main -- crates/remote/src/routes.rs` shows no change to `is_funnel_public` |
| C-2 | The credential choice is a typed argument: `login_options` passes `OptionsCredentials::Discoverable`; only `unlock_options` and `camera_options` pass `Registered`. `options_response` emits `allow_credentials` **only** when `allow` is `Some`; the login response keeps exactly `{challenge, rp_id, timeout_ms}` for every caller, including a Funnel caller presenting a valid session cookie (no decision keyed on session presence). | `server.rs::{OptionsCredentials, issue_options, options_response, login_options, unlock_options, camera_options}` | `test_rdb_s1`, `test_rdb_s2`, `test_rdb_login_options_never_name_a_credential`, `auth_server_tests` test 36 (unchanged) |
| C-3 | `allow_credentials(&file)` is computed from the **same** snapshot that passed the non-empty check, inside the same `auth` mutex hold, after the lockout/limiter checks and after the challenge is successfully issued or at least never on a refusal path; no refusal (`403`, `409`, `429`, `503`) carries a credential id. No second store read. | `server.rs::Shared::issue_options` | `test_rdb_refusals_never_name_a_credential`, `test_rdb_named_options_refused_without_funnel_session`; review |
| C-4 | Registration `transports` decoding: absent/`null`/`[]` ⇒ `EMPTY`; more than `MAX_TRANSPORT_FIELD_ENTRIES` (8) entries ⇒ `BodyMalformed`; any entry with `str::len()` (UTF-8 **bytes**, not `chars().count()`) > `MAX_TRANSPORT_NAME_BYTES` (16) ⇒ `BodyMalformed`; non-array or non-string ⇒ serde error ⇒ `BodyMalformed`. Bounds are checked before any per-entry work; the check happens in `decode_registration`, i.e. before `parse_client_data` and `check_code`, and its failure is counted on `LimitKey::Tailnet` through the existing `shared.fail(key)` path. | `auth.rs::{RegisterVerifyBody, decode_registration}`, `lib.rs` constants | `test_rdb_register_transports_bounds`, **new** `test_rdb_register_transports_byte_bound_and_exact_names` (auditor) |
| C-5 | Name matching is exact and case-sensitive (`AuthenticatorTransport::from_name`): `"Internal"`, `"HYBRID"`, `"USB"` are unknown values, ignored at registration (never stored, never lower-cased), and refused on read of the store (`StoreError::Malformed`). Unknown names are ignored, duplicates collapsed, output in canonical `ALL` order. | `credentials.rs::{AuthenticatorTransport::from_name, TransportSet}`, `auth.rs::decode_registration` | `test_rdb_transport_set_canonical_and_bounded`, `test_rdb_store_transports_roundtrip_and_strict_parse`, `test_rdb_register_stores_allowlisted_transports`, **new** `test_rdb_register_transports_byte_bound_and_exact_names` |
| C-6 | `TransportSet` keeps a private `u8` field; no constructor or `From`/`Deserialize` impl can set a bit ≥ 6; `names()` returns at most 6 entries. Bit shifts use the index of `ALL` (< 6), never a value from input; no `as` cast from an unbounded integer. | `credentials.rs::TransportSet` | `test_rdb_transport_set_canonical_and_bounded`; clippy `-D warnings` (`arithmetic_side_effects`, `cast_*`) |
| C-7 | Store format: `RecordJson` keeps `#[serde(deny_unknown_fields)]`; the new key is `#[serde(default, skip_serializing_if = "Vec::is_empty")]`; `record_from` rejects > 6 entries, unknown/wrongly cased names and duplicates with `StoreError::Malformed` (fail closed, existing session drop); an empty set encodes byte-identically to the legacy format; the parse still runs only after the 16 KiB size check. `StoreJson`, `STORE_VERSION`, `MAX_CREDENTIAL_STORE_BYTES` unchanged. | `credentials.rs::{RecordJson, record_from, parse_store, encode_store}` | `test_rdb_store_transports_roundtrip_and_strict_parse`, existing `credentials_tests` (test 27 size bound) |
| C-8 | `register_verify` reads `registration.transports` (a `Copy` value) **before** `drop(registration)` and moves it into the `store_update` closure; no other change to the order of gates (lockout → bounded body → decode → `rp` → client data → code → challenge → verify → store). `persist_counter` keeps mutating the record in place (transports preserved). | `server.rs::{register_verify, persist_counter}` | `test_rdb_register_stores_allowlisted_transports`, `test_rdb_counter_update_keeps_transports`, existing `auth_server_tests` |
| C-9 | WebAuthn verification untouched: `webauthn.rs` and `check_assertion` must not read the allow list or the stored transports; UV, `userHandle` required, BE/BS, counter, origin, `crossOrigin`/`topOrigin`, challenge purpose/class/binding rules unchanged. No `BE=0` enforcement (OD-1). | `webauthn.rs` (no diff), `server.rs::check_assertion` | `test_rdb_assertion_rules_unchanged_with_allow_list`; `git diff main -- crates/remote/src/webauthn.rs` empty |
| C-10 | Privacy: no `tracing` call (any level) may carry `allow_credentials`, a credential id (base64url or hex) or a transport list; `PasskeyRecord`, `PasskeyFile`, `RegisterVerifyBody` keep their redacted `Debug`; `TransportSet`'s derived `Debug` is allowed (it carries no identifier) but must not be logged either. | `server.rs`, `credentials.rs`, `auth.rs` | `test_rdb_options_never_log_a_credential_id`, `auth_store_tests` Debug redaction test, invariant log-keyword audit |
| C-11 | Zero panic surface in new production code: no `unwrap`/`expect`/indexing/unchecked arithmetic; `serde_json::json!` construction only; `allow_credentials` is pure and infallible. The pre-existing `encode_store` `unwrap_or_default()` is not extended with a new fallible path (adding a `Vec<String>` field keeps serialisation infallible). | all touched `src` files | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`; grep of checklist item 1 returns 0 |
| C-12 | Page (`app.js`): `allowCredentials` appears exactly once (the single `publicKey.allowCredentials` assignment inside `getAssertion`), set only when `nameCredentials === true` **and** `Array.isArray(options.allow_credentials)` **and** non-empty; at most 4 entries; entries with a non-string `id` skipped; `transports` filtered to `PASSKEY_TRANSPORTS` and only when the server value is an array. `requestLogin` passes `false`; `requestUnlock` and `startCameraView` pass `true`. All three calls stay inside a `.then` callback so a decode throw rejects the chain (no unhandled error). | `assets/app.js::{getAssertion, requestLogin, requestUnlock, startCameraView}` | `test_rdb_s1`, `test_rdb_s2`, `test_rmc_s18` (AM-3) |
| C-13 | Page registration: adds `authenticatorAttachment: "platform"` inside `authenticatorSelection` and top-level `hints: ["client-device"]`; `residentKey: "required"`, `requireResidentKey: true`, `userVerification: "required"`, `attestation: "none"`, `alg: -7`, `excludeCredentials` unchanged. `credentialTransports(response)` checks `typeof response.getTransports === "function"`, calls it inside `try`/`catch` (throw ⇒ `[]`), accepts only an array result, keeps only string members of `PASSKEY_TRANSPORTS`, at most 8. No `innerHTML`, `eval(`, storage API, conditional mediation, or new external resource; CSP unchanged. | `assets/app.js` enrollment handler, `credentialTransports` | `test_rdb_s3`, `test_rdb_s4`, existing `remote_passkey_contract` S-tests (CSP, no storage, no `innerHTML`) |
| C-14 | Unchanged surfaces: CSP and response headers, CSRF (`X-Soos-Action` + required `Origin`), limiter keys and lockout, RC-1, `MAX_AUTH_BODY_BYTES`, asset set, challenge pool bounds; the existing compile-time body assertion stays and the new one (with `MAX_TRANSPORT_FIELD_ENTRIES * (MAX_TRANSPORT_NAME_BYTES + 3)`) is added beside it. | `lib.rs`, `routes.rs`, `server.rs` | existing `auth_server_tests`, `camera_server_tests`, `remote_passkey_contract`; build (const assert) |
| C-15 | Test integrity: only AM-1 … AM-5 (spec §9, PE-6) modify existing tests; every other `forbidden` needle of test 51 and test 36 stay unchanged; the developer modifies no test file (helpers in `tests/common` may be added, not changed). | `crates/remote/tests/**`, `tests/invariants/src/**` | `git diff 2d8c8f0 -- crates/remote/tests tests/invariants` after Phase 4 shows no test change (this audit commit excepted) |
| C-16 | Docs (traceability phase) stay English and must not instruct the owner to run anything that disables a security gate (no `allow_funnel` registration, no CSP or origin relaxation); the Proton Pass procedure deletes the old passkey before re-enrolling and uses `soos-remote enroll-code` over the tailnet only. The downgrade note (A-6) is written in the walkthrough. | `Docs/REMOTE_COMPANION.md` §2h, §2b, §6, §8; walkthrough 195 | `test_rdb_s6_docs_adr_and_walkthrough`; candid review |

## Test added by this audit (strictly additive)

`crates/remote/tests/direct_biometric_tests.rs::test_rdb_register_transports_byte_bound_and_exact_names`
(RDB6; C-4, C-5). Gap closed: the bounds test uses only ASCII, so an implementation counting characters
instead of bytes, or lower-casing names, would have passed it. The new test sends nine two-byte characters
(18 bytes, 9 characters) ⇒ `400 bad_request`, nothing stored, code file kept; then
`["Internal","HYBRID","USB","éééééééé"]` (the last entry exactly 16 bytes) ⇒ `200 registered` with no
`transports` key stored and the default `["internal","hybrid"]` named in the next unlock options.

Red evidence (current code): `expected 200 registered` — left `(400, "bad_request")` (the registration with a
`transports` key is refused by the current `deny_unknown_fields` body). `cargo fmt --check` and
`cargo clippy -p soos-remote --test direct_biometric_tests -- -D warnings` clean. No existing test modified.

## Pre-existing violations found (not introduced by this change)

- None blocking. Observation: `credentials.rs::encode_store` ends with `serde_json::to_vec(&json).unwrap_or_default()`;
  serialisation of these plain structs is infallible, so the empty fallback is unreachable. Not in scope; C-11
  forbids adding a fallible field to it.

## Residual risks reviewed

- R-1 (credential ids disclosed to authenticated callers) accepted by OD-4 and the ADR: ids are opaque, cannot
  sign, and the callers already authenticated with one of them (Funnel session) or are allowlisted tailnet
  identities.
- R-3 (`userHandle` still required with an allow list) is fail-closed; covered by RDB10 on hardware.
- A-6 (downgrade refuses a store with `transports`) is fail-closed (`503 store_unavailable`), never fail-open.

## Clearance: CLEARED
