//! Pure WebAuthn Level 3 §7.1 / §7.2 verification subset (ES256, attestation `none`;
//! architect spec §4.1, ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey
//! Authentication for `soos-remote`").
//!
//! No I/O, no clock, no logging. Every input is bounded before it is decoded: the
//! `clientDataJSON` by `MAX_CLIENT_DATA_JSON_BYTES`, the attestation object by
//! `MAX_ATTESTATION_OBJECT_BYTES`, every CBOR parse by a recursion limit, the signature by
//! `MAX_SIGNATURE_BYTES`. User verification is checked unconditionally on both ceremonies.
//! Error texts are fixed reason classes that never carry a value.

use base64ct::{Base64UrlUnpadded, Encoding};
use ciborium::value::Value;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{
    ASSERTION_AUTH_DATA_LEN, CHALLENGE_BYTES, COSE_ALG_ES256, MAX_ATTESTATION_OBJECT_BYTES,
    MAX_CLIENT_DATA_JSON_BYTES, MAX_CREDENTIAL_ID_BYTES, MAX_SIGNATURE_BYTES, USER_HANDLE_BYTES,
};

/// Nesting limit of every CBOR parse (attestation object and COSE key).
const CBOR_RECURSION_LIMIT: usize = 16;
/// `rpIdHash` (32) + flags (1) + signCount (4).
const AUTH_DATA_HEADER_LEN: usize = 37;
/// AAGUID length inside attested credential data.
const AAGUID_LEN: usize = 16;
/// Credential id length field.
const CREDENTIAL_ID_LEN_FIELD: usize = 2;

/// SHA-256 of `bytes`.
fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Relying-party constants derived once from `rp_id`.
#[derive(Clone)]
pub struct RelyingParty {
    rp_id: String,
    origin: String,
    rp_id_hash: [u8; 32],
}

impl RelyingParty {
    /// `rp_id` is the validated configuration value; the origin is exactly `https://` +
    /// `rp_id` (no port, spec S-1).
    #[must_use]
    pub fn new(rp_id: &str) -> Self {
        Self {
            rp_id: rp_id.to_string(),
            origin: format!("https://{rp_id}"),
            rp_id_hash: sha256(rp_id.as_bytes()),
        }
    }

    /// The RP ID.
    #[must_use]
    pub fn rp_id(&self) -> &str {
        &self.rp_id
    }

    /// Exactly `https://` + RP ID.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }
}

/// Expected ceremony type of `clientDataJSON.type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeremonyType {
    /// `"webauthn.create"`.
    Create,
    /// `"webauthn.get"`.
    Get,
}

impl CeremonyType {
    fn wire(self) -> &'static str {
        match self {
            Self::Create => "webauthn.create",
            Self::Get => "webauthn.get",
        }
    }
}

/// Validated `clientDataJSON` (no `Debug`: the challenge is a secret of the ceremony).
pub struct ClientData {
    /// Decoded challenge.
    pub challenge: [u8; CHALLENGE_BYTES],
}

/// The members of `clientDataJSON` the verification reads. Unknown members are ignored
/// (L3 §5.8.1); a duplicate known member is refused by the derived deserializer.
#[derive(Deserialize)]
struct RawClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
    #[serde(rename = "crossOrigin")]
    cross_origin: Option<bool>,
    #[serde(rename = "topOrigin")]
    top_origin: Option<serde::de::IgnoredAny>,
}

/// Pure. `raw` ≤ `MAX_CLIENT_DATA_JSON_BYTES`, a UTF-8 JSON object with string `type`,
/// `challenge` and `origin`; `type` must equal `expected`; `origin` must equal
/// `rp.origin()` byte for byte; `crossOrigin` absent or `false`; `topOrigin` absent; the
/// challenge is base64url without padding decoding to exactly `CHALLENGE_BYTES`.
///
/// # Errors
///
/// [`WebAuthnError::ClientDataMalformed`], [`WebAuthnError::WrongType`],
/// [`WebAuthnError::OriginMismatch`], [`WebAuthnError::CrossOrigin`].
pub fn parse_client_data(
    rp: &RelyingParty,
    raw: &[u8],
    expected: CeremonyType,
) -> Result<ClientData, WebAuthnError> {
    if raw.is_empty() || raw.len() > MAX_CLIENT_DATA_JSON_BYTES {
        return Err(WebAuthnError::ClientDataMalformed);
    }
    let parsed: RawClientData =
        serde_json::from_slice(raw).map_err(|_| WebAuthnError::ClientDataMalformed)?;
    if parsed.kind != expected.wire() {
        return Err(WebAuthnError::WrongType);
    }
    if parsed.origin.as_bytes() != rp.origin.as_bytes() {
        return Err(WebAuthnError::OriginMismatch);
    }
    if parsed.cross_origin == Some(true) || parsed.top_origin.is_some() {
        return Err(WebAuthnError::CrossOrigin);
    }
    let decoded = b64url_decode(&parsed.challenge, CHALLENGE_BYTES)
        .map_err(|()| WebAuthnError::ClientDataMalformed)?;
    let challenge: [u8; CHALLENGE_BYTES] = decoded
        .as_slice()
        .try_into()
        .map_err(|_| WebAuthnError::ClientDataMalformed)?;
    Ok(ClientData { challenge })
}

/// Flags byte (L3 §6.1).
pub mod flags {
    /// User present.
    pub const UP: u8 = 0x01;
    /// User verified.
    pub const UV: u8 = 0x04;
    /// Backup eligible.
    pub const BE: u8 = 0x08;
    /// Backup state.
    pub const BS: u8 = 0x10;
    /// Attested credential data included.
    pub const AT: u8 = 0x40;
    /// Extension data included.
    pub const ED: u8 = 0x80;
}

/// A verified new credential (no `Debug`: it carries the credential id and key).
pub struct NewCredential {
    /// Credential id.
    pub credential_id: Vec<u8>,
    /// SEC1 uncompressed point.
    pub public_key: [u8; 65],
    /// Signature counter.
    pub sign_count: u32,
    /// BE flag.
    pub backup_eligible: bool,
    /// BS flag.
    pub backup_state: bool,
}

/// Parses one CBOR item from the front of `bytes` under the recursion limit; returns the
/// item and the bytes left after it.
fn cbor_item(bytes: &[u8]) -> Option<(Value, &[u8])> {
    let mut cursor = bytes;
    let value: Value =
        ciborium::de::from_reader_with_recursion_limit(&mut cursor, CBOR_RECURSION_LIMIT).ok()?;
    Some((value, cursor))
}

/// The value of a text key that occurs exactly once in `map`.
fn text_entry<'a>(map: &'a [(Value, Value)], key: &str) -> Option<&'a Value> {
    let mut found = None;
    for (k, v) in map {
        if k.as_text() == Some(key) {
            if found.is_some() {
                return None;
            }
            found = Some(v);
        }
    }
    found
}

/// Big-endian `u32` at `offset` of `bytes`.
fn be_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let raw: [u8; 4] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u32::from_be_bytes(raw))
}

/// Flag checks shared by both ceremonies, in the spec order: `rpIdHash`, UP, UV.
fn check_rp_and_user(rp: &RelyingParty, auth_data: &[u8]) -> Result<u8, WebAuthnError> {
    let hash = auth_data
        .get(..32)
        .ok_or(WebAuthnError::AuthDataMalformed)?;
    if !bool::from(hash.ct_eq(&rp.rp_id_hash)) {
        return Err(WebAuthnError::RpIdHashMismatch);
    }
    let flags = *auth_data.get(32).ok_or(WebAuthnError::AuthDataMalformed)?;
    if flags & flags::UP == 0 {
        return Err(WebAuthnError::UserPresenceMissing);
    }
    if flags & flags::UV == 0 {
        return Err(WebAuthnError::UserVerificationMissing);
    }
    Ok(flags)
}

/// Pure; L3 §7.1 subset. The client data is checked by the caller with
/// [`parse_client_data`] (and its challenge consumed) before this call. Checks, in order:
/// attestation object bound; CBOR map with exactly `fmt`, `attStmt`, `authData` and no
/// trailing byte; `fmt == "none"` with an empty `attStmt`; authData length; `rpIdHash`; UP;
/// UV; BE clear ⇒ BS clear; AT set; ED clear; credential id 1..=`MAX_CREDENTIAL_ID_BYTES`
/// within bounds; a strict ES256 COSE key with no byte after it, on the curve; `raw_id`
/// equal to the credential id.
///
/// # Errors
///
/// Every registration [`WebAuthnError`] variant.
pub fn verify_registration(
    rp: &RelyingParty,
    raw_id: &[u8],
    attestation_object: &[u8],
) -> Result<NewCredential, WebAuthnError> {
    if attestation_object.is_empty() || attestation_object.len() > MAX_ATTESTATION_OBJECT_BYTES {
        return Err(WebAuthnError::AttestationMalformed);
    }
    let (top, rest) = cbor_item(attestation_object).ok_or(WebAuthnError::AttestationMalformed)?;
    if !rest.is_empty() {
        return Err(WebAuthnError::AttestationMalformed);
    }
    let map = top.as_map().ok_or(WebAuthnError::AttestationMalformed)?;
    if map.len() != 3 {
        return Err(WebAuthnError::AttestationMalformed);
    }
    let fmt = text_entry(map, "fmt").ok_or(WebAuthnError::AttestationMalformed)?;
    let att_stmt = text_entry(map, "attStmt").ok_or(WebAuthnError::AttestationMalformed)?;
    let auth_data = text_entry(map, "authData")
        .and_then(Value::as_bytes)
        .ok_or(WebAuthnError::AttestationMalformed)?;
    if fmt.as_text().ok_or(WebAuthnError::AttestationMalformed)? != "none" {
        return Err(WebAuthnError::UnsupportedAttestation);
    }
    match att_stmt.as_map() {
        Some(entries) if entries.is_empty() => {}
        _ => return Err(WebAuthnError::UnsupportedAttestation),
    }

    let fixed = AUTH_DATA_HEADER_LEN
        .saturating_add(AAGUID_LEN)
        .saturating_add(CREDENTIAL_ID_LEN_FIELD);
    if auth_data.len() < fixed {
        return Err(WebAuthnError::AuthDataMalformed);
    }
    let flags = check_rp_and_user(rp, auth_data)?;
    let backup_eligible = flags & flags::BE != 0;
    let backup_state = flags & flags::BS != 0;
    if backup_state && !backup_eligible {
        return Err(WebAuthnError::BackupFlagsInvalid);
    }
    if flags & flags::AT == 0 {
        return Err(WebAuthnError::AuthDataMalformed);
    }
    if flags & flags::ED != 0 {
        return Err(WebAuthnError::UnexpectedExtensions);
    }
    let sign_count = be_u32(auth_data, 33).ok_or(WebAuthnError::AuthDataMalformed)?;
    let len_offset = AUTH_DATA_HEADER_LEN.saturating_add(AAGUID_LEN);
    let len_bytes: [u8; 2] = auth_data
        .get(len_offset..fixed)
        .and_then(|b| b.try_into().ok())
        .ok_or(WebAuthnError::AuthDataMalformed)?;
    let id_len = usize::from(u16::from_be_bytes(len_bytes));
    if id_len == 0 || id_len > MAX_CREDENTIAL_ID_BYTES {
        return Err(WebAuthnError::CredentialIdInvalid);
    }
    let id_end = fixed
        .checked_add(id_len)
        .ok_or(WebAuthnError::AuthDataMalformed)?;
    let credential_id = auth_data
        .get(fixed..id_end)
        .ok_or(WebAuthnError::AuthDataMalformed)?;
    let cose = auth_data
        .get(id_end..)
        .ok_or(WebAuthnError::AuthDataMalformed)?;
    let public_key = parse_cose_key(cose)?;
    if raw_id.len() != credential_id.len() || !bool::from(raw_id.ct_eq(credential_id)) {
        return Err(WebAuthnError::CredentialIdMismatch);
    }
    Ok(NewCredential {
        credential_id: credential_id.to_vec(),
        public_key,
        sign_count,
        backup_eligible,
        backup_state,
    })
}

/// Strict ES256 COSE key: a CBOR map with exactly the five integer labels
/// `{1: 2, 3: -7, -1: 1, -2: x(32), -3: y(32)}`, nothing after it, a point on the curve.
fn parse_cose_key(cose: &[u8]) -> Result<[u8; 65], WebAuthnError> {
    let (value, rest) = cbor_item(cose).ok_or(WebAuthnError::PublicKeyMalformed)?;
    if !rest.is_empty() {
        return Err(WebAuthnError::PublicKeyMalformed);
    }
    let map = value.as_map().ok_or(WebAuthnError::PublicKeyMalformed)?;
    if map.len() != 5 {
        return Err(WebAuthnError::PublicKeyMalformed);
    }
    let mut slots: [Option<&Value>; 5] = [None; 5];
    for (key, entry) in map {
        let label = key
            .as_integer()
            .map(i128::from)
            .ok_or(WebAuthnError::PublicKeyMalformed)?;
        let index = match label {
            1 => 0,
            3 => 1,
            -1 => 2,
            -2 => 3,
            -3 => 4,
            _ => return Err(WebAuthnError::PublicKeyMalformed),
        };
        let slot = slots
            .get_mut(index)
            .ok_or(WebAuthnError::PublicKeyMalformed)?;
        if slot.is_some() {
            return Err(WebAuthnError::PublicKeyMalformed);
        }
        *slot = Some(entry);
    }
    let [Some(kty), Some(alg), Some(crv), Some(x), Some(y)] = slots else {
        return Err(WebAuthnError::PublicKeyMalformed);
    };
    let int = |v: &Value| v.as_integer().map(i128::from);
    if int(kty) != Some(2) {
        return Err(WebAuthnError::UnsupportedAlgorithm);
    }
    if int(alg) != Some(i128::from(COSE_ALG_ES256)) {
        return Err(WebAuthnError::UnsupportedAlgorithm);
    }
    if int(crv) != Some(1) {
        return Err(WebAuthnError::UnsupportedAlgorithm);
    }
    let coordinate = |v: &Value| -> Result<[u8; 32], WebAuthnError> {
        v.as_bytes()
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or(WebAuthnError::PublicKeyMalformed)
    };
    let x = coordinate(x)?;
    let y = coordinate(y)?;
    let mut point = [0u8; 65];
    let (tag, coordinates) = point.split_at_mut(1);
    let (px, py) = coordinates.split_at_mut(32);
    tag.copy_from_slice(&[0x04]);
    px.copy_from_slice(&x);
    py.copy_from_slice(&y);
    VerifyingKey::from_sec1_bytes(&point).map_err(|_| WebAuthnError::PublicKeyMalformed)?;
    Ok(point)
}

/// The stored record view the assertion check needs.
pub struct StoredCredential<'a> {
    /// Credential id.
    pub credential_id: &'a [u8],
    /// SEC1 uncompressed point.
    pub public_key: &'a [u8; 65],
    /// Stored counter.
    pub sign_count: u32,
    /// Stored BE flag.
    pub backup_eligible: bool,
}

/// Successful assertion: values to persist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssertionOutcome {
    /// Received counter.
    pub sign_count: u32,
    /// Received BS flag.
    pub backup_state: bool,
}

/// Pure; L3 §7.2 subset. `client_data_raw` is the exact received `clientDataJSON`, already
/// validated by [`parse_client_data`] with [`CeremonyType::Get`] and its challenge consumed by
/// the caller. Checks, in order: `user_handle` present and equal to `stored_user_handle`
/// (constant time); authData length exactly `ASSERTION_AUTH_DATA_LEN`; `rpIdHash`; UP; UV;
/// AT clear; ED clear; BE clear ⇒ BS clear; BE equal to the stored value; a DER signature of
/// at most `MAX_SIGNATURE_BYTES` verified with ES256 over `authenticator_data ||
/// SHA-256(client_data_raw)` (high-S accepted, never normalised); the counter rule (when
/// either counter is non-zero the received one must be strictly greater).
///
/// # Errors
///
/// Every assertion [`WebAuthnError`] variant.
pub fn verify_assertion(
    rp: &RelyingParty,
    credential: &StoredCredential<'_>,
    stored_user_handle: &[u8; USER_HANDLE_BYTES],
    client_data_raw: &[u8],
    authenticator_data: &[u8],
    signature: &[u8],
    user_handle: Option<&[u8]>,
) -> Result<AssertionOutcome, WebAuthnError> {
    let handle = user_handle.ok_or(WebAuthnError::UserHandleMismatch)?;
    if handle.len() != USER_HANDLE_BYTES || !bool::from(handle.ct_eq(stored_user_handle)) {
        return Err(WebAuthnError::UserHandleMismatch);
    }
    if authenticator_data.len() != ASSERTION_AUTH_DATA_LEN {
        return Err(WebAuthnError::AuthDataMalformed);
    }
    let flags = check_rp_and_user(rp, authenticator_data)?;
    if flags & flags::AT != 0 {
        return Err(WebAuthnError::AuthDataMalformed);
    }
    if flags & flags::ED != 0 {
        return Err(WebAuthnError::UnexpectedExtensions);
    }
    let backup_eligible = flags & flags::BE != 0;
    let backup_state = flags & flags::BS != 0;
    if backup_state && !backup_eligible {
        return Err(WebAuthnError::BackupFlagsInvalid);
    }
    if backup_eligible != credential.backup_eligible {
        return Err(WebAuthnError::BackupEligibilityChanged);
    }
    if signature.is_empty() || signature.len() > MAX_SIGNATURE_BYTES {
        return Err(WebAuthnError::SignatureMalformed);
    }
    let parsed = Signature::from_der(signature).map_err(|_| WebAuthnError::SignatureMalformed)?;
    let key = VerifyingKey::from_sec1_bytes(credential.public_key)
        .map_err(|_| WebAuthnError::PublicKeyMalformed)?;
    let mut message = Vec::with_capacity(ASSERTION_AUTH_DATA_LEN.saturating_add(32));
    message.extend_from_slice(authenticator_data);
    message.extend_from_slice(&sha256(client_data_raw));
    key.verify(&message, &parsed)
        .map_err(|_| WebAuthnError::SignatureInvalid)?;
    let received = be_u32(authenticator_data, 33).ok_or(WebAuthnError::AuthDataMalformed)?;
    if (received != 0 || credential.sign_count != 0) && received <= credential.sign_count {
        return Err(WebAuthnError::SignCountRegression);
    }
    Ok(AssertionOutcome {
        sign_count: received,
        backup_state,
    })
}

/// Verification failure; fixed English reason classes, never a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WebAuthnError {
    /// `clientDataJSON` is not the expected bounded JSON object.
    #[error("client data malformed")]
    ClientDataMalformed,
    /// `clientDataJSON.type` is not the expected ceremony.
    #[error("wrong ceremony type")]
    WrongType,
    /// `clientDataJSON.origin` is not `https://<rp_id>`.
    #[error("origin mismatch")]
    OriginMismatch,
    /// `crossOrigin: true` or a `topOrigin`.
    #[error("cross-origin ceremony")]
    CrossOrigin,
    /// The attestation object is not the expected CBOR map.
    #[error("attestation object malformed")]
    AttestationMalformed,
    /// Attestation other than `none`.
    #[error("unsupported attestation format")]
    UnsupportedAttestation,
    /// Authenticator data length or structure.
    #[error("authenticator data malformed")]
    AuthDataMalformed,
    /// `rpIdHash` is not SHA-256 of the RP ID.
    #[error("rp id hash mismatch")]
    RpIdHashMismatch,
    /// UP clear.
    #[error("user presence missing")]
    UserPresenceMissing,
    /// UV clear.
    #[error("user verification missing")]
    UserVerificationMissing,
    /// BS set without BE.
    #[error("backup flags invalid")]
    BackupFlagsInvalid,
    /// ED set.
    #[error("unexpected extensions")]
    UnexpectedExtensions,
    /// Credential id length out of bounds.
    #[error("credential id invalid")]
    CredentialIdInvalid,
    /// The credential id differs from the response id.
    #[error("credential id mismatch")]
    CredentialIdMismatch,
    /// COSE key structure or point.
    #[error("public key malformed")]
    PublicKeyMalformed,
    /// Not ES256 on P-256.
    #[error("unsupported algorithm")]
    UnsupportedAlgorithm,
    /// Signature size or DER.
    #[error("signature malformed")]
    SignatureMalformed,
    /// Signature does not verify.
    #[error("signature invalid")]
    SignatureInvalid,
    /// `userHandle` absent or not the stored one.
    #[error("user handle mismatch")]
    UserHandleMismatch,
    /// Non-increasing counter.
    #[error("sign count regression")]
    SignCountRegression,
    /// BE differs from the stored value.
    #[error("backup eligibility changed")]
    BackupEligibilityChanged,
}

/// base64url without padding, bounded: the encoded length is checked against
/// `max_decoded * 4 / 3 + 4` before decoding, then the decoded length against
/// `max_decoded`; padding and bytes outside the URL-safe alphabet are refused.
///
/// # Errors
///
/// `()` on any malformed or oversize input.
#[allow(
    clippy::result_unit_err,
    reason = "Signature fixed by the architect spec §4.1"
)]
pub fn b64url_decode(input: &str, max_decoded: usize) -> Result<Vec<u8>, ()> {
    let max_encoded = max_decoded
        .saturating_mul(4)
        .checked_div(3)
        .unwrap_or(0)
        .saturating_add(4);
    if input.len() > max_encoded {
        return Err(());
    }
    let decoded = Base64UrlUnpadded::decode_vec(input).map_err(|_| ())?;
    if decoded.len() > max_decoded {
        return Err(());
    }
    Ok(decoded)
}

/// base64url without padding.
#[must_use]
pub fn b64url_encode(bytes: &[u8]) -> String {
    Base64UrlUnpadded::encode_string(bytes)
}
