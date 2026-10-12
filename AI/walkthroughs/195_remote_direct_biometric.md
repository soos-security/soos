# Walkthrough 195 — soos-remote: Direct Biometric Prompt on Android

- **Date**: 2026-10-12
- **Issue**: GitHub #354, sub-issue 1 (GitHub-only; no `AI/BACKLOG.md` entry, not registered in
  `scripts/sync_issue.py`; commits say `Refs #354`). **Branch**: `feat/remote-direct-biometric`.
  Nothing is pushed, installed, enabled or restarted by the agents; no agent runs `tailscale`, `sudo`
  or `curl` against a live endpoint. Merge only after explicit owner approval (OD-6).
- **ADR**: "[2026-10-12] Named Credentials on Authenticated Passkey Ceremonies of `soos-remote`"
  (`AI/DECISIONS.md`); it amends, without superseding, the 2026-10-06 passkey entry.
- **Documents**: spec `AI/architect_spec_remote_direct_biometric.md` (APPROVED, PE-1 to PE-6); plan
  evaluation `AI/plan_evaluator_report.md`; tester contract
  `AI/tester_contract_remote_direct_biometric.md`; auditor constraints
  `AI/auditor_constraints_remote_direct_biometric.md` (C-1 to C-16, CLEARED).
- **Matrix criteria**: RDB1–RDB10 (component `remote-direct-biometric`; RDB10 is the owner hardware
  check, pending).

## 1. Context & Objectives

On Android, the unlock and camera-start passkey prompts of the phone page opened a credential-provider
picker (Google Password Manager, Proton Pass, ...) before the fingerprint prompt, because every
ceremony was discoverable (no `allowCredentials`). The owner decided (issue #354, owner decisions
parts 1 and 2):

- OD-1: device-bound and password-manager passkeys stay accepted (no `BE=0` policy);
- OD-2: remote unlock keeps working over Funnel;
- OD-3: sign-in stays discoverable: no credential id in the public login options;
- OD-4: unlock and camera start, which already require a session, name the registered credentials;
- OD-5: the docs explain the Android preferred passkey service and the Proton Pass re-enrollment.

## 2. Design

| Area | Change |
|---|---|
| `credentials.rs` | `AuthenticatorTransport` (the six WebAuthn L3 names, canonical order), `TransportSet` (private `u8` bitset), `PasskeyRecord.transports`, optional `transports` key in the store record (omitted when empty, strictly validated on read), `allow_credentials(&PasskeyFile)`. |
| `auth.rs` | `RegisterVerifyBody.transports` (optional), bounded decoding: ≤ 8 entries of ≤ 16 UTF-8 bytes else `400 bad_request`; unknown names ignored, duplicates collapsed, exact case-sensitive names. |
| `server.rs` | `OptionsCredentials { Discoverable, Registered }` argument of `issue_options`; `login_options` passes `Discoverable`, `unlock_options` and `camera_options` pass `Registered`; the list is built from the same store snapshot that passed the non-empty check under the same mutex hold; `register_verify` stores the transports. |
| `lib.rs` | `MAX_TRANSPORT_FIELD_ENTRIES` (8), `MAX_TRANSPORT_NAME_BYTES` (16), one more compile-time body bound. |
| `app.js` | `getAssertion(options, nameCredentials)` sets the allow list only for unlock and camera; registration adds `authenticatorAttachment: "platform"`, `hints: ["client-device"]` and the filtered `getTransports()` list. |

Unchanged: WebAuthn verification (UV, `userHandle` required, BE/BS, counter, origin, challenge
purpose/class/binding), CSP, CSRF, Funnel gates (`is_funnel_public`), limiter keys, RC-1,
`MAX_AUTH_BODY_BYTES`. The allow list is a UX hint and never reaches the verifier.

## 3. Tests

- `crates/remote/tests/direct_biometric_tests.rs`: 12 end-to-end tests (RDB1–RDB4, RDB6–RDB8).
- `crates/remote/tests/credentials_tests.rs`: 3 store and allow-list tests (RDB7).
- `tests/invariants/src/remote_direct_biometric_contract.rs`: S-1 to S-6 (RDB3–RDB6, RDB9).
- Approved amendments AM-1 to AM-5 (spec §9, PE-6).

## 4. Implementation notes

- An empty `TransportSet` is not serialised, so a store without transports encodes byte-identically to
  the previous format; legacy records are named with `TransportSet::DEFAULT_ALLOW`
  (`["internal", "hybrid"]`).
- **Downgrade (A-6)**: an older `soos-remote` refuses a store file holding a `transports` key
  (`deny_unknown_fields`) and fails closed (`503 store_unavailable`). Rollback: re-enroll the passkeys,
  or delete the `transports` keys from `remote-passkeys.json` by hand.

## 5. Owner hardware check (RDB10, pending)

On the owner's Android phone, after re-enrolling a passkey stored on the phone
(`Docs/REMOTE_COMPANION.md` §2h item 5), unlock and camera start show the fingerprint/face prompt
directly with no provider picker, over the tailnet and over Funnel; sign-in may still show the picker;
the iPhone flow still works.
