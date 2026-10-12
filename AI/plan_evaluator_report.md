# Plan Evaluation Report

- **Date**: 2026-10-12
- **Issue**: GitHub-only #354, sub-issue 1 (web quick win), no backlog id — soos-remote direct biometric prompt on Android
- **Spec**: `AI/architect_spec_remote_direct_biometric.md` (commit `aa48f25`, with the ADR "[2026-10-12] Named Credentials on Authenticated Passkey Ceremonies of `soos-remote`")
- **Branch**: `feat/remote-direct-biometric`
- **Base commit**: `fb2977c` (`origin/main`)
- **Round**: 1
- **Owner decisions checked**: issue #354 comments "Owner clarification", "Owner decisions (2026-10-12)", "Owner decisions, part 2 (2026-10-12)".

## 1. Coverage Matrix

| Acceptance line (task, owner decisions) | Spec element | Status |
|---|---|---|
| (1a) The login ceremony stays discoverable: no `allowCredentials`, no credential id in the unauthenticated login options (OD part 2) | D-2 (`OptionsCredentials::Discoverable`, type-level), §3.5 `getAssertion(options.body, false)`, RDB3, test 36 unchanged, `test_rdb_login_options_never_name_a_credential`, `test_rdb_s1` | Covered (see PE-3) |
| (1b) Unlock and camera-start options return the registered credential ids with stored transports, else `["internal","hybrid"]`; the page passes them as `allowCredentials` | D-1, D-3, §3.1 `allow_credentials`, §3.3, §3.5, RDB1, RDB2 | Covered (see PE-1, PE-2) |
| (1c) These options routes are never reachable unauthenticated over Funnel | D-7, V-4, V-5, RDB4 (`test_rdb_named_options_refused_without_funnel_session`, `test_rdb_s5`) | Covered, verified in code (§2) |
| (2a) Registration adds `authenticatorAttachment: "platform"` and `hints: ["client-device"]` | D-4, §3.5, RDB5 | Covered (see PE-5) |
| (2b) Transports from `getTransports()` stored, bounded, allowlisted, backward compatible | D-5, §3.1, §3.2, §3.4, RDB6, RDB7, A-2, A-5, A-6 | Covered |
| (3) `Docs/REMOTE_COMPANION.md` §2h: preferred passkey service, delete the Proton Pass passkey and re-enroll on the phone with `soos-remote enroll-code` | §10, RDB9 | Covered (see PE-5) |
| (4) Owner hardware row (pending): unlock and camera start show fingerprint/face directly, sign-in may still show the picker | RDB10 | Covered |
| Both device-bound and password-manager passkeys accepted (no BE=0 enforcement) | OD-1, D-6, ADR item (4) | Covered: no verifier change |
| CSP, CSRF, Funnel gates, WebAuthn verification, RC-1 unchanged; remote unlock over Funnel stays supported | D-6, D-7, D-8, §7, RDB8 | Covered |
| Zero test weakening: amendments listed AM-n and justified | §9 AM-1 … AM-5 | Covered; classification in PE-6 |
| GitHub-only issue: no `scripts/sync_issue.py` registration, `Refs #354` | Spec header | Covered |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `getAssertion` has no `allowCredentials`; call sites unlock/login/camera | `crates/remote/assets/app.js:270-279`, `:771`, `:820`, `:1686` | as stated | Yes |
| Registration has no `authenticatorAttachment`/`hints`; `residentKey: "required"`, `requireResidentKey: true`, UV required, attestation `none`, `alg: -7`; `excludeCredentials` already sent (tailnet only) | `app.js:862-879` | as stated | Yes |
| Shared `issue_options` loads the store snapshot under the auth mutex with `Ok(Some(file)) if !file.passkeys.is_empty()` then `options_response(rp_id, challenge)` | `server.rs:546-583`, `:1378` | as stated; refusal order `403 passkeys_not_configured` → `429` → `409`/`503` → `503` → `429 too_many_challenges` | Yes |
| Login options pass `ChallengePurpose::Login`, Funnel only (`403 forbidden` on the tailnet) | `server.rs:1401-1413` | as stated | Yes |
| Funnel gate: non-`is_funnel_public` route without a session ⇒ `403 login_required` before routing; register routes always `403 forbidden` | `server.rs:1190-1205`, `routes.rs:379-390` | `is_funnel_public` = `Asset`, `AuthState`, `LoginOptions`, `LoginVerify`, `Logout`, `NotFound`, `MethodNotAllowed` (no `UnlockOptions`, no `CameraOptions`) | Yes |
| Tailnet callers need exactly one allowed `Tailscale-User-Login` (non-empty allowlist) | `identity.rs:276-304`, `config.rs:233-237` | as stated | Yes |
| `unlock_options`/`camera_options`: CSRF with required `Origin` before any store read; `403 unlock_disabled`/camera gates before `issue_options` | `server.rs:1416-1437`, `:2176-2197` | as stated | Yes |
| `RecordJson` and `StoreJson` are `deny_unknown_fields`; `encode_store` field order; `PasskeyRecord` derives `Clone, PartialEq, Eq` with redacted `Debug` | `credentials.rs:37-189` | as stated | Yes |
| `RegisterVerifyBody` is `deny_unknown_fields`; `decode_registration` runs before `check_code` (a malformed body never consumes the enrollment code) | `auth.rs:625-741`, `server.rs:1745-1790` | as stated | Yes |
| `persist_counter` mutates the record in place (other fields preserved) | `server.rs:586-610` | as stated | Yes |
| `MAX_AUTH_BODY_BYTES` 8192; `MAX_ATTESTATION_OBJECT_BYTES` 2048; `MAX_CREDENTIAL_ID_BYTES` 1023; `MAX_CLIENT_DATA_JSON_BYTES` 1024; existing const assertion | `lib.rs:171-217`, `:279-285` | as stated (5713 + 168 = 5881 < 8192) | Yes |
| `MAX_PASSKEYS` 4; `MAX_CREDENTIAL_STORE_BYTES` 16 384 | `lib.rs:215-217` | as stated; worst record with six transports ≈ 1.7 KiB, 4 records ≈ 6.8 KiB | Yes |
| `MAX_AUTH_FAILURES` 5 per `AUTH_FAILURE_WINDOW_MS` 300 000 | `lib.rs:235-237` | as stated | Yes |
| Every response carries `Cache-Control: no-store` (named options not cached) | `http.rs:263` | as stated | Yes |
| Existing tests pinning the old behaviour: test 39 (`auth_server_tests.rs:675-679`), camera test 24 (`camera_server_tests.rs:246-249`), invariants test 51 (`remote_passkey_contract.rs:308-347`), literals `credentials_tests.rs:42`, `auth_store_tests.rs:778` | grep of `allowCredentials`, `allow_credentials`, `"challenge", "rp_id", "timeout_ms"`, `PasskeyRecord {`, `issue_options`, `options_response`, `getAssertion` over `crates/` and `tests/` | exactly these; test 36 (`auth_server_tests.rs:386-398`) also pins the login shape and is left unchanged; no invariant pins the `§2h` item numbering (RAN-S9 uses needles only) | Yes (exhaustive) |
| `soos-remote passkeys list` / `passkeys remove N` exist | `main.rs:260-330` | as stated | Yes |

## 3. Pillar Analysis

### Pillar 1 — Architecture and threat model
- Failure scenario considered: an anonymous Funnel client calls `POST /api/auth/unlock/options` (or camera) and harvests the credential ids. Excluded: the pre-routing gate refuses every non-public route without a valid session (`server.rs:1199`), and the spec pins this statically (`test_rdb_s5`) and behaviourally (no cookie, unknown cookie, revoked session).
- Failure scenario considered: a future refactor routes login through `Registered`. Excluded by the `OptionsCredentials` argument and the static pin `test_rdb_s1` plus unchanged test 36.
- Failure scenario considered: the allow list influences verification (e.g. an implementation checks the assertion id against the list instead of the store). Excluded: D-6, allow list never reaches `check_assertion`; `test_rdb_assertion_rules_unchanged_with_allow_list`.
- Residual R-1 (authenticated caller learns opaque ids) matches the owner decision and the existing `exclude_credentials` precedent on the tailnet register options.
- Result: PASS.

### Pillar 2 — PAM deadline and concurrency
- Failure scenario considered: added work under the auth mutex delays other requests. ≤ 4 base64url encodings of ≤ 1023 bytes from the snapshot already loaded; no extra I/O, no second store read (no TOCTOU). `crates/pam` untouched.
- Result: PASS (not on the PAM path).

### Pillar 3 — Panic safety and fail-closed
- Failure scenario considered: a corrupted `transports` key in the store. Strict parse ⇒ `StoreError::Malformed` ⇒ existing fail-closed path (`503 store_unavailable`, sessions dropped), never an authorization. Downgrade (A-6) likewise fails closed.
- Failure scenario considered: `getTransports` throws or is missing on an old browser. `try`/`catch` ⇒ `[]` ⇒ `EMPTY` ⇒ default transports.
- Result: PASS.

### Pillar 4 — Dependencies
- No new crate, no JS library. Result: PASS.

### Pillar 5 — Data confidentiality
- Failure scenario considered: a `debug!` of the options response logs ids. No new log line; `test_rdb_options_never_log_a_credential_id` plus the existing `server_tests.rs:1982` leak test. `PasskeyRecord`/`RegisterVerifyBody` `Debug` stay redacted. Body bounded at 8 KiB before deserialisation, so the `Vec<String>` cannot be abused for allocation.
- Result: PASS.

### Pillar 6 — Test integrity and test power
- Failure scenario considered: a developer always emits `DEFAULT_ALLOW` regardless of stored transports. Caught by `test_rdb_register_stores_allowlisted_transports` (`usb` included) but not by RDB1/RDB2, whose stored set canonicalises to the default (PE-2).
- Failure scenario considered: login options name credentials when the Funnel caller already holds a session. Not exercised behaviourally (PE-3).
- Failure scenario considered: the developer follows the §3.5 comment wording, which itself contains the literal `allowCredentials`, so the exact-count assertions of AM-3 and `test_rdb_s1` fail against a correct implementation (PE-1).
- Amendments AM-1 … AM-5 classified in PE-6.
- Result: FINDINGS (MINOR only).

## 4. Findings

- **PE-1 [MINOR]** §3.5 gives `getAssertion` a doc comment "... are passed as allowCredentials so the phone ...", while AM-3 and `test_rdb_s1` require `allowCredentials` to occur **exactly once** in `app.js`. Required change: no comment, string or identifier other than the single `publicKey.allowCredentials` assignment may contain the literal (write "allow list" in comments). The tester and developer must apply this wording.
- **PE-2 [MINOR]** RDB1/RDB2 behaviour tests store `{Hybrid, Internal}`, whose canonical output `["internal","hybrid"]` equals `DEFAULT_ALLOW`, so they cannot distinguish stored from default transports. Required change: the stored record in `test_rdb_unlock_options_name_every_registered_credential` and `test_rdb_camera_options_name_every_registered_credential` uses a set different from the default (e.g. `{Hybrid}` or `{Internal, Usb}`), next to the legacy record that must carry `["internal","hybrid"]`.
- **PE-3 [MINOR]** `test_rdb_login_options_never_name_a_credential` must also call login options from a Funnel caller presenting a **valid session cookie** (the rotation path) and assert the exact key set and absence of both ids, so an implementation keyed on session presence fails.
- **PE-4 [MINOR]** `test_rdb_register_transports_bounds` lists four malformed inputs but asserts "5 such ⇒ `429`". Required change: add a fifth malformed input (e.g. `"transports": "internal"` (string) or `[null]`), then assert the next register verify answers `429 rate_limited`; also assert the enrollment code file still exists after the malformed bodies (the decode runs before `check_code`).
- **PE-5 [MINOR, docs]** Consequences of A-4 and of `excludeCredentials` for the §2h procedure: (a) state that enrollment must be done on the phone itself (a desktop browser or a cross-device QR flow may be refused with `authenticatorAttachment: "platform"`); (b) keep "delete the passkey in Proton Pass" before re-enrolling, and say that if the phone reports the passkey "already exists" the old one is still present in an enabled provider. Wording only; no test change beyond the needles of `test_rdb_s6`.
- **PE-6 [OBSERVATION — amendment classification]** AM-1(b) and AM-3 touch security-relevant assertions. Evaluated as **owner-required exact replacements, not relaxations**: OD part 2 explicitly requires the unlock and camera options of an authenticated caller to carry the credential ids and the page to send `allowCredentials`, which is impossible while those two assertions stand. The property they protected (S-3: no credential id to an anonymous client, sign-in never names a credential) is preserved and strengthened: AM-1(b) becomes an exact-content assertion on an authenticated tailnet caller; the anonymous direction stays pinned by unchanged test 36 and new `test_rdb_login_options_never_name_a_credential`, `test_rdb_named_options_refused_without_funnel_session`, `test_rdb_refusals_never_name_a_credential`; AM-3 replaces the needle by an exact count of 1 plus the `getAssertion(options.body, false)` call-site pin, following the in-test "Contract migration" precedent (service worker). AM-2 is an exact key-set replacement; AM-4/AM-5 are additive setup. Conditions: every other `forbidden` needle of test 51 stays unchanged, and no further existing test is modified. **Approved.**

## 5. Verdict

No CRITICAL or MAJOR finding. PE-1 to PE-5 are MINOR plan clarifications to be applied by the tester (PE-1 wording rule, PE-2, PE-3, PE-4) and by the traceability/docs phase (PE-5); PE-6 approves amendments AM-1 to AM-5 as listed in spec §9.

VALIDATION_VERDICT: APPROVED
