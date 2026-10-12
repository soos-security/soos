# Candid Review Report

- **Date**: 2026-10-12
- **Target Branch**: `feat/remote-direct-biometric`
- **Base (merge-base)**: `fb2977c9`
- **Reviewed-Diff-Fingerprint**: `4a4d0c3c617be0f4923924f5037ec9e8d281b886604031a9b23a4b18d65c5345`
- **Audited Files**: `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_direct_biometric.md`, `AI/auditor_constraints_remote_direct_biometric.md`, `AI/tester_contract_remote_direct_biometric.md`, `AI/walkthroughs/195_remote_direct_biometric.md`, `Docs/REMOTE_COMPANION.md`, `crates/remote/assets/app.js`, `crates/remote/src/auth.rs`, `crates/remote/src/credentials.rs`, `crates/remote/src/lib.rs`, `crates/remote/src/server.rs`, `crates/remote/tests/auth_server_tests.rs`, `crates/remote/tests/auth_store_tests.rs`, `crates/remote/tests/camera_server_tests.rs`, `crates/remote/tests/credentials_tests.rs`, `crates/remote/tests/direct_biometric_tests.rs`, `tests/invariants/src/lib.rs`, `tests/invariants/src/remote_direct_biometric_contract.rs`, `tests/invariants/src/remote_passkey_contract.rs`

## 1. Executive Summary

The change makes the `soos-remote` unlock and camera-start passkey ceremonies name every
registered credential (`allow_credentials` = `{id, transports}` per record) so Android goes
straight to the biometric prompt, keeps sign-in discoverable (no credential id to an anonymous
Funnel caller), adds `authenticatorAttachment: "platform"` and `hints: ["client-device"]` at
registration, and stores browser-reported transports in an optional, strictly validated record
key. The diff is confined to the user-level `soos-remote` crate, its page and docs; PAM, daemon
and IPC are untouched. No CRITICAL or MAJOR defect found. `cargo test -p soos-remote` (0 failures),
`cargo test --locked --all-features -p soos-remote --no-fail-fast` (386 passed, 0 failed) and
`cargo test --locked --all-features -p soos-invariants` (571 passed, 0 failed) were re-run for
this review and match the counts claimed in the walkthrough and the matrix.

**Re-review scope (this fingerprint).** Since the previous fingerprint
(`64cd78f1…`), the diff gained only documentation (commit `37c2bdb`):
`AI/VERIFICATION_MATRIX.md` component `remote-direct-biometric` (RDB1–RDB10) and the rewritten
walkthrough 195. The whole raw diff was re-read; production code and tests are byte-identical to
the previously approved diff. The new docs were checked against the code:
- every test cited in RDB1–RDB9 exists (the matrix citation invariants pass in the 571 run);
- per-file counts in walkthrough §8 (`direct_biometric_tests` 12, `credentials_tests` 7,
  `auth_server_tests` 17, `auth_store_tests` 6, `camera_server_tests` 15) match the `#[test]` /
  `#[tokio::test]` counts in the tree;
- cited commits `7aa93dc`, `2d8c8f0`, `7f2f395`, `c1ec02b`, `d2c581d` exist with the described
  subjects; `git diff 2d8c8f0 c1ec02b -- crates/remote/tests tests/invariants` shows only the
  auditor's additive 45-line test (C-15 claim true), and the developer commit touches no test;
- walkthrough §2/§5 claims (`persist_counter` mutates the record in place, `is_funnel_public`
  untouched, `webauthn.rs` and `routes.rs` without diff, bounds checked before `check_code`) are
  true in the code; AM-1..AM-5 descriptions match the test hunks exactly.

## 2. Test Changes

Mechanical listing (step 3):

- Removed check lines: `crates/remote/tests/auth_server_tests.rs` test 39
  (`assert!(!body.contains(owner credential id))`) and the `"allowCredentials"` needle of the
  forbidden list in `remote_passkey_contract.rs` test 51 (plus the JS comment line in `app.js`,
  not a test).
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.

Justification per change (tester contract "Contract Migration" table AM-1..AM-5, ADR 2026-10-12):

- **AM-1** (`auth_server_tests` test 39): the key-set assertion is amended to include
  `allow_credentials`, and the removed non-disclosure assertion is replaced by an exact-equality
  assertion on `allow_credentials` (`[{"id": <owner>, "transports": ["internal","hybrid"]}]`).
  Stricter than before; the anonymous non-disclosure property moved to the login route, which is
  pinned by test 36 and the new `test_rdb_login_options_never_name_a_credential`,
  `test_rdb_named_options_refused_without_funnel_session` and
  `test_rdb_refusals_never_name_a_credential`. Justified by OD-4.
- **AM-2** (`camera_server_tests` test 24): key set gains `allow_credentials`. Justified by OD-4.
- **AM-3** (test 51): the forbidden needle is replaced by two positive constraints
  (`allowCredentials` occurs exactly once, sign-in calls `getAssertion(options.body, false)`);
  every other forbidden needle is unchanged. Justified by OD-3/OD-4.
- **AM-4 / AM-5**: setup-only `transports: Default::default()` for the new public field.

No test weakening without an owner-decision-backed contract migration.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Anonymous Funnel caller requests `POST /api/auth/unlock/options` or `/camera/options` to harvest
  credential ids → `is_funnel_public` (`routes.rs:379`) excludes both routes, so the dispatcher
  answers `403 login_required` before the handler; covered by RDB S5 and
  `test_rdb_named_options_refused_without_funnel_session`. PASS.
- Refusal paths (`409 no_passkey`, `429`, store error, `503` random failure) carrying ids →
  `allow` is computed only in the non-empty-store arm and only passed to `options_response` on
  success. PASS.
- TOCTOU between the non-empty check and the list → both come from the same `store_snapshot()`
  value. PASS.
- Sign-count persistence dropping transports → the record is mutated in place
  (`server.rs:612`), the only `PasskeyRecord` construction in production is registration;
  covered by `test_rdb_counter_update_keeps_transports`. PASS.
- Legacy store files without the key → `#[serde(default)]` plus `skip_serializing_if` keeps the
  encoding byte-identical when empty. PASS.
- Allow list affecting verification → the list is only an options hint; `verify_assertion` and
  its inputs are unchanged (`test_rdb_assertion_rules_unchanged_with_allow_list`). PASS.
- Client sends `["usb"]` only → stored as-is, later offered as the only transport. Self-inflicted
  by the enrolling (tailnet + one-time code) owner; no security impact. PASS.

### PAM Concurrency & Deadlines

No change to `crates/pam`, daemon or IPC. N/A, PASS.

### Panic Safety & Fail-Closed

- New code has no `unwrap`/`expect`/indexing; `body.as_object_mut()` is matched with `if let`;
  `TransportSet` bits are a closed `match`. Store parsing of an unknown, duplicate or > 6
  transport name is `StoreError::Malformed` (fail closed, same as other malformed fields). PASS.

### Test Integrity & Anti-Weakening

See §2. New tests would fail against plausible wrong implementations (ids on login, ids on
refusals, ids for an anonymous Funnel caller, lost transports on counter update, non-canonical
order, case-insensitive names, oversized field). PASS.

### Memory, Bounds & Secrets

- Registration `transports` field: serde decodes inside the existing `MAX_AUTH_BODY_BYTES` body
  bound, then `> 8` entries or any entry `> 16` bytes ⇒ `400`; a compile-time assertion keeps
  the body bound consistent. Store side: at most 6 names per record, 4 records, well inside
  `MAX_CREDENTIAL_STORE_BYTES`. PASS.
- Credential ids are public identifiers; `RegisterVerifyBody` Debug stays redacted;
  `test_rdb_options_never_log_a_credential_id` checks logs. PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. N/A, PASS.

### English-Only Policy

All code, comments, docs and walkthrough text in the diff are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/remote/src/credentials.rs:221` (`RecordJson` with `deny_unknown_fields`) —
  a rollback to an older `soos-remote` refuses a store holding a `transports` key. Already
  documented as accepted risk A-6 (spec) and in walkthrough 195 (rollback = re-enroll or delete
  the key by hand). No action required.
- **[SUGGESTION]** `crates/remote/src/lib.rs:290` — the new compile-time body bound counts
  `name + 3` per entry but relies on the shared `256` slack for the `"transports":[]` key; it
  holds with a wide margin (8192 bytes), so it is informative only.

- **[SUGGESTION]** `AI/walkthroughs/195_remote_direct_biometric.md` §7 — it cites the earlier
  report fingerprint `64cd78f1…`; the committed report now carries `4a4d0c3c…` (the docs-only
  re-review). The text says the report predates the traceability commit, so it is historically
  accurate; optionally refresh the fingerprint at the next docs touch (doing so would change the
  diff and require another review).

## 5. Final Verdict

**VERDICT: APPROVED**
