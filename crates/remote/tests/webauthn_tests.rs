//! Contract tests of the pure WebAuthn verification subset of `soos-remote` (ADR 2026-10-06
//! "Tailscale Funnel Access and In-House Passkey Authentication for `soos-remote`",
//! architect spec §4.1, tests 16–22, matrix RMC30 / RMC31 / RMC38).
//!
//! Fixtures are deterministic: P-256 keys from fixed seeds, RFC 6979 signatures, CBOR built
//! with `ciborium`, and the verbatim WebAuthn L3 §16.2 vector as a negative "UV missing"
//! fixture. Nothing here touches the network, a socket or logind.

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

use ciborium::value::Value;
use p256::ecdsa::signature::Verifier;

use passkey::*;
use soos_remote::webauthn::{
    b64url_decode, b64url_encode, flags, parse_client_data, verify_assertion, verify_registration,
    AssertionOutcome, CeremonyType, RelyingParty, StoredCredential, WebAuthnError,
};
use soos_remote::{
    ASSERTION_AUTH_DATA_LEN, CHALLENGE_BYTES, MAX_ATTESTATION_OBJECT_BYTES,
    MAX_CLIENT_DATA_JSON_BYTES, MAX_CREDENTIAL_ID_BYTES, MAX_SIGNATURE_BYTES, USER_HANDLE_BYTES,
};

const HOST: &str = "pc.tail1234.ts.net";
const ORIGIN: &str = "https://pc.tail1234.ts.net";
const HANDLE: [u8; USER_HANDLE_BYTES] = [0x5a; USER_HANDLE_BYTES];

fn rp() -> RelyingParty {
    RelyingParty::new(HOST)
}

fn challenge() -> [u8; CHALLENGE_BYTES] {
    let mut c = [0u8; CHALLENGE_BYTES];
    for (i, b) in c.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(7).wrapping_add(3);
    }
    c
}

fn get_data() -> Vec<u8> {
    client_data("webauthn.get", &b64url(&challenge()), ORIGIN, "")
}

// ---------------------------------------------------------------------------------------
// Test 16 — clientDataJSON
// ---------------------------------------------------------------------------------------

/// Test 16 (RMC30/31): `type`, byte-exact `origin` (no port, no trailing `/`), `crossOrigin`,
/// `topOrigin`, unknown keys tolerated, duplicate keys refused, challenge base64url without
/// padding of exactly `CHALLENGE_BYTES`, size bound `MAX_CLIENT_DATA_JSON_BYTES`.
#[test]
fn test_rmc_parse_client_data_rules() {
    let rp = rp();
    assert_eq!(rp.rp_id(), HOST);
    assert_eq!(rp.origin(), ORIGIN, "origin is exactly https:// + rp_id");

    let ok = parse_client_data(&rp, &get_data(), CeremonyType::Get).expect("valid get");
    assert_eq!(ok.challenge, challenge());
    let create = client_data("webauthn.create", &b64url(&challenge()), ORIGIN, "");
    assert_eq!(
        parse_client_data(&rp, &create, CeremonyType::Create)
            .expect("valid create")
            .challenge,
        challenge()
    );
    // crossOrigin absent is accepted; unknown keys (even nested) are ignored.
    let absent = format!(
        "{{\"type\":\"webauthn.get\",\"challenge\":\"{}\",\"origin\":\"{ORIGIN}\"}}",
        b64url(&challenge())
    );
    assert!(parse_client_data(&rp, absent.as_bytes(), CeremonyType::Get).is_ok());
    let extra = client_data(
        "webauthn.get",
        &b64url(&challenge()),
        ORIGIN,
        ",\"extraData\":\"future\",\"other\":{\"nested\":[1,2,3]}",
    );
    assert!(parse_client_data(&rp, &extra, CeremonyType::Get).is_ok());

    // Ceremony type.
    assert_eq!(
        parse_client_data(&rp, &get_data(), CeremonyType::Create).err(),
        Some(WebAuthnError::WrongType)
    );
    assert_eq!(
        parse_client_data(&rp, &create, CeremonyType::Get).err(),
        Some(WebAuthnError::WrongType)
    );
    let foo = client_data("webauthn.foo", &b64url(&challenge()), ORIGIN, "");
    assert_eq!(
        parse_client_data(&rp, &foo, CeremonyType::Get).err(),
        Some(WebAuthnError::WrongType)
    );

    // Origin: byte for byte.
    for origin in [
        "https://pc.tail1234.ts.net:443",
        "https://pc.tail1234.ts.net/",
        "http://pc.tail1234.ts.net",
        "https://PC.tail1234.ts.net",
        "https://other.tail1234.ts.net",
        "https://tail1234.ts.net",
        "https://pc.tail1234.ts.net.",
        "null",
        "",
    ] {
        let data = client_data("webauthn.get", &b64url(&challenge()), origin, "");
        assert_eq!(
            parse_client_data(&rp, &data, CeremonyType::Get).err(),
            Some(WebAuthnError::OriginMismatch),
            "{origin:?}"
        );
    }

    // crossOrigin / topOrigin.
    let cross = format!(
        "{{\"type\":\"webauthn.get\",\"challenge\":\"{}\",\"origin\":\"{ORIGIN}\",\"crossOrigin\":true}}",
        b64url(&challenge())
    );
    assert_eq!(
        parse_client_data(&rp, cross.as_bytes(), CeremonyType::Get).err(),
        Some(WebAuthnError::CrossOrigin)
    );
    let top = client_data(
        "webauthn.get",
        &b64url(&challenge()),
        ORIGIN,
        ",\"topOrigin\":\"https://evil.example\"",
    );
    let err = parse_client_data(&rp, &top, CeremonyType::Get).err();
    assert!(
        matches!(
            err,
            Some(WebAuthnError::CrossOrigin | WebAuthnError::ClientDataMalformed)
        ),
        "topOrigin is refused: {err:?}"
    );

    // Duplicate known keys are refused (no first/last-wins ambiguity).
    let duplicate = format!(
        "{{\"type\":\"webauthn.create\",\"type\":\"webauthn.get\",\"challenge\":\"{}\",\"origin\":\"{ORIGIN}\"}}",
        b64url(&challenge())
    );
    assert_eq!(
        parse_client_data(&rp, duplicate.as_bytes(), CeremonyType::Get).err(),
        Some(WebAuthnError::ClientDataMalformed)
    );
    let duplicate_origin = format!(
        "{{\"type\":\"webauthn.get\",\"challenge\":\"{}\",\"origin\":\"https://evil.example\",\"origin\":\"{ORIGIN}\"}}",
        b64url(&challenge())
    );
    assert!(
        parse_client_data(&rp, duplicate_origin.as_bytes(), CeremonyType::Get).is_err(),
        "a duplicate origin key is refused"
    );

    // Challenge encoding and length.
    let good = b64url(&challenge());
    for bad in [
        format!("{good}="),
        good.replace('-', "+").replace('_', "/") + "+/",
        b64url(&[7u8; CHALLENGE_BYTES - 1]),
        b64url(&[7u8; CHALLENGE_BYTES + 1]),
        String::new(),
        "!!!!".to_string(),
    ] {
        let data = client_data("webauthn.get", &bad, ORIGIN, "");
        assert!(
            parse_client_data(&rp, &data, CeremonyType::Get).is_err(),
            "challenge {bad:?} is refused"
        );
    }

    // Shape.
    for raw in [
        b"".to_vec(),
        b"[]".to_vec(),
        b"\"webauthn.get\"".to_vec(),
        b"{".to_vec(),
        vec![0xff, 0xfe, b'{', b'}'],
        format!("{{\"type\":1,\"challenge\":\"{good}\",\"origin\":\"{ORIGIN}\"}}").into_bytes(),
        format!("{{\"challenge\":\"{good}\",\"origin\":\"{ORIGIN}\"}}").into_bytes(),
        format!("{{\"type\":\"webauthn.get\",\"origin\":\"{ORIGIN}\"}}").into_bytes(),
        format!("{{\"type\":\"webauthn.get\",\"challenge\":\"{good}\"}}").into_bytes(),
    ] {
        assert_eq!(
            parse_client_data(&rp, &raw, CeremonyType::Get).err(),
            Some(WebAuthnError::ClientDataMalformed),
            "{:?}",
            String::from_utf8_lossy(&raw)
        );
    }

    // Size bound: exactly MAX_CLIENT_DATA_JSON_BYTES is accepted, one more byte refused.
    let base = client_data("webauthn.get", &good, ORIGIN, ",\"pad\":\"\"");
    let fill = MAX_CLIENT_DATA_JSON_BYTES - base.len();
    let at_bound = client_data(
        "webauthn.get",
        &good,
        ORIGIN,
        &format!(",\"pad\":\"{}\"", "a".repeat(fill)),
    );
    assert_eq!(at_bound.len(), MAX_CLIENT_DATA_JSON_BYTES);
    assert!(parse_client_data(&rp, &at_bound, CeremonyType::Get).is_ok());
    let over = client_data(
        "webauthn.get",
        &good,
        ORIGIN,
        &format!(",\"pad\":\"{}\"", "a".repeat(fill + 1)),
    );
    assert_eq!(over.len(), MAX_CLIENT_DATA_JSON_BYTES + 1);
    assert_eq!(
        parse_client_data(&rp, &over, CeremonyType::Get).err(),
        Some(WebAuthnError::ClientDataMalformed)
    );

    // The verbatim L3 §16.2 registration client data (with its extraData key) is valid
    // for its own RP.
    let l3 = RelyingParty::new(l3_16_2::RP_ID);
    let parsed = parse_client_data(&l3, &hex(l3_16_2::REG_CLIENT_DATA), CeremonyType::Create)
        .expect("L3 §16.2 client data");
    assert_eq!(
        parsed.challenge.to_vec(),
        b64url_decode_ref("AMMPt4UxxGTStncdq417YDwBFi8vpIa-pw8oOuVW4TA")
    );
}

fn b64url_decode_ref(text: &str) -> Vec<u8> {
    passkey::b64url_decode(text)
}

// ---------------------------------------------------------------------------------------
// Tests 17–19 — registration
// ---------------------------------------------------------------------------------------

/// Test 17 (RMC30): a generated key with `UP|UV|AT` (with or without `BE|BS`) yields the
/// credential id, the SEC1 point, the counter and the backup flags.
#[test]
fn test_rmc_verify_registration_accepts_a_valid_none_attestation() {
    let device = Authenticator::device(0x21, b"device-credential-0001");
    let att = registration(&device, HOST, UP | UV | AT);
    let new = verify_registration(&rp(), &device.credential_id, &att).expect("valid");
    assert_eq!(new.credential_id, device.credential_id);
    assert_eq!(new.public_key, device.public_key());
    assert_eq!(new.sign_count, 0);
    assert!(!new.backup_eligible);
    assert!(!new.backup_state);

    let synced = Authenticator::owner();
    let att = registration(&synced, HOST, UP | UV | AT | BE | BS);
    let new = verify_registration(&rp(), &synced.credential_id, &att).expect("valid synced");
    assert_eq!(new.public_key, synced.public_key());
    assert!(new.backup_eligible);
    assert!(new.backup_state);

    let att = registration(&synced, HOST, UP | UV | AT | BE);
    let new = verify_registration(&rp(), &synced.credential_id, &att).expect("BE without BS");
    assert!(new.backup_eligible);
    assert!(!new.backup_state);

    // A non-zero initial counter is kept.
    let att = none_attestation(&registration_auth_data(
        HOST,
        UP | UV | AT,
        42,
        &device.credential_id,
        &device.cose_key(),
    ));
    let new = verify_registration(&rp(), &device.credential_id, &att).expect("counter 42");
    assert_eq!(new.sign_count, 42);

    // Bounds: a 1-byte and a MAX_CREDENTIAL_ID_BYTES credential id are accepted.
    for len in [1, MAX_CREDENTIAL_ID_BYTES] {
        let id = vec![0xab; len];
        let att = none_attestation(&registration_auth_data(
            HOST,
            UP | UV | AT,
            0,
            &id,
            &device.cose_key(),
        ));
        assert!(att.len() <= MAX_ATTESTATION_OBJECT_BYTES);
        let new = verify_registration(&rp(), &id, &att).expect("id length in bounds");
        assert_eq!(new.credential_id, id);
    }
}

fn reg_err(raw_id: &[u8], att: &[u8]) -> WebAuthnError {
    match verify_registration(&rp(), raw_id, att) {
        Ok(_) => panic!("registration must be refused"),
        Err(e) => e,
    }
}

fn reg_with_cose(device: &Authenticator, cose: &[u8]) -> Vec<u8> {
    none_attestation(&registration_auth_data(
        HOST,
        UP | UV | AT,
        0,
        &device.credential_id,
        cose,
    ))
}

/// Test 18 (RMC30): every registration rejection of spec §4.1, each against an otherwise
/// valid fixture.
#[test]
#[allow(
    clippy::type_complexity,
    reason = "Table-driven COSE mutations of the fixture entries"
)]
fn test_rmc_verify_registration_rejections() {
    use WebAuthnError::*;
    let device = Authenticator::device(0x22, b"device-credential-0002");
    let id = device.credential_id.clone();
    let auth_data = registration_auth_data(HOST, UP | UV | AT, 0, &id, &device.cose_key());
    assert!(
        verify_registration(&rp(), &id, &none_attestation(&auth_data)).is_ok(),
        "baseline is valid"
    );

    // Top level.
    let packed = attestation_from_entries(vec![
        (text("fmt"), text("packed")),
        (text("attStmt"), Value::Map(Vec::new())),
        (text("authData"), Value::Bytes(auth_data.clone())),
    ]);
    assert_eq!(reg_err(&id, &packed), UnsupportedAttestation);
    let with_stmt = attestation_from_entries(vec![
        (text("fmt"), text("none")),
        (text("attStmt"), Value::Map(vec![(text("alg"), int(-7))])),
        (text("authData"), Value::Bytes(auth_data.clone())),
    ]);
    assert!(matches!(
        reg_err(&id, &with_stmt),
        UnsupportedAttestation | AttestationMalformed
    ));
    let extra_key = attestation_from_entries(vec![
        (text("fmt"), text("none")),
        (text("attStmt"), Value::Map(Vec::new())),
        (text("authData"), Value::Bytes(auth_data.clone())),
        (text("ep"), Value::Bool(true)),
    ]);
    assert_eq!(reg_err(&id, &extra_key), AttestationMalformed);
    let missing = attestation_from_entries(vec![
        (text("fmt"), text("none")),
        (text("attStmt"), Value::Map(Vec::new())),
    ]);
    assert_eq!(reg_err(&id, &missing), AttestationMalformed);
    let duplicate = attestation_from_entries(vec![
        (text("fmt"), text("none")),
        (text("fmt"), text("none")),
        (text("attStmt"), Value::Map(Vec::new())),
        (text("authData"), Value::Bytes(auth_data.clone())),
    ]);
    assert_eq!(reg_err(&id, &duplicate), AttestationMalformed);
    let mut trailing = none_attestation(&auth_data);
    trailing.push(0x00);
    assert_eq!(reg_err(&id, &trailing), AttestationMalformed);
    assert_eq!(reg_err(&id, b"\xff\x00garbage"), AttestationMalformed);
    assert_eq!(reg_err(&id, &cbor(&text("none"))), AttestationMalformed);
    assert_eq!(reg_err(&id, b""), AttestationMalformed);
    let oversize = attestation_from_entries(vec![
        (text("fmt"), text("none")),
        (text("attStmt"), Value::Map(Vec::new())),
        (
            text("authData"),
            Value::Bytes({
                let mut a = auth_data.clone();
                a.extend(vec![0u8; MAX_ATTESTATION_OBJECT_BYTES]);
                a
            }),
        ),
    ]);
    assert!(oversize.len() > MAX_ATTESTATION_OBJECT_BYTES);
    assert_eq!(reg_err(&id, &oversize), AttestationMalformed);

    // Authenticator data.
    assert_eq!(
        reg_err(&id, &none_attestation(&auth_data[..37 + 16 + 1])),
        AuthDataMalformed
    );
    let other_rp = registration_auth_data(
        "other.tail1234.ts.net",
        UP | UV | AT,
        0,
        &id,
        &device.cose_key(),
    );
    assert_eq!(reg_err(&id, &none_attestation(&other_rp)), RpIdHashMismatch);
    let flagged =
        |f: u8| none_attestation(&registration_auth_data(HOST, f, 0, &id, &device.cose_key()));
    assert_eq!(reg_err(&id, &flagged(UV | AT)), UserPresenceMissing);
    assert_eq!(reg_err(&id, &flagged(UP | AT)), UserVerificationMissing);
    assert_eq!(
        reg_err(&id, &flagged(UP | UV | AT | BS)),
        BackupFlagsInvalid
    );
    assert!(matches!(
        reg_err(&id, &flagged(UP | UV)),
        AuthDataMalformed | AttestationMalformed
    ));
    assert_eq!(
        reg_err(&id, &flagged(UP | UV | AT | ED)),
        UnexpectedExtensions
    );

    // Credential id.
    let empty_id = none_attestation(&registration_auth_data(
        HOST,
        UP | UV | AT,
        0,
        &[],
        &device.cose_key(),
    ));
    assert_eq!(reg_err(&[], &empty_id), CredentialIdInvalid);
    let long_id = vec![0xcd; MAX_CREDENTIAL_ID_BYTES + 1];
    let long = none_attestation(&registration_auth_data(
        HOST,
        UP | UV | AT,
        0,
        &long_id,
        &device.cose_key(),
    ));
    assert!(matches!(
        reg_err(&long_id, &long),
        CredentialIdInvalid | AttestationMalformed
    ));
    let mut overrun = auth_data.clone();
    overrun[53] = 0x7f;
    overrun[54] = 0xff;
    assert!(matches!(
        reg_err(&id, &none_attestation(&overrun)),
        AuthDataMalformed | CredentialIdInvalid
    ));
    assert_eq!(
        reg_err(b"another-raw-id", &none_attestation(&auth_data)),
        CredentialIdMismatch
    );

    // COSE key.
    let entries = device.cose_entries();
    let with = |mutate: &dyn Fn(&mut Vec<(Value, Value)>)| {
        let mut e = entries.clone();
        mutate(&mut e);
        reg_with_cose(&device, &cbor(&Value::Map(e)))
    };
    assert_eq!(
        reg_err(&id, &with(&|e| e.push((int(4), int(1))))),
        PublicKeyMalformed,
        "extra COSE key"
    );
    assert_eq!(
        reg_err(&id, &with(&|e| e.insert(1, (int(1), int(2))))),
        PublicKeyMalformed,
        "duplicate COSE key"
    );
    assert_eq!(
        reg_err(&id, &with(&|e| e[1] = (int(3), int(-8)))),
        UnsupportedAlgorithm,
        "alg -8 (EdDSA)"
    );
    assert!(
        matches!(
            reg_err(&id, &with(&|e| e[2] = (int(-1), int(2)))),
            UnsupportedAlgorithm | PublicKeyMalformed
        ),
        "crv 2 (P-384)"
    );
    assert!(
        matches!(
            reg_err(&id, &with(&|e| e[0] = (int(1), int(1)))),
            UnsupportedAlgorithm | PublicKeyMalformed
        ),
        "kty 1 (OKP)"
    );
    assert_eq!(
        reg_err(&id, &with(&|e| e[4] = (int(-3), Value::Bool(true)))),
        PublicKeyMalformed,
        "compressed point"
    );
    assert_eq!(
        reg_err(
            &id,
            &with(&|e| e[3] = (int(-2), Value::Bytes(vec![1u8; 31])))
        ),
        PublicKeyMalformed,
        "31-byte x"
    );
    assert_eq!(
        reg_err(
            &id,
            &with(&|e| e[3] = (int(-2), Value::Bytes(vec![1u8; 33])))
        ),
        PublicKeyMalformed,
        "33-byte x"
    );
    assert_eq!(
        reg_err(
            &id,
            &with(&|e| {
                e.remove(1);
            })
        ),
        PublicKeyMalformed,
        "alg missing"
    );
    let mut off_curve_y = device.y();
    off_curve_y[31] ^= 0x01;
    assert_eq!(
        reg_err(
            &id,
            &with(&|e| e[4] = (int(-3), Value::Bytes(off_curve_y.clone())))
        ),
        PublicKeyMalformed,
        "point off the curve"
    );
    assert_eq!(
        reg_err(
            &id,
            &with(&|e| {
                e[3] = (int(-2), Value::Bytes(vec![0u8; 32]));
                e[4] = (int(-3), Value::Bytes(vec![0u8; 32]));
            })
        ),
        PublicKeyMalformed,
        "identity / zero point"
    );
    let text_keys = Value::Map(
        entries
            .iter()
            .map(|(k, v)| {
                let n: i128 = k.as_integer().unwrap().into();
                (text(&n.to_string()), v.clone())
            })
            .collect(),
    );
    assert_eq!(
        reg_err(&id, &reg_with_cose(&device, &cbor(&text_keys))),
        PublicKeyMalformed,
        "text COSE labels"
    );
    let mut trailing_cose = device.cose_key();
    trailing_cose.push(0x00);
    assert!(
        matches!(
            reg_err(&id, &reg_with_cose(&device, &trailing_cose)),
            PublicKeyMalformed | AuthDataMalformed
        ),
        "byte after the COSE key"
    );
}

/// Test 19 (RMC31): the verbatim WebAuthn L3 §16.2 vector (UV clear) is refused for missing
/// user verification, both at registration and at assertion, although its assertion
/// signature verifies. With only the UV bit patched in, the registration is accepted (the
/// UV check alone refused it).
#[test]
fn test_rmc_l3_vector_16_2_is_rejected_for_missing_uv() {
    let l3 = RelyingParty::new(l3_16_2::RP_ID);
    let credential_id = hex(l3_16_2::CREDENTIAL_ID);
    let att = hex(l3_16_2::ATTESTATION_OBJECT);
    assert_eq!(
        verify_registration(&l3, &credential_id, &att).err(),
        Some(WebAuthnError::UserVerificationMissing)
    );

    // Patch the flags byte (0x59 → 0x5d) inside the CBOR byte string.
    let mut patched = att.clone();
    let rp_hash = sha256(l3_16_2::RP_ID.as_bytes());
    let pos = patched
        .windows(32)
        .position(|w| w == rp_hash)
        .expect("rpIdHash inside the attestation object")
        + 32;
    assert_eq!(patched[pos], 0x59);
    patched[pos] |= UV;
    let new = verify_registration(&l3, &credential_id, &patched).expect("UV patched in");
    let mut sec1 = vec![0x04];
    sec1.extend(hex(l3_16_2::PUBLIC_KEY_X));
    sec1.extend(hex(l3_16_2::PUBLIC_KEY_Y));
    assert_eq!(new.public_key.to_vec(), sec1);
    assert_eq!(new.credential_id, credential_id);

    // Assertion: the signature is genuinely valid (checked with p256 directly) ...
    let auth_data = hex(l3_16_2::AUTHENTICATOR_DATA);
    let client = hex(l3_16_2::GET_CLIENT_DATA);
    let signature = hex(l3_16_2::SIGNATURE);
    let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(&sec1).unwrap();
    let mut message = auth_data.clone();
    message.extend_from_slice(&sha256(&client));
    key.verify(
        &message,
        &p256::ecdsa::Signature::from_der(&signature).unwrap(),
    )
    .expect("the official vector signature verifies");
    assert!(parse_client_data(&l3, &client, CeremonyType::Get).is_ok());
    // ... and the assertion is still refused because UV is clear (flags 0x19).
    let public_key: [u8; 65] = sec1.clone().try_into().unwrap();
    let stored = StoredCredential {
        credential_id: &credential_id,
        public_key: &public_key,
        sign_count: 0,
        backup_eligible: true,
    };
    assert_eq!(
        verify_assertion(
            &l3,
            &stored,
            &HANDLE,
            &client,
            &auth_data,
            &signature,
            Some(&HANDLE)
        )
        .err(),
        Some(WebAuthnError::UserVerificationMissing)
    );
}

// ---------------------------------------------------------------------------------------
// Tests 20–21 — assertion
// ---------------------------------------------------------------------------------------

struct Fixture {
    auth: Authenticator,
    public_key: [u8; 65],
    client: Vec<u8>,
}

impl Fixture {
    fn new(auth: Authenticator) -> Self {
        Self {
            public_key: auth.public_key(),
            auth,
            client: get_data(),
        }
    }

    fn stored(&self, sign_count: u32, backup_eligible: bool) -> StoredCredential<'_> {
        StoredCredential {
            credential_id: &self.auth.credential_id,
            public_key: &self.public_key,
            sign_count,
            backup_eligible,
        }
    }

    /// Signed assertion with `flags` and `count`; returns (authenticator data, signature).
    fn assertion(&self, flags: u8, count: u32) -> (Vec<u8>, Vec<u8>) {
        let ad = assertion_auth_data(HOST, flags, count);
        let sig = self.auth.sign(&ad, &self.client);
        (ad, sig)
    }

    fn verify(
        &self,
        stored: &StoredCredential<'_>,
        ad: &[u8],
        sig: &[u8],
        handle: Option<&[u8]>,
    ) -> Result<AssertionOutcome, WebAuthnError> {
        verify_assertion(&rp(), stored, &HANDLE, &self.client, ad, sig, handle)
    }
}

/// Test 20 (RMC31): a valid UV assertion is accepted, including the `n - s` variant of the
/// same signature (one of the two is high-S; never normalised nor refused), and a synced
/// `0/0` assertion.
#[test]
fn test_rmc_verify_assertion_accepts_a_valid_uv_assertion() {
    let f = Fixture::new(Authenticator::device(0x31, b"device-credential-0031"));
    let (ad, sig) = f.assertion(UP | UV, 1);
    assert_eq!(ad.len(), ASSERTION_AUTH_DATA_LEN);
    assert_eq!(
        f.verify(&f.stored(0, false), &ad, &sig, Some(&HANDLE)),
        Ok(AssertionOutcome {
            sign_count: 1,
            backup_state: false
        })
    );
    let complement = s_complement(&sig);
    assert_ne!(complement, sig);
    assert_eq!(
        f.verify(&f.stored(0, false), &ad, &complement, Some(&HANDLE)),
        Ok(AssertionOutcome {
            sign_count: 1,
            backup_state: false
        }),
        "both s and n - s verify (high-S accepted)"
    );

    let synced = Fixture::new(Authenticator::owner());
    let (ad, sig) = synced.assertion(UP | UV | BE | BS, 0);
    assert_eq!(
        synced.verify(&synced.stored(0, true), &ad, &sig, Some(&HANDLE)),
        Ok(AssertionOutcome {
            sign_count: 0,
            backup_state: true
        })
    );
    let (ad, sig) = synced.assertion(UP | UV | BE, 0);
    assert_eq!(
        synced.verify(&synced.stored(0, true), &ad, &sig, Some(&HANDLE)),
        Ok(AssertionOutcome {
            sign_count: 0,
            backup_state: false
        }),
        "BS may change while BE stays"
    );
    assert_eq!(flags::UV, UV);
    assert_eq!(flags::UP, UP);
    assert_eq!(flags::BE, BE);
    assert_eq!(flags::BS, BS);
    assert_eq!(flags::AT, AT);
    assert_eq!(flags::ED, ED);
}

/// Test 21 (RMC31): every assertion rejection, each against an otherwise valid and
/// correctly signed fixture; the counter rule (`5→5`, `5→3`, `5→0` refused; `0/0` and `0→7`
/// accepted).
#[test]
fn test_rmc_verify_assertion_rejections() {
    use WebAuthnError::*;
    let f = Fixture::new(Authenticator::device(0x32, b"device-credential-0032"));
    let stored = f.stored(0, false);
    let (ad, sig) = f.assertion(UP | UV, 1);
    assert!(f.verify(&stored, &ad, &sig, Some(&HANDLE)).is_ok());

    // User handle.
    assert_eq!(f.verify(&stored, &ad, &sig, None), Err(UserHandleMismatch));
    assert_eq!(
        f.verify(&stored, &ad, &sig, Some(&[0x5b; USER_HANDLE_BYTES])),
        Err(UserHandleMismatch)
    );
    assert_eq!(
        f.verify(&stored, &ad, &sig, Some(&HANDLE[..15])),
        Err(UserHandleMismatch)
    );
    let mut long = HANDLE.to_vec();
    long.push(0x5a);
    assert_eq!(
        f.verify(&stored, &ad, &sig, Some(&long)),
        Err(UserHandleMismatch)
    );

    // Authenticator data length (signed, so only the length is wrong).
    for len in [ASSERTION_AUTH_DATA_LEN - 1, ASSERTION_AUTH_DATA_LEN + 1] {
        let mut bad = assertion_auth_data(HOST, UP | UV, 1);
        bad.resize(len, 0);
        let sig = f.auth.sign(&bad, &f.client);
        assert_eq!(
            f.verify(&stored, &bad, &sig, Some(&HANDLE)),
            Err(AuthDataMalformed),
            "{len}"
        );
    }
    let other = assertion_auth_data("other.tail1234.ts.net", UP | UV, 1);
    let other_sig = f.auth.sign(&other, &f.client);
    assert_eq!(
        f.verify(&stored, &other, &other_sig, Some(&HANDLE)),
        Err(RpIdHashMismatch)
    );

    // Flags.
    let check = |flags: u8, stored: &StoredCredential<'_>| {
        let (ad, sig) = f.assertion(flags, 1);
        f.verify(stored, &ad, &sig, Some(&HANDLE))
    };
    assert_eq!(check(UV, &stored), Err(UserPresenceMissing));
    assert_eq!(check(UP, &stored), Err(UserVerificationMissing));
    assert_eq!(
        check(UP | BE | BS, &f.stored(0, true)),
        Err(UserVerificationMissing)
    );
    assert!(matches!(
        check(UP | UV | AT, &stored),
        Err(AuthDataMalformed | UnexpectedExtensions)
    ));
    assert_eq!(check(UP | UV | ED, &stored), Err(UnexpectedExtensions));
    assert_eq!(check(UP | UV | BS, &stored), Err(BackupFlagsInvalid));
    assert_eq!(
        check(UP | UV | BE | BS, &stored),
        Err(BackupEligibilityChanged),
        "BE appeared"
    );
    assert_eq!(
        check(UP | UV, &f.stored(0, true)),
        Err(BackupEligibilityChanged),
        "BE disappeared"
    );

    // Signature.
    let mut oversize = sig.clone();
    oversize.resize(MAX_SIGNATURE_BYTES + 1, 0);
    assert_eq!(
        f.verify(&stored, &ad, &oversize, Some(&HANDLE)),
        Err(SignatureMalformed)
    );
    assert_eq!(
        f.verify(&stored, &ad, b"not a DER signature", Some(&HANDLE)),
        Err(SignatureMalformed)
    );
    assert_eq!(
        f.verify(&stored, &ad, &[], Some(&HANDLE)),
        Err(SignatureMalformed)
    );
    let wrong_key = Authenticator::device(0x33, b"x").sign(&ad, &f.client);
    assert_eq!(
        f.verify(&stored, &ad, &wrong_key, Some(&HANDLE)),
        Err(SignatureInvalid)
    );
    // Signed over a re-serialised client data (same members, different bytes).
    let reserialised = format!(
        "{{ \"type\": \"webauthn.get\", \"challenge\": \"{}\", \"origin\": \"{ORIGIN}\", \"crossOrigin\": false }}",
        b64url(&challenge())
    );
    let sig_other_bytes = f.auth.sign(&ad, reserialised.as_bytes());
    assert_eq!(
        f.verify(&stored, &ad, &sig_other_bytes, Some(&HANDLE)),
        Err(SignatureInvalid)
    );
    // The authenticator data is covered by the signature.
    let mut tampered = ad.clone();
    tampered[33..37].copy_from_slice(&2u32.to_be_bytes());
    assert_eq!(
        f.verify(&stored, &tampered, &sig, Some(&HANDLE)),
        Err(SignatureInvalid),
        "the counter is covered by the signature"
    );

    // Counter rule.
    let counted = |stored_count: u32, received: u32| {
        let (ad, sig) = f.assertion(UP | UV, received);
        f.verify(&f.stored(stored_count, false), &ad, &sig, Some(&HANDLE))
    };
    assert_eq!(counted(5, 5), Err(SignCountRegression));
    assert_eq!(counted(5, 3), Err(SignCountRegression));
    assert_eq!(counted(5, 0), Err(SignCountRegression));
    assert_eq!(
        counted(0, 0),
        Ok(AssertionOutcome {
            sign_count: 0,
            backup_state: false
        })
    );
    assert_eq!(
        counted(0, 7),
        Ok(AssertionOutcome {
            sign_count: 7,
            backup_state: false
        })
    );
    assert_eq!(
        counted(5, 6),
        Ok(AssertionOutcome {
            sign_count: 6,
            backup_state: false
        })
    );
    assert_eq!(
        counted(u32::MAX - 1, u32::MAX),
        Ok(AssertionOutcome {
            sign_count: u32::MAX,
            backup_state: false
        })
    );
    assert_eq!(counted(u32::MAX, u32::MAX), Err(SignCountRegression));
}

// ---------------------------------------------------------------------------------------
// Test 22 — privacy of errors; base64url helpers
// ---------------------------------------------------------------------------------------

/// Test 22 (RMC38, D-H): every variant's text is a fixed English reason class; no input
/// value can appear in it.
#[test]
fn test_rmc_webauthn_errors_never_echo_values() {
    use WebAuthnError::*;
    let table = [
        (ClientDataMalformed, "client data malformed"),
        (WrongType, "wrong ceremony type"),
        (OriginMismatch, "origin mismatch"),
        (CrossOrigin, "cross-origin ceremony"),
        (AttestationMalformed, "attestation object malformed"),
        (UnsupportedAttestation, "unsupported attestation format"),
        (AuthDataMalformed, "authenticator data malformed"),
        (RpIdHashMismatch, "rp id hash mismatch"),
        (UserPresenceMissing, "user presence missing"),
        (UserVerificationMissing, "user verification missing"),
        (BackupFlagsInvalid, "backup flags invalid"),
        (UnexpectedExtensions, "unexpected extensions"),
        (CredentialIdInvalid, "credential id invalid"),
        (CredentialIdMismatch, "credential id mismatch"),
        (PublicKeyMalformed, "public key malformed"),
        (UnsupportedAlgorithm, "unsupported algorithm"),
        (SignatureMalformed, "signature malformed"),
        (SignatureInvalid, "signature invalid"),
        (UserHandleMismatch, "user handle mismatch"),
        (SignCountRegression, "sign count regression"),
        (BackupEligibilityChanged, "backup eligibility changed"),
    ];
    for (err, text) in table {
        assert_eq!(err.to_string(), text);
        assert!(
            !text.bytes().any(|b| b.is_ascii_digit()),
            "{text} carries no value"
        );
        assert!(!format!("{err:?}").contains(HOST));
    }
    // A refused input never comes back inside the error (Copy enum, no payload).
    let secret_origin = "https://secret-value-1234.example";
    let data = client_data("webauthn.get", &b64url(&challenge()), secret_origin, "");
    let err = parse_client_data(&rp(), &data, CeremonyType::Get)
        .err()
        .expect("refused");
    assert!(!format!("{err} {err:?}").contains("secret-value"));
}

/// Spec §4.1 helpers: base64url without padding; padding, the standard alphabet and an
/// oversize input are refused (the length is checked before decoding).
#[test]
fn test_rmc_b64url_helpers_are_strict() {
    for len in 0..70usize {
        let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
        let encoded = b64url_encode(&bytes);
        assert_eq!(
            encoded,
            b64url(&bytes),
            "encoder matches RFC 4648 §5 for {len}"
        );
        assert_eq!(b64url_decode(&encoded, 70), Ok(bytes.clone()));
    }
    assert_eq!(b64url_decode("AQID", 3), Ok(vec![1, 2, 3]));
    assert_eq!(
        b64url_decode("AQID", 2),
        Err(()),
        "decoded length over the bound"
    );
    assert_eq!(b64url_decode("AQ==", 4), Err(()), "padding refused");
    assert_eq!(
        b64url_decode("+/8", 4),
        Err(()),
        "standard alphabet refused"
    );
    assert_eq!(b64url_decode("AQ\n", 4), Err(()));
    assert_eq!(b64url_decode("A", 4), Err(()), "impossible length");
    let huge = "A".repeat(10_000);
    assert_eq!(b64url_decode(&huge, 32), Err(()));
}

/// Fixture guard (not a product contract): the deterministic fixtures are what they claim
/// to be, checked with `p256` and `ciborium` directly, so a red result above can only come
/// from the code under test.
#[test]
fn test_rmc_passkey_fixtures_are_self_consistent() {
    let device = Authenticator::device(0x41, b"fixture-credential");
    let ad = assertion_auth_data(HOST, UP | UV, 9);
    assert_eq!(ad.len(), 37);
    assert_eq!(ad[..32], sha256(HOST.as_bytes()));
    assert_eq!(ad[32], UP | UV);
    let client = get_data();
    let sig = device.sign(&ad, &client);
    let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(&device.public_key()).unwrap();
    let mut message = ad.clone();
    message.extend_from_slice(&sha256(&client));
    key.verify(&message, &p256::ecdsa::Signature::from_der(&sig).unwrap())
        .expect("fixture signature verifies");
    key.verify(
        &message,
        &p256::ecdsa::Signature::from_der(&s_complement(&sig)).unwrap(),
    )
    .expect("complement verifies");
    assert!(sig.len() <= MAX_SIGNATURE_BYTES);

    let att = registration(&device, HOST, UP | UV | AT);
    let value: Value = ciborium::de::from_reader(att.as_slice()).unwrap();
    let map = value.as_map().unwrap();
    assert_eq!(map.len(), 3);
    let auth_data = map
        .iter()
        .find(|(k, _)| k.as_text() == Some("authData"))
        .unwrap()
        .1
        .as_bytes()
        .unwrap()
        .clone();
    let id_len = u16::from_be_bytes([auth_data[53], auth_data[54]]) as usize;
    assert_eq!(&auth_data[55..55 + id_len], device.credential_id.as_slice());
    let cose: Value = ciborium::de::from_reader(&auth_data[55 + id_len..]).unwrap();
    assert_eq!(cose.as_map().unwrap().len(), 5);

    let parsed: serde_json::Value = serde_json::from_slice(&client).unwrap();
    assert_eq!(parsed["type"], "webauthn.get");
    assert_eq!(
        passkey::b64url_decode(parsed["challenge"].as_str().unwrap()),
        challenge().to_vec()
    );
    for len in 0..40usize {
        let bytes: Vec<u8> = (0..len as u8).collect();
        assert_eq!(passkey::b64url_decode(&b64url(&bytes)), bytes);
    }
    assert_eq!(b64url(b"\xfb\xff"), "-_8");
}
