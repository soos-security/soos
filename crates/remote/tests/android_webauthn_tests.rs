//! Android coverage contract of the pure WebAuthn verification subset of `soos-remote`
//! (GitHub #349, ADR 2026-10-09 "Android Support for the `soos-remote` Phone Companion and Web
//! Push", architect spec `AI/architect_spec_remote_android.md` §0.1 and §11.2; matrix RAN10).
//!
//! Chrome on Android adds an `other_keys_can_be_added_here` member to `clientDataJSON`; Google
//! Password Manager creates synced passkeys (BE=1, BS=1), with a zero counter that never
//! moves and a real AAGUID in the `none` attestation. The verifier already accepts these
//! (spec §0.1); this suite pins that behaviour. Deterministic fixtures, no network.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

#[path = "common/passkey.rs"]
mod passkey;

use passkey::*;
use soos_remote::webauthn::{
    parse_client_data, verify_assertion, verify_registration, AssertionOutcome, CeremonyType,
    RelyingParty, StoredCredential, WebAuthnError,
};
use soos_remote::{CHALLENGE_BYTES, USER_HANDLE_BYTES};

const HOST: &str = "pc.tail1234.ts.net";
const ORIGIN: &str = "https://pc.tail1234.ts.net";
const HANDLE: [u8; USER_HANDLE_BYTES] = [0x5a; USER_HANDLE_BYTES];

/// The AAGUID of Google Password Manager passkeys.
const GPM_AAGUID: &str = "ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4";

/// The member Chrome appends (verbatim, as shipped).
const CHROME_EXTRA: &str = ",\"other_keys_can_be_added_here\":\"do not compare clientDataJSON against a template. See https://goo.gl/yabPex\"";

fn rp() -> RelyingParty {
    RelyingParty::new(HOST)
}

fn challenge() -> [u8; CHALLENGE_BYTES] {
    let mut c = [0u8; CHALLENGE_BYTES];
    for (i, b) in c.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(11).wrapping_add(5);
    }
    c
}

/// Chrome's `clientDataJSON` of `kind` with `crossOrigin` = `cross_origin`.
fn chrome_client_data(kind: &str, cross_origin: bool) -> Vec<u8> {
    format!(
        "{{\"type\":\"{kind}\",\"challenge\":\"{}\",\"origin\":\"{ORIGIN}\",\"crossOrigin\":{cross_origin}{CHROME_EXTRA}}}",
        b64url(&challenge())
    )
    .into_bytes()
}

/// The 16 AAGUID bytes of a canonical UUID string.
fn aaguid_bytes(uuid: &str) -> [u8; 16] {
    let raw = hex(&uuid.replace('-', ""));
    raw.as_slice().try_into().expect("16 bytes")
}

/// Registration `authenticatorData` like `passkey::registration_auth_data`, but with the
/// given AAGUID instead of zeros.
fn registration_auth_data_with_aaguid(
    rp_id: &str,
    flags: u8,
    sign_count: u32,
    aaguid: [u8; 16],
    credential_id: &[u8],
    cose_key: &[u8],
) -> Vec<u8> {
    let mut out = sha256(rp_id.as_bytes()).to_vec();
    out.push(flags);
    out.extend_from_slice(&sign_count.to_be_bytes());
    out.extend_from_slice(&aaguid);
    out.extend_from_slice(&(credential_id.len() as u16).to_be_bytes());
    out.extend_from_slice(credential_id);
    out.extend_from_slice(cose_key);
    out
}

/// A Google Password Manager passkey (synced: BE|BS on every ceremony).
fn gpm() -> Authenticator {
    Authenticator::new(0x2a, b"gpm-passkey-credential-id-0042-android", true)
}

/// RAN10 (spec §11.2): Chrome's `clientDataJSON` with its extra member parses for both
/// ceremonies and a signed assertion over it verifies; `crossOrigin: true` is still refused.
#[test]
fn test_ran_chrome_client_data_with_extra_members_verifies() {
    let rp = rp();
    for (kind, ceremony) in [
        ("webauthn.get", CeremonyType::Get),
        ("webauthn.create", CeremonyType::Create),
    ] {
        let data = chrome_client_data(kind, false);
        let parsed = parse_client_data(&rp, &data, ceremony)
            .unwrap_or_else(|e| panic!("{kind}: Chrome clientDataJSON refused: {e:?}"));
        assert_eq!(parsed.challenge, challenge(), "{kind}");
        let cross = chrome_client_data(kind, true);
        assert_eq!(
            parse_client_data(&rp, &cross, ceremony).err(),
            Some(WebAuthnError::CrossOrigin),
            "{kind}: crossOrigin true is refused"
        );
    }

    // A signed assertion over the exact Chrome bytes verifies.
    let auth = gpm();
    let public_key = auth.public_key();
    let stored = StoredCredential {
        credential_id: &auth.credential_id,
        public_key: &public_key,
        sign_count: 0,
        backup_eligible: true,
    };
    let data = chrome_client_data("webauthn.get", false);
    let ad = assertion_auth_data(HOST, UP | UV | BE | BS, 0);
    let sig = auth.sign(&ad, &data);
    assert_eq!(
        verify_assertion(&rp, &stored, &HANDLE, &data, &ad, &sig, Some(&HANDLE)),
        Ok(AssertionOutcome {
            sign_count: 0,
            backup_state: true
        })
    );
    // The signature covers the exact bytes: dropping Chrome's member breaks it.
    let without = client_data("webauthn.get", &b64url(&challenge()), ORIGIN, "");
    assert_eq!(
        verify_assertion(&rp, &stored, &HANDLE, &without, &ad, &sig, Some(&HANDLE)),
        Err(WebAuthnError::SignatureInvalid)
    );
}

/// RAN10 (spec §11.2): a Google Password Manager registration (flags `UP|UV|AT|BE|BS` =
/// 0x5D, counter 0, AAGUID `ea9b8d66-…`, `fmt: "none"`, empty `attStmt`) is accepted with
/// both backup flags and a zero counter.
#[test]
fn test_ran_google_password_manager_registration_verifies() {
    let auth = gpm();
    let flags = UP | UV | AT | BE | BS;
    assert_eq!(flags, 0x5D);
    let aaguid = aaguid_bytes(GPM_AAGUID);
    assert_eq!(aaguid[0], 0xea);
    assert_eq!(aaguid[15], 0xd4);
    let auth_data = registration_auth_data_with_aaguid(
        HOST,
        flags,
        0,
        aaguid,
        &auth.credential_id,
        &auth.cose_key(),
    );
    assert_eq!(
        &auth_data[37..53],
        &aaguid,
        "the AAGUID sits after rpIdHash|flags|counter"
    );
    let attestation = none_attestation(&auth_data);
    let new = verify_registration(&rp(), &auth.credential_id, &attestation)
        .unwrap_or_else(|e| panic!("Google Password Manager registration refused: {e:?}"));
    assert_eq!(new.credential_id, auth.credential_id);
    assert_eq!(new.public_key, auth.public_key());
    assert_eq!(new.sign_count, 0);
    assert!(new.backup_eligible, "BE");
    assert!(new.backup_state, "BS");

    // Its Chrome create clientDataJSON parses too.
    let data = chrome_client_data("webauthn.create", false);
    assert!(parse_client_data(&rp(), &data, CeremonyType::Create).is_ok());
}

/// RAN10 (spec §11.2): two consecutive zero-counter assertions of a synced passkey verify
/// against a stored zero counter; BS may flip 1 → 0 → 1; BE=0 against a stored BE=1 is still
/// refused.
#[test]
fn test_ran_google_password_manager_assertion_with_zero_counter_verifies() {
    let rp = rp();
    let auth = gpm();
    let public_key = auth.public_key();
    let stored = StoredCredential {
        credential_id: &auth.credential_id,
        public_key: &public_key,
        sign_count: 0,
        backup_eligible: true,
    };
    let data = chrome_client_data("webauthn.get", false);
    for (round, backup_state) in [true, true, false, true].into_iter().enumerate() {
        let flags = if backup_state {
            UP | UV | BE | BS
        } else {
            UP | UV | BE
        };
        let ad = assertion_auth_data(HOST, flags, 0);
        let sig = auth.sign(&ad, &data);
        assert_eq!(
            verify_assertion(&rp, &stored, &HANDLE, &data, &ad, &sig, Some(&HANDLE)),
            Ok(AssertionOutcome {
                sign_count: 0,
                backup_state
            }),
            "round {round}: zero counter, BS {backup_state}"
        );
    }
    let ad = assertion_auth_data(HOST, UP | UV, 0);
    let sig = auth.sign(&ad, &data);
    assert_eq!(
        verify_assertion(&rp, &stored, &HANDLE, &data, &ad, &sig, Some(&HANDLE)),
        Err(WebAuthnError::BackupEligibilityChanged),
        "BE 0 against a stored BE 1"
    );
}
